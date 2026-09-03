//! `/v1/payment_intents`, end to end: the real `vpay_api::router` on a real
//! socket, over a real Postgres, driven by the real merchant SDK.
//!
//! `vpay-api`'s own unit tests cover the pieces — form decoding, validation
//! bounds, the state table, the error envelope — and each of them substitutes
//! something: a lazy pool, a request that never authenticates, a handler
//! called as a function. The claims that only this file can make are the ones
//! that span the whole path:
//!
//! 1. an intent created through the SDK reads back through the SDK, with the
//!    same values and the deployment's `livemode`;
//! 2. a replayed `Idempotency-Key` returns the *same object* and creates no
//!    second row;
//! 3. the same key with a different body is the documented `400`
//!    `idempotency_error` / `idempotency_key_in_use`;
//! 4. a second confirm cannot produce a second charge — one charge per
//!    intent, forever;
//! 5. a confirm reaches the linked rail adapter, and an unreachable rail
//!    answers `502` while leaving the `submitting` charge row and the
//!    status-less `provider_requests` row a recovery pass will need;
//! 6. `cancel` is legal only from `requires_payment_method`;
//! 7. cursor paging walks forward and backward over 25 rows and comes back
//!    to the same page;
//! 8. one merchant cannot read another's intent, and the refusal is **byte
//!    for byte** the answer for an id that never existed;
//! 9. every route the router registers is behind the authentication boundary
//!    (Step 2's D3);
//! 10. a `POST` with no `Idempotency-Key` is the documented `400`;
//! 11. a claim is ended on *every* path out of a `POST`, including the one
//!     where the work succeeded and only the write that records the response
//!     failed — see test 17, which injects that failure in Postgres because
//!     it is not otherwise reachable from outside the process.
//!
//! Step 5b (`docs/plans/2026-09-03-step5b-stripe-sdk.md`) adds four more,
//! all of which are about headers and refusals a Stripe SDK reads and which
//! therefore only exist once the whole stack has run:
//!
//! 12. every response carries the request id under Stripe's `request-id` as
//!     well as `x-request-id`, with **one** value, including on a 404
//!     fallback and on the 401 decided before routing;
//! 13. `stripe-should-retry` carries `Classify::retry`'s answer rather than
//!     the status code's — `true` on an in-flight key's 400, `false` on a
//!     lifecycle 409;
//! 14. `confirm=true` on create is a 400 naming `confirm`, and
//!     `confirm=false` is not;
//! 15. nothing stripe-node adds of its own accord — `Stripe-Version`,
//!     `Stripe-Account`, its telemetry headers, `expand[]`, a
//!     `stripe-node-retry-<uuid>` idempotency key — turns a valid request
//!     into a refusal;
//! 16. the Stripe fields that decide *where or when money moves*
//!     (`capture_method` other than `automatic`, `application_fee_amount`,
//!     `transfer_data`, `on_behalf_of`) are refused on create **and** on
//!     confirm, and the refusal leaves the `Idempotency-Key` unspent so the
//!     corrected retry under the same key goes through;
//! 17. a **replayed** error response does not carry `stripe-should-retry`,
//!     which is a documented gap rather than a claim — see
//!     `vpay_api`'s `STRIPE_SHOULD_RETRY_HEADER`.
//!
//! # What is deliberately not claimed here
//!
//! **Nothing in this file shows a payment being taken.** Every rail it
//! configures is unreachable on purpose, because this suite starts no
//! container for one: what it proves about `confirm` is the ordering and the
//! rows, not the rail. A confirm a rail *accepts* — `processing`,
//! `requires_action`, `next_action`, a declined charge — is
//! `backends/tests/integration/tests/confirm_rails.rs`, which stubs both
//! rails as WireMock hosts.
//!
//! `just sdk-conformance-node` cannot be pointed at this server — it verifies
//! an assertion shape only — so nothing here claims Node/Rust SDK parity
//! against a live vpay. The Rust SDK is exercised; the Node one is not.
//!
//! # Why the SDK and not a hand-rolled client
//!
//! `vpay-sdk` (`sdks/rust`) is the artefact a merchant integrates, not a test
//! double. Where a raw `reqwest` request appears below it is because the SDK
//! deliberately cannot express it: a request with no bearer token (test 9), a
//! `POST` with no `Idempotency-Key` (test 10 — the SDK always sends one), and
//! the byte-level body comparison test 8 needs.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::{BTreeMap, HashMap};

use anyhow::Context as _;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, MERCHANT_AUDIENCE, ProviderHost};
use vpay_db::Repositories;
use vpay_sdk::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, Credentials, IntentStatus,
    ListPaymentIntentsParams, PaymentMethodType, RequestOptions,
};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, merchant_client_with_scopes,
    migrated_postgres, serve,
};

/// The merchant every test acts as, and the tenant it acts for. Never the
/// same string: a query filtered by `client_id` instead of `merchant_id`
/// would otherwise pass.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The second merchant, which exists only so test 8 has someone else's
/// intent to fail to read.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

/// The third merchant: registered, mapped to a tenant, and registered for
/// **no scopes at all**. It exists so test 11 can tell a `403` about the
/// credential's *authorisation* apart from every other `403` this surface
/// can produce — an unregistered `client_id` is also 403, so a test using an
/// unregistered client would pass with no scope check in the code at all.
const CLIENT_C: &str = "gamma-yaounde";
const MERCHANT_C: &str = "gamma-yaounde-tenant";

/// The push rail (MTN) and the redirect rail (Orange), both configured and
/// both enabled — the flow branch in `confirm` is only meaningful with one of
/// each.
const PUSH_RAIL: &str = "mtn_momo";
const REDIRECT_RAIL: &str = "orange_money";

/// The currency this deployment configures. XAF is zero-decimal, so `5000`
/// is 5,000 FCFA (`docs/flows/money.md`).
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

// ------------------------------------------------------------------ harness

/// A running vpay server, its database, and the two merchants' credentials.
struct Harness {
    _container: ContainerAsync<PostgresImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    /// The plain `sqlx` pool, for the fixtures that read or force schema
    /// state no repository method owns.
    pool: PgPool,
    base_url: String,
    /// Merchant A's, B's and C's private keys, PEM-encoded.
    pem_a: String,
    pem_b: String,
    pem_c: String,
    /// The server's own signing key, so a test can mint a bearer token
    /// directly for the raw requests the SDK cannot make.
    signing_key: LoadedSigningKey,
}

impl Harness {
    fn sdk(&self, client_id: &str, pem: &str) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(&self.base_url)
            .credentials(
                Credentials::rsa_pem(client_id, pem).expect("the generated PEM parses as RSA"),
            )
            .build()
            .expect("the SDK client builds from a base URL and a credential")
    }

    /// Merchant A's SDK client — the one almost every test uses.
    fn a(&self) -> vpay_sdk::Client {
        self.sdk(CLIENT_A, &self.pem_a)
    }

    /// Merchant B's SDK client.
    fn b(&self) -> vpay_sdk::Client {
        self.sdk(CLIENT_B, &self.pem_b)
    }

    /// Merchant C's SDK client — the one registered for no scopes.
    fn c(&self) -> vpay_sdk::Client {
        self.sdk(CLIENT_C, &self.pem_c)
    }

    /// A bearer token for `client_id`, minted with the server's own signer.
    ///
    /// The same shape the OP would mint — same issuer, same key, same
    /// `vpay:v1` audience — so a request carrying it is indistinguishable to
    /// `/v1` from one the SDK obtained. It exists because the SDK holds its
    /// token privately, and the raw-request tests below need one in hand.
    fn bearer(&self, client_id: &str) -> String {
        self.bearer_with_scope(client_id, Some(vpay_api::SCOPE_PAYMENTS_WRITE))
    }

    /// The same, with the `scope` claim named explicitly.
    ///
    /// The OP grants a client's registered scopes when a token request names
    /// none (RFC 6749 §3.3, `vpay_api::op::token::token_handler`), so a
    /// hand-minted token has to say what it carries or it would be *less*
    /// authorised than the one the SDK obtains — and every raw-request test
    /// below would be asserting against a `403` instead of the answer it is
    /// actually about.
    fn bearer_with_scope(&self, client_id: &str, scope: Option<&str>) -> String {
        self.signing_key
            .token_manager()
            .issue_client_token_with_extra(
                client_id,
                900,
                scope.map(str::to_owned),
                Some(MERCHANT_AUDIENCE.to_owned()),
                HashMap::new(),
            )
            .expect("the server's own signer mints a merchant token")
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// The configuration this deployment runs under: three merchants, two rails,
/// one currency, `livemode: false`.
///
/// `push_rail_enabled` exists for one test
/// (`a_replay_survives_the_rail_being_disabled`), which stands a second
/// server up over the same database with the rail an intent was created for
/// switched off — the configuration change an operator makes between a
/// merchant's request and their retry.
fn config_with(
    base_url: &str,
    jwks_a: Value,
    jwks_b: Value,
    jwks_c: Value,
    push_rail_enabled: bool,
) -> Config {
    Config {
        deployment: Deployment {
            name: "payment-intents".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![
            ProviderHost {
                code: PUSH_RAIL.to_owned(),
                enabled: push_rail_enabled,
                host: HostEntry {
                    // Deliberately unreachable, and fully configured
                    // otherwise. This suite is about the *surface*: it starts
                    // no rail stub, so every confirm below ends in
                    // `Category::Rail` — which is a real answer from the real
                    // adapter, over the real HTTP client, and is what makes
                    // "the confirm reached the rail" observable here without
                    // a container. The accepted-confirm cases live in
                    // `confirm_rails.rs`, against WireMock.
                    //
                    // The settings and credentials below are the keys
                    // `vpay_config::REQUIRED_RAIL_KEYS` insists on; without
                    // them the adapter answers `ProviderError::Config` (500,
                    // misconfigured) before it opens a socket, and these
                    // tests would be asserting against our own YAML rather
                    // than against an unreachable rail.
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
                // The intents these suites create are XAF, and a confirm
                // whose intent currency is not its rail's is a `400` before
                // any charge exists (`vpay_api`'s `currencies_agree`) — which
                // is `confirm_rails.rs`'s subject, not this file's.
                currency: "XAF".to_owned(),
                credentials: BTreeMap::from([
                    (
                        "subscription_key".to_owned(),
                        "stub-subscription-key".to_owned(),
                    ),
                    ("api_key".to_owned(), "stub-api-key".to_owned()),
                ]),
            },
            ProviderHost {
                code: REDIRECT_RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: "http://127.0.0.1:1/orange-money-webpay/dev".to_owned(),
                    label: "unreachable-by-design".to_owned(),
                },
                settings: BTreeMap::from([
                    ("env".to_owned(), "dev".to_owned()),
                    ("lang".to_owned(), "en".to_owned()),
                ]),
                callback_url: None,
                currency: "XAF".to_owned(),
                credentials: BTreeMap::from([
                    ("merchant_key".to_owned(), "stub-merchant-key".to_owned()),
                    ("client_id".to_owned(), "stub-client-id".to_owned()),
                    ("client_secret".to_owned(), "stub-client-secret".to_owned()),
                ]),
            },
        ],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            merchant_client(CLIENT_A, MERCHANT_A, jwks_a),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
            // Registered, and registered for nothing — see `CLIENT_C`.
            merchant_client_with_scopes(CLIENT_C, MERCHANT_C, jwks_c, &[]),
        ],
        dashboard_client: None,
    }
}

/// Boots a real server, in `vpay-server`'s own order: migrate, run boot step
/// 4, announce the signing key, bind, then serve.
///
/// A harness that assembled things in a different order would be testing a
/// different program — and boot step 4 in particular is not optional here:
/// `charges.provider_code` and `charges.currency_code` are foreign keys, so
/// without it every confirm below would fail on a constraint instead of on
/// the thing it is testing.
async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let (pem_b, jwks_b) = generate_key();
    let (pem_c, jwks_c) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b, jwks_c, true)
    })
    .await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        pem_a,
        pem_b,
        pem_c,
        signing_key: served.signing_key,
    })
}

fn raw_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
}

/// `CreatePaymentIntentParams` for the push rail, with one metadata entry so
/// the round trip covers a nested form key.
fn create_params() -> CreatePaymentIntentParams {
    CreatePaymentIntentParams {
        amount: AMOUNT,
        currency: CURRENCY.to_owned(),
        payment_method_types: vec![PaymentMethodType::MtnMomo],
        metadata: BTreeMap::from([("order_id".to_owned(), "1234".to_owned())]),
        description: Some("Order #42 (rush)".to_owned()),
    }
}

/// The `ApiError` fields a test cares about, pulled out of the SDK's typed
/// error. Panics on any other variant, because "the request never produced an
/// HTTP response" is never the answer one of these tests is asserting.
fn api_error(error: vpay_sdk::Error) -> (u16, String, Option<String>, Option<String>) {
    match error {
        vpay_sdk::Error::Api {
            status,
            kind,
            code,
            param,
            ..
        } => (status, kind, code, param),
        other => panic!("expected a vpay API error envelope, got {other:?}"),
    }
}

async fn count(pool: &PgPool, sql: &str, bind: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>(sql)
        .bind(bind)
        .fetch_one(pool)
        .await
        .context("counting rows")
}

// ------------------------------------------------------------------ test 1

/// The round trip: what the SDK sent is what the SDK reads back, including
/// the fields the *deployment* owns rather than the caller.
#[tokio::test]
async fn create_then_retrieve_round_trips_through_the_sdk() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let created = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating a payment intent through the SDK")?;

    assert!(
        created.id.starts_with("pi_"),
        "an intent id is prefixed: {}",
        created.id
    );
    assert_eq!(created.object, "payment_intent");
    assert_eq!(created.amount, AMOUNT);
    assert_eq!(created.currency, CURRENCY, "lowercase on the wire");
    assert_eq!(created.status, IntentStatus::RequiresPaymentMethod);
    assert_eq!(created.payment_method_types, vec![PUSH_RAIL.to_owned()]);
    assert_eq!(created.next_action, None, "nothing has been confirmed");
    assert_eq!(created.last_payment_error, None);
    assert_eq!(
        created.metadata.get("order_id").map(String::as_str),
        Some("1234")
    );
    assert_eq!(created.description.as_deref(), Some("Order #42 (rush)"));
    assert!(
        !created.livemode,
        "the deployment is livemode: false, and the object says so"
    );
    assert!(created.created > 0, "created is unix seconds, not zero");

    let retrieved = client
        .payment_intents()
        .retrieve(&created.id)
        .await
        .context("retrieving the intent just created")?;
    assert_eq!(
        retrieved, created,
        "a retrieve must return the object the create returned, field for field"
    );

    // The row is the merchant's, not the client's — the whole tenancy
    // boundary, read straight from the table.
    let merchant_id: String =
        sqlx::query_scalar("SELECT merchant_id FROM payment_intents WHERE id = $1")
            .bind(&created.id)
            .fetch_one(&harness.pool)
            .await
            .context("reading the stored merchant_id")?;
    assert_eq!(
        merchant_id, MERCHANT_A,
        "the row is filed under the tenant, never under the client_id"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 2

/// A replayed key returns the stored answer and does no work: the object is
/// identical *and* there is exactly one row.
///
/// The row count is the half that matters. Returning the same object could
/// also happen if the second request created a second intent and the two
/// happened to look alike; `count(*) = 1` is what rules that out.
#[tokio::test]
async fn a_replayed_idempotency_key_returns_the_same_object_and_no_second_row() -> anyhow::Result<()>
{
    let harness = harness().await?;
    let client = harness.a();
    let opts = RequestOptions::new().with_idempotency_key("order-1234-create");

    let first = client
        .payment_intents()
        .create(create_params(), opts.clone())
        .await
        .context("the first create")?;
    let second = client
        .payment_intents()
        .create(create_params(), opts)
        .await
        .context("the same request under the same key")?;

    assert_eq!(
        first, second,
        "a replay must answer with the stored response, not a new object"
    );

    let rows = count(
        &harness.pool,
        "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(rows, 1, "the replay must not have created a second intent");

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 3

/// The same key with a *different* body is refused, with the envelope
/// `docs/api/README.md` documents.
///
/// `idempotency_error` / `idempotency_key_in_use` is
/// `vpay_core::Category::Idempotency`'s own kind and code — the API does not
/// choose them at this call site (ADR-0011), which is why they are asserted
/// verbatim here.
#[tokio::test]
async fn a_reused_key_with_a_different_body_is_the_400_envelope() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();
    let opts = RequestOptions::new().with_idempotency_key("order-1234-create");

    client
        .payment_intents()
        .create(create_params(), opts.clone())
        .await
        .context("the first create")?;

    let mut different = create_params();
    different.amount = AMOUNT + 1;
    let error = client
        .payment_intents()
        .create(different, opts)
        .await
        .expect_err("the same key with a different body must be refused");

    let (status, kind, code, _param) = api_error(error);
    assert_eq!(status, 400);
    assert_eq!(kind, "idempotency_error");
    assert_eq!(code.as_deref(), Some("idempotency_key_in_use"));

    let rows = count(
        &harness.pool,
        "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(rows, 1, "the refused request must not have created a row");

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 4

/// "One charge per intent, forever" (`AGENTS.md`), observed from outside: the
/// second confirm is a `409` and there is still exactly one charge row.
///
/// Both confirms reach the rail — the first one gets the `502` of test 5,
/// because this suite's rails are unreachable by design — so this is
/// specifically about the *charge*, not about the confirm succeeding.
#[tokio::test]
async fn a_second_confirm_cannot_produce_a_second_charge() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to confirm")?;

    let first = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new(),
        )
        .await
        .expect_err("this suite's rail is unreachable, so a confirm cannot succeed");
    assert_eq!(
        api_error(first).0,
        502,
        "the first confirm reaches the rail — or rather, fails to"
    );

    let second = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new(),
        )
        .await
        .expect_err("the intent already has a charge");
    let (status, kind, _code, _param) = api_error(second);
    assert_eq!(
        status, 409,
        "a second confirm is a conflict, not a second charge"
    );
    assert_eq!(kind, "invalid_request_error");

    let charges = count(
        &harness.pool,
        "SELECT count(*) FROM charges WHERE payment_intent_id = $1",
        &intent.id,
    )
    .await?;
    assert_eq!(charges, 1, "one charge per intent, forever");

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 5

/// The confirm path when the rail never answers: the `502`, **and** the two
/// rows it deliberately leaves behind.
///
/// The rows are not incidental. `docs/flows/crash-safety.md` requires the
/// reference to be durable before anything is submitted, so the charge is
/// committed in `submitting` and the attempt is recorded with no status
/// *before* the adapter is called. What a crash between those writes and the
/// answer would leave is exactly what an unreachable rail leaves — which is
/// why asserting only the `502` would let someone delete both writes and
/// still pass.
///
/// **This used to be the `501` case**, when no adapter implemented `submit`.
/// The rows asserted are the same ones; what changed is that the request now
/// reaches a socket. The confirms that *succeed* are in `confirm_rails.rs`,
/// which needs a WireMock container and therefore does not belong here.
#[tokio::test]
async fn confirm_reaches_the_rail_and_an_unreachable_one_leaves_the_recovery_rows()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to confirm")?;

    let error = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new(),
        )
        .await
        .expect_err("nothing is listening on the configured rail host");

    let (status, kind, code, _param) = api_error(error);
    assert_eq!(status, 502, "the rail is the failing party, not the caller");
    assert_eq!(kind, "api_error");
    assert_eq!(
        code.as_deref(),
        Some("provider_unavailable"),
        "the answer is the adapter's own `Transport`, classified once \
         (docs/flows/errors.md) — never an invented failure and never a decline"
    );

    // The charge: committed before the call, in the initial state, carrying
    // the reference it would have submitted under.
    let (charge_id, charge_state, provider_code, reference): (String, String, String, uuid::Uuid) =
        sqlx::query_as(
            "SELECT id, state::text, provider_code, provider_reference_id \
         FROM charges WHERE payment_intent_id = $1",
        )
        .bind(&intent.id)
        .fetch_one(&harness.pool)
        .await
        .context("the charge row a confirm commits before submitting")?;
    assert_eq!(
        charge_state, "submitting",
        "the charge stays in the state it was committed in; nothing was submitted"
    );
    assert_eq!(provider_code, PUSH_RAIL);

    // The attempt: recorded before the call, still with no response.
    let (attempt_status, error_kind, responded_at, attempt_reference): (
        Option<i32>,
        Option<String>,
        Option<time::OffsetDateTime>,
        uuid::Uuid,
    ) = sqlx::query_as(
        "SELECT status_code, error_kind, responded_at, provider_reference_id \
         FROM provider_requests WHERE charge_id = $1",
    )
    .bind(&charge_id)
    .fetch_one(&harness.pool)
    .await
    .context("the provider_requests row a confirm inserts before submitting")?;
    assert_eq!(
        attempt_status, None,
        "no HTTP status: nothing was sent, so status_code stays NULL"
    );
    assert_eq!(
        responded_at, None,
        "the (status_code IS NULL) = (responded_at IS NULL) CHECK, observed"
    );
    assert_eq!(
        error_kind.as_deref(),
        Some("provider_unavailable"),
        "the attempt records why it ended, using the error's own classification code"
    );
    assert_eq!(
        attempt_reference, reference,
        "the attempt is recorded against the same reference the charge carries"
    );

    // And the intent itself did not move: nothing was submitted, so nothing
    // about the payer's world changed.
    let after = client
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("re-reading the intent after the failed confirm")?;
    assert_eq!(after.status, IntentStatus::RequiresPaymentMethod);
    assert_eq!(after.next_action, None);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 6

/// `cancel` is a compare-and-swap on the status, so it is legal exactly once
/// and only from `requires_payment_method`.
#[tokio::test]
async fn cancel_is_legal_only_from_requires_payment_method() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to cancel")?;

    let canceled = client
        .payment_intents()
        .cancel(&intent.id, RequestOptions::new())
        .await
        .context("cancelling a fresh intent")?;
    assert_eq!(canceled.status, IntentStatus::Canceled);
    assert_eq!(canceled.id, intent.id);

    // A second cancel: the object exists, its status forbids the move.
    let error = client
        .payment_intents()
        .cancel(&intent.id, RequestOptions::new())
        .await
        .expect_err("a canceled intent cannot be canceled again");
    let (status, kind, _code, _param) = api_error(error);
    assert_eq!(status, 409, "409, not 404: the object is there");
    assert_eq!(kind, "invalid_request_error");

    // And a canceled intent cannot then be confirmed.
    let error = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new(),
        )
        .await
        .expect_err("a canceled intent cannot be confirmed");
    assert_eq!(api_error(error).0, 409);
    let charges = count(
        &harness.pool,
        "SELECT count(*) FROM charges WHERE payment_intent_id = $1",
        &intent.id,
    )
    .await?;
    assert_eq!(
        charges, 0,
        "the refused confirm must not have inserted a charge"
    );

    // Cancelling something that never existed is a 404, not a 409.
    let error = client
        .payment_intents()
        .cancel("pi_00000000000000000000000x", RequestOptions::new())
        .await
        .expect_err("no such intent");
    assert_eq!(api_error(error).0, 404);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 7

/// Cursor paging, forward and back, over 25 rows at `limit: 10`.
///
/// The backward walk is the half worth having: `ending_before` queries
/// ascending and reverses in Rust (Step 2's D8), so a page read backwards has
/// to come back *identical* to the page read forwards — same ids, same order,
/// newest first. Asserting only `has_more` would not notice a reversed page.
#[tokio::test]
async fn list_pages_forward_and_backward_with_cursors() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    // Created one at a time, in order, so `seq` is the creation order and the
    // expected page contents are known.
    let mut created = Vec::new();
    for index in 0..25 {
        let mut params = create_params();
        params.description = Some(format!("intent {index}"));
        created.push(
            client
                .payment_intents()
                .create(params, RequestOptions::new())
                .await
                .with_context(|| format!("creating intent {index}"))?
                .id,
        );
    }
    // Newest first is the order the API answers in.
    let newest_first: Vec<String> = created.iter().rev().cloned().collect();

    let page = |params: ListPaymentIntentsParams| {
        let client = client.clone();
        async move { client.payment_intents().list(params).await }
    };

    let first = page(ListPaymentIntentsParams {
        limit: Some(10),
        ..Default::default()
    })
    .await
    .context("the first page")?;
    assert_eq!(first.object, "list");
    assert_eq!(first.url, "/v1/payment_intents");
    assert!(first.has_more, "25 rows, 10 per page");
    let first_ids: Vec<String> = first.data.iter().map(|i| i.id.clone()).collect();
    assert_eq!(
        first_ids,
        newest_first.get(0..10).expect("25 ids were created")
    );

    let second = page(ListPaymentIntentsParams {
        limit: Some(10),
        starting_after: first_ids.last().cloned(),
        ..Default::default()
    })
    .await
    .context("the second page")?;
    let second_ids: Vec<String> = second.data.iter().map(|i| i.id.clone()).collect();
    assert_eq!(
        second_ids,
        newest_first.get(10..20).expect("25 ids were created")
    );
    assert!(second.has_more);

    let third = page(ListPaymentIntentsParams {
        limit: Some(10),
        starting_after: second_ids.last().cloned(),
        ..Default::default()
    })
    .await
    .context("the third page")?;
    let third_ids: Vec<String> = third.data.iter().map(|i| i.id.clone()).collect();
    assert_eq!(
        third_ids,
        newest_first.get(20..25).expect("25 ids were created")
    );
    assert!(
        !third.has_more,
        "25 rows exactly, so the third page is last"
    );

    // Backwards from the third page's first id: the second page again,
    // identical and in the same order.
    let back = page(ListPaymentIntentsParams {
        limit: Some(10),
        ending_before: third_ids.first().cloned(),
        ..Default::default()
    })
    .await
    .context("paging backwards")?;
    let back_ids: Vec<String> = back.data.iter().map(|i| i.id.clone()).collect();
    assert_eq!(
        back_ids, second_ids,
        "ending_before must return the previous page, newest first"
    );

    // And backwards past the beginning is a short page, not an error.
    let start = page(ListPaymentIntentsParams {
        limit: Some(10),
        ending_before: newest_first.get(3).cloned(),
        ..Default::default()
    })
    .await
    .context("paging backwards past the start")?;
    let start_ids: Vec<String> = start.data.iter().map(|i| i.id.clone()).collect();
    assert_eq!(
        start_ids,
        newest_first.get(0..3).expect("25 ids were created")
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 8

/// Merchant B cannot read merchant A's intent — and the refusal is **byte for
/// byte** the answer for an id that never existed.
///
/// Byte-identical is the whole assertion. A 404 whose body differed (a
/// different message, a different `param`, anything) would turn this API into
/// an oracle for "does this id exist under some other tenant", which is the
/// reason `ApiError::NotFound` is what a foreign id answers rather than
/// `Forbidden`. Compared as raw bytes, over raw HTTP, because the SDK decodes
/// the envelope into a struct and a struct comparison would not notice a
/// difference in a field it does not model.
#[tokio::test]
async fn merchant_b_cannot_read_merchant_as_intent() -> anyhow::Result<()> {
    let harness = harness().await?;

    let intent = harness
        .a()
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("merchant A creates an intent")?;

    let bearer_b = harness.bearer(CLIENT_B);
    let http = raw_client();

    let foreign = http
        .get(harness.url(&format!("/v1/payment_intents/{}", intent.id)))
        .bearer_auth(&bearer_b)
        .send()
        .await
        .context("merchant B asks for merchant A's intent")?;
    let foreign_status = foreign.status().as_u16();
    let foreign_body = foreign.bytes().await.context("the body is readable")?;

    // An id of exactly the same shape that no merchant has ever had.
    let missing_id = "pi_00000000000000000000000x";
    let missing = http
        .get(harness.url(&format!("/v1/payment_intents/{missing_id}")))
        .bearer_auth(&bearer_b)
        .send()
        .await
        .context("merchant B asks for an id that never existed")?;
    let missing_status = missing.status().as_u16();
    let missing_body = missing.bytes().await.context("the body is readable")?;

    assert_eq!(foreign_status, 404);
    assert_eq!(missing_status, 404);

    // The bodies differ only where the *caller's own id* is echoed, which is
    // the id they sent — so compare with each request's own id substituted
    // out. Anything else differing would be a distinguisher.
    let foreign_text = String::from_utf8_lossy(&foreign_body).replace(&intent.id, "<id>");
    let missing_text = String::from_utf8_lossy(&missing_body).replace(missing_id, "<id>");
    assert_eq!(
        foreign_text, missing_text,
        "another merchant's intent and an id that never existed must be indistinguishable"
    );

    // And merchant B's own list does not contain it either.
    let listed = harness
        .b()
        .payment_intents()
        .list(ListPaymentIntentsParams::default())
        .await
        .context("merchant B lists its own intents")?;
    assert!(
        listed.data.is_empty(),
        "merchant B has created nothing, so its list is empty: {:?}",
        listed.data
    );

    // Nor can B confirm or cancel it.
    for path in ["confirm", "cancel"] {
        let response = http
            .post(harness.url(&format!("/v1/payment_intents/{}/{path}", intent.id)))
            .bearer_auth(&bearer_b)
            .header("Idempotency-Key", format!("b-tries-{path}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body("payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000")
            .send()
            .await
            .context("merchant B tries to act on merchant A's intent")?;
        assert_eq!(
            response.status().as_u16(),
            404,
            "a write to a foreign id is the same 404 as a read"
        );
    }

    // A's intent is untouched.
    let still_there = harness
        .a()
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("merchant A can still read its own intent")?;
    assert_eq!(still_there.status, IntentStatus::RequiresPaymentMethod);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 9

/// Step 2's D3, asserted over the router's *own* route table: every path
/// `vpay_api::V1_ROUTES` registers, on every method it registers, answers
/// `401` with no bearer token.
///
/// The table is not a copy — `vpay_api::v1::routes` folds the same constant
/// into the `Router`, so a route cannot exist without appearing here, and
/// this test cannot fall behind the surface it is checking. Removing the
/// `require_merchant_token` layer from the `/v1` nest fails this test on
/// every entry.
///
/// Path parameters are filled with a syntactically valid id: a `401` decided
/// before routing would pass either way, but a `{id}` left literal would make
/// a *routing* failure look like a boundary success.
#[tokio::test]
async fn every_registered_v1_path_answers_401_without_a_token() -> anyhow::Result<()> {
    let harness = harness().await?;
    let http = raw_client();

    assert!(
        !vpay_api::V1_ROUTES.is_empty(),
        "an empty route table would make this test vacuous"
    );

    let mut checked = 0_usize;
    for route in vpay_api::V1_ROUTES {
        let path = route.path.replace("{id}", "pi_00000000000000000000000x");
        for method in route.methods {
            let url = harness.url(&format!("/v1{path}"));
            let request = match *method {
                "GET" => http.get(&url),
                "POST" => http
                    .post(&url)
                    .header("Idempotency-Key", "unauthenticated")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(""),
                other => panic!("this test does not know how to send {other}"),
            };
            let response = request
                .send()
                .await
                .with_context(|| format!("{method} {url}"))?;
            assert_eq!(
                response.status().as_u16(),
                401,
                "{method} /v1{path} must be behind the merchant authentication boundary"
            );
            let body: Value = response.json().await.context("the 401 body is JSON")?;
            assert_eq!(
                body.pointer("/error/type").and_then(Value::as_str),
                Some("authentication_error"),
                "{method} /v1{path}: got {body:#}"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 5,
        "the four /v1 routes answer five method/path pairs; only {checked} were checked"
    );

    // The other side: an unrouted `/v1` path is *also* behind the boundary,
    // so a caller with no token cannot learn which resources exist.
    for path in ["/v1/balance", "/v1/events", "/v1/refunds"] {
        let response = http
            .get(harness.url(path))
            .send()
            .await
            .context("an unrouted /v1 path")?;
        assert_eq!(
            response.status().as_u16(),
            401,
            "{path} is not implemented, and an anonymous caller must not learn that"
        );
    }

    // And with a token, an unrouted path is the honest 404 — the pair of
    // answers that shows the boundary is in front of the fallback too.
    let bearer = harness.bearer(CLIENT_A);
    let response = http
        .get(harness.url("/v1/balance"))
        .bearer_auth(&bearer)
        .send()
        .await
        .context("an authenticated request for an unimplemented resource")?;
    assert_eq!(response.status().as_u16(), 404);
    let body: Value = response.json().await.context("the 404 body is JSON")?;
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("unknown_route"),
        "got {body:#}"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 10

/// D7: an `Idempotency-Key` is required on every `POST` under `/v1`, and its
/// absence is a `400` naming `idempotency_key`.
///
/// Raw HTTP, because the SDK always sends one — which is the correct SDK
/// behaviour and exactly why it cannot express this request.
#[tokio::test]
async fn a_post_without_an_idempotency_key_is_the_documented_400() -> anyhow::Result<()> {
    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let response = http
        .post(harness.url("/v1/payment_intents"))
        .bearer_auth(&bearer)
        .header("content-type", "application/x-www-form-urlencoded")
        .body("amount=5000&currency=xaf&payment_method_types[0]=mtn_momo")
        .send()
        .await
        .context("a POST with no Idempotency-Key")?;

    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.context("the 400 body is JSON")?;
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "got {body:#}"
    );
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("idempotency_key"),
        "the envelope must name the header the caller has to add: {body:#}"
    );

    // Nothing was created.
    let rows = count(
        &harness.pool,
        "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(rows, 0);

    // The same request *with* a key succeeds, so the 400 above is about the
    // header and not about the body.
    let response = http
        .post(harness.url("/v1/payment_intents"))
        .bearer_auth(&bearer)
        .header("Idempotency-Key", "with-a-key")
        .header("content-type", "application/x-www-form-urlencoded")
        .body("amount=5000&currency=xaf&payment_method_types[0]=mtn_momo")
        .send()
        .await
        .context("the same POST with a key")?;
    assert_eq!(response.status().as_u16(), 200);

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 11

/// F1: an intent whose confirm reached the rail cannot be canceled.
///
/// The dangerous version of this test passes the code before the fix:
/// `confirm` commits its charge and then answers `502`, leaving the intent
/// at `requires_payment_method` — so a status-only compare-and-swap cancels
/// it happily, and vpay tells a merchant the payment was withdrawn while the
/// rail may hold the reference it was given. Never say that.
///
/// Asserts all three halves, because the `409` alone would pass a "fix" that
/// simply refused every cancel: the intent must be *unchanged*, and the
/// charge must still be there.
#[tokio::test]
async fn a_confirmed_intent_cannot_be_canceled() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to confirm")?;

    let confirmed = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            RequestOptions::new(),
        )
        .await
        .expect_err("this suite's rail is unreachable");
    assert_eq!(
        api_error(confirmed).0,
        502,
        "the confirm must have reached the rail — this test is about what it left behind"
    );

    let error = client
        .payment_intents()
        .cancel(&intent.id, RequestOptions::new())
        .await
        .expect_err("an intent with a live charge must not be cancellable");
    let (status, kind, _code, _param) = api_error(error);
    assert_eq!(
        status, 409,
        "409, not 200: a payment vpay cannot prove was never submitted must not be reported \
         as withdrawn"
    );
    assert_eq!(kind, "invalid_request_error");

    // The intent did not move.
    let after = client
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("re-reading the intent after the refused cancel")?;
    assert_eq!(
        after.status,
        IntentStatus::RequiresPaymentMethod,
        "the refused cancel must not have changed the status"
    );

    // And the charge the rail was given a reference for is still there.
    let charges = count(
        &harness.pool,
        "SELECT count(*) FROM charges WHERE payment_intent_id = $1",
        &intent.id,
    )
    .await?;
    assert_eq!(charges, 1, "the charge is what makes the cancel unsafe");

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 12

/// F2: a `5xx` hands the `Idempotency-Key` back, so the merchant's retry
/// re-executes instead of being told the first attempt is still running.
///
/// Before the fix, a `5xx` left the key `in_flight` and nothing in the
/// system ever moved that row again: every retry under it was answered "a
/// request with this Idempotency-Key is still in progress" for the life of
/// the deployment. It mattered most when every confirm ended in the
/// adapter's `501`; it matters now for every confirm a rail does not answer,
/// which is the case this test stages.
///
/// **What the retry answers is `409`, not a second `501`, and that is the
/// correct outcome** — the first confirm committed a charge before reaching
/// the rail, so a re-executed retry meets "one charge per intent, forever"
/// and says so. The assertion that matters is therefore *which* 409: the
/// re-executed one names the charge, while the bug's answer names the
/// in-flight key. They are the same status and the same `code`, so the
/// message is what tells them apart.
#[tokio::test]
async fn a_5xx_releases_its_idempotency_key_so_the_retry_re_executes() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to confirm")?;

    let opts = RequestOptions::new().with_idempotency_key("confirm-once");
    let first = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            opts.clone(),
        )
        .await
        .expect_err("this suite's rail is unreachable");
    assert_eq!(api_error(first).0, 502);

    // The row is gone: the key is claimable again by anyone.
    let held = count(
        &harness.pool,
        "SELECT count(*) FROM idempotency_keys WHERE merchant_id = $1 \
         AND idempotency_key = 'confirm-once'",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(
        held, 0,
        "a 5xx is not stored, so the key must be released rather than left claimed"
    );

    let second = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo("237670000000"),
            opts,
        )
        .await
        .expect_err("the intent already has a charge");
    let message = match second {
        vpay_sdk::Error::Api {
            status,
            ref message,
            ..
        } => {
            assert_eq!(status, 409);
            message.clone()
        }
        other => panic!("expected an API error envelope, got {other:?}"),
    };
    // The one-charge rule's 409, in the wording a *live* charge gets: the
    // first confirm left a `submitting` charge the rail may be holding, so
    // this message must not invite a second PaymentIntent
    // (`vpay_api::v1::payment_intents::already_charged`, and
    // `confirm_rails.rs`'s unreachable-rail case, which pins the sentence
    // itself).
    assert!(
        message.contains("do not create a new PaymentIntent"),
        "the retry must have re-executed and met the one-charge rule; instead it was answered \
         {message:?}"
    );
    assert!(
        !message.contains("in progress"),
        "the key was left in flight: {message:?}"
    );

    // And re-executing did not produce a second charge.
    let charges = count(
        &harness.pool,
        "SELECT count(*) FROM charges WHERE payment_intent_id = $1",
        &intent.id,
    )
    .await?;
    assert_eq!(
        charges, 1,
        "releasing the key is safe precisely because the unique index still holds"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 13

/// F3: `/v1` checks the token's `scope`, and a client registered for nothing
/// is refused while the ordinary client is not.
///
/// Both halves are needed. The `403` alone would pass a router that refused
/// everything; the `200` alone would pass one that checked nothing. And
/// merchant C is *registered* — it resolves to a tenant — so this cannot be
/// satisfied by the unregistered-client `403` that was already there.
///
/// The token itself is obtained through the OP by the real SDK: a client
/// registered for no scopes still authenticates, which is the point. What it
/// cannot do is act.
#[tokio::test]
async fn a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not()
-> anyhow::Result<()> {
    let harness = harness().await?;

    let error = harness
        .c()
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .expect_err("a client registered for no scopes may not create a payment intent");
    let (status, kind, code, _param) = api_error(error);
    assert_eq!(
        status, 403,
        "the credential is genuine; what is missing is a scope"
    );
    assert_eq!(kind, "invalid_request_error");
    assert_eq!(code.as_deref(), Some("forbidden"));

    // Reads are refused too: no scope is no scope.
    let error = harness
        .c()
        .payment_intents()
        .list(ListPaymentIntentsParams::default())
        .await
        .expect_err("nor may it read");
    assert_eq!(api_error(error).0, 403);

    // The ordinary client, registered for `payments:write`, is unaffected —
    // and its token carries that scope without ever asking for one.
    let created = harness
        .a()
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("a registered, scoped client must still get through")?;
    assert_eq!(created.status, IntentStatus::RequiresPaymentMethod);

    // A read scope authorises a GET and not a POST. Hand-minted because no
    // registration here holds `payments:read` — the rule under test is the
    // middleware's, not the registry's.
    let http = raw_client();
    let read_only = harness.bearer_with_scope(CLIENT_A, Some(vpay_api::SCOPE_PAYMENTS_READ));
    let response = http
        .get(harness.url("/v1/payment_intents"))
        .bearer_auth(&read_only)
        .send()
        .await
        .context("a read-scoped token listing intents")?;
    assert_eq!(response.status().as_u16(), 200);

    let response = http
        .post(harness.url("/v1/payment_intents"))
        .bearer_auth(&read_only)
        .header("Idempotency-Key", "read-only-tries-to-write")
        .header("content-type", "application/x-www-form-urlencoded")
        .body("amount=5000&currency=xaf&payment_method_types[0]=mtn_momo")
        .send()
        .await
        .context("a read-scoped token creating an intent")?;
    assert_eq!(
        response.status().as_u16(),
        403,
        "a read-only credential must not be able to take a payment"
    );

    // A token with no scope claim at all is refused, not admitted by
    // default — the shape every token had before the OP learned to apply a
    // default scope.
    let unscoped = harness.bearer_with_scope(CLIENT_A, None);
    let response = http
        .get(harness.url("/v1/payment_intents"))
        .bearer_auth(&unscoped)
        .send()
        .await
        .context("an unscoped token")?;
    assert_eq!(response.status().as_u16(), 403);

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 14

/// F4: the two cursor rules `list_page`'s documentation claims are enforced
/// at the boundary, enforced at the boundary.
///
/// Both were silent failures rather than errors: two cursors at once applied
/// *both* predicates and returned a page that is the intersection — a
/// perfectly plausible-looking answer to a question nobody asked — and a
/// mistyped cursor resolves to `NULL` inside the query, which reads as "the
/// end of the list". A merchant paging with a typo saw an empty list and had
/// nothing to fix.
#[tokio::test]
async fn a_list_refuses_two_cursors_and_a_malformed_one() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let first = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("an intent to cursor from")?;
    let second = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("a second intent")?;

    let error = client
        .payment_intents()
        .list(ListPaymentIntentsParams {
            limit: Some(10),
            starting_after: Some(second.id.clone()),
            ending_before: Some(first.id.clone()),
        })
        .await
        .expect_err("two cursors name opposite directions and must be refused");
    let (status, kind, _code, param) = api_error(error);
    assert_eq!(status, 400);
    assert_eq!(kind, "invalid_request_error");
    assert_eq!(param.as_deref(), Some("starting_after"));

    for (starting_after, ending_before, expected_param) in [
        (Some("pi_not-an-id".to_owned()), None, "starting_after"),
        (
            None,
            Some(format!("ch_{}", "0".repeat(24))),
            "ending_before",
        ),
        // The right shape for a charge id, and a real one would still be
        // refused: a cursor names an intent.
        (Some("pi_".to_owned()), None, "starting_after"),
    ] {
        let error = client
            .payment_intents()
            .list(ListPaymentIntentsParams {
                limit: Some(10),
                starting_after,
                ending_before,
            })
            .await
            .expect_err("a malformed cursor must be named, not answered with an empty page");
        let (status, _kind, _code, param) = api_error(error);
        assert_eq!(status, 400);
        assert_eq!(param.as_deref(), Some(expected_param));
    }

    // One cursor, well formed, still works — the refusals above are about
    // the two rules and not about cursors.
    let page = client
        .payment_intents()
        .list(ListPaymentIntentsParams {
            limit: Some(10),
            starting_after: Some(second.id.clone()),
            ..Default::default()
        })
        .await
        .context("one well-formed cursor")?;
    assert_eq!(
        page.data.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        vec![first.id],
    );

    // And a well-formed cursor that names nothing is still an empty page,
    // not a 400: telling those apart would be an existence oracle.
    let page = client
        .payment_intents()
        .list(ListPaymentIntentsParams {
            limit: Some(10),
            starting_after: Some("pi_00000000000000000000000x".to_owned()),
            ..Default::default()
        })
        .await
        .context("a well-formed cursor for an id that never existed")?;
    assert!(page.data.is_empty());

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 15

/// F5: a replay answers what the original answered, even when the request
/// would no longer be valid.
///
/// The configuration a create is validated against is not fixed: an operator
/// can disable a rail, or drop a currency, between a merchant's request and
/// their retry. Validating before claiming the key made the retry a `400`
/// for an intent that already existed — the merchant's own bookkeeping says
/// the payment intent was never created, while vpay holds one.
///
/// The second server is the honest way to stage that: same database, same
/// merchants, same signing key, `mtn_momo` disabled — which is what
/// `vpay-server` comes up as after an operator edits `application.yml` and
/// redeploys.
#[tokio::test]
async fn a_replay_survives_the_rail_being_disabled() -> anyhow::Result<()> {
    let harness = harness().await?;
    let opts = RequestOptions::new().with_idempotency_key("order-9001-create");

    let created = harness
        .a()
        .payment_intents()
        .create(create_params(), opts.clone())
        .await
        .context("the original create, while the rail was enabled")?;

    // The redeploy: a second server over the same database, with the rail
    // the intent was created for switched off.
    let (pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();
    let (_pem_c, jwks_c) = generate_key();
    let (server_pem, _server_jwks) = generate_key();
    let served = serve(&harness.repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b, jwks_c, false)
    })
    .await?;
    let after_change = vpay_sdk::Client::builder(&served.base_url)
        .credentials(
            vpay_sdk::Credentials::rsa_pem(CLIENT_A, &pem_a).expect("the generated PEM parses"),
        )
        .build()
        .expect("the SDK client builds");

    // The rail really is gone: a *new* create naming it is refused.
    let error = after_change
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .expect_err("a disabled rail may not be named on a new intent");
    let (status, _kind, _code, param) = api_error(error);
    assert_eq!(status, 400);
    assert_eq!(param.as_deref(), Some("payment_method_types"));

    // The retry of the original request, under its own key, on the changed
    // deployment: the stored answer, unchanged.
    let replayed = after_change
        .payment_intents()
        .create(create_params(), opts)
        .await
        .context("the replay must answer what the original answered")?;
    assert_eq!(
        replayed, created,
        "a replay is the stored response; what the deployment would answer today is irrelevant"
    );

    // And no second intent was created by any of it.
    let rows = count(
        &harness.pool,
        "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(rows, 1);

    served.server.abort();
    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 16

/// A `POST` under a key whose first request has not finished is answered with
/// its **own** code — `idempotency_key_in_flight` — and not the
/// `invalid_state` a lifecycle conflict carries.
///
/// The distinction is what a merchant's client branches on: `invalid_state`
/// means "your object moved on, look at it", while this means "your own
/// earlier call is still running, wait". Until this had its own variant both
/// rendered as `409 invalid_request_error / invalid_state` and differed only
/// in an English sentence.
///
/// **How the in-flight state is reached, and why not by racing.** The window
/// is however long the first `POST` takes, so two concurrent requests would
/// usually observe a *replay* and only occasionally this — a flaky test
/// asserting the wrong thing most of the time. Instead the first request is
/// made for real and its now-`complete` row is put back into `in_flight`,
/// which is the state that same row was in while the request was running.
/// Nothing is stubbed: the row, its `request_hash` and the retry that meets
/// it are all the server's own. Rebuilding the row from the test side
/// instead would mean recomputing the hash, and therefore guessing the path
/// `axum`'s `nest` leaves on the URI — guess wrong and the claim is a
/// `Mismatch`, i.e. the *neighbouring* error this test exists to tell apart.
#[tokio::test]
async fn a_key_whose_first_request_is_still_running_is_answered_with_its_own_code()
-> anyhow::Result<()> {
    const PATH: &str = "/v1/payment_intents";
    const BODY: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo";
    const KEY: &str = "first-attempt-still-running";

    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let created = http
        .post(harness.url(PATH))
        .bearer_auth(&bearer)
        .header("Idempotency-Key", KEY)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(BODY)
        .send()
        .await
        .context("the first POST under this key")?;
    assert_eq!(created.status().as_u16(), 200);

    // Back to the state that row was in while the request above was still
    // executing: claimed, with nothing stored to replay.
    let reopened = sqlx::query(
        "UPDATE idempotency_keys SET state = 'in_flight', response_status = NULL, \
         response_body = NULL, completed_at = NULL \
         WHERE merchant_id = $1 AND idempotency_key = $2",
    )
    .bind(MERCHANT_A)
    .bind(KEY)
    .execute(&harness.pool)
    .await
    .context("re-opening the stored claim")?;
    assert_eq!(
        reopened.rows_affected(),
        1,
        "the first request must have left exactly one claimed row"
    );

    let response = http
        .post(harness.url(PATH))
        .bearer_auth(&bearer)
        .header("Idempotency-Key", KEY)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(BODY)
        .send()
        .await
        .context("a POST under a key that is still in flight")?;

    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    assert_eq!(
        status,
        // The policy row for `Category::Idempotency`, asked rather than
        // hard-coded: this test is about the `code`, and pinning the number
        // here as well would make it fail for the unrelated reason of
        // someone deliberately moving that row (see
        // `ApiError::IdempotencyKeyInFlight` on Stripe's 409).
        vpay_core::Category::Idempotency.http_status(),
        "got {body:#}"
    );
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("idempotency_error"),
        "an in-flight key is an idempotency problem, not a state problem: {body:#}"
    );
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("idempotency_key_in_flight"),
        "this is the assertion that fails if the answer goes back to a Conflict: {body:#}"
    );
    assert_eq!(
        body.pointer("/error/message").and_then(Value::as_str),
        Some("A request with this Idempotency-Key is still in progress; retry shortly."),
    );
    assert!(
        !serde_json::to_string(&body)?.contains(KEY),
        "the key itself must not be echoed into the body: {body:#}"
    );

    // And the refused request created nothing: the answer is "wait", not
    // "here is a second intent".
    let rows = count(
        &harness.pool,
        "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(
        rows, 1,
        "only the first request's intent may exist; an in-flight refusal creates nothing"
    );

    harness.shutdown().await;
    Ok(())
}
// ----------------------------------------------------------------- test 17

/// A `store` that fails **after** the work is done still hands the key back,
/// so the merchant's retry re-executes rather than being answered "still in
/// progress" for 24 hours.
///
/// `PostRequest::finish` used to `?`-return on this path with the key still
/// claimed. That is the same stuck-`in_flight` bug test 12 pins for a `5xx`,
/// reached through a different door: there the response *is* the failure and
/// the release is obvious, here the response is a perfectly good `200` and
/// only the bookkeeping write fails. Nothing else in the system ever moves
/// such a row, so the merchant's key was dead until it expired.
///
/// # Why the failure is injected in Postgres
///
/// Every other way to make `store` fail is unreachable from a real handler:
/// the response body is always JSON (`json_response`/`value_response`, or
/// `ApiError::into_response`) and always far under `V1_BODY_LIMIT_BYTES`, so
/// the two sibling paths in `finish` cannot be provoked from outside at all.
/// What is left is the write itself, and the honest way to make a real write
/// fail is to make the real database refuse it. The trigger below is fault
/// injection at the infrastructure layer — the same posture `AGENTS.md` takes
/// with WireMock for rails — not a stub inside the process under test: the
/// server, the repository and the SQL are all the shipping ones, and the
/// server never learns that anything is unusual.
///
/// The trigger fires only for the `UPDATE ... SET state = 'complete'` that
/// `store` issues, so `claim` still inserts and `release` still deletes; if
/// it caught those too, the test could not tell "released" from "never
/// claimed".
///
/// Decisive: put the `?` back on `idempotency::store` in
/// `PostRequest::finish` and the second assertion fails — the row is still
/// there, `in_flight`, and the retry is answered
/// `idempotency_key_in_flight`.
#[tokio::test]
async fn a_failed_store_releases_the_key_so_the_retry_re_executes() -> anyhow::Result<()> {
    const PATH: &str = "/v1/payment_intents";
    const BODY: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo";
    const KEY: &str = "store-fails-after-the-work";

    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    sqlx::raw_sql(
        "CREATE FUNCTION refuse_completion() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'injected: completing an idempotency key failed'; END; \
         $$ LANGUAGE plpgsql; \
         CREATE TRIGGER refuse_completion BEFORE UPDATE ON idempotency_keys \
         FOR EACH ROW WHEN (NEW.state = 'complete') EXECUTE FUNCTION refuse_completion();",
    )
    .execute(&harness.pool)
    .await
    .context("installing the fault injection must succeed")?;

    let response = http
        .post(harness.url(PATH))
        .bearer_auth(&bearer)
        .header("Idempotency-Key", KEY)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(BODY)
        .send()
        .await
        .context("a POST whose response cannot be stored")?;
    assert_eq!(
        response.status().as_u16(),
        // `DbError::Query` is `Category::Storage`, so the refused write is
        // reported as one — asked of the policy table rather than pinned as
        // a literal, for the same reason test 16 does it. What this test is
        // about is the row, not the number; the status is asserted only so a
        // silent `200` (a response reported as stored when it was not)
        // cannot pass.
        vpay_core::Category::Storage.http_status(),
        "a response that cannot be recorded for replay is vpay's failure, and is reported as one"
    );

    // The intent itself was created — that write happened before the one
    // that failed — which is exactly why the key has to be handed back
    // rather than frozen around a request the merchant was told failed.
    let intents = count(
        &harness.pool,
        "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(intents, 1);

    // The assertion this test exists for.
    let held = count(
        &harness.pool,
        "SELECT count(*) FROM idempotency_keys WHERE merchant_id = $1 \
         AND idempotency_key = 'store-fails-after-the-work'",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(
        held, 0,
        "a store that failed must release the claim; leaving it in_flight answers every retry \
         under this key `in progress` until it expires"
    );

    // And the merchant's retry re-executes rather than meeting the claim.
    // The fault is lifted first, because a retry that also failed to store
    // would answer 500 for a reason that says nothing about the claim.
    sqlx::raw_sql("DROP TRIGGER refuse_completion ON idempotency_keys;")
        .execute(&harness.pool)
        .await
        .context("lifting the fault injection must succeed")?;

    let response = http
        .post(harness.url(PATH))
        .bearer_auth(&bearer)
        .header("Idempotency-Key", KEY)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(BODY)
        .send()
        .await
        .context("the retry under the released key")?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.context("the body is JSON")?;
    assert_eq!(
        status, 200,
        "the released key must be claimable again; got {body:#}"
    );
    assert_ne!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("idempotency_key_in_flight"),
        "the key was left claimed: {body:#}"
    );

    // Re-executing is what "released" means: a second intent, and the retry
    // is now the replayable one.
    let intents = count(
        &harness.pool,
        "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
        MERCHANT_A,
    )
    .await?;
    assert_eq!(
        intents, 2,
        "the first attempt's intent plus the re-executed retry's — the documented cost of \
         releasing a key whose response could not be stored"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------ tests 18-22: Stripe SDKs
//
// Step 5b (`docs/plans/2026-09-03-step5b-stripe-sdk.md`): a merchant driving
// vpay with the *official* Stripe SDK plus a `config.authenticator`. What
// those SDKs need from the server is not the envelope — that already matches
// — but two response headers and one honest refusal, and none of the three
// is observable from a handler unit test: `request-id` is set by a
// middleware layer, `stripe-should-retry` is set by the one renderer every
// layer's failures pass through, and both have to survive the whole stack on
// responses that no handler produced (a 404 fallback, a 401 decided before
// routing).

/// The Stripe SDK spelling of the request id, and the one this API has
/// always used, side by side on the response.
fn request_id_headers(response: &reqwest::Response) -> (Option<String>, Option<String>) {
    let read = |name: &str| {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().expect("a request id is ascii").to_owned())
    };
    (read("x-request-id"), read("request-id"))
}

/// `stripe-should-retry` as an SDK would read it, or `None` when absent.
fn retry_advice(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get("stripe-should-retry")
        .map(|value| value.to_str().expect("the advisory is ascii").to_owned())
}

// ----------------------------------------------------------------- test 18

/// Every response carries the request id under **both** names, with one
/// value — on a success, on a `400`, on the `404` fallback and on the `401`
/// the authentication boundary decides before routing.
///
/// stripe-node populates `err.requestId` and `obj.lastResponse.requestId`
/// from `headers['request-id']` and looks at no other name, so without this
/// header `Category::Internal`'s public sentence — "Contact support with the
/// request id" — is an instruction a Stripe SDK user cannot follow.
///
/// The four responses are chosen because they are produced in four different
/// places: a handler, an extractor rejection, the router's fallback, and the
/// middleware in front of `/v1`. A layer ordering that put the mirror inside
/// the authentication boundary would pass on the first and fail on the
/// last. **The equality is the assertion**: `request-id` alone would also be
/// satisfied by a second id generator, and a merchant quoting an id support
/// cannot find is worse than a merchant quoting none.
#[tokio::test]
async fn every_response_carries_the_request_id_under_stripes_header_too() -> anyhow::Result<()> {
    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);
    const BODY: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo";

    // 200 — a handler's own response.
    let created = http
        .post(harness.url("/v1/payment_intents"))
        .bearer_auth(&bearer)
        .header("Idempotency-Key", "request-id-on-a-success")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(BODY)
        .send()
        .await
        .context("a create that succeeds")?;

    // 400 — an extractor rejection, decided before the handler runs.
    let no_key = http
        .post(harness.url("/v1/payment_intents"))
        .bearer_auth(&bearer)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(BODY)
        .send()
        .await
        .context("a create with no Idempotency-Key")?;

    // 404 — the router's fallback, with no handler involved at all.
    let unrouted = http
        .get(harness.url("/not_a_vpay_route"))
        .send()
        .await
        .context("an unrouted path")?;

    // 401 — the authentication boundary, outside every handler.
    let unauthenticated = http
        .get(harness.url("/v1/payment_intents"))
        .send()
        .await
        .context("a /v1 request with no bearer token")?;

    for (label, expected_status, response) in [
        ("200", 200, created),
        ("400", 400, no_key),
        ("404", 404, unrouted),
        ("401", 401, unauthenticated),
    ] {
        assert_eq!(response.status().as_u16(), expected_status, "{label}");
        let (x_request_id, stripe) = request_id_headers(&response);
        let x_request_id = x_request_id
            .unwrap_or_else(|| panic!("{label}: no x-request-id on the response at all"));
        assert!(!x_request_id.is_empty(), "{label}");
        assert_eq!(
            stripe.as_deref(),
            Some(x_request_id.as_str()),
            "{label}: `request-id` must be the same id, not a second one"
        );
    }

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 19

/// The two answers whose HTTP status tells a Stripe SDK the *opposite* of
/// what vpay means carry `stripe-should-retry`, and it says what vpay means.
///
/// stripe-node's `RequestSender._shouldRetry` consults the header above its
/// own rules, and its own rules retry every `>= 500` **and every 409**,
/// unconditionally, twice by default:
///
/// * the `502` from an unreachable rail is `Category::Rail`, whose policy is
///   `Retry::AfterBackoff` — so `true`, which agrees with what the SDK would
///   have done anyway;
/// * the `409` from "one charge per intent, forever" is `Category::Conflict`,
///   `Retry::Never` — so `false`, and this is the one that changes what the
///   SDK does. Without it a merchant's lifecycle refusal is re-POSTed twice
///   before they see it.
///
/// Both come out of the same two-confirm sequence
/// `a_5xx_releases_its_idempotency_key_so_the_retry_re_executes` uses, and
/// that test is what pins the accompanying claim this one does not repeat:
/// re-executing the released key does **not** produce a second charge.
/// Nothing here is stubbed — the rail is unreachable by configuration and
/// the `502` is the real adapter over the real HTTP client.
#[tokio::test]
async fn a_rail_failure_and_a_conflict_carry_opposite_retry_advice() -> anyhow::Result<()> {
    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let intent = harness
        .a()
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to confirm")?;

    let confirm = |key: &'static str| {
        http.post(harness.url(&format!("/v1/payment_intents/{}/confirm", intent.id)))
            .bearer_auth(&bearer)
            .header("Idempotency-Key", key)
            .header("content-type", "application/x-www-form-urlencoded")
            .body("payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000")
            .send()
    };

    let unreachable_rail = confirm("advice-first").await.context("the first confirm")?;
    assert_eq!(
        unreachable_rail.status().as_u16(),
        502,
        "this suite's rail is unreachable by configuration"
    );
    assert_eq!(
        retry_advice(&unreachable_rail).as_deref(),
        Some("true"),
        "`Category::Rail` is `Retry::AfterBackoff`; the rail may answer later"
    );

    let already_charged = confirm("advice-second")
        .await
        .context("the second confirm")?;
    assert_eq!(already_charged.status().as_u16(), 409);
    let advice = retry_advice(&already_charged);
    let body: Value = already_charged
        .json()
        .await
        .context("the 409 body is JSON")?;
    // Which 409 this is matters, and it is read off the *same* response as
    // the header above: the in-flight refusal is a 400 and is the next
    // test's subject, and a `Conflict` about the object is what
    // `Retry::Never` is the correct advice for.
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("invalid_state"),
        "got {body:#}"
    );
    assert_eq!(
        advice.as_deref(),
        Some("false"),
        "stripe-node retries every 409 unconditionally; this is what stops it"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 20

/// The `400` for a key whose first request has not finished says **retry**,
/// which is the opposite of what its status code says to a Stripe SDK.
///
/// stripe-node retries no `4xx` at all, and this is the one refusal on this
/// surface that clears itself: the first attempt landing is what ends it.
/// The status stays `400` — `Category::Idempotency`'s policy row, and a
/// maintainer decision recorded on `ApiError::IdempotencyKeyInFlight` — so
/// the header is the only thing that can carry the correct advice.
///
/// The in-flight state is reached the way test 16 reaches it, and for the
/// same reason: racing two requests would usually observe a replay instead.
/// The row, its hash and the retry that meets it are all the server's own.
#[tokio::test]
async fn an_in_flight_idempotency_key_tells_a_stripe_sdk_to_retry_its_400() -> anyhow::Result<()> {
    const PATH: &str = "/v1/payment_intents";
    const BODY: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo";
    // The shape stripe-node generates for every v1 POST
    // (`_defaultIdempotencyKey`), so this also shows vpay accepts one.
    const KEY: &str = "stripe-node-retry-11111111-2222-3333-4444-555555555555";

    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let post = || {
        http.post(harness.url(PATH))
            .bearer_auth(&bearer)
            .header("Idempotency-Key", KEY)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(BODY)
            .send()
    };

    assert_eq!(
        post()
            .await
            .context("the first POST under this key")?
            .status()
            .as_u16(),
        200,
        "a `stripe-node-retry-<uuid>` key is inside vpay's 255-byte printable-ASCII rule"
    );

    let reopened = sqlx::query(
        "UPDATE idempotency_keys SET state = 'in_flight', response_status = NULL, \
         response_body = NULL, completed_at = NULL \
         WHERE merchant_id = $1 AND idempotency_key = $2",
    )
    .bind(MERCHANT_A)
    .bind(KEY)
    .execute(&harness.pool)
    .await
    .context("re-opening the stored claim")?;
    assert_eq!(reopened.rows_affected(), 1);

    let in_flight = post().await.context("a POST under a key still in flight")?;
    assert_eq!(in_flight.status().as_u16(), 400);
    assert_eq!(
        retry_advice(&in_flight).as_deref(),
        Some("true"),
        "the one 4xx on this surface that heals on its own must say so, or a \
         Stripe SDK will never retry it"
    );

    let body: Value = in_flight.json().await.context("the body is JSON")?;
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("idempotency_key_in_flight"),
        "got {body:#}"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 21

/// `confirm=true` on create is refused, naming `confirm`; `confirm=false` is
/// honoured, because that is what this endpoint does anyway.
///
/// `CreateParams` has no `deny_unknown_fields`, so before Step 5b a merchant
/// who copied Stripe's most common snippet got a `200`, an intent in
/// `requires_payment_method`, and the belief that they had charged someone.
/// A silently-dropped field is the one incompatibility a merchant cannot
/// debug from the response.
///
/// The `confirm=false` half is not decoration: it is what fails if the
/// refusal is widened to "reject the field whenever it is present", which
/// would refuse a request vpay can satisfy exactly as written.
#[tokio::test]
async fn confirm_on_create_is_refused_and_names_the_field() -> anyhow::Result<()> {
    const PATH: &str = "/v1/payment_intents";
    const BODY: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo";

    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let create = |key: &'static str, body: String| {
        http.post(harness.url(PATH))
            .bearer_auth(&bearer)
            .header("Idempotency-Key", key)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
    };

    let refused = create("confirm-true", format!("{BODY}&confirm=true"))
        .await
        .context("a create asking to confirm")?;
    assert_eq!(refused.status().as_u16(), 400);
    let body: Value = refused.json().await.context("the body is JSON")?;
    assert_eq!(
        body.pointer("/error/type").and_then(Value::as_str),
        Some("invalid_request_error"),
        "got {body:#}"
    );
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("confirm"),
        "a Stripe SDK points its user at `error.param`: {body:#}"
    );
    assert!(
        body.pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("/v1/payment_intents/{id}/confirm")),
        "the refusal must name the endpoint that does the work: {body:#}"
    );

    // Nothing was created — the refusal is not a half-done create.
    assert_eq!(
        count(
            &harness.pool,
            "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
            MERCHANT_A,
        )
        .await?,
        0,
    );

    let accepted = create("confirm-false", format!("{BODY}&confirm=false"))
        .await
        .context("a create explicitly asking not to confirm")?;
    assert_eq!(
        accepted.status().as_u16(),
        200,
        "`confirm=false` asks for exactly what this endpoint does"
    );
    let created: Value = accepted.json().await.context("the body is JSON")?;
    assert_eq!(
        created.get("status").and_then(Value::as_str),
        Some("requires_payment_method"),
        "got {created:#}"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 22

/// Every header and parameter stripe-node adds of its own accord is accepted
/// and ignored, rather than refused.
///
/// Step 5b's §2 claims "axum ignores unknown headers already — no code
/// change" and "`expand` is already dropped silently"; this is the evidence
/// for the claim, so that a future `deny_unknown_fields`, a stricter header
/// policy or a `Stripe-Version` check breaks a test here instead of breaking
/// a merchant. It is deliberately one request carrying all of them at once,
/// because that is what a real stripe-node call looks like on the wire.
///
/// `Stripe-Account` in particular is accepted rather than refused: vpay has
/// no Connect, and a `400` naming a header a merchant deliberately set is a
/// worse diagnostic than a documented "Connect is not a thing here".
#[tokio::test]
async fn a_request_shaped_the_way_stripe_node_sends_it_is_accepted() -> anyhow::Result<()> {
    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let response = http
        .post(harness.url("/v1/payment_intents"))
        .bearer_auth(&bearer)
        .header("Idempotency-Key", "stripe-node-retry-abc-123")
        .header("content-type", "application/x-www-form-urlencoded")
        // The exact set `RequestSender._makeHeaders` builds, minus the ones
        // this suite already sends. `Stripe-Version` is advertised nowhere
        // and echoed nowhere; a merchant pinning one is pinning nothing.
        .header("Stripe-Version", "2026-08-26.dahlia")
        .header("Stripe-Account", "acct_not_a_thing_here")
        .header("X-Stripe-Client-User-Agent", r#"{"lang":"node"}"#)
        .header(
            "X-Stripe-Client-Telemetry",
            r#"{"last_request_metrics":{}}"#,
        )
        .header("User-Agent", "Stripe/v1 NodeBindings/22.6.1")
        // Indexed arrays, which is how stripe-node encodes them, and
        // `expand`, which vpay does not implement and must not choke on.
        .body(
            "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo\
             &expand[0]=charge&metadata[order_id]=1234",
        )
        .send()
        .await
        .context("a create shaped the way stripe-node sends one")?;

    assert_eq!(
        response.status().as_u16(),
        200,
        "no header or parameter stripe-node adds may turn a valid create into a refusal"
    );
    let created: Value = response.json().await.context("the body is JSON")?;
    // The ignored `expand` did not become a field of the object either.
    assert!(created.get("charge").is_none(), "got {created:#}");
    assert_eq!(
        created
            .pointer("/metadata/order_id")
            .and_then(Value::as_str),
        Some("1234"),
        "got {created:#}"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 23

/// The fields that change **where or when money moves** are refused on
/// create, and the refusal hands the `Idempotency-Key` back.
///
/// Two claims, and the second is the one the `confirm=true` test could not
/// make: it used a different key for its accepted request, so it showed the
/// refusal happened but not that the key survived it. Here the *same* key
/// carries the refused request and then the corrected one. If the refusal
/// stored its 400, the retry would replay the 400; if it left the claim
/// in_flight, the retry would be `idempotency_key_in_flight`. Only a
/// released key produces the `200` asserted below.
///
/// `capture_method=manual` is the case a merchant actually hits: it is one
/// word in a copied Stripe snippet, and ignoring it would take the payer's
/// money at a moment the merchant believes it is only being authorised.
#[tokio::test]
async fn a_refused_create_hands_its_idempotency_key_back() -> anyhow::Result<()> {
    const PATH: &str = "/v1/payment_intents";
    const BODY: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo";
    const KEY: &str = "order-77-attempt-1";

    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let post = |key: &'static str, body: String| {
        http.post(harness.url(PATH))
            .bearer_auth(&bearer)
            .header("Idempotency-Key", key)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
    };

    // Every refused field, each under its own key, each naming itself.
    for (suffix, param) in [
        ("&capture_method=manual", "capture_method"),
        ("&application_fee_amount=250", "application_fee_amount"),
        ("&transfer_data[destination]=acct_x", "transfer_data"),
        ("&on_behalf_of=acct_x", "on_behalf_of"),
    ] {
        let refused = post(param, format!("{BODY}{suffix}"))
            .await
            .with_context(|| format!("a create carrying `{suffix}`"))?;
        assert_eq!(refused.status().as_u16(), 400, "{suffix}");
        let body: Value = refused.json().await.context("the body is JSON")?;
        assert_eq!(
            body.pointer("/error/type").and_then(Value::as_str),
            Some("invalid_request_error"),
            "{suffix}: got {body:#}"
        );
        assert_eq!(
            body.pointer("/error/param").and_then(Value::as_str),
            Some(param),
            "{suffix}: a Stripe SDK points its user at `error.param`: {body:#}"
        );
        assert!(
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("does not support")),
            "{suffix}: the refusal must say vpay cannot do it: {body:#}"
        );
    }

    // Nothing was created by any of them.
    assert_eq!(
        count(
            &harness.pool,
            "SELECT count(*) FROM payment_intents WHERE merchant_id = $1",
            MERCHANT_A,
        )
        .await?,
        0,
    );

    // The key-release claim, on one key: refused, then corrected, then
    // through.
    let refused = post(KEY, format!("{BODY}&capture_method=manual"))
        .await
        .context("the refused attempt under the shared key")?;
    assert_eq!(refused.status().as_u16(), 400);

    let corrected = post(KEY, BODY.to_owned())
        .await
        .context("the corrected attempt under the same key")?;
    assert_eq!(
        corrected.status().as_u16(),
        200,
        "a refusal that stored its 400, or left the claim in flight, would answer 4xx here"
    );
    let created: Value = corrected.json().await.context("the body is JSON")?;
    assert_eq!(
        created.get("status").and_then(Value::as_str),
        Some("requires_payment_method"),
        "got {created:#}"
    );

    // `capture_method=automatic` asks for exactly what vpay does, so it is
    // accepted — the half that fails if the refusal is widened to "the field
    // is present".
    let automatic = post(
        "order-77-automatic",
        format!("{BODY}&capture_method=automatic"),
    )
    .await
    .context("a create asking for the capture behaviour vpay already has")?;
    assert_eq!(automatic.status().as_u16(), 200);

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 24

/// The same refusal on `confirm`, where a merchant who was refused on create
/// would otherwise get the field silently ignored one request later.
///
/// stripe-node's `paymentIntents.confirm` takes `capture_method` and
/// `application_fee_amount` too, so refusing only on create would leave the
/// exact same misunderstanding one call further along — and this one is
/// worse, because the confirm is the request that moves the money.
///
/// The intent is left untouched by the refusal: the check runs before the
/// key is claimed and long before any charge row exists.
///
/// The second half is the one claim 16 makes that the refusals alone do not
/// show — **the refusal leaves the `Idempotency-Key` unspent**, as test 23
/// pins on the create side. A refusal that had stored its `400`, or left the
/// claim in flight, would answer `400` `idempotency_key_in_use` to the
/// corrected confirm under the same key instead of letting it reach the rail.
#[tokio::test]
async fn a_confirm_refuses_the_same_fields_a_create_does() -> anyhow::Result<()> {
    const PMD: &str =
        "payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000";
    // The one key carried across a refusal and its correction.
    const KEY: &str = "confirm-refused-then-corrected";

    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let intent = harness
        .a()
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to confirm")?;

    let confirm = |key: &'static str, body: String| {
        http.post(harness.url(&format!("/v1/payment_intents/{}/confirm", intent.id)))
            .bearer_auth(&bearer)
            .header("Idempotency-Key", key)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
    };

    let refused = confirm("confirm-manual", format!("{PMD}&capture_method=manual"))
        .await
        .context("a confirm asking for a manual capture")?;
    assert_eq!(refused.status().as_u16(), 400);
    let body: Value = refused.json().await.context("the body is JSON")?;
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("capture_method"),
        "got {body:#}"
    );

    let connect = confirm(
        "confirm-connect",
        format!("{PMD}&transfer_data[destination]=acct_x"),
    )
    .await
    .context("a confirm asking to route the money elsewhere")?;
    assert_eq!(connect.status().as_u16(), 400);
    let body: Value = connect.json().await.context("the body is JSON")?;
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("transfer_data"),
        "got {body:#}"
    );

    // No charge was attempted by either refusal, and the intent is where it
    // was — a refused confirm is not a half-done one.
    assert_eq!(
        count(
            &harness.pool,
            "SELECT count(*) FROM charges WHERE payment_intent_id = $1",
            &intent.id,
        )
        .await?,
        0,
    );
    let unchanged = harness
        .a()
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("reading the intent back")?;
    assert_eq!(unchanged.status, intent.status);

    // The key-release claim on the confirm side: refused, then corrected,
    // under one key.
    let refused = confirm(KEY, format!("{PMD}&application_fee_amount=100"))
        .await
        .context("the refused attempt under the shared key")?;
    assert_eq!(refused.status().as_u16(), 400);
    let body: Value = refused.json().await.context("the body is JSON")?;
    assert_eq!(
        body.pointer("/error/param").and_then(Value::as_str),
        Some("application_fee_amount"),
        "got {body:#}"
    );

    // Nothing was written under the key at all — not a stored response, not
    // an in-flight claim. The refusal runs before `claim_or_answer`, so
    // there is no row to release.
    assert_eq!(
        count(
            &harness.pool,
            "SELECT count(*) FROM idempotency_keys WHERE idempotency_key = $1",
            KEY,
        )
        .await?,
        0,
        "a refused confirm must leave the key unclaimed, not spent",
    );

    let corrected = confirm(KEY, PMD.to_owned())
        .await
        .context("the corrected attempt under the same key")?;
    assert_eq!(
        corrected.status().as_u16(),
        502,
        "the corrected confirm reached the rail — this suite's rail is \
         unreachable by configuration, which is test 5's outcome. A refusal \
         that had stored its 400, or left the claim in flight, would answer \
         400 `idempotency_key_in_use` here instead",
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 25

/// A **replayed** error response carries the same `stripe-should-retry` the
/// original did — proven against a real Postgres, through the real replay
/// path.
///
/// This test used to pin the opposite, and the sentence it used to carry is
/// worth keeping: `v1::payment_intents::replay` rebuilt a response from a
/// stored status and body, the advisory was neither, so a merchant retrying
/// under a key whose stored answer was a `409` got that `409` bare and
/// stripe-node applied its own "retry every 409" rule to a refusal waiting
/// cannot fix. Migration `0025` adds `idempotency_keys.response_retry`,
/// `PostRequest::finish` stores what the rendered response actually carried,
/// and `replay` writes those bytes back.
///
/// The stored `409` is reached the way test 19 reaches it, through the real
/// "one charge per intent, forever" refusal — not by editing a row. **The
/// two responses are asserted to be the same body**, so the header is the
/// only thing left that could differ, and it is asserted equal rather than
/// merely present: a `replay` that hard-coded `false` would pass a presence
/// check and fail this one the day a `true`-advised error becomes storable.
#[tokio::test]
async fn a_replayed_error_carries_the_same_retry_advisory_the_original_did() -> anyhow::Result<()> {
    const KEY: &str = "replay-keeps-the-advisory";

    let harness = harness().await?;
    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let intent = harness
        .a()
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent to confirm")?;

    let confirm = |key: &'static str| {
        http.post(harness.url(&format!("/v1/payment_intents/{}/confirm", intent.id)))
            .bearer_auth(&bearer)
            .header("Idempotency-Key", key)
            .header("content-type", "application/x-www-form-urlencoded")
            .body("payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000")
            .send()
    };

    // The first confirm commits a `submitting` charge and then fails at the
    // unreachable rail: a 502, which is *released* rather than stored.
    assert_eq!(
        confirm("advisory-replay-first")
            .await
            .context("the first confirm")?
            .status()
            .as_u16(),
        502,
        "this suite's rail is unreachable by configuration"
    );

    // The second meets `one_charge_per_intent` and is the 409 that gets
    // stored under KEY.
    let original = confirm(KEY).await.context("the stored 409")?;
    assert_eq!(original.status().as_u16(), 409);
    let original_advice = retry_advice(&original);
    assert_eq!(
        original_advice.as_deref(),
        Some("false"),
        "rendered fresh, the advisory is there and says what the classification says"
    );
    let original_body: Value = original.json().await.context("the 409 body is JSON")?;

    // The same key again: the stored response, replayed.
    let replayed = confirm(KEY).await.context("the replay of the stored 409")?;
    assert_eq!(replayed.status().as_u16(), 409);
    assert_eq!(
        retry_advice(&replayed),
        original_advice,
        "a replay must re-emit the advisory the response it replays carried — equality with \
         the original, not merely presence, is what rules out a hard-coded value in `replay`"
    );
    let replayed_body: Value = replayed.json().await.context("the replay body is JSON")?;
    assert_eq!(
        replayed_body, original_body,
        "the body really is the stored one, so the two responses now agree on all three of \
         status, body and advisory"
    );

    harness.shutdown().await;
    Ok(())
}
