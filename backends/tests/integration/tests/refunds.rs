//! `GET /v1/refunds/{id}`, end to end: the real `vpay_api::router` on a real
//! socket, over a real Postgres, driven by the real merchant SDK.
//!
//! Issue #45's decision, and the whole subject of this file: **a refund must
//! have an authoritative read.** `docs/flows/provider-port.md` calls
//! `query_status` "the authoritative read"; `docs/flows/webhooks.md` says
//! delivery is at-least-once and unordered; and `charge.refunded` /
//! `charge.refund.updated` are documented event types that **nothing emits**
//! (`docs/status.md`). Without this route a merchant holding a `re_…` in
//! `pending` has no call and no event that answers what happened to it.
//!
//! The five claims only this file can make:
//!
//! 1. a stored refund reads back through the shipping SDK as the nine keys
//!    `docs/flows/merchant-auth.md` documents;
//! 2. another merchant's refund and an id that never existed are the **byte
//!    for byte** identical `404`, and it is the `resource_missing` envelope —
//!    not the `unknown_route` one an unmounted route would answer, which is
//!    the difference a status-code-only assertion would miss;
//! 3. the API response and an event's `data.object` for the same row are
//!    byte-identical, because one renderer produces both;
//! 4. an id that is not `re_…` is never looked up **even when a row exists
//!    behind it** — the short-circuit is reached, and it answers the same
//!    `404` (added by review 2026-09-05: this claim was the one nothing
//!    checked, and deleting the short-circuit passed everything else);
//! 5. `POST /v1/refunds` is **still** the honest `404`.
//!
//! # Why the rows are written here and not created through `/v1`
//!
//! Nothing in this repository creates a refund. `POST /v1/refunds` is
//! declared and unrouted because it needs `ProviderAdapter::refund`, which is
//! `NotImplemented` on MTN (refunds are the Disbursements product) and
//! `Unsupported` on Orange (its Web Payment product documents no refund API).
//! So the rows below are `INSERT`ed by this suite against the real schema,
//! the way `support::age_the_crash` writes a column no shipping code writes:
//! **`vpay_db::Refunds` deliberately exposes no `create`**, because a write
//! path no shipping code calls is a feature this repository would be claiming
//! it has (`AGENTS.md` rule 2).
//!
//! What that costs, stated rather than hidden: these tests prove the read,
//! the tenancy and the rendering. They prove **nothing** about how a refund
//! comes to exist, because that code does not exist.

// See `tests/support/mod.rs` for why this allow list mirrors the other
// integration suites'.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, MERCHANT_AUDIENCE, ProviderHost};
use vpay_db::{NewEvent, Repositories, TxOutcome, UnitOfWork as _};
use vpay_sdk::{
    CreatePaymentIntentParams, Credentials, PaymentMethodType, RefundStatus, RequestOptions,
};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

/// The merchant every test acts as, and the tenant it acts for. Never the
/// same string: a query filtered by `client_id` instead of `merchant_id`
/// would otherwise pass.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The second merchant, which exists only so the tenancy case has someone
/// else's refund to fail to read.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

const RAIL: &str = "mtn_momo";
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;
const REFUND_AMOUNT: i64 = 2500;

/// An id of exactly the shape `vpay_core::ids::refund_id` mints, that no
/// merchant has ever had.
const MISSING_REFUND_ID: &str = "re_00000000000000000000000x";

// ------------------------------------------------------------------ harness

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    /// The plain `sqlx` pool: this suite writes `refunds` rows, and no
    /// repository method does — see the module header.
    pool: PgPool,
    base_url: String,
    pem_a: String,
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

    fn a(&self) -> vpay_sdk::Client {
        self.sdk(CLIENT_A, &self.pem_a)
    }

    /// A bearer token for `client_id`, minted with the server's own signer —
    /// the same shape the OP mints, for the raw requests the SDK cannot make
    /// (a byte-level body comparison, and a `POST` to an unrouted path).
    fn bearer(&self, client_id: &str) -> String {
        self.signing_key
            .token_manager()
            .issue_client_token_with_extra(
                client_id,
                900,
                Some(vpay_api::SCOPE_PAYMENTS_WRITE.to_owned()),
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

/// Two merchants, one rail, one currency, `livemode: false`.
///
/// The rail is configured and unreachable, exactly as `payment_intents.rs`
/// configures it: nothing here confirms anything, but boot step 4 has to seed
/// `providers` and `currencies` or the `payment_intents` a refund references
/// could not be created at all.
fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "refunds".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![ProviderHost {
            code: RAIL.to_owned(),
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
                (
                    "subscription_key".to_owned(),
                    "stub-subscription-key".to_owned(),
                ),
                ("api_key".to_owned(), "stub-api-key".to_owned()),
            ]),
        }],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            merchant_client(CLIENT_A, MERCHANT_A, jwks_a),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    // Merchant B is registered so its bearer token resolves to a real
    // tenant, and is never driven through the SDK: every request it makes
    // below is a raw one, because what those cases compare is bytes.
    let (_pem_b, jwks_b) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b)
    })
    .await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        pem_a,
        signing_key: served.signing_key,
    })
}

fn raw_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
}

fn create_params() -> CreatePaymentIntentParams {
    CreatePaymentIntentParams {
        amount: AMOUNT,
        currency: CURRENCY.to_owned(),
        payment_method_types: vec![PaymentMethodType::MtnMomo],
        metadata: BTreeMap::new(),
        description: None,
    }
}

/// Writes one `refunds` row against the real schema, and returns its id.
///
/// Raw SQL because there is no repository write and deliberately must not be
/// one — see the module header. Every column the read projects is set to a
/// value distinguishable from the intent's, so a query that returned the
/// wrong row would be visible rather than plausible.
///
/// The `currency_code` is the intent's own, upper-case, because
/// `refunds.currency_code` is a foreign key onto `currencies (code)` and
/// `docs/flows/money.md`'s rule is that a currency is carried verbatim and
/// never converted.
async fn seed_refund(
    pool: &PgPool,
    payment_intent_id: &str,
    status: &str,
    reason: Option<&str>,
) -> anyhow::Result<String> {
    seed_refund_with_id(
        pool,
        &vpay_core::ids::refund_id(),
        payment_intent_id,
        status,
        reason,
    )
    .await
}

/// [`seed_refund`], with the id chosen by the caller.
///
/// `refunds.id` is a bare `TEXT PRIMARY KEY` bounded only by migration
/// `0017`'s `id_length` CHECK, so the database will store an id that
/// `vpay_core::ids::refund_id` would never mint. Nothing in this repository
/// writes such a row — this suite is the only writer there is — and that is
/// exactly why the case below has to write one: it is the only way to reach
/// the `re_` short-circuit in `vpay_api::v1::refunds` with a row that really
/// exists behind it.
async fn seed_refund_with_id(
    pool: &PgPool,
    id: &str,
    payment_intent_id: &str,
    status: &str,
    reason: Option<&str>,
) -> anyhow::Result<String> {
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_intent_id, amount, currency_code, status, reason, metadata) \
         VALUES ($1, $2, $3, 'XAF', $4::refund_status, $5, $6)",
    )
    .bind(id)
    .bind(payment_intent_id)
    .bind(REFUND_AMOUNT)
    .bind(status)
    .bind(reason)
    .bind(json!({ "case": "45" }))
    .execute(pool)
    .await
    .context("seeding a refunds row")?;
    Ok(id.to_owned())
}

/// The intent a refund hangs off, created through the shipping SDK so the
/// foreign key points at a row `/v1` really made.
async fn seed_intent(client: &vpay_sdk::Client) -> anyhow::Result<String> {
    let intent = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .context("creating the intent a refund references")?;
    Ok(intent.id)
}

// ------------------------------------------------------------------ test 1

/// A stored refund reads back through the shipping SDK, as the nine keys the
/// wire contract documents.
///
/// Driven by `vpay_sdk::RefundsResource::retrieve` — the method a merchant
/// integrates, added in the same change as this route (ADR-0015 decision 2)
/// — rather than by a hand-rolled client, so the case proves the two halves
/// of the contract against each other.
#[tokio::test]
async fn a_stored_refund_reads_back_through_the_sdk() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent_id = seed_intent(&client).await?;
    let refund_id = seed_refund(
        &harness.pool,
        &intent_id,
        "pending",
        Some("requested_by_customer"),
    )
    .await?;

    let refund = client
        .refunds()
        .retrieve(&refund_id)
        .await
        .context("retrieving the seeded refund through the SDK")?;

    assert_eq!(refund.id, refund_id);
    assert_eq!(refund.object, "refund");
    assert_eq!(refund.amount, REFUND_AMOUNT);
    assert_eq!(refund.currency, CURRENCY, "lowercase on the wire");
    assert_eq!(refund.payment_intent, intent_id);
    assert_eq!(refund.status, RefundStatus::Pending);
    assert_eq!(refund.reason.as_deref(), Some("requested_by_customer"));
    assert_eq!(refund.metadata.get("case").map(String::as_str), Some("45"));
    assert!(refund.created > 0, "created is unix seconds, not zero");

    // The raw body carries exactly nine keys and no tenth: the SDK's typed
    // struct would silently ignore one it does not model.
    let response = raw_client()
        .get(harness.url(&format!("/v1/refunds/{refund_id}")))
        .bearer_auth(harness.bearer(CLIENT_A))
        .send()
        .await
        .context("the same read over raw HTTP")?;
    assert_eq!(response.status().as_u16(), 200);
    let body: Value = response.json().await.context("a JSON body")?;
    let object = body.as_object().expect("the response is a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "amount",
            "created",
            "currency",
            "id",
            "metadata",
            "object",
            "payment_intent",
            "reason",
            "status",
        ]
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 2

/// Merchant B cannot read merchant A's refund — and the refusal is **byte for
/// byte** the answer for an id that never existed.
///
/// Byte-identical is one half of the assertion; that the envelope is
/// `resource_missing` is the other, and it is load-bearing. Delete the
/// `/refunds/{id}` entry from `vpay_api::V1_ROUTES` and **every** request
/// below still answers `404` — the nest's fallback — so a comparison of the
/// two bodies alone would still pass while the route was gone. The
/// `resource_missing` / `No such refund:` assertions are what fail then.
///
/// `refunds` carries no `merchant_id`: the scope is a join onto the owning
/// intent (`vpay_db::Refunds::get_for_merchant`). Removing that join's
/// `p.merchant_id = $1` predicate makes the first request below a `200`.
#[tokio::test]
async fn merchant_b_cannot_read_merchant_as_refund() -> anyhow::Result<()> {
    let harness = harness().await?;

    let intent_id = seed_intent(&harness.a()).await?;
    let refund_id = seed_refund(&harness.pool, &intent_id, "pending", None).await?;

    let bearer_b = harness.bearer(CLIENT_B);
    let http = raw_client();

    let foreign = http
        .get(harness.url(&format!("/v1/refunds/{refund_id}")))
        .bearer_auth(&bearer_b)
        .send()
        .await
        .context("merchant B asks for merchant A's refund")?;
    let foreign_status = foreign.status().as_u16();
    let foreign_body = foreign.bytes().await.context("the body is readable")?;

    let missing = http
        .get(harness.url(&format!("/v1/refunds/{MISSING_REFUND_ID}")))
        .bearer_auth(&bearer_b)
        .send()
        .await
        .context("merchant B asks for an id that never existed")?;
    let missing_status = missing.status().as_u16();
    let missing_body = missing.bytes().await.context("the body is readable")?;

    assert_eq!(foreign_status, 404);
    assert_eq!(missing_status, 404);

    // The bodies differ only where the caller's own id is echoed back, which
    // is the id they sent — so substitute each request's own id out. Anything
    // else differing would be a distinguisher.
    let foreign_text = String::from_utf8_lossy(&foreign_body).replace(&refund_id, "<id>");
    let missing_text = String::from_utf8_lossy(&missing_body).replace(MISSING_REFUND_ID, "<id>");
    assert_eq!(
        foreign_text, missing_text,
        "another merchant's refund and an id that never existed must be indistinguishable"
    );

    // …and it is the *resource* envelope, which is what proves the route is
    // mounted at all. An unmounted `/v1/refunds/{id}` answers the nest's
    // `unknown_route` fallback, which is also a `404` and also identical for
    // both ids.
    let envelope: Value = serde_json::from_slice(&missing_body).context("a JSON body")?;
    assert_eq!(
        envelope.pointer("/error/code").and_then(Value::as_str),
        Some("resource_missing"),
        "got {envelope:#}"
    );
    assert_eq!(
        envelope.pointer("/error/message").and_then(Value::as_str),
        Some(format!("No such refund: {MISSING_REFUND_ID}").as_str()),
        "got {envelope:#}"
    );

    // An id of another resource's shape is the same answer again — the
    // `re_` short-circuit in `vpay_api::v1::refunds` saves the query and
    // changes nothing a caller can see.
    let wrong_shape = http
        .get(harness.url(&format!("/v1/refunds/{intent_id}")))
        .bearer_auth(&bearer_b)
        .send()
        .await
        .context("a pi_ id sent to the refund route")?;
    assert_eq!(wrong_shape.status().as_u16(), 404);
    let wrong_body = wrong_shape.bytes().await.context("the body is readable")?;
    assert_eq!(
        String::from_utf8_lossy(&wrong_body).replace(&intent_id, "<id>"),
        missing_text,
        "a malformed id must not be distinguishable from a missing one"
    );

    // Merchant A can still read its own.
    let still_there = harness
        .a()
        .refunds()
        .retrieve(&refund_id)
        .await
        .context("merchant A can still read its own refund")?;
    assert_eq!(still_there.id, refund_id);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------- test 2 (b)

/// The `re_` short-circuit in `vpay_api::v1::refunds` is **reached**, and its
/// answer is the same `404` as everything else.
///
/// Added by review on 2026-09-05. Without it the short-circuit is decoration:
/// deleting the three lines
///
/// ```text
/// if !id.starts_with(vpay_core::ids::REFUND_PREFIX) {
///     return Ok(None);
/// }
/// ```
///
/// left every other case in this file, and all seven `vpay-api` refund unit
/// tests, green — because no test had a row behind a mis-prefixed id, so the
/// query the short-circuit skips returned `None` anyway and the two paths were
/// indistinguishable. This case puts a real row there. It is `MERCHANT_A`'s
/// own refund, asked for by `MERCHANT_A`, so tenancy cannot be what refuses
/// it: with the short-circuit the answer is the `resource_missing` `404`, and
/// without it the answer is a `200` carrying the row.
///
/// The row is one no shipping code could write (`vpay_core::ids::refund_id`
/// is the only minter of a `refunds` id, and it always mints `re_…`), which is
/// the point — pinning what the route does with an id it will refuse to look
/// up is the only way to pin that it refuses to look it up at all.
#[tokio::test]
async fn a_refund_id_without_the_re_prefix_is_never_looked_up() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent_id = seed_intent(&client).await?;
    // The same suffix a real id would have, behind a prefix `refund_id` never
    // mints — so the *only* thing that can distinguish it is the prefix.
    let minted = vpay_core::ids::refund_id();
    let mis_prefixed = format!(
        "xx_{}",
        minted
            .strip_prefix(vpay_core::ids::REFUND_PREFIX)
            .expect("vpay_core mints refund ids behind its own prefix")
    );
    seed_refund_with_id(
        &harness.pool,
        &mis_prefixed,
        &intent_id,
        "pending",
        Some("requested_by_customer"),
    )
    .await?;

    // Guard against a vacuous pass: the row this case is about is really in
    // the database, and really hangs off MERCHANT_A's intent.
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM refunds r \
         JOIN payment_intents p ON p.id = r.payment_intent_id \
         WHERE r.id = $1 AND p.merchant_id = $2",
    )
    .bind(&mis_prefixed)
    .bind(MERCHANT_A)
    .fetch_one(&harness.pool)
    .await
    .context("the mis-prefixed row is stored and belongs to merchant A")?;
    assert_eq!(stored, 1, "the row this case is about must exist");

    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let refused = http
        .get(harness.url(&format!("/v1/refunds/{mis_prefixed}")))
        .bearer_auth(&bearer)
        .send()
        .await
        .context("merchant A asks for its own mis-prefixed refund")?;
    assert_eq!(
        refused.status().as_u16(),
        404,
        "an id that is not `re_…` is never looked up, even when a row exists behind it"
    );
    let refused_body = refused.bytes().await.context("the body is readable")?;

    let missing = http
        .get(harness.url(&format!("/v1/refunds/{MISSING_REFUND_ID}")))
        .bearer_auth(&bearer)
        .send()
        .await
        .context("merchant A asks for an id that never existed")?;
    assert_eq!(missing.status().as_u16(), 404);
    let missing_body = missing.bytes().await.context("the body is readable")?;

    // Same envelope, same wording, differing only where the caller's own id is
    // echoed back — so the short-circuit is not a distinguisher either.
    assert_eq!(
        String::from_utf8_lossy(&refused_body).replace(&mis_prefixed, "<id>"),
        String::from_utf8_lossy(&missing_body).replace(MISSING_REFUND_ID, "<id>"),
        "the short-circuit must answer the same 404 as a missing id"
    );
    let envelope: Value = serde_json::from_slice(&refused_body).context("a JSON body")?;
    assert_eq!(
        envelope.pointer("/error/code").and_then(Value::as_str),
        Some("resource_missing"),
        "got {envelope:#}"
    );

    // And a properly prefixed refund on the same intent still reads, so what
    // refused the one above was the prefix and nothing else about this fixture.
    let sibling = seed_refund(&harness.pool, &intent_id, "pending", None).await?;
    let readable = client
        .refunds()
        .retrieve(&sibling)
        .await
        .context("a `re_…` refund on the same intent still reads")?;
    assert_eq!(readable.id, sibling);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 3

/// One renderer: the API response and an event's `data.object` for the same
/// refund row are **byte-identical**.
///
/// `docs/flows/webhooks.md` commits to `charge.refunded` and
/// `charge.refund.updated`, and **nothing emits either** — so this case
/// writes the event row itself, through the same
/// `vpay_db::TxRepositories::insert_in_tx` the settlement transaction uses,
/// with `data` rendered by `vpay_api::model::RefundObject` from the row the
/// route will read. That is exactly what the eventual writer will have to do,
/// and pinning it now is what stops the two surfaces drifting before it
/// exists.
///
/// The comparison is on the **serialised bytes**, not on parsed values: a
/// handler that hand-built a map for its response — different key set,
/// `created` in milliseconds, `payment_intent_id` instead of
/// `payment_intent` — would still parse, and would still be a merchant
/// verifying a signature against a body that no longer matches the one they
/// re-fetched.
#[tokio::test]
async fn the_api_response_and_an_events_payload_for_one_refund_are_byte_identical()
-> anyhow::Result<()> {
    let harness = harness().await?;

    let intent_id = seed_intent(&harness.a()).await?;
    let refund_id = seed_refund(&harness.pool, &intent_id, "succeeded", Some("duplicate")).await?;

    // The renderer, applied to the stored row — the one call the future
    // emitter of `charge.refund.updated` will make.
    let row =
        vpay_db::Refunds::get_for_merchant(harness.repositories.as_ref(), MERCHANT_A, &refund_id)
            .await
            .context("reading the row back through the repository")?
            .expect("the row this test just wrote");
    let rendered = serde_json::to_value(
        vpay_api::model::RefundObject::try_from(&row).expect("the stored row renders"),
    )
    .context("serialising the rendered refund")?;

    let event = harness
        .repositories
        .transaction(|tx| {
            let rendered = rendered.clone();
            let refund_id = refund_id.clone();
            Box::pin(async move {
                let row = tx
                    .insert_in_tx(&NewEvent {
                        id: vpay_db::events::event_id(),
                        merchant_id: MERCHANT_A.to_owned(),
                        livemode: false,
                        event_type: "charge.refund.updated".to_owned(),
                        object_id: refund_id,
                        data: rendered,
                    })
                    .await?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(row))
            })
        })
        .await
        .context("writing the event this suite renders into")?
        .into_inner();

    let http = raw_client();
    let bearer = harness.bearer(CLIENT_A);

    let refund_bytes = http
        .get(harness.url(&format!("/v1/refunds/{refund_id}")))
        .bearer_auth(&bearer)
        .send()
        .await
        .context("the refund read")?
        .bytes()
        .await
        .context("the body is readable")?;

    let event_bytes = http
        .get(harness.url(&format!("/v1/events/{}", event.id)))
        .bearer_auth(&bearer)
        .send()
        .await
        .context("the event read")?
        .bytes()
        .await
        .context("the body is readable")?;

    let event_body: Value = serde_json::from_slice(&event_bytes).context("a JSON body")?;
    let payload = event_body
        .pointer("/data/object")
        .expect("an event carries data.object");
    let payload_bytes = serde_json::to_vec(payload).context("re-serialising data.object")?;

    assert_eq!(
        String::from_utf8_lossy(&refund_bytes),
        String::from_utf8_lossy(&payload_bytes),
        "the API response and the event payload for one refund row must be byte-identical"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 4

/// Creating a refund is **still** the honest `404`, and the read did not
/// quietly bring a write with it.
///
/// `POST /v1/refunds` needs `ProviderAdapter::refund`; `mtn_momo::refund` is
/// `NotImplemented` and Orange Money answers `Unsupported`. The route is
/// declared in `docs/flows/merchant-auth.md` and mounted nowhere, so an
/// authenticated caller gets the nest's `unknown_route` — a `200` there would
/// mean someone invented a resource.
#[tokio::test]
async fn creating_a_refund_is_still_the_honest_404() -> anyhow::Result<()> {
    let harness = harness().await?;
    let intent_id = seed_intent(&harness.a()).await?;

    let response = raw_client()
        .post(harness.url("/v1/refunds"))
        .bearer_auth(harness.bearer(CLIENT_A))
        .header("Idempotency-Key", "a-refund-nobody-can-create")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("payment_intent={intent_id}"))
        .send()
        .await
        .context("POST /v1/refunds")?;

    assert_eq!(response.status().as_u16(), 404);
    let body: Value = response.json().await.context("a JSON body")?;
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("unknown_route"),
        "creation is unrouted, not merely empty: {body:#}"
    );

    // And nothing was written.
    let refunds: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds")
        .fetch_one(&harness.pool)
        .await
        .context("counting refunds")?;
    assert_eq!(refunds, 0, "no route creates a refund row");

    harness.shutdown().await;
    Ok(())
}
