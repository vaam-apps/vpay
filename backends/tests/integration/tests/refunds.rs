//! `/v1/refunds`, end to end: the real `vpay_api::router` on a real socket,
//! over a real Postgres, with the real MTN adapter talking HTTP to a real
//! WireMock — and, for the five read cases, no rail at all.
//!
//! # Two subjects, two harnesses
//!
//! Cases 1 to 3 are **issue #45's**: a refund must have an authoritative
//! read. They seed their rows directly and boot a server with one
//! unreachable rail, so that a change to the create path cannot quietly
//! change what they measure.
//!
//! Cases 4 to 14 are **Wave 3's**: the create, the update, the list and the
//! cancel, RFC-0003 § 2. They need a rail that answers, so they boot the MTN
//! stub the conformance suite drives.
//!
//! The claims only this file can make:
//!
//! 1. a stored refund reads back through the shipping SDK as the ten keys
//!    `docs/flows/merchant-auth.md` documents — nine since issue #45 and
//!    **ten since issue #46's `fee`**, which is `null` here because no rail
//!    reports a refund fee and nothing in this repository writes the column;
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
//! 5. a refund created through `/v1` is `pending`, reserved on its intent,
//!    evented, and instructed to the rail **under its own reference** — the
//!    silent money bug RFC-0003 open question 7 is about;
//! 6. a rail that cannot refund fails the refund and gives the reservation
//!    back; a payee the rail does not know is refused before any transfer;
//!    and a bad destination is the caller's `400`, never a `502` about a rail
//!    that was never asked.
//!
//! # What none of it proves
//!
//! **That money has ever come back.** The MTN stub answers the documented
//! `202 ACCEPTED` of a product **no REAL credential for exists in this
//! project and this repository has never called**; `orange_money::refund` is a declared
//! `NotImplemented` token; and nothing settles a `pending` refund, because
//! the port has no refund status read (RFC-0003 open question 8). A `201`
//! from these cases means vpay wrote a refund and instructed a stub.
//!
//! It also proves nothing about **how an intent becomes `succeeded`**: these
//! cases write that state and the charge under it directly, exactly as they
//! seed a `refunds` row, because this file's subject is the refund surface.
//! `confirm_rails.rs` and `worker_e2e.rs` are the suites that drive the
//! charge path.

// See `tests/support/mod.rs` for why this allow list mirrors the other
// integration suites'.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers::GenericImage;
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
    /// The plain `sqlx` pool: this suite seeds its `refunds` rows itself
    /// rather than through `Refunds::create`, which exists since RFC-0003 § 3
    /// but is not reachable from `/v1` — see the module header.
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
            surfaces: None,
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
        staff_auth: vpay_config::StaffAuth::default(),
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
        customer: None,
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
         VALUES ($1, $2, $3, 'XAF', $4, $5, $6)",
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

/// A stored refund reads back through the shipping SDK, as the ten keys the
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
    // Issue #46's tenth key. `seed_refund` writes no `fee`, and migration
    // `0031` gives the column no `DEFAULT`, so the honest answer for this row
    // is "the rail reported nothing" — which the SDK models as `None` and
    // must never render as `Some(0)`.
    assert_eq!(
        refund.fee, None,
        "a row seeded without a fee reads back as unknown, not as free"
    );

    // The raw body carries exactly ten keys and no eleventh: the SDK's typed
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
            "fee",
            "id",
            "metadata",
            "object",
            "payment_intent",
            "reason",
            "status",
        ]
    );
    // Present **and** null, which is the distinction the whole of issue #46
    // is about: an absent key would have passed a `get("fee").is_none()`
    // check and would be indistinguishable, in either SDK, from a field the
    // merchant never learns exists.
    assert_eq!(
        object.get("fee"),
        Some(&Value::Null),
        "`fee` is present and null for a row written without one: {body:#}"
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
/// **Both** documented refund event types are driven, not one: the two are
/// the same object in `data.object`, both are claimed to carry it in
/// `docs/flows/webhooks.md`, `docs/flows/merchant-auth.md` and
/// `docs/status.md`, and a writer that rendered `charge.refunded` by hand
/// would pass a case that only ever wrote `charge.refund.updated`.
///
/// The payload's key set is asserted here too, and `fee` in particular is
/// asserted **present and `null`** (issue #46): a webhook body is signed,
/// stored and delivered at-least-once, so a key that went missing on this
/// surface would reach merchants who never re-read the object.
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

    for event_type in ["charge.refunded", "charge.refund.updated"] {
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
                            event_type: event_type.to_owned(),
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
        assert_eq!(
            event_body.pointer("/type").and_then(Value::as_str),
            Some(event_type),
            "the event this leg reads back is the one it wrote: {event_body:#}"
        );
        let payload = event_body
            .pointer("/data/object")
            .expect("an event carries data.object");
        let payload_bytes = serde_json::to_vec(payload).context("re-serialising data.object")?;

        assert_eq!(
            String::from_utf8_lossy(&refund_bytes),
            String::from_utf8_lossy(&payload_bytes),
            "the API response and the {event_type} payload for one refund row must be \
             byte-identical"
        );

        let object = payload.as_object().expect("data.object is a JSON object");
        assert_eq!(
            object.len(),
            10,
            "a delivered {event_type} body is the same ten keys the API renders: {payload:#}"
        );
        assert_eq!(
            object.get("fee"),
            Some(&Value::Null),
            "`fee` is present and null in a delivered {event_type} body, not absent"
        );
    }

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 4

/// `POST /v1/refunds` is routed, and creates a real refund against a real
/// rail stub.
///
/// This case replaced `creating_a_refund_is_still_the_honest_404` on
/// 2026-09-16, when the handler landed. What that case asserted — that the
/// route answered the nest's `unknown_route` and that nothing wrote a
/// `refunds` row — is now false by design; the mounted surface is pinned
/// instead by `the_refund_resource_is_mounted_for_exactly_five_methods` in
/// `vpay_api::v1`, which fails if any of the four is removed.
///
/// **What a `201` here does and does not mean.** The rail stub answered
/// MTN's documented `202 ACCEPTED` and nothing else: the port has no refund
/// status read, so the refund is `pending` and stays `pending`, and no money
/// has moved in any deployment — no REAL MTN Disbursements credential exists
/// in this project (the e2e/demo stack's subscription key is a stub aimed at
/// WireMock) and the product has never been called. See the module
/// header.
#[tokio::test]
async fn a_refund_is_created_pending_and_the_rail_is_instructed() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;

    let response = harness
        .create_refund(&intent_id, Some(REFUND_AMOUNT), PAYEE)
        .await?;
    assert_eq!(response.status, 201, "{:#}", response.body);

    assert_eq!(
        response.body.get("status").and_then(Value::as_str),
        Some("pending"),
        "an Ok from the rail is an acceptance, never a settlement: {:#}",
        response.body
    );
    assert_eq!(
        response.body.get("amount").and_then(Value::as_i64),
        Some(REFUND_AMOUNT)
    );
    assert_eq!(
        response.body.get("payment_intent").and_then(Value::as_str),
        Some(intent_id.as_str())
    );
    // The destination is on no wire object. RFC-0003 rejected carrying a
    // payee's number anywhere a webhook or a stored response could keep it,
    // and the ten-key tripwire is the unit-level half of this.
    let rendered = serde_json::to_string(&response.body)?;
    assert!(
        !rendered.contains(PAYEE_CANONICAL) && !rendered.contains(PAYEE),
        "the payee's number reached the response body: {rendered}"
    );

    let refund_id = response.id();

    // The reservation is on the intent, and `amount_refunded` is not: the
    // money has not come back, it is promised.
    let (refunded, pending): (i64, i64) = sqlx::query_as(
        "SELECT amount_refunded, amount_refund_pending FROM payment_intents WHERE id = $1",
    )
    .bind(&intent_id)
    .fetch_one(&harness.pool)
    .await
    .context("reading the intent's counters")?;
    assert_eq!(refunded, 0, "nothing has settled");
    assert_eq!(pending, REFUND_AMOUNT, "the amount is reserved");

    // The rail was really asked, under the refund's own reference.
    let (reference, attempts): (uuid::Uuid, i64) = sqlx::query_as(
        "SELECT r.provider_reference_id, \
                (SELECT count(*) FROM provider_requests q \
                  WHERE q.operation = 'refund' AND q.provider_reference_id = r.provider_reference_id) \
         FROM refunds r WHERE r.id = $1",
    )
    .bind(&refund_id)
    .fetch_one(&harness.pool)
    .await
    .context("reading the refund's reference")?;
    assert_eq!(attempts, 1, "one recorded refund attempt, before the call");
    assert_eq!(
        harness
            .transfers_with_reference(&reference.to_string())
            .await?,
        1,
        "the rail received one transfer addressed by the refund's own reference"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 5

/// **Two partial refunds of one charge carry two references, and both reach
/// the rail.**
///
/// The case RFC-0003 open question 7 is about, and the one that would have
/// caught the silent money bug: `ProviderAdapter::refund` takes one
/// `ChargeRef`, and if this handler filled it with the **charge's**
/// reference, MTN would answer the second transfer `409
/// RESOURCE_ALREADY_EXIST` — which the adapter reads as *accepted*, correctly,
/// because that is the crash-retry story. The merchant would be told the
/// second refund happened, the payer would receive nothing, and no error
/// would appear anywhere.
///
/// So the assertion is in three parts, and all three are needed:
///
/// 1. the two refunds carry two different `provider_reference_id`s;
/// 2. neither is the charge's;
/// 3. the **rail's own journal** shows two transfers, one per reference.
///
/// Parts 1 and 2 alone would pass an implementation that minted a reference
/// and then sent a different one. `the_transfer_is_addressed_by_the_reference_the_core_supplied`
/// pins the adapter's half of the same claim.
#[tokio::test]
async fn two_partial_refunds_of_one_charge_carry_two_references() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;

    let first = harness.create_refund(&intent_id, Some(2000), PAYEE).await?;
    assert_eq!(first.status, 201, "{:#}", first.body);
    let second = harness.create_refund(&intent_id, Some(1500), PAYEE).await?;
    assert_eq!(second.status, 201, "{:#}", second.body);

    let references: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT provider_reference_id FROM refunds WHERE payment_intent_id = $1 \
         ORDER BY created_at, id",
    )
    .bind(&intent_id)
    .fetch_all(&harness.pool)
    .await
    .context("reading both refunds' references")?;
    let [first_reference, second_reference] = references.as_slice() else {
        panic!("two refunds were created, so there are two references: {references:?}");
    };
    assert_ne!(
        first_reference, second_reference,
        "two refunds of one charge must not share a rail reference"
    );

    let charge_reference: uuid::Uuid = sqlx::query_scalar(
        "SELECT provider_reference_id FROM charges WHERE payment_intent_id = $1",
    )
    .bind(&intent_id)
    .fetch_one(&harness.pool)
    .await
    .context("reading the charge's reference")?;
    for reference in &references {
        assert_ne!(
            *reference, charge_reference,
            "a refund must not be addressed by the charge's reference"
        );
        assert_eq!(
            harness
                .transfers_with_reference(&reference.to_string())
                .await?,
            1,
            "each refund reached the rail under its own reference"
        );
    }

    // And both are reserved: 3 500 of 5 000 is promised, none of it settled.
    let (refunded, pending): (i64, i64) = sqlx::query_as(
        "SELECT amount_refunded, amount_refund_pending FROM payment_intents WHERE id = $1",
    )
    .bind(&intent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!((refunded, pending), (0, 3500));

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 6

/// The first `charge.refunded` this repository has ever emitted, and its body
/// is the API's own.
///
/// `docs/flows/webhooks.md` listed both refund types with "— nothing" in the
/// "written by" column from the day the vocabulary was closed. This is the
/// writer. The event is in the **same transaction** as the row it reports, so
/// there is no window in which a refund exists and no merchant hears about
/// it, and its `data.object` is byte-identical to what `GET /v1/refunds/{id}`
/// answers — one renderer, which is what test 3 asserts for the read.
#[tokio::test]
async fn a_created_refund_emits_charge_refunded_with_the_api_body() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;

    let created = harness
        .create_refund(&intent_id, Some(REFUND_AMOUNT), PAYEE)
        .await?;
    assert_eq!(created.status, 201, "{:#}", created.body);
    let refund_id = created.id();

    let (event_type, payload, livemode): (String, Value, bool) =
        sqlx::query_as("SELECT type, data, livemode FROM events WHERE object_id = $1")
            .bind(&refund_id)
            .fetch_one(&harness.pool)
            .await
            .context("the refund's event")?;

    assert_eq!(event_type, "charge.refunded");
    assert!(!livemode, "the intent's livemode, not a deployment default");
    assert_eq!(
        payload, created.body,
        "the event body and the API response are one renderer"
    );
    assert!(
        !serde_json::to_string(&payload)?.contains(PAYEE_CANONICAL),
        "a payee's number must never reach a signed, stored, replayed event body"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 7

/// A rail that cannot refund fails the refund, gives the reservation back,
/// and says so in an event.
///
/// `orange_money::refund` is a declared `ProviderError::NotImplemented`
/// token: an Orange refund is an outbound transfer and this repository has no
/// specification for one (RFC-0003 § 5). The instruction was therefore never
/// given, which is what makes releasing the reservation safe — and what makes
/// releasing it **necessary**: an intent holding a reservation for a refund
/// that will never happen cannot be refunded again up to its own amount.
///
/// No HTTP is attempted, which is why this case needs no Orange stub.
#[tokio::test]
async fn an_unbuilt_rail_refund_fails_and_releases_its_reservation() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent_on(AMOUNT, ORANGE_RAIL).await?;

    // `amount` omitted: Orange declares `supports_partial_refunds: false`,
    // so the only refund this rail takes is the whole of what is left — a
    // refusal this handler makes on the capability and never on the code,
    // and one that reaches the rail check below only because it passes.
    let response = harness
        .create_refund_on(&intent_id, None, PAYEE, ORANGE_RAIL)
        .await?;
    assert_eq!(
        response.status, 501,
        "an unbuilt rail is `not_implemented`, never a decline and never a success: {:#}",
        response.body
    );

    let (status, failure_code): (String, Option<String>) =
        sqlx::query_as("SELECT status, failure_code FROM refunds WHERE payment_intent_id = $1")
            .bind(&intent_id)
            .fetch_one(&harness.pool)
            .await
            .context("the failed refund")?;
    assert_eq!(status, "failed");
    assert_eq!(failure_code.as_deref(), Some("provider_error"));

    let pending: i64 =
        sqlx::query_scalar("SELECT amount_refund_pending FROM payment_intents WHERE id = $1")
            .bind(&intent_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(
        pending, 0,
        "the reservation of a refund the rail never took must go back"
    );

    let types: Vec<String> =
        sqlx::query_scalar("SELECT type FROM events WHERE object_id LIKE 're\\_%' ORDER BY seq")
            .fetch_all(&harness.pool)
            .await
            .context("the refund's events")?;
    assert_eq!(
        types,
        vec![
            "charge.refunded".to_owned(),
            "charge.refund.updated".to_owned()
        ],
        "the merchant is told the refund was created and then that it failed"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 8

/// The destination is refused on the **capability**, and a bad one is the
/// caller's `400` — never the rail's `502`.
///
/// Both halves matter and neither implies the other:
///
/// * a `Required` rail with no `destination` is a `400` naming the parameter,
///   which is what stops `ProviderAdapter::refund` ever being called with the
///   `None` it is entitled to assume it never sees;
/// * a payee `parse_destination` refuses is **also** a `400` naming the
///   parameter. That error is `ProviderError::Malformed`, whose
///   classification is written for a rail that answered gibberish: forwarded
///   unchanged it is a `502`, `stripe-should-retry: true`, and the sentence
///   "The payment rail is temporarily unavailable" — for a typo in the
///   merchant's own request. `a_malformed_destination_is_classified_as_a_rail_fault`
///   measures that classification one layer down; this is the case that fails
///   if a future author `?`s the error instead of translating it.
#[tokio::test]
async fn a_missing_or_malformed_destination_is_the_callers_400() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;

    let missing = harness
        .post_refund_form(&format!("payment_intent={intent_id}&amount=1000"))
        .await?;
    assert_eq!(missing.status, 400, "{:#}", missing.body);
    assert_eq!(
        missing.body.pointer("/error/param").and_then(Value::as_str),
        Some("destination"),
        "the refusal names the parameter an SDK points a form field at: {:#}",
        missing.body
    );

    // A bare national number: `RefundTarget::mobile_money` requires the `+`,
    // because a market-agnostic crate has no country to attach one to.
    let malformed = harness
        .post_refund_form(&format!(
            "payment_intent={intent_id}&amount=1000&destination[{RAIL}][msisdn]=600000200"
        ))
        .await?;
    assert_eq!(
        malformed.status, 400,
        "a merchant's typo is not a rail outage: {:#}",
        malformed.body
    );
    assert_eq!(
        malformed
            .body
            .pointer("/error/param")
            .and_then(Value::as_str),
        Some("destination")
    );
    assert_eq!(
        malformed
            .body
            .pointer("/error/type")
            .and_then(Value::as_str),
        Some("invalid_request_error"),
        "not `api_error`, which is what Category::Rail renders: {:#}",
        malformed.body
    );
    assert!(
        !serde_json::to_string(&malformed.body)?.contains("600000200"),
        "a refusal must not echo the number it refused: {:#}",
        malformed.body
    );

    // Nothing was written by either refusal: both are decided before the
    // transaction opens.
    let refunds: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(refunds, 0);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 9

/// A payee the rail has no record of is refused **before** any money is
/// instructed.
///
/// This is the caller issue #47 built `account_holder_name` for. `237600000404`
/// is `basicuserinfo.json`'s "no record" number and *also* the number
/// `transfer.json` answers `PAYEE_NOT_FOUND` for — so the two-layer claim is
/// real: the lookup refuses it first, and the transfer mapping that would
/// have refused it second is never reached.
#[tokio::test]
async fn a_payee_the_rail_does_not_know_is_refused_before_the_transfer() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;

    let response = harness
        .create_refund(&intent_id, Some(1000), UNREGISTERED_PAYEE)
        .await?;
    assert_eq!(response.status, 400, "{:#}", response.body);
    assert_eq!(
        response
            .body
            .pointer("/error/param")
            .and_then(Value::as_str),
        Some("destination")
    );

    assert_eq!(
        harness.transfers_total().await?,
        0,
        "no transfer may be instructed for a payee the rail has no record of"
    );
    let refunds: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(refunds, 0, "and no row is written");

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 10

/// An over-refund is refused by the **database**, as `over_refund`, and the
/// refusal leaves nothing behind.
///
/// `no_over_refund` (migration `0003`) is the guard, in the `UPDATE` that
/// takes the reservation — not a read-then-compare in Rust, which two
/// concurrent refunds would both pass. The `409`/`over_refund` code is what
/// tells a merchant to re-send with a smaller amount rather than to stop.
#[tokio::test]
async fn refunding_more_than_is_left_is_a_409_over_refund() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;

    let first = harness.create_refund(&intent_id, Some(4000), PAYEE).await?;
    assert_eq!(first.status, 201, "{:#}", first.body);

    let second = harness.create_refund(&intent_id, Some(2000), PAYEE).await?;
    assert_eq!(second.status, 409, "{:#}", second.body);
    assert_eq!(
        second.body.pointer("/error/code").and_then(Value::as_str),
        Some("over_refund"),
        "distinguishable from `resource_conflict`: {:#}",
        second.body
    );

    let (refunds, pending): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM refunds), \
                (SELECT amount_refund_pending FROM payment_intents WHERE id = $1)",
    )
    .bind(&intent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(refunds, 1, "the refused refund wrote no row");
    assert_eq!(pending, 4000, "and reserved nothing");

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 11

/// The update changes `metadata` and nothing else, and emits
/// `charge.refund.updated`.
///
/// Stripe's contract twice over: the merge is key-wise (a key sent empty is
/// removed, the rest of the stored map survives), and `metadata` is the only
/// field the endpoint takes. A merchant who sends `amount` is **told**, not
/// answered `200` with the original amount — which is the difference between
/// a merchant discovering the refusal now and discovering it in a settlement
/// statement.
#[tokio::test]
async fn an_update_merges_metadata_and_refuses_everything_else() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;
    let created = harness
        .post_refund_form(&format!(
            "payment_intent={intent_id}&amount=1000\
             &destination[{RAIL}][msisdn]={PAYEE}&metadata[order_id]=1234&metadata[ship]=dhl"
        ))
        .await?;
    assert_eq!(created.status, 201, "{:#}", created.body);
    let refund_id = created.id();

    let updated = harness
        .post_form(
            &format!("/v1/refunds/{refund_id}"),
            "metadata[order_id]=5678&metadata[ship]=",
        )
        .await?;
    assert_eq!(updated.status, 200, "{:#}", updated.body);
    assert_eq!(
        updated
            .body
            .pointer("/metadata/order_id")
            .and_then(Value::as_str),
        Some("5678"),
        "the sent key wins"
    );
    assert_eq!(
        updated.body.pointer("/metadata/ship"),
        None,
        "a key sent empty is removed, not set to null: {:#}",
        updated.body
    );
    assert_eq!(
        updated.body.get("amount").and_then(Value::as_i64),
        Some(1000),
        "an update touches no money"
    );

    let refused = harness
        .post_form(&format!("/v1/refunds/{refund_id}"), "amount=1")
        .await?;
    assert_eq!(refused.status, 400, "{:#}", refused.body);
    assert_eq!(
        refused.body.pointer("/error/param").and_then(Value::as_str),
        Some("amount")
    );

    let types: Vec<String> =
        sqlx::query_scalar("SELECT type FROM events WHERE object_id = $1 ORDER BY seq")
            .bind(&refund_id)
            .fetch_all(&harness.pool)
            .await?;
    assert_eq!(
        types,
        vec![
            "charge.refunded".to_owned(),
            "charge.refund.updated".to_owned()
        ],
        "one event for the create and one for the update, and none for the refusal"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 12

/// A `pending` refund **no rail was ever given** cancels, gives its
/// reservation back, and cannot be cancelled twice.
///
/// The second cancel is a `409` because the statement's `AND status =
/// 'pending'` refused it — the state machine is the `WHERE` clause, not a
/// check beside it. Nothing is reversed in the ledger, because a `pending`
/// refund posted nothing (`docs/flows/ledger.md` § "When refunds post").
///
/// # Why this case no longer creates its refund through `POST /v1/refunds`
///
/// Because that route hands the refund to a rail, and since 2026-09-16 such a
/// refund is deliberately **not** cancellable — see
/// `a_refund_the_rail_has_already_been_instructed_is_not_cancelable`, which is
/// the money case this one used to contradict. Until then this case created
/// its refund through the route, cancelled it while MTN's journal held its
/// transfer, and asserted the `200`: it pinned the double-payout bug as the
/// contract.
///
/// The state it builds instead is the one remaining state a cancel is honest
/// in, and it is not invented — it is `docs/flows/crash-safety.md`'s first
/// recovery row ("no `provider_requests` row: crashed before the POST"),
/// written directly exactly as `worker_recovery.rs` writes a charge's three.
/// Aged ninety seconds for the same reason that suite ages its fixtures: a
/// row written a millisecond ago is indistinguishable from a create that is
/// still running, and the statement refuses it.
#[tokio::test]
async fn a_pending_refund_cancels_once_and_gives_its_reservation_back() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;
    let refund_id = harness
        .seed_crashed_create(&intent_id, REFUND_AMOUNT)
        .await?;
    harness.age_past_the_cancel_window(&refund_id).await?;

    let canceled = harness
        .post_form(&format!("/v1/refunds/{refund_id}/cancel"), "")
        .await?;
    assert_eq!(canceled.status, 200, "{:#}", canceled.body);
    assert_eq!(
        canceled.body.get("status").and_then(Value::as_str),
        Some("canceled")
    );

    let pending: i64 =
        sqlx::query_scalar("SELECT amount_refund_pending FROM payment_intents WHERE id = $1")
            .bind(&intent_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(pending, 0, "the reservation goes back");

    let again = harness
        .post_form(&format!("/v1/refunds/{refund_id}/cancel"), "")
        .await?;
    assert_eq!(again.status, 409, "{:#}", again.body);

    let ledger: i64 = sqlx::query_scalar("SELECT count(*) FROM ledger_transactions")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(ledger, 0, "a pending refund posted nothing to reverse");

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- test 12b

/// **A refund the rail has already been given cannot be canceled, and that is
/// what stops a second transfer.**
///
/// The money case. `POST /v1/refunds` writes its `provider_requests` row
/// before the transfer and nothing settles a refund (RFC-0003 open question
/// 8), so every refund this route creates sits `pending` for ever with its
/// instruction already accepted by MTN. A cancel of such a refund writes
/// `canceled` — a promise that no money will move — and gives back the
/// reservation `no_over_refund` is computed from.
///
/// Measured 2026-09-16 with the guard removed, which is also the mutation
/// that must make this case fail: a 5 000 charge, one full refund accepted by
/// the rail, `POST /v1/refunds/{id}/cancel` answering **200 `canceled`**,
/// the reservation back to 0, and a second full refund accepted — **two**
/// 5 000 transfers on WireMock's own journal against one 5 000 charge, with
/// no error anywhere and the merchant told the first refund had been called
/// off.
///
/// The decisive assertion is the last one, and it is the rail's journal
/// rather than the database: the `409`, the held reservation and the
/// still-`pending` row would all pass an implementation that refused the
/// cancel and then let the second refund through some other way.
#[tokio::test]
async fn a_refund_the_rail_has_already_been_instructed_is_not_cancelable() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;

    // The whole charge, so the only thing that can refuse a second refund is
    // the reservation this one holds.
    let created = harness.create_refund(&intent_id, None, PAYEE).await?;
    assert_eq!(created.status, 201, "{:#}", created.body);
    let refund_id = created.id();
    assert_eq!(
        harness.transfers_total().await?,
        1,
        "the rail was given the instruction, which is the premise of this case"
    );

    // **Aged past the cancel window, and the case is not decisive without
    // it.** A refund created a moment ago is also refused by the "the create
    // may still be running" predicate, so with this line missing the
    // rail-instructed predicate can be deleted outright and all 18 cases stay
    // green — measured 2026-09-16, and the diagnosis even reported the same
    // sentence, because it reads "instructed" before "too young". Aged, the
    // only predicate left that can refuse this refund is the one this case
    // is named after.
    harness.age_past_the_cancel_window(&refund_id).await?;

    let canceled = harness
        .post_form(&format!("/v1/refunds/{refund_id}/cancel"), "")
        .await?;
    assert_eq!(
        canceled.status, 409,
        "a refund whose transfer is with the rail must not be cancelable: {:#}",
        canceled.body
    );
    assert!(
        canceled
            .body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("already been given to the payment rail")),
        "the refusal must say why, so a merchant reconciles instead of retrying: {:#}",
        canceled.body
    );

    // The row and the reservation are exactly as they were.
    let status: String = sqlx::query_scalar("SELECT status FROM refunds WHERE id = $1")
        .bind(&refund_id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(status, "pending", "the refusal wrote nothing");
    let (refunded, pending): (i64, i64) = sqlx::query_as(
        "SELECT amount_refunded, amount_refund_pending FROM payment_intents WHERE id = $1",
    )
    .bind(&intent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        (refunded, pending),
        (0, AMOUNT),
        "the reservation is still held, which is what refuses the second refund"
    );

    // And no `charge.refund.updated` was emitted for a transition that did
    // not happen: the create's `charge.refunded` is the only event there is.
    let events: Vec<String> = sqlx::query_scalar("SELECT type FROM events WHERE object_id = $1")
        .bind(&refund_id)
        .fetch_all(&harness.pool)
        .await?;
    assert_eq!(events, vec!["charge.refunded".to_owned()]);

    // The merchant tries again anyway. This is the sequence that pays a payee
    // twice, and the reservation is what stops it.
    let second = harness.create_refund(&intent_id, None, PAYEE).await?;
    assert_eq!(
        second.status, 409,
        "the whole charge is already reserved by the pending refund: {:#}",
        second.body
    );
    assert_eq!(
        harness.transfers_total().await?,
        1,
        "one refund instructed, one transfer at the rail — a second here is a payee paid twice"
    );

    harness.shutdown().await;
    Ok(())
}

// --------------------------------------------------------------- test 12c

/// A refund young enough that its create may still be running is not canceled
/// either.
///
/// The attempt row is committed by a statement of its own, after the
/// transaction that writes the refund, so there is a window in which a
/// committed `pending` refund carries no attempt row and is nonetheless about
/// to be sent to a rail. Cancel it in that window and the transfer goes out
/// against a `canceled` refund whose reservation has been handed back — the
/// same double payout by a narrower door.
///
/// `docs/flows/crash-safety.md`'s own answer to the identical ambiguity on a
/// `submitting` charge, applied here: younger than the window, nothing on
/// disk distinguishes a create that crashed from one that is still going, so
/// the answer is wait rather than act. The fixture is the crashed-create
/// state **unaged**, which is exactly how `worker_recovery.rs`'s fourth and
/// fifth cases are built.
#[tokio::test]
async fn a_refund_whose_create_may_still_be_running_is_not_canceled_yet() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;
    let refund_id = harness
        .seed_crashed_create(&intent_id, REFUND_AMOUNT)
        .await?;

    let canceled = harness
        .post_form(&format!("/v1/refunds/{refund_id}/cancel"), "")
        .await?;
    assert_eq!(canceled.status, 409, "{:#}", canceled.body);
    assert!(
        canceled
            .body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("may still be on its way")),
        "{:#}",
        canceled.body
    );

    let pending: i64 =
        sqlx::query_scalar("SELECT amount_refund_pending FROM payment_intents WHERE id = $1")
            .bind(&intent_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(pending, REFUND_AMOUNT, "nothing was released");

    // Aged past the window, the same request is the `200` the case above
    // asserts — so what this pins is the window and not a second refusal.
    harness.age_past_the_cancel_window(&refund_id).await?;
    let later = harness
        .post_form(&format!("/v1/refunds/{refund_id}/cancel"), "")
        .await?;
    assert_eq!(later.status, 200, "{:#}", later.body);

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 13

/// The list is this merchant's, newest first, and the `payment_intent` filter
/// narrows it without widening the scope it is applied inside.
///
/// The tenancy half is the one worth stating: merchant B's refund is absent
/// from A's list **and** absent from A's list filtered by B's intent id — a
/// filter that resolved outside the tenant predicate would turn the
/// collection into an oracle for other merchants' ids.
#[tokio::test]
async fn the_list_is_merchant_scoped_and_filterable_by_intent() -> anyhow::Result<()> {
    let harness = rail_harness().await?;

    let first_intent = harness.captured_intent(AMOUNT).await?;
    let second_intent = harness.captured_intent(AMOUNT).await?;
    let first = harness
        .create_refund(&first_intent, Some(1000), PAYEE)
        .await?
        .id();
    let second = harness
        .create_refund(&second_intent, Some(1500), PAYEE)
        .await?
        .id();

    // Merchant B's own refund, written straight to the table: this suite's
    // subject is A's view of the collection, and seeding B's row is what
    // `seed_refund` is for.
    let other_intent = harness.captured_intent_for(MERCHANT_B, AMOUNT).await?;
    let hidden = seed_refund(&harness.pool, &other_intent, "pending", None).await?;

    let page = harness.get_json("/v1/refunds").await?;
    let ids: Vec<&str> = page
        .body
        .get("data")
        .and_then(Value::as_array)
        .expect("a list object")
        .iter()
        .filter_map(|object| object.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        vec![second.as_str(), first.as_str()],
        "newest first, and only this merchant's: {:#}",
        page.body
    );
    assert!(!ids.contains(&hidden.as_str()));
    assert_eq!(
        page.body.get("url").and_then(Value::as_str),
        Some("/v1/refunds")
    );

    let filtered = harness
        .get_json(&format!("/v1/refunds?payment_intent={first_intent}"))
        .await?;
    let filtered_ids: Vec<&str> = filtered
        .body
        .get("data")
        .and_then(Value::as_array)
        .expect("a list object")
        .iter()
        .filter_map(|object| object.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(filtered_ids, vec![first.as_str()]);

    // Another merchant's intent id is an empty page, not a 404 and not a
    // window into their refunds.
    let foreign = harness
        .get_json(&format!("/v1/refunds?payment_intent={other_intent}"))
        .await?;
    assert_eq!(
        foreign
            .body
            .get("data")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "{:#}",
        foreign.body
    );

    // A cursor from the wrong vocabulary names its own parameter.
    let wrong_cursor = harness
        .get_json(&format!("/v1/refunds?starting_after={first_intent}"))
        .await?;
    assert_eq!(wrong_cursor.status, 400, "{:#}", wrong_cursor.body);

    harness.shutdown().await;
    Ok(())
}

// -------------------------------------------------------------- test 13b

/// **The uniform `404` holds on the routes that WRITE, not only on the read.**
///
/// Added by review, 2026-09-16. `merchant_b_cannot_read_merchant_as_refund`
/// proves the property for `GET /v1/refunds/{id}` and proves it well — bodies
/// byte for byte, the `resource_missing` envelope rather than the nest's
/// `unknown_route`. Nothing proved it for `POST /v1/refunds/{id}` or
/// `POST /v1/refunds/{id}/cancel`, and those two are where it is easiest to
/// lose: both read the refund before their write in order to tell a `404`
/// from a `409`, and `cancel_once` re-reads a second time to render the
/// *status* of the row that refused it. A version of either read that was not
/// merchant-scoped would answer a foreign refund's owner-only `409` — "this
/// one is `succeeded`" — and that single sentence confirms both that the id
/// exists and what state it is in.
///
/// Two ids, indistinguishable by construction: another merchant's real refund
/// and a `re_…` that names nothing. Both verbs, and the assertion is on the
/// whole body with each request's own id substituted out, exactly as the read
/// case does it.
#[tokio::test]
async fn the_write_routes_answer_the_same_404_for_a_foreign_refund_as_for_a_missing_one()
-> anyhow::Result<()> {
    let harness = rail_harness().await?;

    // Merchant B's own refund, seeded straight to the table for
    // `the_list_is_merchant_scoped_and_filterable_by_intent`'s reason: what
    // is under test is A's answer, and B's row only has to exist.
    let other_intent = harness.captured_intent_for(MERCHANT_B, AMOUNT).await?;
    let foreign_id = seed_refund(&harness.pool, &other_intent, "pending", None).await?;

    // Each pair is (what merchant A sends, the route it sends it to).
    for (body, suffix) in [("metadata[note]=probe", ""), ("", "/cancel")] {
        let foreign = harness
            .post_form(&format!("/v1/refunds/{foreign_id}{suffix}"), body)
            .await?;
        let missing = harness
            .post_form(&format!("/v1/refunds/{MISSING_REFUND_ID}{suffix}"), body)
            .await?;

        assert_eq!(foreign.status, 404, "{:#}", foreign.body);
        assert_eq!(missing.status, 404, "{:#}", missing.body);

        // The *resource* envelope, which is also what proves the route is
        // mounted: an unmounted path answers the nest's `unknown_route`,
        // which is also a `404` and also the same for both ids.
        assert_eq!(
            missing.body.pointer("/error/code").and_then(Value::as_str),
            Some("resource_missing"),
            "POST /v1/refunds/{{id}}{suffix}: {:#}",
            missing.body
        );

        let foreign_text = serde_json::to_string(&foreign.body)?.replace(&foreign_id, "<id>");
        let missing_text = serde_json::to_string(&missing.body)?.replace(MISSING_REFUND_ID, "<id>");
        assert_eq!(
            foreign_text, missing_text,
            "POST /v1/refunds/{{id}}{suffix} distinguishes another merchant's refund from an id \
             that never existed"
        );
    }

    // And merchant B's refund is untouched by either attempt — the `404` is a
    // refusal, not a silent no-op on somebody else's row.
    let status: String = sqlx::query_scalar("SELECT status FROM refunds WHERE id = $1")
        .bind(&foreign_id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(status, "pending");

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 14

/// A replayed `Idempotency-Key` answers the stored response and creates no
/// second refund.
///
/// Unlike a charge — which `one_charge_per_intent` makes unrepeatable at the
/// database — two refunds of one intent are a legitimate thing a merchant
/// does, so the key is the **only** thing standing between a retried request
/// and a second transfer. That makes this case the one that would catch a
/// handler which released the key on a path that had already instructed the
/// rail.
#[tokio::test]
async fn a_replayed_key_answers_the_stored_refund_and_creates_no_second() -> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;
    let body =
        format!("payment_intent={intent_id}&amount=1000&destination[{RAIL}][msisdn]={PAYEE}");
    let key = "w3routes-one-refund-only";

    let first = harness.post_refund_form_with_key(&body, key).await?;
    assert_eq!(first.status, 201, "{:#}", first.body);
    let second = harness.post_refund_form_with_key(&body, key).await?;
    assert_eq!(second.status, 201, "{:#}", second.body);
    assert_eq!(first.body, second.body, "the replay is the stored response");

    let refunds: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(refunds, 1, "a replay must not write a second refund");
    assert_eq!(
        harness.transfers_total().await?,
        1,
        "and must not instruct a second transfer"
    );

    harness.shutdown().await;
    Ok(())
}

// ----------------------------------------------------------------- test 15

/// **A replay answers the refund the merchant already has, even when the
/// intent has moved on since.**
///
/// This is the case that decides where the `Idempotency-Key` claim sits, and
/// it is the one an ordering that "validates first" gets wrong. A merchant
/// whose `201` was lost to a timeout retries under the same key; by then the
/// intent has nothing left to refund, because the refund they are retrying
/// took the rest of it. Every refusal in the create reads mutable rows — the
/// intent's counters among them — so resolving before claiming answers `409
/// over_refund` for a refund that **exists and is theirs**, and the merchant
/// has no way to learn its id.
///
/// The rule this pins is `vpay_api::v1::customers::create`'s, stated there
/// and now stated in `v1::refunds`: **a replay answers whatever the original
/// answered, whatever has changed since.** Move `claim_or_answer` below
/// `resolve_target` and this case fails; every other case in this file still
/// passes, including `a_replayed_key_answers_the_stored_refund_and_creates_no_second`,
/// because that one replays against an intent nothing has changed.
#[tokio::test]
async fn a_replayed_key_answers_the_stored_refund_even_when_the_intent_has_moved_on()
-> anyhow::Result<()> {
    let harness = rail_harness().await?;
    let intent_id = harness.captured_intent(AMOUNT).await?;
    // **`amount` omitted**, and that is the whole shape of the case: a full
    // refund is "all of what is left", so the second request's own meaning
    // depends on what the first one did. A body carrying an explicit `amount`
    // would not reach the divergence at all — the handler resolves such a
    // request identically both times and only the *database* refuses it,
    // after the claim, which is not the ordering under test. Measured
    // 2026-09-16: with an explicit `amount` this case passed under both
    // orderings, which is why it does not carry one.
    let body = format!("payment_intent={intent_id}&destination[{RAIL}][msisdn]={PAYEE}");
    let key = "w3routes-a-timeout-is-not-a-second-refund";

    let first = harness.post_refund_form_with_key(&body, key).await?;
    assert_eq!(first.status, 201, "{:#}", first.body);
    assert_eq!(
        first.body.get("amount").and_then(Value::as_i64),
        Some(AMOUNT),
        "a full refund is the whole of what is left, so the intent has nothing left now"
    );

    // The same request again — the merchant never saw the answer.
    let replay = harness.post_refund_form_with_key(&body, key).await?;
    assert_eq!(
        replay.status, 201,
        "a replay must answer the stored response, not re-run the rules against an intent \
         the first request itself changed: {:#}",
        replay.body
    );
    assert_eq!(first.body, replay.body, "and byte for byte the same body");

    // Proof the state really had moved on: the same body under a *new* key is
    // the `409` the replay must not have been given.
    let fresh = harness.post_refund_form(&body).await?;
    assert_eq!(fresh.status, 409, "{:#}", fresh.body);
    assert!(
        fresh
            .body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("nothing left to refund")),
        "{:#}",
        fresh.body
    );

    let refunds: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(refunds, 1);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------- the rail harness

/// The payee every refund here nominates — `basicuserinfo.json`'s
/// registered-holder number, so a reader can look it up in one table.
const PAYEE: &str = "+237600000200";

/// The same number in the shape the rail is handed, which is what a response
/// or an event body must never contain.
const PAYEE_CANONICAL: &str = "237600000200";

/// `basicuserinfo.json`'s "the rail has no record" number, which is also the
/// number `transfer.json` answers `PAYEE_NOT_FOUND` for.
const UNREGISTERED_PAYEE: &str = "+237600000404";

/// The rail whose refund is written but has never been called for real.
const ORANGE_RAIL: &str = "orange_money";

/// One raw `/v1` answer: the status and the parsed body, which is what every
/// assertion above reads.
struct Answer {
    status: u16,
    body: Value,
}

impl Answer {
    fn id(&self) -> String {
        self.body
            .get("id")
            .and_then(Value::as_str)
            .expect("a created refund has an id")
            .to_owned()
    }
}

/// Postgres, an MTN WireMock, and a server wired to both.
///
/// A second harness beside [`Harness`], not a replacement for it: the five
/// cases above seed their rows directly and deliberately have **no** rail at
/// all, so that a change to the create path cannot quietly change what they
/// measure. These cases are the opposite — they are about the create path,
/// and they need a rail that answers.
struct RailHarness {
    _container: ContainerAsync<PostgresImage>,
    _mtn: ContainerAsync<GenericImage>,
    mtn_origin: String,
    server: tokio::task::JoinHandle<()>,
    pool: PgPool,
    base_url: String,
    pem_a: String,
    signing_key: LoadedSigningKey,
}

impl RailHarness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

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

    fn a(&self) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(&self.base_url)
            .credentials(
                Credentials::rsa_pem(CLIENT_A, &self.pem_a).expect("the generated PEM parses"),
            )
            .build()
            .expect("the SDK client builds")
    }

    /// An intent that has captured money, on the default rail.
    ///
    /// The intent is created through the shipping SDK and then **moved to
    /// `succeeded` with a charge row written directly**, which is the same
    /// device `seed_refund` uses and for the same reason: this suite's
    /// subject is the refund surface, and driving a confirm and a settlement
    /// to get there would make every case here depend on the charge path's
    /// stubs. What that costs, stated rather than hidden: nothing in this
    /// file proves how an intent becomes `succeeded` —
    /// `backends/tests/integration/tests/confirm_rails.rs` and `worker_e2e.rs`
    /// are the suites that do.
    async fn captured_intent(&self, amount: i64) -> anyhow::Result<String> {
        self.captured_intent_on(amount, RAIL).await
    }

    async fn captured_intent_on(&self, amount: i64, rail: &str) -> anyhow::Result<String> {
        let intent = self
            .a()
            .payment_intents()
            .create(create_params(), RequestOptions::new())
            .await
            .context("creating the intent a refund comes off")?;
        self.capture(&intent.id, amount, rail).await?;
        Ok(intent.id)
    }

    /// The same, for a merchant whose SDK credential this harness does not
    /// hold: the row is written whole, because the only thing the tenancy
    /// cases need is *an intent belonging to someone else*.
    async fn captured_intent_for(&self, merchant_id: &str, amount: i64) -> anyhow::Result<String> {
        let id = vpay_core::ids::payment_intent_id();
        sqlx::query(
            "INSERT INTO payment_intents \
                 (id, merchant_id, livemode, amount, amount_received, currency_code, status, \
                  payment_method_types, metadata, client_secret_suffix) \
             VALUES ($1, $2, false, $3, $3, 'XAF', 'succeeded', $4, '{}'::jsonb, $5)",
        )
        .bind(&id)
        .bind(merchant_id)
        .bind(amount)
        .bind(json!([RAIL]))
        .bind("x".repeat(32))
        .execute(&self.pool)
        .await
        .context("seeding another merchant's captured intent")?;
        self.capture(&id, amount, RAIL).await?;
        Ok(id)
    }

    /// Moves an intent to `succeeded` and gives it the charge a refund is
    /// attributed to.
    async fn capture(&self, intent_id: &str, amount: i64, rail: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE payment_intents SET status = 'succeeded', amount_received = $2 WHERE id = $1",
        )
        .bind(intent_id)
        .bind(amount)
        .execute(&self.pool)
        .await
        .context("marking the intent captured")?;

        sqlx::query(
            "INSERT INTO charges \
                 (id, payment_intent_id, provider_code, provider_reference_id, state, amount, \
                  currency_code, payer_ref) \
             VALUES ($1, $2, $3, $4, 'succeeded', $5, 'XAF', $6)",
        )
        .bind(vpay_core::ids::charge_id())
        .bind(intent_id)
        .bind(rail)
        .bind(uuid::Uuid::new_v4())
        .bind(amount)
        // The payer's own number, which is not the payee's: a refund that
        // sent the money back to the payer by accident would pass a test
        // whose two numbers were the same string.
        .bind("237600000111")
        .execute(&self.pool)
        .await
        .context("seeding the charge a refund comes off")?;
        Ok(())
    }

    /// The state a create leaves when it dies between committing the refund
    /// and recording the attempt it is about to make: a `pending` refund with
    /// its amount reserved, a `provider_reference_id` of its own, and **no
    /// `provider_requests` row**.
    ///
    /// `docs/flows/crash-safety.md`'s first recovery row, for a refund. It is
    /// written directly rather than caused, exactly as `worker_recovery.rs`
    /// writes a charge's three kill points, and for that suite's stated
    /// reason: the moment being staged is the one *before* any network call,
    /// so there is nothing for a signal to land during.
    ///
    /// The reservation is taken by the same `UPDATE` shape the create path
    /// uses, so `no_over_refund` governs this row as it governs a real one —
    /// a fixture that skipped it would be a refund the intent does not know
    /// about, which is not the state under test.
    async fn seed_crashed_create(&self, intent_id: &str, amount: i64) -> anyhow::Result<String> {
        let id = vpay_core::ids::refund_id();
        sqlx::query(
            "UPDATE payment_intents \
             SET amount_refund_pending = amount_refund_pending + $2 WHERE id = $1",
        )
        .bind(intent_id)
        .bind(amount)
        .execute(&self.pool)
        .await
        .context("reserving the crashed create's amount")?;

        sqlx::query(
            "INSERT INTO refunds \
                 (id, payment_intent_id, charge_id, amount, currency_code, status, metadata, \
                  provider_reference_id) \
             VALUES ($1, $2, (SELECT id FROM charges WHERE payment_intent_id = $2), $3, 'XAF', \
                     'pending', '{}'::jsonb, $4)",
        )
        .bind(&id)
        .bind(intent_id)
        .bind(amount)
        .bind(uuid::Uuid::new_v4())
        .execute(&self.pool)
        .await
        .context("seeding the refund a crashed create left behind")?;
        Ok(id)
    }

    /// Moves a seeded refund ninety seconds into the past.
    ///
    /// `worker_recovery.rs`'s `support::age_the_crash`, spelled for this
    /// table: the cancel statement refuses a refund younger than its window,
    /// because a young refund with no attempt row is indistinguishable from a
    /// create that is still running.
    ///
    /// Every case that is about some *other* cancel predicate must age its
    /// refund first, or it measures this window instead of the rule it names.
    /// Measured 2026-09-16: with
    /// `a_refund_the_rail_has_already_been_instructed_is_not_cancelable`
    /// leaving its refund young, deleting the rail-instructed predicate
    /// altogether left all 18 cases green — the age predicate was answering
    /// the same `409` and even the same message, because the diagnosis reads
    /// "instructed" before "too young".
    async fn age_past_the_cancel_window(&self, refund_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE refunds SET created_at = now() - interval '90 seconds' \
             WHERE id = $1 AND status = 'pending'",
        )
        .bind(refund_id)
        .execute(&self.pool)
        .await
        .context("aging the crashed create")?;
        Ok(())
    }

    async fn create_refund(
        &self,
        intent_id: &str,
        amount: Option<i64>,
        payee: &str,
    ) -> anyhow::Result<Answer> {
        self.create_refund_on(intent_id, amount, payee, RAIL).await
    }

    async fn create_refund_on(
        &self,
        intent_id: &str,
        amount: Option<i64>,
        payee: &str,
        rail: &str,
    ) -> anyhow::Result<Answer> {
        let amount = amount.map_or_else(String::new, |amount| format!("&amount={amount}"));
        self.post_refund_form(&format!(
            "payment_intent={intent_id}{amount}&destination[{rail}][msisdn]={payee}"
        ))
        .await
    }

    async fn post_refund_form(&self, body: &str) -> anyhow::Result<Answer> {
        let key = format!("w3routes-{}", uuid::Uuid::new_v4());
        self.post_refund_form_with_key(body, &key).await
    }

    async fn post_refund_form_with_key(&self, body: &str, key: &str) -> anyhow::Result<Answer> {
        self.post("/v1/refunds", body, key).await
    }

    async fn post_form(&self, path: &str, body: &str) -> anyhow::Result<Answer> {
        let key = format!("w3routes-{}", uuid::Uuid::new_v4());
        self.post(path, body, &key).await
    }

    async fn post(&self, path: &str, body: &str, key: &str) -> anyhow::Result<Answer> {
        let response = raw_client()
            .post(self.url(path))
            .bearer_auth(self.bearer(CLIENT_A))
            .header("Idempotency-Key", key)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_owned())
            .send()
            .await
            .with_context(|| format!("POST {path}"))?;
        let status = response.status().as_u16();
        let body = response.json().await.context("a JSON body")?;
        Ok(Answer { status, body })
    }

    async fn get_json(&self, path: &str) -> anyhow::Result<Answer> {
        let response = raw_client()
            .get(self.url(path))
            .bearer_auth(self.bearer(CLIENT_A))
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        let status = response.status().as_u16();
        let body = response.json().await.context("a JSON body")?;
        Ok(Answer { status, body })
    }

    /// How many transfers the MTN stub recorded under one `X-Reference-Id`.
    ///
    /// The rail's own journal is the only witness for what vpay *sent*: the
    /// database says which reference was minted and the response says the
    /// refund was accepted, and neither can tell a reference that reached MTN
    /// from one that was replaced on the way.
    async fn transfers_with_reference(&self, reference: &str) -> anyhow::Result<usize> {
        self.transfers(&format!(
            r#"{{"method":"POST","urlPath":"/disbursement/v1_0/transfer",
                 "headers":{{"X-Reference-Id":{{"equalTo":"{reference}"}}}}}}"#
        ))
        .await
    }

    async fn transfers_total(&self) -> anyhow::Result<usize> {
        self.transfers(r#"{"method":"POST","urlPath":"/disbursement/v1_0/transfer"}"#)
            .await
    }

    /// The count, dug out of the admin response by hand for the same reason
    /// `confirm_rails.rs`'s twin does it: the body is `{"count": N, …}`, one
    /// integer after one key, and a `serde_json` parse here would say "the
    /// shape changed" where this says "the rail was told the wrong thing".
    async fn transfers(&self, pattern: &str) -> anyhow::Result<usize> {
        let text = reqwest::Client::new()
            .post(format!("{}/__admin/requests/count", self.mtn_origin))
            .body(pattern.to_owned())
            .send()
            .await
            .context("the MTN stub's admin API answers")?
            .text()
            .await
            .context("the count response is readable")?;
        let (_, after) = text
            .split_once("\"count\"")
            .with_context(|| format!("no count in the admin response: {text}"))?;
        let digits: String = after
            .chars()
            .skip_while(|character| !character.is_ascii_digit())
            .take_while(char::is_ascii_digit)
            .collect();
        digits
            .parse()
            .with_context(|| format!("count is not a number in: {text}"))
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// The `wiremock/{rail}` root the conformance suite and `compose.yml` both
/// use — one set of mappings, so a stub this suite drives is the stub the
/// conformance suite drives.
fn mappings_dir(rail: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

/// Two rails, two merchants, one currency, `livemode: false`.
///
/// MTN carries **both** products' credentials, and every Disbursements value
/// differs from its Collections twin — which is the whole point: a
/// configuration that gave both the same strings could not tell "the refund
/// path read the Disbursements keys" from "the refund path read whatever was
/// there", and `transfer.json` matches on the Disbursements bearer and
/// subscription key for exactly that reason.
///
/// Orange is configured at an address nothing listens on, deliberately: its
/// refund is a declared `NotImplemented` token, so a refund on that rail must
/// be answered without a socket being opened. A test that passed because a
/// stub said no would be measuring the stub.
fn rail_config(base_url: &str, mtn_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "refunds-rails".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: vec![
            ProviderHost {
                code: RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: mtn_url.to_owned(),
                    label: "mtn-wiremock".to_owned(),
                },
                settings: BTreeMap::from([
                    ("target_environment".to_owned(), "sandbox".to_owned()),
                    (
                        "api_user".to_owned(),
                        "11111111-2222-3333-4444-555555555555".to_owned(),
                    ),
                    (
                        "disbursement_api_user".to_owned(),
                        "66666666-7777-8888-9999-000000000000".to_owned(),
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
                    (
                        "disbursement_subscription_key".to_owned(),
                        "stub-disbursement-subscription-key".to_owned(),
                    ),
                    (
                        "disbursement_api_key".to_owned(),
                        "stub-disbursement-api-key".to_owned(),
                    ),
                ]),
            },
            ProviderHost {
                code: ORANGE_RAIL.to_owned(),
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
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
        staff_auth: vpay_config::StaffAuth::default(),
    }
}

async fn rail_harness() -> anyhow::Result<RailHarness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;
    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let mtn_origin = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let mtn_url = mtn_origin.clone();
    let served = serve(&repositories, &server_pem, |base_url| {
        rail_config(base_url, &mtn_url, jwks_a, jwks_b)
    })
    .await?;

    Ok(RailHarness {
        _container: container,
        _mtn: mtn,
        mtn_origin,
        server: served.server,
        pool,
        base_url: served.base_url,
        pem_a,
        signing_key: served.signing_key,
    })
}
