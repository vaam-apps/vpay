//! `/dash/v1`, end to end: the real `vpay_api::router` on a real socket, over
//! a real Postgres, with a dashboard client bound to exactly one of the two
//! merchants configured.
//!
//! # What this file does NOT claim, stated before anything else
//!
//! **It does not prove that a staff member can sign in, because nobody can.**
//! vpay serves `client_credentials` and nothing else; the PKCE
//! authorization-code grant `docs/flows/dashboard-auth.md` prescribes is not
//! built, and building it needs a decision about how a human authenticates that this
//! repository has never taken (`authkestra_op::handlers::authorize::handle_authorize`
//! takes an already-authenticated `Identity` as a parameter, and there is no
//! staff table, credential store or login page to produce one). So every
//! token below is minted **by the test**, through the shipping mint path
//! (`authkestra_engine::token::TokenManager::issue_client_token`, the same
//! function `handle_client_credentials` calls) using the server's own signing
//! key, which the server then verifies through its own published JWKS over a
//! real socket.
//!
//! That makes this suite a test of the **resource server**: which rows a
//! validly-minted dashboard token may read, and which requests are refused.
//! It is not a test of an issuer, and there is no issuer to test. The day the
//! grant is served, the tokens here should come from it instead and every
//! assertion below should still hold.
//!
//! # What it does claim
//!
//! 1. a dashboard token bound to merchant A lists **A's** intents and not
//!    B's, over a list that contains both merchants' rows;
//! 2. A's dashboard token retrieving **B's** intent by id gets the same
//!    `404`, byte for byte, as a well-formed id that never existed;
//! 3. a **merchant**-audience token — a real `/v1` credential, minted the way
//!    a merchant mints one — is refused on `/dash/v1`;
//! 4. a dashboard-audience token whose `client_id` is not the registered
//!    dashboard client's is refused, even though its audience is right;
//! 5. a dashboard token carrying the wrong scope is refused;
//! 6. an **expired** dashboard token is a `401`;
//! 7. no request without a bearer token reaches any `/dash/v1` route;
//! 8. the detail read carries the charge, the refunds and the event
//!    timeline, and carries **no `client_secret`** — which `GET
//!    /v1/payment_intents/{id}` does carry;
//! 9. the `status` filter narrows the list, and an unknown status is a `400`
//!    naming `status` rather than an empty page;
//! 10. a deployment that registers no `dashboard_client` mounts no nest at
//!     all: `/dash/v1/payment_intents` is the honest `404`, not a `401`;
//! 11. the detail read renders the events and refunds an intent *has*, both
//!     objects' events oldest first, and no other tenant's event — added
//!     2026-09-06 by the review, which measured that both repository reads
//!     could return an empty `Vec` forever with every other case green;
//! 12. a cursor naming **another merchant's** intent positions nothing and
//!     answers exactly what an id nothing ever wrote answers — added
//!     2026-09-06 by the review, which measured that both cursor subqueries
//!     could lose their tenancy predicate with 35 tests still green;
//! 13. a write method is refused by the *boundary*, not by the route table —
//!     added 2026-09-06 by the review, which measured that
//!     `dash::required_scope` could answer the read scope for every method
//!     with all twelve other cases green.
//!
//! # Why raw `reqwest` and no SDK
//!
//! There is no dashboard SDK and there must not be one: `docs/sdks/parity.md`
//! covers merchant surfaces, and a `/dash/v1` method in a merchant SDK would
//! be a merchant credential reaching for a staff surface. The dashboard's own
//! client is the Next.js app's server side, which speaks HTTP.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{
    Config, CurrencyEntry, DASHBOARD_MERCHANT_CLAIM, DashboardClient, Deployment, HostEntry,
    MERCHANT_AUDIENCE, ProviderHost,
};
use vpay_db::{NewPaymentIntent, Repositories, UnitOfWork as _};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

/// The tenant the dashboard is bound to, and the credential that acts for it
/// on `/v1`. Never the same string, for `payment_intents.rs`' reason: a query
/// filtered by `client_id` instead of `merchant_id` would otherwise pass.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The tenant the dashboard is **not** bound to. Its rows exist so that
/// "lists A's intents" is a claim with something to fail against — a suite
/// with one merchant proves nothing about a tenancy filter.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

/// The registered dashboard client, and its single scope.
const DASHBOARD_CLIENT: &str = "vpay-dashboard";
const DASHBOARD_SCOPE: &str = "dashboard:read";

/// The `sub` of every staff token this suite mints.
///
/// A `stf_…`, because that is what the authorization-code grant puts there —
/// and the point of ADR-0017's rename of `ResourceClaims::client_id` to
/// `subject`: under this grant `sub` is a **person**, not a credential, and
/// nothing in `require_dashboard_token` authorises against it any more.
const STAFF_ID: &str = "stf_0000000000000000000dash";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "XAF";
const AMOUNT: i64 = 5000;

/// The recognisable prefix every seeded intent's `client_secret_suffix`
/// carries, so test 8 can grep a rendered body for it.
const SECRET_MARKER: &str = "dashsuffixmarker";

/// How long the tokens this suite mints live. Long enough that no test can
/// fail on wall-clock, short enough to be obviously not a session.
const TOKEN_TTL_SECS: u64 = 300;

// ------------------------------------------------------------------ harness

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    #[allow(dead_code)]
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    #[allow(dead_code)]
    pool: PgPool,
    base_url: String,
    signing_key: LoadedSigningKey,
    /// The server's own private key, PEM-encoded. Held only so
    /// [`Harness::expired_dashboard_token`] can sign one token by hand —
    /// `LoadedSigningKey` deliberately exposes no way to mint a token with an
    /// `exp` in the past, and it should not grow one.
    server_pem: String,
}

impl Harness {
    /// The token a signed-in staff member holds.
    ///
    /// **Rewritten for [ADR-0017]**, and the rewrite is the change this suite
    /// exists to record. It used to be `issue_client_token` — a
    /// `client_credentials` shape, `sub` = the client id, `aud` =
    /// `vpay:dash/v1`, no merchant claim. That token is now **refused**, and
    /// `a_client_credentials_token_is_refused_on_dash_v1` below is what pins
    /// it: no machine client may read the dashboard.
    ///
    /// What replaces it is exactly what
    /// `vpay_api::staff::oauth::token` mints — `issue_user_token_with_extra`,
    /// `sub` = the staff member, `aud` = the dashboard client id, plus the
    /// merchant claim — so what this suite presents is the claim set the
    /// shipping grant produces rather than a hand-assembled object that
    /// happens to validate.
    ///
    /// [ADR-0017]: ../../../../docs/adr/0017-staff-authentication.md
    fn dashboard_token(&self) -> String {
        self.staff_token(DASHBOARD_CLIENT, DASHBOARD_SCOPE, Some(MERCHANT_A))
    }

    /// A staff token, with every part of it a parameter so a test can move
    /// exactly one.
    ///
    /// `merchant` is `Option` because "no merchant claim at all" is a case
    /// with its own test — it is what every `client_credentials` token looks
    /// like.
    fn staff_token(&self, audience: &str, scope: &str, merchant: Option<&str>) -> String {
        self.staff_token_for(STAFF_ID, audience, scope, merchant)
    }

    /// [`Self::staff_token`] with the **subject** a parameter too.
    ///
    /// One caller: the test that presents a token for a `stf_…` no
    /// `staff_members` row names. That is a case only this suite can build —
    /// `staff_sign_in.rs` mints nothing and every subject it presents came
    /// out of a real sign-in, so its staff row exists by construction.
    fn staff_token_for(
        &self,
        subject: &str,
        audience: &str,
        scope: &str,
        merchant: Option<&str>,
    ) -> String {
        let mut extra = std::collections::HashMap::new();
        if let Some(merchant) = merchant {
            extra.insert(
                DASHBOARD_MERCHANT_CLAIM.to_owned(),
                Value::String(merchant.to_owned()),
            );
        }
        self.signing_key
            .token_manager()
            .issue_user_token_with_extra(
                vpay_api::op::dashboard::token_identity_for(subject),
                TOKEN_TTL_SECS,
                Some(scope.to_owned()),
                Some(audience.to_owned()),
                extra,
            )
            .expect("the server's own signing key mints a token")
    }

    /// The `client_credentials` token this surface used to accept, minted
    /// through the shipping function `/v1` uses.
    ///
    /// Two callers: the test that proves such a token is now refused on
    /// `/dash/v1`, and the two that present a real *merchant* credential —
    /// which is the same shape, for a different client and a different
    /// audience.
    fn machine_token(&self, client_id: &str, audience: &str, scope: &str) -> String {
        self.signing_key
            .token_manager()
            .issue_client_token(
                client_id,
                TOKEN_TTL_SECS,
                Some(scope.to_owned()),
                Some(audience.to_owned()),
            )
            .expect("the server's own signing key mints a token")
    }

    /// The one token this suite assembles by hand: `exp` in the past by more
    /// than `jsonwebtoken`'s 60-second default leeway.
    ///
    /// Everything else about it — algorithm, `kid`, issuer, audience,
    /// `client_id`, scope — matches [`Self::dashboard_token`] exactly, so the
    /// only thing the `401` can be about is the expiry.
    fn expired_dashboard_token(&self) -> String {
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": format!("{}/v1/oauth", self.base_url),
            "aud": DASHBOARD_CLIENT,
            "sub": STAFF_ID,
            "scope": DASHBOARD_SCOPE,
            DASHBOARD_MERCHANT_CLAIM: MERCHANT_A,
            "iat": now - 3_600,
            "exp": now - 600,
        });
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(self.signing_key.kid().to_owned());
        jsonwebtoken::encode(
            &header,
            &claims,
            &jsonwebtoken::EncodingKey::from_rsa_pem(self.server_pem.as_bytes())
                .expect("the generated PEM parses as RSA"),
        )
        .expect("signing an expired token succeeds")
    }

    async fn get(&self, path: &str, token: Option<&str>) -> anyhow::Result<(u16, String)> {
        self.request(reqwest::Method::GET, path, token).await
    }

    /// The same, with the method as an argument — for the one test that is
    /// about a method this surface refuses.
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        token: Option<&str>,
    ) -> anyhow::Result<(u16, String)> {
        let mut request = reqwest::Client::builder()
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
            .request(method, format!("{}{path}", self.base_url));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.context("sending a /dash/v1 request")?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((status, body))
    }

    /// `GET` with a dashboard token, decoded — the shape almost every test
    /// below wants.
    async fn dash_json(&self, path: &str) -> anyhow::Result<(u16, Value)> {
        let (status, body) = self.get(path, Some(&self.dashboard_token())).await?;
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value, dashboard: bool) -> Config {
    Config {
        deployment: Deployment {
            name: "dashboard-read-surface".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![ProviderHost {
            code: PUSH_RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
                // Unreachable on purpose: this suite reads rows, it never
                // confirms anything, so no rail is ever called.
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
        // The whole point of the `dashboard` flag: test 10 boots the *same*
        // configuration with this set to `None` and asserts the nest is not
        // mounted. Two configurations that differed in more than this field
        // would make that test's `404` attributable to something else.
        dashboard_client: dashboard.then(|| DashboardClient {
            client_id: DASHBOARD_CLIENT.to_owned(),
            merchant_id: MERCHANT_A.to_owned(),
            redirect_uris: vec![format!("{base_url}/dash/v1/callback")],
            scope: DASHBOARD_SCOPE.to_owned(),
            client_secret: None,
        }),
        // No staff secrets, deliberately: this suite is about the READ
        // surface's boundary and mints its tokens directly, so a staff login
        // mounted beside it would be scenery. `staff_sign_in.rs` is the suite
        // that exercises the grant end to end.
        staff_auth: vpay_config::StaffAuth::default(),
    }
}

async fn harness_with(dashboard: bool) -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;

    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b, dashboard)
    })
    .await?;

    seed_staff(repositories.as_ref()).await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        signing_key: served.signing_key,
        server_pem,
    })
}

async fn harness() -> anyhow::Result<Harness> {
    harness_with(true).await
}

/// Writes the `staff_members` row every token this suite mints names.
///
/// **Required since the exp24 review (findings F1 and F6).**
/// `require_dashboard_token` reads the row its `sub` names and refuses a
/// `disabled` one, a missing one, or one whose `merchant_id` is not the
/// binding — so a token for a `stf_…` nobody created is now a `403`, which is
/// the right answer and which eight tests here were relying on not happening.
///
/// Seeding it is a strengthening rather than a workaround: what this suite
/// presents is meant to be exactly what the authorization-code grant
/// produces, and that grant's `sub` is `oauth_authorization_codes.staff_id`,
/// a column with a foreign key to this table. A token whose subject names no
/// person was never a thing the shipping mint could produce.
///
/// The password hash is a placeholder and is never verified: nothing in this
/// suite signs in. `staff_sign_in.rs` is where a real credential is exercised.
async fn seed_staff(repositories: &dyn Repositories) -> anyhow::Result<()> {
    vpay_db::Staff::create(
        repositories,
        vpay_db::NewStaff {
            id: STAFF_ID.to_owned(),
            merchant_id: MERCHANT_A.to_owned(),
            email: "dash-reader@example.test".to_owned(),
            display_name: "Dash Reader".to_owned(),
            // Never verified here — see this function's doc.
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$notarealhash"
                .to_owned(),
            now: time::OffsetDateTime::now_utc(),
        },
    )
    .await
    .context("seeding the staff member every token in this suite names")?;
    Ok(())
}

/// Writes one payment intent directly through the repository.
///
/// Not through `POST /v1/payment_intents`: this suite's subject is the read
/// surface, and creating through the merchant API would make every test
/// depend on the merchant SDK's credential flow as well. The rows are the
/// same rows — the same `NewPaymentIntent` the create handler builds.
async fn seed_intent(
    repositories: &dyn Repositories,
    merchant_id: &str,
    id: &str,
) -> anyhow::Result<()> {
    repositories
        .insert(&NewPaymentIntent {
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
            // A recognisable literal, unlike `support::confirmed_intent`'s
            // generated suffix, and deliberately: test 8 greps the rendered
            // body for `SECRET_MARKER`, so a fixture whose secret nothing
            // could recognise would make "no client_secret was leaked" a
            // claim that passes whether the renderer leaks or not. Padded to
            // migration `0026`'s 32-character floor, which exists because a
            // short suffix is a guessable credential.
            client_secret_suffix: format!("{SECRET_MARKER}{id:x<32}"),
            created_at: time::OffsetDateTime::now_utc(),
        })
        .await
        .with_context(|| format!("seeding {id} for {merchant_id}"))?;
    Ok(())
}

/// One field of a JSON body, by RFC 6901 pointer, panicking with the pointer
/// when it is absent.
///
/// Not `body["data"]`: `clippy::indexing_slicing` is denied in these suites,
/// and the lint is right here for a reason beyond panics — `serde_json`'s
/// `Index` impl answers `Value::Null` for a key that is not there, so an
/// assertion against a *mistyped* path compares `Null` with `Null` and
/// passes. A missing field must fail the test that names it.
fn at<'a>(body: &'a Value, pointer: &str) -> &'a Value {
    body.pointer(pointer)
        .unwrap_or_else(|| panic!("{pointer} is absent from {body}"))
}

// ------------------------------------------------------------------ test 1

/// A dashboard token sees its own merchant's intents and none of the other's.
///
/// The decisive mutation: make `require_dashboard_token` insert a
/// `MerchantScope` built from anything other than
/// `DashboardBinding::merchant_id` — the token's own `client_id`, say — and
/// this fails, because `vpay-dashboard` is not a `merchant_id` any row
/// carries and the list comes back empty.
#[tokio::test]
async fn the_dashboard_lists_only_the_merchant_it_is_bound_to() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_dash_a_one").await?;
    seed_intent(repositories, MERCHANT_A, "pi_dash_a_two").await?;
    seed_intent(repositories, MERCHANT_B, "pi_dash_b_one").await?;

    let (status, body) = harness.dash_json("/dash/v1/payment_intents").await?;
    assert_eq!(status, 200, "{body}");

    let ids: Vec<&str> = at(&body, "/data")
        .as_array()
        .expect("the list envelope carries an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();

    assert_eq!(ids.len(), 2, "{body}");
    assert!(ids.contains(&"pi_dash_a_one"), "{body}");
    assert!(ids.contains(&"pi_dash_a_two"), "{body}");
    assert!(
        !ids.contains(&"pi_dash_b_one"),
        "the dashboard is bound to {MERCHANT_A} and must not see {MERCHANT_B}'s rows: {body}"
    );
    assert_eq!(at(&body, "/url"), "/dash/v1/payment_intents", "{body}");

    // Over the serialised body, for the reason
    // `the_detail_read_carries_the_charge_and_never_the_client_secret` gives:
    // `PaymentIntentObject` has no such field, but rendering
    // `PaymentIntentWithSecret` here instead is one word that still compiles,
    // and the *list* had no assertion against it until 2026-09-06. Both
    // seeded intents carry `SECRET_MARKER` in their suffix, so there is
    // something to leak.
    let rendered = body.to_string();
    assert!(
        !rendered.contains("client_secret") && !rendered.contains(SECRET_MARKER),
        "the staff payments list must not render the payer credential: {rendered}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 2

/// Another merchant's id and an id that never existed are the **same**
/// answer, byte for byte.
///
/// Byte-for-byte and not merely "both 404": a body that named the resource
/// differently, or echoed a different id, would let anyone holding a
/// dashboard token enumerate which payment intents exist on the other tenant.
/// That is exactly the property `/v1`'s own test 8 pins, applied to the
/// surface that has a *fixed* tenant rather than a resolved one.
#[tokio::test]
async fn another_merchants_intent_is_indistinguishable_from_one_that_never_existed()
-> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_B, "pi_dash_b_only").await?;

    let token = harness.dashboard_token();
    let (foreign_status, foreign_body) = harness
        .get("/dash/v1/payment_intents/pi_dash_b_only", Some(&token))
        .await?;
    let (absent_status, absent_body) = harness
        .get("/dash/v1/payment_intents/pi_dash_b_only", Some(&token))
        .await?;

    assert_eq!(foreign_status, 404, "{foreign_body}");
    assert_eq!(absent_status, 404, "{absent_body}");

    // The same id in both requests, so the bodies are comparable: what is
    // being asserted is that the answer does not depend on whether the row
    // exists. A second request with a *different* id would differ in the
    // echoed id alone and prove nothing.
    let (never_status, never_body) = harness
        .get(
            "/dash/v1/payment_intents/pi_never_written_at_all",
            Some(&token),
        )
        .await?;
    assert_eq!(never_status, 404, "{never_body}");
    assert_eq!(
        foreign_body.replace("pi_dash_b_only", "X"),
        never_body.replace("pi_never_written_at_all", "X"),
        "the two 404s differ in more than the id they echo"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 3

/// A merchant-audience token is refused on `/dash/v1`.
///
/// The other half of `merchant_auth.md`'s
/// `a_dashboard_audience_token_is_refused_on_v1`, which has had no partner
/// since it was written because no `/dash/v1` route existed to refuse
/// anything.
///
/// The decisive mutation: build the dashboard nest's validator with
/// `vpay_config::MERCHANT_AUDIENCE`, or give one validator both audiences,
/// and this passes a `/v1` credential straight into the staff surface.
#[tokio::test]
async fn a_merchant_audience_token_is_refused_on_dash_v1() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    // A real `/v1` credential: merchant A's own client id, the `/v1`
    // audience, the scopes `merchant_client` registers.
    let merchant_token =
        harness.machine_token(CLIENT_A, MERCHANT_AUDIENCE, "payments:read payments:write");

    let (status, body) = harness
        .get("/dash/v1/payment_intents", Some(&merchant_token))
        .await?;
    assert_eq!(
        status, 401,
        "a merchant credential must not read the staff surface: {body}"
    );

    // And the same token still works where it belongs, so the 401 above is
    // about the surface rather than about a token this test minted wrongly.
    let (v1_status, v1_body) = harness
        .get("/v1/payment_intents", Some(&merchant_token))
        .await?;
    assert_eq!(v1_status, 200, "{v1_body}");
    Ok(())
}

// ------------------------------------------------------------------ test 4

/// A token minted for **another** dashboard client is refused.
///
/// This is the check that stops "one dashboard client per tenant" from being
/// a promise the code does not keep. The token below is correct in every
/// other respect — vpay's own signature, the dashboard scope, the merchant
/// claim, unexpired — and its `aud` is a different client.
///
/// **Where the refusal comes from changed with ADR-0017 and the test is kept
/// deliberately.** It used to be `require_dashboard_token`'s
/// `claims.client_id != binding.client_id` arm, comparing the token's `sub`.
/// The audience *is* the client id now, so the refusal is
/// `JwtValidator`'s own `set_audience` and the answer is a `401` rather than
/// a `403`. The property is the same and the mutation is different: build
/// the validator with any other audience and this returns `200` with
/// merchant A's payments.
#[tokio::test]
async fn a_dashboard_token_for_an_unregistered_client_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    let impostor = harness.staff_token("some-other-dashboard", DASHBOARD_SCOPE, Some(MERCHANT_A));
    let (status, body) = harness
        .get("/dash/v1/payment_intents", Some(&impostor))
        .await?;

    assert_eq!(status, 401, "{body}");
    assert!(
        !body.contains("pi_dash_a_only"),
        "a refused request must not carry rows: {body}"
    );
    Ok(())
}

/// **No machine client may read the dashboard** (ADR-0017 decision 3).
///
/// A `client_credentials` token minted for the registered dashboard client,
/// with the registered scope and the right audience, is refused — because it
/// carries no merchant claim, and nothing but the staff authorization-code
/// grant stamps one. That makes the refusal a property of the mint rather
/// than of a list somebody maintains.
///
/// **This is a tightening**, and it is the one behaviour change ADR-0017
/// makes to a surface that already worked: before it, a token of exactly this
/// shape was the *only* thing `/dash/v1` accepted.
///
/// The decisive mutation: delete the `claims.merchant` arm from
/// `require_dashboard_token` and this returns `200` with merchant A's
/// payments.
#[tokio::test]
async fn a_client_credentials_token_is_refused_on_dash_v1() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    let machine = harness.machine_token(DASHBOARD_CLIENT, DASHBOARD_CLIENT, DASHBOARD_SCOPE);
    let (status, body) = harness
        .get("/dash/v1/payment_intents", Some(&machine))
        .await?;

    assert_eq!(status, 403, "{body}");
    assert!(
        !body.contains("pi_dash_a_only"),
        "a refused request must not carry rows: {body}"
    );
    Ok(())
}

/// A staff token whose merchant claim is **another tenant** is refused.
///
/// The claim is compared against `dashboard_client.merchant_id`, never used
/// as the tenant — so this test's failure mode without the check is a `200`
/// carrying merchant **A**'s rows, not merchant B's. That is the point worth
/// pinning: the claim is a second lock on the same door, and a forged one
/// buys a `403` rather than another merchant's data.
#[tokio::test]
async fn a_token_whose_merchant_claim_is_not_the_binding_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    let wrong_tenant = harness.staff_token(DASHBOARD_CLIENT, DASHBOARD_SCOPE, Some(MERCHANT_B));
    let (status, body) = harness
        .get("/dash/v1/payment_intents", Some(&wrong_tenant))
        .await?;

    assert_eq!(status, 403, "{body}");
    assert!(!body.contains("pi_dash_a_only"), "{body}");
    Ok(())
}

// ------------------------------------------------------------------ test 5

/// A dashboard token without the registration's scope is refused.
///
/// The decisive mutation: delete the `claims.has_scope(required_scope)` arm
/// and this returns `200`.
#[tokio::test]
async fn a_dashboard_token_without_the_registered_scope_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;

    // A plausible wrong scope rather than an empty one: an empty `scope`
    // claim is also refused by a `has_scope` that was deleted, because
    // `ResourceClaims::has_scope` on an empty string is false either way —
    // but a *populated, wrong* scope is what a real misconfiguration looks
    // like, and it is the case a naive "the token has some scope" check
    // would let through.
    let wrong_scope = harness.staff_token(DASHBOARD_CLIENT, "payments:write", Some(MERCHANT_A));
    let (status, body) = harness
        .get("/dash/v1/payment_intents", Some(&wrong_scope))
        .await?;

    assert_eq!(status, 403, "{body}");
    Ok(())
}

// ------------------------------------------------------------------ test 6

/// An expired dashboard token is a `401`.
///
/// `docs/flows/dashboard-auth.md` has no revocation endpoint and mitigates
/// that with a short access-token TTL — a mitigation that is worth exactly as
/// much as the expiry check behind it.
#[tokio::test]
async fn an_expired_dashboard_token_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;

    let (status, body) = harness
        .get(
            "/dash/v1/payment_intents",
            Some(&harness.expired_dashboard_token()),
        )
        .await?;
    assert_eq!(status, 401, "{body}");

    // The same claims, unexpired, are accepted — so the 401 is the expiry
    // and not something else about a hand-assembled token.
    let (fresh_status, fresh_body) = harness
        .get("/dash/v1/payment_intents", Some(&harness.dashboard_token()))
        .await?;
    assert_eq!(fresh_status, 200, "{fresh_body}");
    Ok(())
}

// ------------------------------------------------------------------ test 7

/// Every route in `DASH_ROUTES` refuses an unauthenticated caller.
///
/// Walks the constant rather than listing paths, for `V1_ROUTES`' reason: a
/// route added to the table without a token check is what this catches, and a
/// hand-written list would simply not mention it.
#[tokio::test]
async fn no_dash_route_is_reachable_without_a_token() -> anyhow::Result<()> {
    let harness = harness().await?;

    for route in vpay_api::DASH_ROUTES {
        // `{id}` is a path parameter; any well-formed value does, because the
        // refusal happens before the router matches.
        let path = format!(
            "{}{}",
            vpay_api::DASH_NEST,
            route.path.replace("{id}", "pi_anything")
        );
        let (status, body) = harness.get(&path, None).await?;
        assert_eq!(status, 401, "{path} answered {status}: {body}");
    }
    Ok(())
}

// ------------------------------------------------------------------ test 8

/// The detail read assembles the charge, the refunds and the timeline — and
/// never renders a `client_secret`, which the merchant surface's own
/// `retrieve` does.
///
/// The two halves are asserted together on purpose: a detail endpoint that
/// returned the *merchant* rendering would pass a "does it have a charge"
/// test and would be handing a staff surface the credential that confirms
/// the payment from any browser.
#[tokio::test]
async fn the_detail_read_carries_the_charge_and_never_the_client_secret() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();
    seed_intent(repositories, MERCHANT_A, "pi_dash_detail").await?;

    // Through the same `TxRepositories::insert_for_intent` the confirm path
    // uses, in a transaction, because that is the only way `charges` is ever
    // written.
    let charge_id = "ch_dash_detail".to_owned();
    repositories
        .transaction(|tx| {
            let charge_id = charge_id.clone();
            Box::pin(async move {
                tx.insert_for_intent(&vpay_db::NewCharge {
                    id: charge_id,
                    payment_intent_id: "pi_dash_detail".to_owned(),
                    provider_code: PUSH_RAIL.to_owned(),
                    provider_reference_id: uuid::Uuid::new_v4(),
                    provider_ref_extra: None,
                    redirect_url: None,
                    return_url: None,
                    state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
                    amount: AMOUNT,
                    currency_code: CURRENCY.to_owned(),
                    payer_ref: None,
                    // `None`, because nothing in this system writes it — see
                    // `vpay_api::dash::payment_intents::ChargeSummary`.
                    // Written out rather than left implicit so the day it
                    // *is* written, this fixture is where a reader looks.
                    payer_ref_masked: None,
                })
                .await
                .context("seeding a charge")?;
                Ok::<_, anyhow::Error>(vpay_db::TxOutcome::Commit(()))
            })
        })
        .await?;

    let (status, body) = harness
        .dash_json("/dash/v1/payment_intents/pi_dash_detail")
        .await?;
    assert_eq!(status, 200, "{body}");

    assert_eq!(at(&body, "/object"), "dashboard.payment_detail", "{body}");
    assert_eq!(at(&body, "/payment_intent/id"), "pi_dash_detail", "{body}");
    assert_eq!(at(&body, "/charge/id"), "ch_dash_detail", "{body}");
    assert_eq!(at(&body, "/charge/provider_code"), PUSH_RAIL, "{body}");
    assert_eq!(
        at(&body, "/charge/payer_ref_masked"),
        &Value::Null,
        "{body}"
    );
    assert_eq!(at(&body, "/refunds"), &serde_json::json!([]), "{body}");
    assert_eq!(at(&body, "/events"), &serde_json::json!([]), "{body}");

    let rendered = body.to_string();
    assert!(
        !rendered.contains("client_secret"),
        "the staff surface must not render the payer credential: {rendered}"
    );
    assert!(
        !rendered.contains(SECRET_MARKER),
        "nor the suffix it is built from: {rendered}"
    );

    // The *merchant* surface does render it, from the same row — so the
    // assertion above is about this handler's choice and not about the row
    // having no secret to leak.
    let merchant_token = harness.machine_token(CLIENT_A, MERCHANT_AUDIENCE, "payments:read");
    let (v1_status, v1_body) = harness
        .get("/v1/payment_intents/pi_dash_detail", Some(&merchant_token))
        .await?;
    assert_eq!(v1_status, 200, "{v1_body}");
    assert!(
        v1_body.contains(SECRET_MARKER),
        "the merchant surface is expected to carry it: {v1_body}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 9

/// The `status` filter narrows the page, and an unknown status is a `400`
/// naming `status`.
///
/// The second half is the one worth having: a filter that passed an unknown
/// value through to the `WHERE` clause would answer an empty list, and "no
/// payments are `succeded`" is a sentence an operator reads as an answer
/// about their payments rather than about their typo.
#[tokio::test]
async fn the_status_filter_narrows_the_page_and_refuses_an_unknown_status() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_dash_open_one").await?;
    seed_intent(repositories, MERCHANT_A, "pi_dash_open_two").await?;
    seed_intent(repositories, MERCHANT_A, "pi_dash_canceled").await?;
    repositories
        .transition(
            MERCHANT_A,
            "pi_dash_canceled",
            vpay_core::IntentStatus::INITIAL.as_wire_str(),
            vpay_core::IntentStatus::Canceled.as_wire_str(),
        )
        .await
        .context("cancelling one intent so the filter has two groups")?
        .expect("the transition fires");

    let (status, body) = harness
        .dash_json("/dash/v1/payment_intents?status=canceled")
        .await?;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = at(&body, "/data")
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, vec!["pi_dash_canceled"], "{body}");

    let (unknown_status, unknown_body) = harness
        .dash_json("/dash/v1/payment_intents?status=succeded")
        .await?;
    assert_eq!(unknown_status, 400, "{unknown_body}");
    assert_eq!(
        at(&unknown_body, "/error/param"),
        "status",
        "{unknown_body}"
    );
    Ok(())
}

// ----------------------------------------------------------------- test 10

/// A deployment that registers no `dashboard_client` mounts no `/dash/v1`
/// nest, so every path under it is the honest `404`.
///
/// `404` and not `401`: a `401` tells a caller that a credential would help,
/// and this deployment could never issue one. It is also the only way an
/// operator can tell "I forgot to configure the dashboard" apart from "my
/// token is wrong".
///
/// # What mutation testing found here, recorded rather than smoothed over
///
/// Mounting the nest unconditionally does **not** make this fail. Two
/// independent guards produce the same `404` — `vpay_api::router` does not
/// mount the nest, and `require_dashboard_token` answers `not_found` when
/// either the validator or the binding is absent — so removing one leaves
/// the other. Removing *both* (unconditional mount, plus that guard weakened
/// to a `401`) does fail this test, which was measured on 2026-09-06.
///
/// That redundancy is deliberate and is why neither is deleted: a router
/// assembled by a future binary is not obliged to consult the first, and the
/// second is the one that holds if it does not.
#[tokio::test]
async fn a_deployment_with_no_dashboard_client_mounts_no_dash_nest() -> anyhow::Result<()> {
    let harness = harness_with(false).await?;
    seed_intent(
        harness.repositories.as_ref(),
        MERCHANT_A,
        "pi_dash_unmounted",
    )
    .await?;

    // With no token at all — a mounted nest would answer 401 here.
    let (anonymous, anonymous_body) = harness.get("/dash/v1/payment_intents", None).await?;
    assert_eq!(anonymous, 404, "{anonymous_body}");

    // And with a token that would be perfectly valid on a dashboarded
    // deployment, so the 404 is about the nest and not about the credential.
    let (with_token, with_token_body) = harness
        .get("/dash/v1/payment_intents", Some(&harness.dashboard_token()))
        .await?;
    assert_eq!(with_token, 404, "{with_token_body}");
    Ok(())
}

// ----------------------------------------------------------------- test 11

/// The detail read renders the events and the refunds an intent actually
/// has — and renders **only this tenant's** events.
///
/// # Why this test exists
///
/// Every other assertion in this file about `refunds` and `events` is
/// `== []`, over fixtures that have neither. Mutation testing on 2026-09-06
/// measured the consequence: making **both** `Events::list_for_objects` and
/// `Refunds::list_for_intent` `return Ok(Vec::new())` unconditionally left
/// all ten tests green. Two repository methods whose only caller is the
/// detail route could answer "nothing happened to this payment" forever, and
/// `AGENTS.md` rule 2 names that shape by hand: unwritten code "never returns
/// a plausible-looking success, an empty list, or a zero".
///
/// # The second half, and why it needs a row nothing writes
///
/// `events.object_id` is a plain `TEXT NOT NULL` with **no foreign key**
/// (migration `0018`) that points into three different tables depending on
/// `type`. So `list_for_objects`' `merchant_id = $1` predicate is the *only*
/// thing keeping another tenant's event out of this tenant's timeline, and
/// the only way to exercise it is a row no code path produces: an event owned
/// by `MERCHANT_B` whose `object_id` is `MERCHANT_A`'s intent. It is written
/// through the shipping `TxRepositories::insert_in_tx` — the row is real, its
/// `merchant_id` is simply the other one.
///
/// `Refunds::list_for_intent`'s own `p.merchant_id = $1` cannot be mutated
/// into a failure and is deliberately not asserted: `refunds.payment_intent_id`
/// is a real foreign key onto `payment_intents(id)`, so a refund reachable
/// from A's intent *is* A's. That predicate is defence in depth, and saying so
/// is more honest than a test that would pass with it deleted.
///
/// The decisive mutations: `Ok(Vec::new())` from either repository method,
/// or dropping `merchant_id = $1` from `Events::list_for_objects`.
#[tokio::test]
async fn the_detail_read_renders_the_timeline_and_the_refunds_it_has() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_dash_timeline").await?;
    seed_intent(repositories, MERCHANT_B, "pi_dash_timeline_b").await?;

    let charge_id = "ch_dash_timeline".to_owned();
    repositories
        .transaction(|tx| {
            let charge_id = charge_id.clone();
            Box::pin(async move {
                tx.insert_for_intent(&vpay_db::NewCharge {
                    id: charge_id.clone(),
                    payment_intent_id: "pi_dash_timeline".to_owned(),
                    provider_code: PUSH_RAIL.to_owned(),
                    provider_reference_id: uuid::Uuid::new_v4(),
                    provider_ref_extra: None,
                    redirect_url: None,
                    return_url: None,
                    state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
                    amount: AMOUNT,
                    currency_code: CURRENCY.to_owned(),
                    payer_ref: None,
                    payer_ref_masked: None,
                })
                .await
                .context("seeding a charge")?;

                // One event about the intent and one about the *charge* —
                // the two-object case `Events::list_for_objects` takes a
                // slice for. A timeline that asked only about the intent
                // would render the first and silently drop the second.
                for (id, event_type, object_id) in [
                    (
                        "evt_dash_timeline_created",
                        "payment_intent.created",
                        "pi_dash_timeline",
                    ),
                    (
                        "evt_dash_timeline_refunded",
                        "charge.refunded",
                        charge_id.as_str(),
                    ),
                ] {
                    tx.insert_in_tx(&vpay_db::NewEvent {
                        id: id.to_owned(),
                        merchant_id: MERCHANT_A.to_owned(),
                        livemode: false,
                        event_type: event_type.to_owned(),
                        object_id: object_id.to_owned(),
                        data: serde_json::json!({ "object": { "id": object_id } }),
                    })
                    .await
                    .context("seeding an event")?;
                }

                // The row no code path produces: MERCHANT_B's event pointing
                // at MERCHANT_A's intent. See this test's doc comment.
                tx.insert_in_tx(&vpay_db::NewEvent {
                    id: "evt_dash_timeline_foreign".to_owned(),
                    merchant_id: MERCHANT_B.to_owned(),
                    livemode: false,
                    event_type: "payment_intent.canceled".to_owned(),
                    object_id: "pi_dash_timeline".to_owned(),
                    data: serde_json::json!({ "object": { "id": "pi_dash_timeline" } }),
                })
                .await
                .context("seeding the foreign event")?;

                Ok::<_, anyhow::Error>(vpay_db::TxOutcome::Commit(()))
            })
        })
        .await?;

    // `refunds` has no writer anywhere in this workspace — `vpay_db::refunds`
    // is a read-only module and says why — so the row goes in the way
    // `tests/refunds.rs` puts one in.
    for (id, intent) in [
        ("re_dash_timeline_one", "pi_dash_timeline"),
        ("re_dash_timeline_two", "pi_dash_timeline"),
        // The other tenant's refund, on the other tenant's intent: it must
        // not appear, and it is what makes "two refunds" a count rather than
        // "every refund in the table".
        ("re_dash_timeline_b", "pi_dash_timeline_b"),
    ] {
        sqlx::query(
            "INSERT INTO refunds (id, payment_intent_id, amount, currency_code, status, metadata) \
             VALUES ($1, $2, $3, 'XAF', 'pending'::refund_status, '{}'::jsonb)",
        )
        .bind(id)
        .bind(intent)
        .bind(AMOUNT)
        .execute(&harness.pool)
        .await
        .with_context(|| format!("seeding refund {id}"))?;
    }

    let (status, body) = harness
        .dash_json("/dash/v1/payment_intents/pi_dash_timeline")
        .await?;
    assert_eq!(status, 200, "{body}");

    let event_ids: Vec<&str> = at(&body, "/events")
        .as_array()
        .expect("the timeline is an array")
        .iter()
        .filter_map(|event| event.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        event_ids,
        vec!["evt_dash_timeline_created", "evt_dash_timeline_refunded"],
        "the timeline must carry both objects' events, oldest first, and \
         nothing belonging to {MERCHANT_B}: {body}"
    );
    assert_eq!(
        at(&body, "/events/1/type"),
        "charge.refunded",
        "the charge's event must be rendered with its own type: {body}"
    );
    assert_eq!(
        at(&body, "/events/1/object_id"),
        "ch_dash_timeline",
        "{body}"
    );

    let refund_ids: Vec<&str> = at(&body, "/refunds")
        .as_array()
        .expect("the refunds are an array")
        .iter()
        .filter_map(|refund| refund.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        refund_ids,
        vec!["re_dash_timeline_one", "re_dash_timeline_two"],
        "{body}"
    );

    Ok(())
}

// ----------------------------------------------------------------- test 12

/// A cursor is resolved **inside the bound tenant**: one naming another
/// merchant's intent positions nothing, and answers an empty page rather
/// than a slice of this merchant's list.
///
/// # Why this is worth a test on this surface
///
/// `another_merchants_intent_is_indistinguishable_from_one_that_never_existed`
/// pins that property for `retrieve`. It holds for the *list* only because
/// both cursor subqueries in `PaymentIntents::list_page_filtered` carry
/// `AND merchant_id = $1`, so a foreign id resolves to `NULL`, `seq < NULL`
/// is `NULL`, and no row matches. Drop that predicate and the subquery
/// resolves to the other tenant's `seq` instead: the page comes back
/// populated with *this* tenant's rows, which turns `starting_after` into an
/// existence oracle for ids the caller may not read — the same thing
/// `retrieve` refuses to be, reached by a different door.
///
/// Measured on 2026-09-06: with that predicate removed from both subqueries,
/// all 35 tests in `dashboard_read_surface` and `payment_intents` passed. The
/// predicate is **pre-existing and unchanged** — byte for byte what `3694e34`
/// shipped — and `/v1` had no test for it either; it is pinned here because
/// `/dash/v1` is the second surface now standing on that statement.
///
/// The ids are generated by `vpay_core::ids::payment_intent_id` rather than
/// spelled out like every other fixture in this file, because a cursor —
/// unlike a path parameter — is checked for well-formedness before it reaches
/// SQL (`v1::paging::validated_cursor`), and a readable id would be a `400`
/// that never exercised the tenancy predicate at all.
///
/// The decisive mutation: delete `AND merchant_id = $1` from either cursor
/// subquery in `list_page_filtered`.
#[tokio::test]
async fn a_cursor_naming_another_merchants_intent_answers_an_empty_page() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    // B's row first, so its `seq` is the *lowest*: a `starting_after` that
    // resolved to it would answer every one of A's rows, which is the
    // loudest form the leak can take and the one this asserts against.
    let b_intent = vpay_core::ids::payment_intent_id();
    let a_older = vpay_core::ids::payment_intent_id();
    let a_newer = vpay_core::ids::payment_intent_id();
    seed_intent(repositories, MERCHANT_B, &b_intent).await?;
    seed_intent(repositories, MERCHANT_A, &a_older).await?;
    seed_intent(repositories, MERCHANT_A, &a_newer).await?;

    // The control: A's own cursor pages A's own list, so the empty pages
    // below are about the tenant and not about cursors being broken.
    let (own_status, own_body) = harness
        .dash_json(&format!(
            "/dash/v1/payment_intents?starting_after={a_newer}"
        ))
        .await?;
    assert_eq!(own_status, 200, "{own_body}");
    let own_ids: Vec<&str> = at(&own_body, "/data")
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(own_ids, vec![a_older.as_str()], "{own_body}");

    // A well-formed id nothing ever wrote, so "the other tenant's row" and
    // "no such row" are held to the same answer here too.
    let never_written = vpay_core::ids::payment_intent_id();

    for cursor in [&b_intent, &never_written] {
        for param in ["starting_after", "ending_before"] {
            let (status, body) = harness
                .dash_json(&format!("/dash/v1/payment_intents?{param}={cursor}"))
                .await?;
            assert_eq!(status, 200, "{param}={cursor}: {body}");
            assert_eq!(
                at(&body, "/data"),
                &serde_json::json!([]),
                "{param}={cursor} must position nothing and answer an empty page"
            );
        }
    }

    // And the two answers are the same answer, so the list cannot be used to
    // learn whether an id exists on another tenant.
    let (foreign, foreign_body) = harness
        .get(
            &format!("/dash/v1/payment_intents?starting_after={b_intent}"),
            Some(&harness.dashboard_token()),
        )
        .await?;
    let (absent, absent_body) = harness
        .get(
            &format!("/dash/v1/payment_intents?starting_after={never_written}"),
            Some(&harness.dashboard_token()),
        )
        .await?;
    assert_eq!((foreign, absent), (200, 200));
    assert_eq!(
        foreign_body, absent_body,
        "another tenant's cursor must answer exactly what an unwritten one does"
    );
    Ok(())
}

// ----------------------------------------------------------------- test 13

/// A write method is refused by `require_dashboard_token` itself, with a
/// credential that is valid in every other respect — not by the route table
/// happening to mount no `post(..)`.
///
/// # Why the distinction is the whole point
///
/// `dash/mod.rs`, `require_dashboard_token`'s own doc,
/// `docs/flows/dashboard.md`, `docs/reference/vpay-api.md` and
/// `docs/status.md` all say a non-read method is refused **before the router
/// matches**, and that this is what makes "the dashboard is read-only"
/// structural rather than a promise: ADR-0008 requires an `audit_log` row per
/// dashboard write and none exists, so a write must not reach a handler at
/// all. Mutation testing on 2026-09-06 found that claim untested — making
/// `dash::required_scope` answer the registration's scope for *every* method
/// left all twelve other cases green, because axum then answered `405` from
/// the route table and no assertion could tell the two apart.
///
/// `403` is what distinguishes them, and it is deliberate: `405` would be the
/// route table's answer ("wrong method for this path"), `403` is the
/// boundary's ("you may not write here at all"). Asserted on a path that
/// **is** mounted, so a `405` would be the honest alternative answer and this
/// really is a choice between the two.
///
/// The decisive mutation: make `dash::required_scope` return
/// `Some(binding.scope.as_str())` for every method — this then reads `405`.
#[tokio::test]
async fn a_write_method_is_refused_by_the_boundary_not_by_the_route_table() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(
        harness.repositories.as_ref(),
        MERCHANT_A,
        "pi_dash_readonly",
    )
    .await?;

    let token = harness.dashboard_token();
    for method in [
        reqwest::Method::POST,
        reqwest::Method::PUT,
        reqwest::Method::PATCH,
        reqwest::Method::DELETE,
    ] {
        for path in [
            "/dash/v1/payment_intents",
            "/dash/v1/payment_intents/pi_dash_readonly",
        ] {
            let (status, body) = harness.request(method.clone(), path, Some(&token)).await?;
            assert_eq!(
                status, 403,
                "{method} {path} must be refused by the boundary (403), not by the route \
                 table (405) and not answered: {body}"
            );
            assert!(
                !body.contains("pi_dash_readonly") || path.contains("pi_dash_readonly"),
                "a refused request must not carry rows: {body}"
            );
        }
    }

    // The same credential still reads, so the 403s are about the method.
    let (read_status, read_body) = harness
        .get("/dash/v1/payment_intents", Some(&token))
        .await?;
    assert_eq!(read_status, 200, "{read_body}");

    // And `HEAD` — which axum answers from the same `get(..)` handler — is a
    // read, so it is not caught by the refusal.
    let (head_status, _) = harness
        .request(
            reqwest::Method::HEAD,
            "/dash/v1/payment_intents",
            Some(&token),
        )
        .await?;
    assert_eq!(head_status, 200);
    Ok(())
}

// ------------------------------------------------------------------ test 16

/// **A validly signed, in-audience, in-tenant, in-scope token whose `sub`
/// names no staff member is refused.**
///
/// Added by the exp24 review (findings F1 and F6). `require_dashboard_token`
/// reads the `staff_members` row its `sub` names, and this is the third of
/// the three cases that read shares one answer with — the other two are a
/// `disabled` row and a row belonging to another merchant, both of which
/// `staff_sign_in.rs` drives through a real sign-in.
///
/// This one can only be built here, by minting: a subject with no row is not
/// something the shipping grant can produce, because
/// `oauth_authorization_codes.staff_id` has a foreign key to the table. It is
/// worth pinning anyway, and for a reason the delivered surface makes plain —
/// **this whole suite minted such tokens for fifteen tests and nothing
/// noticed**, because until the review nothing on this path read
/// `staff_members` at all. The refusal is what a deleted account looks like.
///
/// The three assertions that make it decisive: the control token works, the
/// unknown subject is refused, and the refusal carries none of the tenant's
/// rows.
#[tokio::test]
async fn a_token_whose_subject_names_no_staff_member_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    // The control: everything about this token is right, including its
    // subject, and it reads.
    let (status, body) = harness.dash_json("/dash/v1/payment_intents").await?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body.get("data").and_then(Value::as_array).map(Vec::len),
        Some(1),
        "the control reads the bound merchant's intent: {body}"
    );

    // One thing moved: a subject nobody created.
    let orphan = harness.staff_token_for(
        "stf_0000000000000000000ghost",
        DASHBOARD_CLIENT,
        DASHBOARD_SCOPE,
        Some(MERCHANT_A),
    );
    let (status, body) = harness
        .get("/dash/v1/payment_intents", Some(&orphan))
        .await?;
    assert_eq!(
        status, 403,
        "a token naming a staff member who does not exist must be refused: a deleted account's \
         credential is not a credential: {body}"
    );
    assert!(
        !body.contains("pi_dash_a_only"),
        "and the refusal carries none of the tenant's rows: {body}"
    );
    Ok(())
}
