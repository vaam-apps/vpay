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
//!     could return an empty `Vec` forever with every other case green.
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
    Config, CurrencyEntry, DASHBOARD_AUDIENCE, DashboardClient, Deployment, HostEntry,
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
    /// A token the registered dashboard client would hold, if anything could
    /// issue it — see this file's header.
    ///
    /// Minted through `TokenManager::issue_client_token`, the shipping
    /// function, so the claim set is the one vpay's own OP produces rather
    /// than a hand-assembled JSON object that happens to validate.
    fn dashboard_token(&self) -> String {
        self.token(DASHBOARD_CLIENT, DASHBOARD_AUDIENCE, DASHBOARD_SCOPE)
    }

    fn token(&self, client_id: &str, audience: &str, scope: &str) -> String {
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
            "aud": DASHBOARD_AUDIENCE,
            "sub": DASHBOARD_CLIENT,
            "scope": DASHBOARD_SCOPE,
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
        let mut request = reqwest::Client::builder()
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
            .get(format!("{}{path}", self.base_url));
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
/// `Surface::Merchant`, or give one validator both audiences, and this passes
/// a `/v1` credential straight into the staff surface.
#[tokio::test]
async fn a_merchant_audience_token_is_refused_on_dash_v1() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    // A real `/v1` credential: merchant A's own client id, the `/v1`
    // audience, the scopes `merchant_client` registers.
    let merchant_token = harness.token(CLIENT_A, MERCHANT_AUDIENCE, "payments:read payments:write");

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

/// The right audience is not enough: the `client_id` must be the registered
/// dashboard client's.
///
/// This is the check that stops "one dashboard client per tenant" from being
/// a promise the code does not keep. The token below is correct in every
/// other respect — vpay's own signature, the dashboard audience, the
/// dashboard scope, unexpired — and names a different client.
///
/// The decisive mutation: delete the `claims.client_id != binding.client_id`
/// arm from `require_dashboard_token` and this returns `200` with merchant
/// A's payments.
#[tokio::test]
async fn a_dashboard_token_for_an_unregistered_client_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    let impostor = harness.token("some-other-dashboard", DASHBOARD_AUDIENCE, DASHBOARD_SCOPE);
    let (status, body) = harness
        .get("/dash/v1/payment_intents", Some(&impostor))
        .await?;

    assert_eq!(status, 403, "{body}");
    assert!(
        !body.contains("pi_dash_a_only"),
        "a refused request must not carry rows: {body}"
    );
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
    let wrong_scope = harness.token(DASHBOARD_CLIENT, DASHBOARD_AUDIENCE, "payments:write");
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
    let merchant_token = harness.token(CLIENT_A, MERCHANT_AUDIENCE, "payments:read");
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
