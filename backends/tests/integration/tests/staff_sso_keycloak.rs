//! Staff single sign-on against a **real Keycloak**
//! ([ADR-0030](../../../../docs/adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
//!
//! `staff_sso.rs` simulates the provider over HTTP and proves what vpay does
//! with a token. This file proves the other half, which no simulation can: that
//! the tokens a **real** provider issues are the tokens vpay's rules were
//! written for. It boots `quay.io/keycloak/keycloak:26.6.1` with
//! `tests/fixtures/keycloak-realm.json` (a client, a groups mapper, an `amr`
//! mapper, three users), drives the browser's half of the flow by hand — the
//! login form, the OTP form, the redirect — and hands the resulting `code` to
//! vpay's real callback, over a real Postgres, exactly as the dashboard does.
//!
//! # What the realm does, and why it is not Keycloak's default
//!
//! Keycloak's default browser flow issues `acr "1"` and an **empty** `amr`
//! after a password and an OTP — measured 2026-10-19 — so a rule written
//! against either would refuse every real sign-in. `amr` is only populated
//! once the flow's executions carry a `default.reference.value`, which the
//! realm's `browser-amr` flow sets (`pwd` on the password form, `otp` on the
//! OTP form). That is a **realm setting an operator must make**, and
//! `docs/flows/dashboard-auth/sso.md` says so.
//!
//! # The three measured behaviours this pins
//!
//! 1. password + OTP at the provider gives `amr ["pwd","otp"]` and vpay signs
//!    the person in under `accepted_amr: ["otp"]`;
//! 2. a person whose OTP is **enrolled during the sign-in** (Keycloak's
//!    `CONFIGURE_TOTP` required action) gets a token whose `amr` is `["pwd"]`
//!    only — so vpay's one `prompt=login` retry fires, Keycloak re-runs its
//!    forms, and the second token carries `otp`;
//! 3. a repeat sign-in completed from Keycloak's own SSO cookie still carries
//!    `amr ["pwd","otp"]` (with `acr "0"`), so it is accepted without a retry
//!    — **but only because the realm sets `default.reference.maxAge`** on the
//!    two executions. Without it Keycloak drops `amr` from an SSO-cookie token
//!    after about a second, and every repeat sign-in would be sent round the
//!    provider's forms by vpay's retry.
//!
//! Slow (one Keycloak boot, about fifteen seconds) and heavy (a JVM), so it is
//! **one test**, one container, run in sequence.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sqlx::Row as _;
use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::runners::AsyncRunner as _;
use testcontainers::{GenericImage, ImageExt as _};
use time::OffsetDateTime;
use vpay_api::staff_auth::totp::{Totp, step_at};
use vpay_config::{
    Config, CurrencyEntry, DashboardClient, Deployment, HostEntry, ProviderHost, RateLimits,
    StaffAuth, StaffSso,
};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

/// The realm import, with `__NAME__` placeholders where a secret goes. **No
/// password, client secret or admin credential is written down anywhere in this
/// repository**: [`Secrets::generate`] draws each from the OS CSPRNG per run
/// and [`Secrets::realm`] substitutes them, so a leaked fixture opens nothing.
const REALM_TEMPLATE: &str = include_str!("fixtures/keycloak-realm.json");
/// The TOTP key `ada`'s and `mallory`'s OTP credentials were imported with.
/// Keycloak uses the **raw bytes** of the imported value as the HMAC key (not
/// its base32 decoding), which is why this is a byte string and not a seed.
const IMPORTED_TOTP_KEY: &[u8] = b"12345678901234567890";

const SSO_CLIENT_ID: &str = "vpay-staff";
const SSO_REDIRECT_URI: &str = "http://127.0.0.1:3000/api/auth/sso/callback";

const MERCHANT_A: &str = "acme-cameroon-tenant";

// ------------------------------------------------------------- the browser --

/// A browser's cookie jar and nothing else: the flow needs the cookies
/// Keycloak sets (it marks them `Secure` even over http, so a scripted client
/// must send them regardless) and manual redirect handling.
#[derive(Default)]
struct Browser {
    cookies: BTreeMap<String, String>,
}

/// One response, reduced to what the script reads.
struct Page {
    status: u16,
    location: Option<String>,
    body: String,
}

impl Browser {
    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a plain-HTTP client builds")
    }

    fn absorb(&mut self, response: &reqwest::Response) {
        for value in response.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, attributes)) = text.split_once(';').or(Some((text, ""))) else {
                continue;
            };
            let Some((name, cookie)) = pair.split_once('=') else {
                continue;
            };
            let expired = attributes.to_ascii_lowercase().contains("max-age=0");
            if cookie.is_empty() || expired {
                self.cookies.remove(name.trim());
            } else {
                self.cookies
                    .insert(name.trim().to_owned(), cookie.to_owned());
            }
        }
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn get(&mut self, url: &str) -> anyhow::Result<Page> {
        let response = Self::client()
            .get(url)
            .header(reqwest::header::COOKIE, self.cookie_header())
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        self.finish(response).await
    }

    async fn post(&mut self, url: &str, fields: &[(String, String)]) -> anyhow::Result<Page> {
        let response = Self::client()
            .post(url)
            .header(reqwest::header::COOKIE, self.cookie_header())
            .form(fields)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        self.finish(response).await
    }

    async fn finish(&mut self, response: reqwest::Response) -> anyhow::Result<Page> {
        self.absorb(&response);
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.text().await.unwrap_or_default();
        Ok(Page {
            status,
            location,
            body,
        })
    }

    /// Follows redirects until the next one would leave for vpay's own SSO
    /// callback URL (which nothing is listening on), and returns that URL; or
    /// returns the page it stopped on.
    async fn follow(&mut self, mut page: Page) -> anyhow::Result<Result<String, Page>> {
        for _ in 0..12 {
            match (page.status, page.location.clone()) {
                (301..=303 | 307 | 308, Some(location)) => {
                    if location.starts_with(SSO_REDIRECT_URI) {
                        return Ok(Ok(location));
                    }
                    page = self.get(&location).await?;
                }
                _ => return Ok(Err(page)),
            }
        }
        anyhow::bail!("too many redirects")
    }
}

/// The `action` and every named `<input>` of the first `<form>` on a page.
///
/// A tolerant scan rather than an HTML parser: Keycloak's login pages are
/// stable, and a dependency added to read three forms would be the larger
/// risk. HTML entities in `action` (`&amp;`) are the only escaping that
/// matters.
fn form_of(page: &str) -> anyhow::Result<(String, Vec<(String, String)>)> {
    let start = page.find("<form").context("the page has a form")?;
    let rest = page.get(start..).context("slicing at the form")?;
    let end = rest.find("</form>").context("the form closes")?;
    let form = rest.get(..end).context("the form body")?;

    let action = attribute(form, "action")
        .context("the form has an action")?
        .replace("&amp;", "&");

    let mut fields = Vec::new();
    let mut cursor = form;
    while let Some(at) = cursor.find("<input") {
        let tail = cursor.get(at..).context("input tail")?;
        let close = tail.find('>').context("input closes")?;
        let tag = tail.get(..close).context("input tag")?;
        if let Some(name) = attribute(tag, "name") {
            let value = attribute(tag, "value").unwrap_or_default();
            fields.push((name, value.replace("&amp;", "&").replace("&#x3d;", "=")));
        }
        cursor = tail.get(close..).context("after the input")?;
    }
    Ok((action, fields))
}

/// `name="value"` out of one tag's text.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=\"");
    let at = tag.find(&needle)? + needle.len();
    let rest = tag.get(at..)?;
    Some(rest.get(..rest.find('"')?)?.to_owned())
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

fn set(fields: &mut Vec<(String, String)>, name: &str, value: &str) {
    fields.retain(|(existing, _)| existing != name);
    fields.push((name.to_owned(), value.to_owned()));
}

fn query(location: &str, name: &str) -> Option<String> {
    reqwest::Url::parse(location)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

// ------------------------------------------------------------ the scenario --

/// Every credential this run uses, random per run (256 bits from the same
/// `OsRng` path vpay mints session tokens with).
struct Secrets {
    ada: String,
    grace: String,
    mallory: String,
    client_secret: String,
    admin: String,
}

impl Secrets {
    fn generate() -> Self {
        let fresh = || vpay_api::staff_auth::tokens::mint().token;
        Self {
            ada: fresh(),
            grace: fresh(),
            mallory: fresh(),
            client_secret: fresh(),
            admin: fresh(),
        }
    }

    /// The realm import with this run's secrets in it.
    fn realm(&self) -> String {
        REALM_TEMPLATE
            .replace("__ADA_PASSWORD__", &self.ada)
            .replace("__GRACE_PASSWORD__", &self.grace)
            .replace("__MALLORY_PASSWORD__", &self.mallory)
            .replace("__CLIENT_SECRET__", &self.client_secret)
    }
}

struct Stack {
    secrets: Secrets,
    _keycloak: testcontainers::ContainerAsync<GenericImage>,
    _postgres: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    _server: tokio::task::JoinHandle<()>,
    pool: sqlx::PgPool,
    base_url: String,
}

async fn stack() -> anyhow::Result<Stack> {
    ensure_crypto_provider_installed();
    let secrets = Secrets::generate();

    let keycloak = GenericImage::new("quay.io/keycloak/keycloak", "26.6.1")
        .with_exposed_port(8080.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Listening on:"))
        .with_env_var("KC_BOOTSTRAP_ADMIN_USERNAME", "admin")
        .with_env_var("KC_BOOTSTRAP_ADMIN_PASSWORD", secrets.admin.clone())
        .with_copy_to(
            "/opt/keycloak/data/import/realm.json",
            secrets.realm().into_bytes(),
        )
        .with_cmd(["start-dev", "--import-realm"])
        .start()
        .await
        .context("starting Keycloak (quay.io/keycloak/keycloak:26.6.1 is pulled on first use)")?;
    let port = keycloak.get_host_port_ipv4(8080).await?;
    let issuer = format!("http://127.0.0.1:{port}/realms/vaam-staff");

    let (postgres, repositories, pool) = migrated_postgres().await?;
    let (server_pem, _jwks) = generate_key();
    let (_pem, jwks_a) = generate_key();

    let staff_auth = StaffAuth {
        password_pepper: Some("a-test-pepper-for-the-keycloak-suite".to_owned()),
        totp_encryption_key: Some(URL_SAFE_NO_PAD.encode([9_u8; 32])),
        trusted_proxies: Vec::new(),
        rate_limits: RateLimits::default(),
        access_token_ttl_seconds: StaffAuth::DEFAULT_ACCESS_TOKEN_TTL_SECONDS,
        sso: Some(StaffSso {
            issuer,
            client_id: SSO_CLIENT_ID.to_owned(),
            client_secret: secrets.client_secret.clone(),
            redirect_uri: SSO_REDIRECT_URI.to_owned(),
            scopes: vec!["openid".into(), "email".into(), "profile".into()],
            required_group: "vpay-staff".to_owned(),
            groups_claim: "groups".to_owned(),
            // The measured production shape: Keycloak's `acr` proves nothing
            // about a second factor, `amr` does.
            accepted_acr: Vec::new(),
            accepted_amr: vec!["otp".to_owned()],
        }),
    };
    let served = serve(&repositories, &server_pem, move |base_url| Config {
        deployment: Deployment {
            name: "staff-sso-keycloak".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: vec![ProviderHost {
            code: "mtn_momo".to_owned(),
            enabled: true,
            host: HostEntry {
                url: "http://127.0.0.1:1".to_owned(),
                label: "unreachable-by-design".to_owned(),
            },
            settings: BTreeMap::from([
                ("target_environment".to_owned(), "sandbox".to_owned()),
                (
                    "api_user".to_owned(),
                    "11111111-2222-3333-4444-555555555555".to_owned(),
                ),
            ]),
            callback_url: None,
            currency: "XAF".to_owned(),
            credentials: BTreeMap::from([
                ("subscription_key".to_owned(), "stub".to_owned()),
                ("api_key".to_owned(), "stub".to_owned()),
            ]),
            display_name: None,
            callbacks: None,
        }],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![merchant_client("acme-cameroon", MERCHANT_A, jwks_a)],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: Some(DashboardClient {
            client_id: "vpay-dashboard".to_owned(),
            merchant_id: MERCHANT_A.to_owned(),
            redirect_uris: vec!["http://127.0.0.1:3000/api/auth/callback".to_owned()],
            scope: "dashboard:read".to_owned(),
            client_secret: None,
        }),
        staff_auth,
        routing: Default::default(),
    })
    .await?;

    Ok(Stack {
        secrets,
        _keycloak: keycloak,
        _postgres: postgres,
        _server: served.server,
        pool,
        base_url: served.base_url,
    })
}

impl Stack {
    async fn start(&self) -> anyhow::Result<(String, String)> {
        let response = Browser::client()
            .post(format!("{}/dash/v1/staff/sso/start", self.base_url))
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "start: {}",
            response.status()
        );
        let body: Value = response.json().await?;
        Ok((
            field(&body, "session")
                .as_str()
                .context("session")?
                .to_owned(),
            field(&body, "authorization_url")
                .as_str()
                .context("url")?
                .to_owned(),
        ))
    }

    async fn callback(&self, pending: &str, redirect: &str) -> anyhow::Result<(u16, Value)> {
        let code = query(redirect, "code").context("the redirect carries a code")?;
        let state = query(redirect, "state").context("the redirect carries a state")?;
        let response = Browser::client()
            .post(format!("{}/dash/v1/staff/sso/callback", self.base_url))
            .header(vpay_api::staff::SESSION_HEADER, pending)
            .form(&[("code", code), ("state", state)])
            .send()
            .await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query(sql)
            .fetch_one(&self.pool)
            .await
            .expect("count")
            .get::<i64, _>(0)
    }
}

/// Walks Keycloak's forms as a browser would: whatever page comes back is
/// either the password form, the OTP form, the OTP-enrolment form, or the end
/// of the line — a redirect to vpay's callback URL.
async fn sign_in_at_keycloak(
    browser: &mut Browser,
    authorization_url: &str,
    username: &str,
    password: &str,
    otp_key: Option<&[u8]>,
    otp_step_offset: i64,
) -> anyhow::Result<KeycloakResult> {
    let mut enrolled_secret = None;
    let first = browser.get(authorization_url).await?;
    let mut page = match browser.follow(first).await? {
        Ok(redirect) => {
            return Ok(KeycloakResult {
                redirect,
                enrolled_secret,
            });
        }
        Err(page) => page,
    };

    for _ in 0..6 {
        let body = page.body.clone();
        let (action, mut fields) = form_of(&body)?;
        let names: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();

        if names.contains(&"username") || names.contains(&"password") {
            // With `prompt=login` over an existing Keycloak session the form
            // remembers the user and asks for the password alone.
            if names.contains(&"username") {
                set(&mut fields, "username", username);
            }
            set(&mut fields, "password", password);
        } else if names.contains(&"totpSecret") {
            // CONFIGURE_TOTP: the secret is in the form; Keycloak verifies the
            // code against its raw bytes.
            let secret = fields
                .iter()
                .find(|(name, _)| name == "totpSecret")
                .map(|(_, value)| value.clone())
                .context("totpSecret")?;
            enrolled_secret = Some(secret.clone().into_bytes());
            let totp = Totp::new(secret.into_bytes());
            set(
                &mut fields,
                "totp",
                &totp.code_at_step(step_at(OffsetDateTime::now_utc().unix_timestamp())),
            );
            set(&mut fields, "userLabel", "test phone");
        } else if names.contains(&"otp") {
            let key = otp_key.context("an OTP form appeared for a user with no key")?;
            let totp = Totp::new(key.to_vec());
            set(
                &mut fields,
                "otp",
                &totp.code_at_step(
                    step_at(OffsetDateTime::now_utc().unix_timestamp()) + otp_step_offset,
                ),
            );
        } else {
            anyhow::bail!("an unexpected Keycloak form: {names:?}");
        }

        let posted = browser.post(&action, &fields).await?;
        match browser.follow(posted).await? {
            Ok(redirect) => {
                return Ok(KeycloakResult {
                    redirect,
                    enrolled_secret,
                });
            }
            Err(next) => page = next,
        }
    }
    anyhow::bail!("Keycloak never redirected back")
}

/// Where the browser ended up, and the OTP secret Keycloak generated if the
/// walk included enrolling one.
struct KeycloakResult {
    /// vpay's SSO callback URL with `code` and `state`.
    redirect: String,
    /// The raw key of an OTP credential enrolled during this walk.
    enrolled_secret: Option<Vec<u8>>,
}

#[tokio::test]
async fn a_real_keycloak_signs_staff_in_and_the_measured_amr_behaviour_is_handled()
-> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("vpay_api=info")
        .try_init();
    let stack = stack().await?;

    // ---- 1. password + OTP: amr ["pwd","otp"], signed in -------------------
    let mut ada = Browser::default();
    let (pending, url) = stack.start().await?;
    let walked = sign_in_at_keycloak(
        &mut ada,
        &url,
        "ada",
        &stack.secrets.ada,
        Some(IMPORTED_TOTP_KEY),
        0,
    )
    .await?;
    let (status, body) = stack.callback(&pending, &walked.redirect).await?;
    assert_eq!(
        (status, field(&body, "outcome").as_str()),
        (200, Some("signed_in")),
        "{body}"
    );
    assert_eq!(stack.count("SELECT count(*) FROM staff_members").await, 1);
    let row = sqlx::query("SELECT email, display_name, merchant_id, is_admin FROM staff_members")
        .fetch_one(&stack.pool)
        .await?;
    assert_eq!(row.get::<String, _>("email"), "ada@example.test");
    assert_eq!(row.get::<String, _>("display_name"), "Ada Lovelace");
    assert_eq!(row.get::<String, _>("merchant_id"), MERCHANT_A);
    assert!(!row.get::<bool, _>("is_admin"));

    // ---- 3. a repeat sign-in from Keycloak's SSO cookie ---------------------
    // No form this time: Keycloak answers the authorization request with a
    // redirect carrying a code. Its token has acr "0" and still amr
    // ["pwd","otp"] — so no retry, and the existing link is reused.
    // Three seconds on purpose. Without `default.reference.maxAge` on the two
    // executions, Keycloak drops `amr` from a token issued off its SSO cookie
    // after about one second (measured 2026-10-20: acr "0", amr [] at 2.5 s,
    // amr ["pwd","otp"] inside a second) — and a person is never that quick.
    // The realm sets it to the SSO session length, and this wait is what makes
    // that setting load-bearing in the test rather than a coincidence of speed.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let (pending, url) = stack.start().await?;
    let walked = sign_in_at_keycloak(&mut ada, &url, "ada", &stack.secrets.ada, None, 0).await?;
    let (status, body) = stack.callback(&pending, &walked.redirect).await?;
    assert_eq!(
        (status, field(&body, "outcome").as_str()),
        (200, Some("signed_in")),
        "{body}"
    );
    assert_eq!(
        stack.count("SELECT count(*) FROM staff_members").await,
        1,
        "the link is reused"
    );
    assert_eq!(
        stack
            .count("SELECT count(*) FROM credentials WHERE kind = 'oidc'")
            .await,
        1
    );

    // ---- 2. OTP enrolled during the sign-in: amr ["pwd"], one retry ---------
    let mut grace = Browser::default();
    let (pending, url) = stack.start().await?;
    let first =
        sign_in_at_keycloak(&mut grace, &url, "grace", &stack.secrets.grace, None, 0).await?;
    let enrolled = first
        .enrolled_secret
        .clone()
        .context("grace's first sign-in must have enrolled an OTP (CONFIGURE_TOTP)")?;
    let (status, body) = stack.callback(&pending, &first.redirect).await?;
    assert_eq!(
        (status, field(&body, "outcome").as_str()),
        (200, Some("reauthenticate")),
        "a token whose amr is [\"pwd\"] only earns the retry: {body}"
    );
    assert_eq!(
        stack.count("SELECT count(*) FROM staff_members").await,
        1,
        "nobody provisioned yet"
    );
    let retry_url = field(&body, "authorization_url")
        .as_str()
        .context("the retry URL")?
        .to_owned();
    let retry_pending = field(&body, "session")
        .as_str()
        .context("the retry flow")?
        .to_owned();
    assert!(retry_url.contains("prompt=login"), "{retry_url}");

    // Keycloak's SSO cookie is in the jar, so without `prompt=login` it would
    // answer from the cookie. With it, the forms run again, and the OTP form
    // now exists because the credential was enrolled a moment ago. Its code is
    // for the NEXT step: the enrolment code is spent (a spent code is refused
    // within its window), and the step after it is inside Keycloak's
    // look-ahead.
    let second = sign_in_at_keycloak(
        &mut grace,
        &retry_url,
        "grace",
        &stack.secrets.grace,
        Some(&enrolled),
        1,
    )
    .await?;
    let (status, body) = stack.callback(&retry_pending, &second.redirect).await?;
    assert_eq!(
        (status, field(&body, "outcome").as_str()),
        (200, Some("signed_in")),
        "the retry's token carries otp: {body}"
    );
    assert_eq!(stack.count("SELECT count(*) FROM staff_members").await, 2);

    // ---- 4. a user outside the group is refused, and earns no retry ---------
    let mut mallory = Browser::default();
    let (pending, url) = stack.start().await?;
    let walked = sign_in_at_keycloak(
        &mut mallory,
        &url,
        "mallory",
        &stack.secrets.mallory,
        Some(IMPORTED_TOTP_KEY),
        0,
    )
    .await?;
    let (status, body) = stack.callback(&pending, &walked.redirect).await?;
    assert_eq!(status, 401, "{body}");
    assert_eq!(
        stack.count("SELECT count(*) FROM staff_members").await,
        2,
        "mallory was not provisioned"
    );
    Ok(())
}
