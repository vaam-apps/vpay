//! Staff sign-in, end to end: the real `vpay_api::router` on a real socket,
//! over a real Postgres, from `vpay-server staff add` to a `/dash/v1` read
//! ([ADR-0017](../../../../docs/adr/0017-staff-authentication.md)).
//!
//! # What changed, and why this file exists at all
//!
//! `dashboard_read_surface.rs`'s header says, in the first paragraph, that
//! **nobody can sign in**: every token in that suite is minted by the test,
//! because the authorization-code grant was not served and building it needed
//! a decision nobody had taken.
//!
//! That sentence stopped being true on 2026-09-07. This suite mints **no
//! token at all**. Every one it presents came out of `POST
//! /dash/v1/oauth/token` after a password, a TOTP code and a PKCE exchange,
//! and the server verified it through its own published JWKS over the same
//! socket. The one thing that never happened before this file is what it
//! opens with: a sign-in.
//!
//! # What it claims
//!
//! 1. **the happy path**: a staff member created by the CLI's own code path
//!    signs in with password + TOTP, enrols at first sign-in, replaces the
//!    one-time password, completes the PKCE exchange, and reads their own
//!    merchant's payments from `/dash/v1`;
//! 2. a **wrong password** is refused, and takes the same code path as an
//!    address with no account at all;
//! 3. a **disabled** staff member is refused, and re-reading the row on every
//!    request means disabling one takes effect on their next request;
//! 4. a **replayed TOTP code** is refused — the same six digits, inside the
//!    same 30-second step, twice;
//! 5. a **PKCE verifier mismatch** is refused at the exchange;
//! 6. a **reused authorization code** is refused, and the second attempt is
//!    refused whatever else about it is right;
//! 7. a token minted for merchant **A** cannot list merchant **B**;
//! 8. an **idle** session is refused after 30 minutes, and the absolute bound
//!    refuses a session however recently it was used;
//! 9. **signing out revokes the access token**, because the token is
//!    unobtainable once the row is gone;
//! 10. `/authorize` refuses a session that has presented only a password.
//!
//! # Why the CLI's path and not its process
//!
//! Case 1 creates its staff member through `vpay_db::Staff::create` with a
//! hash from `vpay_api::staff_auth`, which is every line
//! `vpay-server staff add` runs after argument parsing. The subprocess test —
//! that the *binary* accepts the flags, refuses an unregistered merchant and
//! prints a password on stdout — is `vpay-server`'s own `tests/cli.rs`, where
//! every other subprocess assertion about that binary already lives.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sqlx::Row as _;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use time::{Duration, OffsetDateTime};
use vpay_api::staff_auth::{StaffCredentials, totp};
use vpay_config::{
    Config, CurrencyEntry, DashboardClient, Deployment, HostEntry, ProviderHost, StaffAuth,
};
use vpay_db::{NewPaymentIntent, NewStaff, Repositories};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

const DASHBOARD_CLIENT: &str = "vpay-dashboard";
const DASHBOARD_SCOPE: &str = "dashboard:read";
/// The one registered redirect URI. Matched byte for byte at `/authorize` and
/// again at `/token`.
const REDIRECT_URI: &str = "http://127.0.0.1:3000/api/auth/callback";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "XAF";
const AMOUNT: i64 = 5000;

const STAFF_EMAIL: &str = "ada@example.test";
const STAFF_NAME: &str = "Ada Lovelace";
/// The password `staff add` would have printed. A literal here so a test can
/// present it and a wrong one beside it.
const ONE_TIME_PASSWORD: &str = "c8m7k4t2p9r3v6x0z5b1n8q4w7";
const NEW_PASSWORD: &str = "a-much-longer-chosen-password";

/// The deployment's pepper, as a `${VAR}`-free literal because this
/// configuration is built in memory rather than loaded from a file — the
/// `LiteralSecret` rule applies to a *document*, and there is none here.
const PEPPER: &str = "a-test-pepper-for-the-staff-sign-in-suite";

/// RFC 7636's own verifier and challenge (Appendix B), so the PKCE pair this
/// suite presents is one a published document says goes together.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

// ------------------------------------------------------------------ harness

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    #[allow(dead_code)]
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    base_url: String,
    credentials: StaffCredentials,
}

impl Harness {
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
    }

    async fn post_form(
        &self,
        path: &str,
        session: Option<&str>,
        form: &[(&str, &str)],
    ) -> anyhow::Result<(u16, Value)> {
        let mut request = self
            .client()
            .post(format!("{}{path}", self.base_url))
            .form(form);
        if let Some(session) = session {
            request = request.header(vpay_api::staff::SESSION_HEADER, session);
        }
        let response = request.send().await.context("sending a staff request")?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    async fn get_json(
        &self,
        path: &str,
        session: Option<&str>,
        bearer: Option<&str>,
    ) -> anyhow::Result<(u16, Value)> {
        let mut request = self.client().get(format!("{}{path}", self.base_url));
        if let Some(session) = session {
            request = request.header(vpay_api::staff::SESSION_HEADER, session);
        }
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        let response = request.send().await.context("sending a request")?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    /// `GET /dash/v1/oauth/authorize`, returning the status and the `code`
    /// query parameter of the `Location` header if there was one.
    async fn authorize(
        &self,
        session: &str,
        challenge: &str,
    ) -> anyhow::Result<(u16, Option<String>)> {
        let response = self
            .client()
            .get(format!("{}/dash/v1/oauth/authorize", self.base_url))
            .header(vpay_api::staff::SESSION_HEADER, session)
            .query(&[
                ("client_id", DASHBOARD_CLIENT),
                ("redirect_uri", REDIRECT_URI),
                ("response_type", "code"),
                ("scope", DASHBOARD_SCOPE),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
            ])
            .send()
            .await
            .context("sending an authorize request")?;

        let status = response.status().as_u16();
        let code = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(code_in_query);
        Ok((status, code))
    }

    /// `POST /dash/v1/oauth/token`.
    async fn exchange(&self, code: &str, verifier: &str) -> anyhow::Result<(u16, Value)> {
        self.post_form(
            "/dash/v1/oauth/token",
            None,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", REDIRECT_URI),
                ("code_verifier", verifier),
                ("client_id", DASHBOARD_CLIENT),
            ],
        )
        .await
    }

    /// Everything from a password to an access token, for the tests whose
    /// subject is what happens *after* one.
    async fn sign_in(&self) -> anyhow::Result<SignedIn> {
        let (status, body) = self
            .post_form(
                "/dash/v1/staff/login",
                None,
                &[("email", STAFF_EMAIL), ("password", ONE_TIME_PASSWORD)],
            )
            .await?;
        anyhow::ensure!(status == 200, "login: {status} {body}");

        let session = field(&body, "session")
            .as_str()
            .context("a session token")?
            .to_owned();
        let sealed = field(&body, "enrolment")
            .as_str()
            .context("an enrolment blob")?
            .to_owned();
        let secret = self
            .credentials
            .open_secret(&sealed)
            .context("opening the enrolment blob with the deployment key")?;
        let totp = totp::Totp::new(secret);

        let code = totp.code_at_step(totp::step_at(OffsetDateTime::now_utc().unix_timestamp()));
        let (status, body) = self
            .post_form(
                "/dash/v1/staff/totp",
                Some(&session),
                &[("code", &code), ("enrolment", &sealed)],
            )
            .await?;
        anyhow::ensure!(status == 200, "totp: {status} {body}");
        anyhow::ensure!(
            field(&body, "password_change_required") == &Value::Bool(true),
            "a freshly created staff member must be told to replace the printed password: {body}"
        );

        let (status, body) = self
            .post_form(
                "/dash/v1/staff/password",
                Some(&session),
                &[("new_password", NEW_PASSWORD)],
            )
            .await?;
        anyhow::ensure!(status == 200, "password: {status} {body}");

        Ok(SignedIn { session, totp })
    }

    /// A login stopped after the password, on an **unenrolled** account: the
    /// session, the sealed secret it was handed, and a `Totp` over that
    /// secret.
    ///
    /// The building block for the enrolment-race case. `sign_in` completes
    /// enrolment; this deliberately does not.
    async fn begin_enrolment(&self) -> anyhow::Result<Enrolling> {
        let (status, body) = self
            .post_form(
                "/dash/v1/staff/login",
                None,
                &[("email", STAFF_EMAIL), ("password", ONE_TIME_PASSWORD)],
            )
            .await?;
        anyhow::ensure!(status == 200, "login: {status} {body}");

        let session = field(&body, "session")
            .as_str()
            .context("a session token")?
            .to_owned();
        let sealed = field(&body, "enrolment")
            .as_str()
            .context("an enrolment blob — this account must be unenrolled")?
            .to_owned();
        let totp = totp::Totp::new(self.credentials.open_secret(&sealed)?);
        Ok(Enrolling {
            session,
            sealed,
            totp,
        })
    }

    /// A `Totp` over the secret an **enrolled** account actually stores, read
    /// back out of the row and opened with the deployment key.
    ///
    /// The only way a test can generate a code for an account it did not
    /// enrol in the same function — and it goes through the same
    /// `open_secret` the server does, so a change to the sealing format
    /// breaks it here rather than silently somewhere else.
    async fn enrolled_totp(&self) -> anyhow::Result<totp::Totp> {
        let sealed: String = sqlx::query("SELECT totp_secret FROM staff_members WHERE email = $1")
            .bind(STAFF_EMAIL)
            .fetch_one(&self.repositories.op_store_pool())
            .await
            .context("reading the stored TOTP secret")?
            .get("totp_secret");
        Ok(totp::Totp::new(self.credentials.open_secret(&sealed)?))
    }

    /// A signed-in session all the way to a `/dash/v1` bearer token.
    async fn access_token(&self) -> anyhow::Result<(String, String)> {
        let signed_in = self.sign_in().await?;
        let (status, code) = self.authorize(&signed_in.session, CHALLENGE).await?;
        anyhow::ensure!(status == 302, "authorize answered {status}");
        let code = code.context("the redirect carries a code")?;

        let (status, body) = self.exchange(&code, VERIFIER).await?;
        anyhow::ensure!(status == 200, "token: {status} {body}");
        let token = field(&body, "access_token")
            .as_str()
            .context("an access token")?
            .to_owned();
        Ok((signed_in.session, token))
    }
}

/// The `code` parameter of a `Location` header, without a URL parser.
///
/// The value is `authkestra_op`'s own base64url code, which contains no `&`
/// or `=`, so splitting is exact — and a URL parser in this suite would be a
/// dependency added to read one query parameter.
fn code_in_query(location: &str) -> Option<String> {
    location
        .split_once('?')?
        .1
        .split('&')
        .find_map(|pair| pair.strip_prefix("code=").map(str::to_owned))
}

/// One field of a JSON body, or `Null`.
///
/// Not `body["field"]`: `clippy::indexing_slicing` is denied in these suites,
/// and `serde_json::Value`'s `Index` impl panics on a type mismatch — so a
/// handler that answered an array where a test expects an object would fail
/// as a panic rather than as an assertion naming the field.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// Lower-case hex, the spelling `staff_sessions.id` carries.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A login stopped after the password, on an unenrolled account.
struct Enrolling {
    session: String,
    sealed: String,
    totp: totp::Totp,
}

/// A session that has presented both factors, and the TOTP it did it with.
struct SignedIn {
    session: String,
    totp: totp::Totp,
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "staff-sign-in".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![ProviderHost {
            code: PUSH_RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
                // Unreachable on purpose: this suite signs people in and
                // reads rows; no rail is ever called.
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
            currency: CURRENCY.to_owned(),
            credentials: BTreeMap::from([
                (
                    "subscription_key".to_owned(),
                    "stub-subscription-key".to_owned(),
                ),
                ("api_key".to_owned(), "stub-api-key".to_owned()),
            ]),
        }],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            merchant_client(CLIENT_A, MERCHANT_A, jwks_a),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: Some(DashboardClient {
            client_id: DASHBOARD_CLIENT.to_owned(),
            merchant_id: MERCHANT_A.to_owned(),
            redirect_uris: vec![REDIRECT_URI.to_owned()],
            scope: DASHBOARD_SCOPE.to_owned(),
            client_secret: None,
        }),
        staff_auth: StaffAuth {
            password_pepper: Some(PEPPER.to_owned()),
            // Thirty-two bytes, base64url. Any 32 bytes will do; what matters
            // is that the suite's own `StaffCredentials` uses the same value,
            // because opening the enrolment blob is how a test learns the
            // TOTP secret it must generate codes from.
            totp_encryption_key: Some(URL_SAFE_NO_PAD.encode([9_u8; 32])),
        },
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, _pool) = migrated_postgres().await?;
    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b)
    })
    .await?;

    let credentials = StaffCredentials::new(PEPPER, &URL_SAFE_NO_PAD.encode([9_u8; 32]))
        .expect("the suite's own copy of the deployment secrets");

    // The staff member `vpay-server staff add` would have created: the same
    // `NewStaff`, the same argon2id hash, the same lower-cased address.
    add_staff(repositories.as_ref(), &credentials, MERCHANT_A).await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        base_url: served.base_url,
        credentials,
    })
}

async fn add_staff(
    repositories: &dyn Repositories,
    credentials: &StaffCredentials,
    merchant_id: &str,
) -> anyhow::Result<String> {
    let id = vpay_core::ids::staff_id();
    vpay_db::Staff::create(
        repositories,
        NewStaff {
            id: id.clone(),
            merchant_id: merchant_id.to_owned(),
            email: STAFF_EMAIL.to_owned(),
            display_name: STAFF_NAME.to_owned(),
            password_hash: credentials
                .hash_password(ONE_TIME_PASSWORD)
                .expect("hashing the one-time password"),
            now: OffsetDateTime::now_utc(),
        },
    )
    .await
    .context("creating the suite's staff member")?;
    Ok(id)
}

async fn seed_intent(
    repositories: &dyn Repositories,
    merchant_id: &str,
    id: &str,
) -> anyhow::Result<()> {
    vpay_db::PaymentIntents::insert(
        repositories,
        &NewPaymentIntent {
            id: id.to_owned(),
            merchant_id: merchant_id.to_owned(),
            livemode: false,
            amount: AMOUNT,
            currency_code: CURRENCY.to_owned(),
            status: vpay_core::IntentStatus::INITIAL.as_wire_str().to_owned(),
            payment_method_types: serde_json::json!([PUSH_RAIL]),
            metadata: serde_json::json!({}),
            description: None,
            customer_id: None,
            last_payment_error_code: None,
            last_payment_error_message: None,
            // Padded to migration 0026's 32-character floor, which exists
            // because a short suffix is a guessable credential.
            client_secret_suffix: format!("signinsuffix{id:x<32}"),
            created_at: OffsetDateTime::now_utc(),
        },
    )
    .await
    .with_context(|| format!("seeding {id}"))?;
    Ok(())
}

/// `SHA256(verifier)`, base64url — for the one test that needs a challenge
/// its verifier does not satisfy.
fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(sha256(verifier.as_bytes()))
}

/// SHA-256, through `authkestra_crypto_util`'s own digest so this suite adds
/// no dependency of its own for one hash. The session digest below has to be
/// the *same* function `vpay_api::staff_auth::tokens::digest` uses, which is
/// what makes "the row is gone" checkable from outside the crate.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

// ------------------------------------------------------------------ test 1

/// **The sentence this whole file exists for**: a staff member signs in and
/// reads their merchant's payments.
///
/// Password, mandatory TOTP enrolment, a code from the enrolled secret, the
/// replacement of the printed one-time password, the PKCE authorization
/// request, the exchange, and a `/dash/v1` read with the token that came out
/// of it. Nothing here is minted by the test.
#[tokio::test]
async fn a_staff_member_signs_in_and_reads_their_own_merchants_payments() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_signin_a").await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_B, "pi_signin_b").await?;

    let (session, token) = harness.access_token().await?;

    // The session now carries the token, which is what makes signing out a
    // revocation.
    let (status, body) = harness
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        field(&body, "merchant_id"),
        &Value::String(MERCHANT_A.to_owned())
    );
    assert_eq!(
        field(&body, "password_change_required"),
        &Value::Bool(false),
        "the printed password was replaced: {body}"
    );
    assert_eq!(
        field(&body, "access_token").as_str(),
        Some(token.as_str()),
        "the session is where the dashboard reads its token from: {body}"
    );

    let (status, body) = harness
        .get_json("/dash/v1/payment_intents", None, Some(&token))
        .await?;
    assert_eq!(status, 200, "{body}");
    let rendered = body.to_string();
    assert!(rendered.contains("pi_signin_a"), "{rendered}");
    assert!(
        !rendered.contains("pi_signin_b"),
        "the dashboard is bound to {MERCHANT_A}: {rendered}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 2

/// A wrong password is refused, and so is an address with no account —
/// **identically**.
///
/// Same status, same body, so a caller cannot use the login form to discover
/// which addresses exist. The timing half of the property is
/// `StaffCredentials::verify_absent_account`, which does the same argon2id
/// work for both; this asserts the observable half.
///
/// The decisive mutation: return early from `login` when `find_by_email`
/// answers `None`, and the two bodies stay equal but the timing does not —
/// which is why the *unit* test for the dummy hash exists beside this one.
#[tokio::test]
async fn a_wrong_password_and_an_unknown_address_are_the_same_refusal() -> anyhow::Result<()> {
    let harness = harness().await?;

    let (wrong_status, wrong_body) = harness
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", STAFF_EMAIL), ("password", "not-the-password")],
        )
        .await?;
    let (absent_status, absent_body) = harness
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", "nobody@example.test"), ("password", "whatever")],
        )
        .await?;

    assert_eq!(wrong_status, 401, "{wrong_body}");
    assert_eq!(absent_status, 401, "{absent_body}");
    assert_eq!(
        wrong_body, absent_body,
        "a login form that answered differently for an address with no account would be an \
         account-enumeration oracle"
    );
    assert!(
        !wrong_body.to_string().contains(STAFF_EMAIL),
        "the refusal must not echo the address back: {wrong_body}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 3

/// A disabled staff member is refused — **on their next request**, not at
/// their next login, and on **both** credentials they hold.
///
/// Two credentials come out of one sign-in and they are checked in two
/// different places, so this asserts both:
///
/// * the **session** (`x-vpay-staff-session`), refused by `load_session`,
///   which re-reads the staff row on every request rather than trusting what
///   the session recorded;
/// * the **bearer token**, refused by `require_dashboard_token`, which reads
///   the `staff_members` row its `sub` names.
///
/// **The second half is what the exp24 review added (finding F1), and the
/// first draft of this test is why it was needed.** That draft wrote
/// `let (session, _token)` and checked only `/dash/v1/staff/session` — so a
/// disabled staff member went on listing their merchant's payment intents
/// with the token they already held, for the whole 15-minute access-token
/// TTL, while this file, `docs/status.md` and ADR-0017 all said
/// `staff_members.status` was the per-person kill switch that "takes effect
/// on their next request". A revocation that a payments dashboard honours a
/// quarter of an hour late is not a revocation.
///
/// The decisive mutations: cache the staff row on the session (the session
/// half returns `200`), and delete the `Staff::find` arm from
/// `require_dashboard_token` (the bearer half returns `200`).
#[tokio::test]
async fn disabling_a_staff_member_refuses_their_live_session() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (session, token) = harness.access_token().await?;

    // Live before, on both credentials.
    let (status, _) = harness
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(status, 200);
    let (status, body) = harness
        .get_json("/dash/v1/payment_intents", None, Some(&token))
        .await?;
    assert_eq!(
        status, 200,
        "the bearer token reads before the account is disabled: {body}"
    );

    // Disabled directly in the table: there is no HTTP endpoint that
    // disables a staff member, deliberately (ADR-0017 gives the CLI one
    // subcommand), so this is what an operator's `UPDATE` would do.
    sqlx::query("UPDATE staff_members SET status = 'disabled' WHERE email = $1")
        .bind(STAFF_EMAIL)
        .execute(&harness.repositories.op_store_pool())
        .await
        .context("disabling the staff member")?;

    let (status, body) = harness
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(
        status, 401,
        "a disabled account's live session must stop working at once: {body}"
    );

    // THE HALF THAT MATTERS: the bearer token is the credential that reads a
    // merchant's rows, and it was minted before the account was disabled.
    let (status, body) = harness
        .get_json("/dash/v1/payment_intents", None, Some(&token))
        .await?;
    assert_eq!(
        status, 403,
        "a disabled account's already-minted /dash/v1 token must stop reading at once; a token          that outlives the disabling by its own TTL makes staff_members.status useless as the          break-glass control ADR-0017 says it is: {body}"
    );

    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", STAFF_EMAIL), ("password", NEW_PASSWORD)],
        )
        .await?;
    assert_eq!(status, 401, "and they cannot sign in again: {body}");
    Ok(())
}

// ------------------------------------------------------------------ test 4

/// **The TOTP replay guard.** The same six digits, inside the same 30-second
/// step, presented twice.
///
/// The second presentation is refused because
/// `Staff::record_totp_step`'s compare-and-swap admits only a step strictly
/// greater than the last accepted one. Without it the +/-1 skew window — the
/// thing that makes TOTP usable across a clock drift — would be a 90-second
/// replay window.
///
/// The decisive mutation: drop `.where_(last_totp_step().lt(step))` from
/// `record_totp_step` and the second sign-in below succeeds.
#[tokio::test]
async fn a_replayed_totp_code_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    let signed_in = harness.sign_in().await?;

    let step = totp::step_at(OffsetDateTime::now_utc().unix_timestamp());
    let code = signed_in.totp.code_at_step(step);

    // A second session for the same person, so the only thing being replayed
    // is the code.
    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", STAFF_EMAIL), ("password", NEW_PASSWORD)],
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let second = field(&body, "session")
        .as_str()
        .context("a session token")?
        .to_owned();
    assert!(
        field(&body, "enrolment").is_null(),
        "an enrolled account must not be offered a fresh secret: {body}"
    );

    let (status, body) = harness
        .post_form("/dash/v1/staff/totp", Some(&second), &[("code", &code)])
        .await?;
    assert_eq!(
        status, 401,
        "the code that completed the first sign-in must not complete a second: {body}"
    );

    // And the next step's code works, so what was refused was the replay and
    // not TOTP itself.
    let next = signed_in.totp.code_at_step(step + 1);
    let (status, body) = harness
        .post_form("/dash/v1/staff/totp", Some(&second), &[("code", &next)])
        .await?;
    assert_eq!(
        status, 200,
        "a code from a later step is a different code: {body}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 5

/// A PKCE verifier that does not match the challenge is refused at the
/// exchange.
///
/// This is what authenticates a public client: the dashboard presents no
/// secret, so the verifier is the whole of its credential at `/token`.
///
/// The decisive mutation: make `verifier_matches` return `true`, or delete
/// its call, and this returns `200` with an access token for a code whoever
/// intercepted it could then spend.
#[tokio::test]
async fn a_pkce_verifier_mismatch_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    let signed_in = harness.sign_in().await?;

    let (status, code) = harness.authorize(&signed_in.session, CHALLENGE).await?;
    assert_eq!(status, 302);
    let code = code.context("the redirect carries a code")?;

    let other = "Ux8VQxKq4Y1v2Zp7Nn0Rr5Ss3Tt6Uu9Vv2Ww5Xx8Yy1";
    assert_ne!(challenge_for(other), CHALLENGE);

    let (status, body) = harness.exchange(&code, other).await?;
    assert_eq!(status, 401, "{body}");
    assert!(
        field(&body, "access_token").is_null(),
        "a refused exchange must not carry a token: {body}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 6

/// An authorization code is **single use**.
///
/// The second exchange is refused although the verifier, the redirect URI and
/// the client are all correct — the code is spent, and
/// `AuthorizationCodes::consume_code`'s compare-and-swap is what spends it.
///
/// The decisive mutation: drop `.where_(consumed_at().is_null())` from that
/// swap, and the second exchange below mints a second token for one login.
#[tokio::test]
async fn an_authorization_code_cannot_be_exchanged_twice() -> anyhow::Result<()> {
    let harness = harness().await?;
    let signed_in = harness.sign_in().await?;

    let (status, code) = harness.authorize(&signed_in.session, CHALLENGE).await?;
    assert_eq!(status, 302);
    let code = code.context("the redirect carries a code")?;

    let (first, first_body) = harness.exchange(&code, VERIFIER).await?;
    assert_eq!(first, 200, "{first_body}");

    let (second, second_body) = harness.exchange(&code, VERIFIER).await?;
    assert_eq!(
        second, 401,
        "a spent code must not mint a second token: {second_body}"
    );
    assert!(
        field(&second_body, "access_token").is_null(),
        "{second_body}"
    );
    Ok(())
}

/// A code presented with the **wrong redirect URI** is refused, and is spent
/// anyway.
///
/// RFC 6749 §4.1.3. The "spent anyway" half is what stops a captured code
/// being probed against a list of candidate URIs.
#[tokio::test]
async fn a_code_redeemed_against_another_redirect_uri_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    let signed_in = harness.sign_in().await?;

    let (status, code) = harness.authorize(&signed_in.session, CHALLENGE).await?;
    assert_eq!(status, 302);
    let code = code.context("the redirect carries a code")?;

    let (status, body) = harness
        .post_form(
            "/dash/v1/oauth/token",
            None,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", "http://127.0.0.1:3000/somewhere-else"),
                ("code_verifier", VERIFIER),
                ("client_id", DASHBOARD_CLIENT),
            ],
        )
        .await?;
    assert_eq!(status, 401, "{body}");

    let (status, body) = harness.exchange(&code, VERIFIER).await?;
    assert_eq!(
        status, 401,
        "the failed attempt still spent the code, so it cannot be probed: {body}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 7

/// A staff member bound to merchant **B** cannot obtain a token at all on a
/// deployment whose dashboard is bound to **A**.
///
/// Refused at `/authorize`, one round trip before `require_dashboard_token`
/// would refuse the token — the same answer, with the log line where the
/// mismatch is visible.
///
/// The decisive mutation: delete the `staff.merchant_id != binding.merchant_id`
/// arm from `authorize` and this returns a `302`; the token it then mints is
/// still refused by `require_dashboard_token`'s merchant claim, which is what
/// makes the two checks two checks.
#[tokio::test]
async fn a_staff_member_of_another_merchant_cannot_obtain_a_dashboard_token() -> anyhow::Result<()>
{
    let harness = harness().await?;

    // A second staff member, bound to the merchant the dashboard is NOT
    // bound to.
    let other_email = "grace@example.test";
    let id = vpay_core::ids::staff_id();
    vpay_db::Staff::create(
        harness.repositories.as_ref(),
        NewStaff {
            id,
            merchant_id: MERCHANT_B.to_owned(),
            email: other_email.to_owned(),
            display_name: "Grace Hopper".to_owned(),
            password_hash: harness
                .credentials
                .hash_password(ONE_TIME_PASSWORD)
                .expect("hashing"),
            now: OffsetDateTime::now_utc(),
        },
    )
    .await?;

    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", other_email), ("password", ONE_TIME_PASSWORD)],
        )
        .await?;
    assert_eq!(status, 200, "their password is fine: {body}");
    let session = field(&body, "session")
        .as_str()
        .context("a session")?
        .to_owned();
    let sealed = field(&body, "enrolment")
        .as_str()
        .context("an enrolment")?
        .to_owned();
    let secret = harness.credentials.open_secret(&sealed)?;
    let code = totp::Totp::new(secret)
        .code_at_step(totp::step_at(OffsetDateTime::now_utc().unix_timestamp()));
    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/totp",
            Some(&session),
            &[("code", &code), ("enrolment", &sealed)],
        )
        .await?;
    assert_eq!(status, 200, "and so is their second factor: {body}");

    // The password change first, so the refusal below is about the tenant and
    // not about the printed password.
    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/password",
            Some(&session),
            &[("new_password", NEW_PASSWORD)],
        )
        .await?;
    assert_eq!(status, 200, "{body}");

    let (status, code) = harness.authorize(&session, CHALLENGE).await?;
    assert_eq!(
        status, 401,
        "a staff member of another merchant must not be minted a code"
    );
    assert!(code.is_none(), "and no code may reach the redirect");
    Ok(())
}

// ------------------------------------------------------------------ test 8

/// An **idle** session is refused after 30 minutes, and the **absolute**
/// bound refuses one however recently it was used.
///
/// The two bounds are moved in the table rather than by waiting: a test that
/// slept for half an hour would be a test nobody runs.
///
/// The decisive mutations: delete either half of `SessionRow::is_live_at` and
/// the corresponding assertion below returns `200`.
#[tokio::test]
async fn an_idle_session_and_an_expired_one_are_both_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (session, _token) = harness.access_token().await?;

    let pool = harness.repositories.op_store_pool();

    // Idle: last seen 31 minutes ago, absolute bound still hours away.
    sqlx::query("UPDATE staff_sessions SET last_seen_at = $1")
        .bind(OffsetDateTime::now_utc() - Duration::minutes(31))
        .execute(&pool)
        .await?;
    let (status, body) = harness
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(status, 401, "an idle session is refused: {body}");

    // Used just now, but past the absolute bound.
    sqlx::query("UPDATE staff_sessions SET last_seen_at = $1, expires_at = $2")
        .bind(OffsetDateTime::now_utc())
        .bind(OffsetDateTime::now_utc() - Duration::seconds(1))
        .execute(&pool)
        .await?;
    let (status, body) = harness
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(
        status, 401,
        "the absolute bound is never extended by use: {body}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 9

/// **Signing out revokes the access token**, which is the deny-list ADR-0009
/// left undecided.
///
/// The token itself is still a valid JWT for the rest of its TTL — nothing
/// can change that, and this test does not pretend otherwise. What signing
/// out removes is the only place the dashboard's own server can read it
/// from, so a browser whose session is gone cannot present it.
#[tokio::test]
async fn signing_out_deletes_the_session_and_with_it_the_access_token() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (session, token) = harness.access_token().await?;

    let (status, body) = harness
        .post_form("/dash/v1/staff/logout", Some(&session), &[])
        .await?;
    assert_eq!(status, 200, "{body}");

    let (status, body) = harness
        .get_json("/dash/v1/staff/session", Some(&session), None)
        .await?;
    assert_eq!(status, 401, "the session is gone: {body}");

    let row = vpay_db::StaffSessions::load(
        harness.repositories.as_ref(),
        &hex(&sha256(session.as_bytes())),
        OffsetDateTime::now_utc(),
    )
    .await?;
    assert!(row.is_none(), "the row is deleted, not flagged");

    // The token is still a valid JWT — stated rather than hidden. The
    // revocation is that nobody can obtain it any more.
    let (status, _) = harness
        .get_json("/dash/v1/payment_intents", None, Some(&token))
        .await?;
    assert_eq!(
        status, 200,
        "a minted token stays valid for its TTL; ADR-0017's Consequences says so"
    );

    // Signing out again is not an error.
    let (status, _) = harness
        .post_form("/dash/v1/staff/logout", Some(&session), &[])
        .await?;
    assert_eq!(status, 200, "sign-out is idempotent");
    Ok(())
}

// ----------------------------------------------------------------- test 10

/// A session that has presented only a password cannot reach `/authorize`.
///
/// The second factor is mandatory, and this is where "mandatory" is
/// enforced: a `pending_totp` session may present a code and do nothing else.
///
/// The decisive mutation: use `load_session` instead of
/// `authenticated_session` in `authorize` and this returns a `302`.
///
/// **The first draft of this test was not decisive, and the mutation is what
/// said so.** It signed in from scratch, so the session it built was
/// `pending_totp` *and* belonged to a staff member who had never replaced the
/// printed one-time password — and `authorize` refuses that too. Under the
/// mutation the test still passed, refusing for the second reason while the
/// first was gone. So this one signs in **fully** first, which clears
/// `password_change_required`, and only then starts a second login and stops
/// after the password: the second-factor check is the only thing left to
/// refuse it. The control at the end is what stops a mutation that refused
/// *every* session from passing.
#[tokio::test]
async fn a_session_that_has_not_presented_a_second_factor_cannot_authorize() -> anyhow::Result<()> {
    let harness = harness().await?;

    // A full sign-in first, purely to clear `password_change_required` — see
    // the doc above. Its own session is then abandoned.
    harness.sign_in().await?;

    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", STAFF_EMAIL), ("password", NEW_PASSWORD)],
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let session = field(&body, "session")
        .as_str()
        .context("a session")?
        .to_owned();

    let (status, code) = harness.authorize(&session, CHALLENGE).await?;
    assert_eq!(status, 401, "a password alone is not a sign-in");
    assert!(code.is_none());

    // The control: the same session, once it HAS presented a code, reaches
    // `/authorize`.
    let totp = harness.enrolled_totp().await?;
    let step = totp::step_at(OffsetDateTime::now_utc().unix_timestamp());
    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/totp",
            Some(&session),
            &[("code", &totp.code_at_step(step + 1))],
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let (status, code) = harness.authorize(&session, CHALLENGE).await?;
    assert_eq!(status, 302, "the same session, with both factors");
    assert!(code.is_some());
    Ok(())
}

/// **Enrolment happens once.** A second first-sign-in cannot replace an
/// enrolled staff member's second factor.
///
/// Two logins are started against an *unenrolled* account, so each is handed
/// its own freshly minted secret. The first completes enrolment; the second
/// then presents a valid code **from its own secret**, and is refused —
/// because `Staff::enrol_totp`'s compare-and-swap guards on
/// `totp_enrolled_at IS NULL` and the first login won it.
///
/// Without that guard the second login would *succeed* and overwrite the
/// stored secret with its own: a second-factor reset performed by whoever
/// reached the enrolment screen second, with nothing but the password in
/// front of it. The step used is deliberately one **ahead** of the first's,
/// so the TOTP replay guard is not what refuses it.
///
/// The decisive mutation: delete
/// `.where_(staff_member::totp_enrolled_at().is_null())` from `enrol_totp`.
#[tokio::test]
async fn a_second_enrolment_cannot_replace_an_enrolled_second_factor() -> anyhow::Result<()> {
    let harness = harness().await?;

    let first = harness.begin_enrolment().await?;
    let second = harness.begin_enrolment().await?;
    assert_ne!(
        first.sealed, second.sealed,
        "each login mints its own secret, or this test proves nothing"
    );

    let step = totp::step_at(OffsetDateTime::now_utc().unix_timestamp());
    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/totp",
            Some(&first.session),
            &[
                ("code", &first.totp.code_at_step(step)),
                ("enrolment", &first.sealed),
            ],
        )
        .await?;
    assert_eq!(status, 200, "the first login enrols: {body}");

    // One step ahead, so the TOTP replay guard cannot be what refuses this.
    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/totp",
            Some(&second.session),
            &[
                ("code", &second.totp.code_at_step(step + 1)),
                ("enrolment", &second.sealed),
            ],
        )
        .await?;
    assert_eq!(
        status, 401,
        "a second enrolment must not replace the stored secret: {body}"
    );

    // "Was not replaced" means: the FIRST login's secret is still the one
    // that authenticates.
    let stored = harness.enrolled_totp().await?;
    assert_eq!(
        stored.code_at_step(step + 2),
        first.totp.code_at_step(step + 2),
        "the stored secret is the first login's"
    );
    assert_ne!(
        stored.code_at_step(step + 2),
        second.totp.code_at_step(step + 2)
    );
    Ok(())
}

/// A staff member who has not replaced the printed one-time password cannot
/// reach `/authorize` either.
///
/// So a credential an operator typed on a terminal can never become a
/// `/dash/v1` token, however long it is ignored.
#[tokio::test]
async fn the_printed_password_cannot_reach_dash_v1() -> anyhow::Result<()> {
    let harness = harness().await?;

    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/login",
            None,
            &[("email", STAFF_EMAIL), ("password", ONE_TIME_PASSWORD)],
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let session = field(&body, "session")
        .as_str()
        .context("a session")?
        .to_owned();
    let sealed = field(&body, "enrolment")
        .as_str()
        .context("an enrolment")?
        .to_owned();
    let secret = harness.credentials.open_secret(&sealed)?;
    let code = totp::Totp::new(secret)
        .code_at_step(totp::step_at(OffsetDateTime::now_utc().unix_timestamp()));

    let (status, body) = harness
        .post_form(
            "/dash/v1/staff/totp",
            Some(&session),
            &[("code", &code), ("enrolment", &sealed)],
        )
        .await?;
    assert_eq!(status, 200, "both factors are fine: {body}");

    let (status, redirect_code) = harness.authorize(&session, CHALLENGE).await?;
    assert_eq!(
        status, 401,
        "the printed password has not been replaced, so no token may be minted"
    );
    assert!(redirect_code.is_none());
    Ok(())
}

/// A deployment that registers a `dashboard_client` and **no** `staff_auth`
/// secrets serves the read surface and no login at all.
///
/// The state master was in before ADR-0017, kept expressible on purpose: "the
/// reads are mounted" and "a human can reach them" are two claims, and a
/// deployment can be in the first without the second.
#[tokio::test]
async fn a_deployment_without_staff_auth_serves_no_login() -> anyhow::Result<()> {
    ensure_crypto_provider_installed();

    let (_container, repositories, _pool) = migrated_postgres().await?;
    let (server_pem, _jwks) = generate_key();
    let (_a, jwks_a) = generate_key();
    let (_b, jwks_b) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        let mut config = config_with(base_url, jwks_a, jwks_b);
        config.staff_auth = StaffAuth::default();
        config
    })
    .await?;

    let response = reqwest::Client::new()
        .post(format!("{}/dash/v1/staff/login", served.base_url))
        .form(&[("email", STAFF_EMAIL), ("password", ONE_TIME_PASSWORD)])
        .send()
        .await?;
    assert_eq!(
        response.status().as_u16(),
        404,
        "no staff login is mounted, and the answer is the honest 404 rather than a 401 that \
         invites a caller to look for a credential this deployment could never issue"
    );
    Ok(())
}

// ----------------------------------------------------------------- test 14

/// **The per-IP half of the sign-in rate limit is per IP**, which until the
/// exp24 review it was not.
///
/// `SignInLimiter::check` takes an `Option<IpAddr>` and counts a `None` under
/// one shared `ip:unknown` key, deliberately, so that removing whatever
/// supplies the address cannot buy an unlimited bucket. The address is
/// supplied by axum's `ConnectInfo`, and `ConnectInfo` is present only when
/// the service is built with `into_make_service_with_connect_info` — which
/// neither `vpay-server` nor this suite's harness did. Every attempt in the
/// process therefore shared **one** ten-per-five-minute budget: ten requests
/// from anywhere locked every staff member out of the dashboard, which is
/// precisely the denial of service `rate_limit`'s own header says a lockout
/// would be, at deployment scale rather than per account.
///
/// The decisive mutation is the fix itself: drop
/// `.into_make_service_with_connect_info::<SocketAddr>()` from
/// `support::serve` and the last assertion below reads `429`.
///
/// Two loopback source addresses against a server bound on `127.0.0.1`. Every
/// burning attempt uses its own address so that the *email* budget can never
/// be what refuses the control.
#[tokio::test]
async fn the_sign_in_rate_limit_is_per_source_address() -> anyhow::Result<()> {
    let harness = harness().await?;

    let from = |ip: [u8; 4]| {
        reqwest::Client::builder()
            .local_address(std::net::IpAddr::from(ip))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a client bound to a loopback source address")
    };

    // Exactly the budget, from one source, on ten distinct addresses.
    for n in 0..10 {
        let response = from([127, 0, 0, 2])
            .post(format!("{}/dash/v1/staff/login", harness.base_url))
            .form(&[
                ("email", format!("burner-{n}@example.test").as_str()),
                ("password", "wrong"),
            ])
            .send()
            .await
            .context("burning one source address's budget")?;
        assert_eq!(
            response.status().as_u16(),
            401,
            "attempt {n} is inside the budget and must be refused on the credential, not the limit"
        );
    }

    // A different source, a different address, its first ever attempt.
    let response = from([127, 0, 0, 3])
        .post(format!("{}/dash/v1/staff/login", harness.base_url))
        .form(&[("email", "innocent@example.test"), ("password", "wrong")])
        .send()
        .await
        .context("the control attempt from a second source address")?;
    assert_eq!(
        response.status().as_u16(),
        401,
        "one source address exhausting its budget must not refuse another: a 429 here means \
         every caller in the deployment shares one bucket and the login is trivially DoS-able"
    );

    // And the exhausted source is still exhausted — the control must not have
    // passed because the limiter stopped working altogether.
    let response = from([127, 0, 0, 2])
        .post(format!("{}/dash/v1/staff/login", harness.base_url))
        .form(&[("email", "burner-0@example.test"), ("password", "wrong")])
        .send()
        .await
        .context("the eleventh attempt from the exhausted source")?;
    assert_eq!(
        response.status().as_u16(),
        429,
        "the source that spent its budget is still over it"
    );
    Ok(())
}

// -------------------------------------------------------------- test 15(b)

/// **The SECOND FACTOR is rate limited too**, which until the exp28 review it
/// was not.
///
/// [ADR-0017](../../../../docs/adr/0017-staff-authentication.md) decision 2
/// says "sign-in is rate limited per email and per IP … failing closed with
/// `429`", and `SignInLimiter::check` was called from `login` and from
/// nowhere else. A TOTP code is **six digits**, `Totp::verify` accepts a
/// one-step skew either side — three live codes at any instant — and the
/// `/staff/totp` path costs no argon2id verification, so a caller holding one
/// password and one `pending_totp` session could guess the second factor at
/// whatever rate the network allowed. Measured against a real stack before
/// the fix: thirty consecutive wrong codes, thirty `401`s, not one `429`.
///
/// The decisive mutation is the fix itself: delete the `limiter.check` from
/// `staff::totp_step` and the assertion below finds no `429` at all.
///
/// Its own source address, for `the_sign_in_rate_limit_is_per_source_address`'
/// reason: the per-IP half of the same budget must not be what refuses these,
/// and no other test's attempts must be able to refuse them either.
///
/// What this does NOT claim: a number. The budget is ten per five minutes per
/// key and the `/staff/login` that produced the session already spent one of
/// them, so the exact index of the first `429` is an implementation detail
/// this test deliberately does not pin — what it pins is that a `429` arrives
/// at all, and inside a number of attempts far below `10^6`.
#[tokio::test]
async fn the_second_factor_is_rate_limited_and_not_only_the_password() -> anyhow::Result<()> {
    let harness = harness().await?;

    let client = reqwest::Client::builder()
        .local_address(std::net::IpAddr::from([127, 0, 0, 4]))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("a client bound to a loopback source address");

    // The password leg, honestly, from this test's own address: a session at
    // `pending_totp` is what an attacker who has phished one password holds.
    let response = client
        .post(format!("{}/dash/v1/staff/login", harness.base_url))
        .form(&[("email", STAFF_EMAIL), ("password", ONE_TIME_PASSWORD)])
        .send()
        .await
        .context("the password leg")?;
    anyhow::ensure!(
        response.status().as_u16() == 200,
        "the password leg must succeed"
    );
    let body: Value = response.json().await.context("the login body")?;
    let session = field(&body, "session")
        .as_str()
        .context("a session token")?
        .to_owned();

    // Wrong codes, one after another. Twelve is comfortably past a budget of
    // ten and is still nothing next to the 10^6 an unbounded second factor
    // costs an attacker.
    let mut statuses = Vec::new();
    for n in 0..12u32 {
        let response = client
            .post(format!("{}/dash/v1/staff/totp", harness.base_url))
            .header(vpay_api::staff::SESSION_HEADER, session.as_str())
            .form(&[("code", format!("{:06}", 100_000 + n).as_str())])
            .send()
            .await
            .context("guessing a second factor")?;
        statuses.push(response.status().as_u16());
    }

    assert_eq!(
        statuses.first().copied(),
        Some(401),
        "the first guess is inside the budget and must be refused on the code, not the limit:          {statuses:?}"
    );
    assert!(
        statuses.contains(&429),
        "twelve consecutive wrong second factors must run into the limit. Without one, six          digits behind a phished password are guessable at line rate: {statuses:?}"
    );
    Ok(())
}

// ----------------------------------------------------------------- test 16

/// **Moving a staff member to another merchant stops their existing token
/// reading the old one**, on the next request rather than at the token's
/// expiry.
///
/// The same shape as the disabled case above, and found by the same question:
/// everything `require_dashboard_token` checked was a statement about the
/// *token*, and none of it was a statement about the *person*.
///
/// `oauth_authorization_codes.merchant_id` is a copy of `staff.merchant_id`
/// taken when the code was issued, and migration `0035`'s own comment says
/// why: "so that a staff row edited between issue and exchange cannot
/// silently move a token to another tenant". That protects the tenant being
/// moved **to**. Nothing protected the tenant being moved **from**, so a
/// staff member reassigned to merchant B went on listing merchant A's
/// payments with the token they already held, for the rest of its 15-minute
/// TTL.
///
/// It refuses nobody who was ever allowed in: `/authorize` will not mint a
/// code for a staff member whose `merchant_id` is not the binding, so any
/// token that exists already satisfied this at issue.
///
/// The decisive mutation: drop `&& staff.merchant_id == binding.merchant_id`
/// from `require_dashboard_token` and the last assertion reads `200`.
#[tokio::test]
async fn moving_a_staff_member_to_another_merchant_refuses_their_existing_token()
-> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_moved_a").await?;
    let (_session, token) = harness.access_token().await?;

    let (status, body) = harness
        .get_json("/dash/v1/payment_intents", None, Some(&token))
        .await?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        field(&body, "data").as_array().map(Vec::len),
        Some(1),
        "the bound merchant's intent is readable before the move: {body}"
    );

    // The operator's `UPDATE`. There is no HTTP endpoint that reassigns a
    // staff member, deliberately — ADR-0017 gives the CLI one subcommand —
    // so this is what an operator's own statement would do.
    sqlx::query("UPDATE staff_members SET merchant_id = $1 WHERE email = $2")
        .bind(MERCHANT_B)
        .bind(STAFF_EMAIL)
        .execute(&harness.repositories.op_store_pool())
        .await
        .context("moving the staff member to the other merchant")?;

    let (status, body) = harness
        .get_json("/dash/v1/payment_intents", None, Some(&token))
        .await?;
    assert_eq!(
        status, 403,
        "a staff member who no longer belongs to this deployment's merchant must stop reading \
         its rows at once, not when their token happens to expire: {body}"
    );
    assert!(
        !body.to_string().contains("pi_moved_a"),
        "and the refusal must carry none of the tenant's data: {body}"
    );
    Ok(())
}
