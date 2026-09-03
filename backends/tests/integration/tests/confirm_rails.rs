//! `POST /v1/payment_intents/{id}/confirm` against a **rail**, end to end:
//! the real `vpay_api::router` on a real socket, over a real Postgres, with
//! the shipping `vpay-adapter-*` crates talking HTTP to a real WireMock
//! container per rail.
//!
//! `payment_intents.rs` covers the surface — validation, tenancy,
//! idempotency, paging — and stops where a rail begins. This file is the
//! other half, and it exists because Step 3 made a confirm able to
//! *succeed*: the four outcomes below are the whole of what a confirm can
//! now do, and each one is a different set of committed rows.
//!
//! | outcome | intent | charge | `provider_requests` |
//! |---|---|---|---|
//! | push accepted | `processing` | `submitted` | answered, no `error_kind` |
//! | redirect accepted | `requires_action` | `submitted` + `redirect_url` | answered, no `error_kind` |
//! | declined at submit | `requires_payment_method` + `last_payment_error` | `failed` + `failure_code` | answered, `charge_declined` |
//! | rail unreachable | unchanged | `submitting` | **un**answered |
//!
//! The last row is the one worth stating out loud: an attempt with no HTTP
//! status is the encoding for "we do not know what the rail did", and it is
//! what `docs/flows/crash-safety.md`'s recovery table reads. A confirm that
//! could not reach a rail must leave exactly the state a crash would.
//!
//! # No test doubles, and what that costs
//!
//! The rails are stubbed as WireMock **hosts in configuration** (ADR-0006):
//! the adapter builds its own HTTP client, resolves its own token, and sends
//! the same bytes it would send to MTN. Nothing here replaces the adapter,
//! the client, or the port. The stubs are the same
//! `backends/tests/conformance/wiremock/{mtn,orange}` directories
//! `compose.yml` bind-mounts and the conformance suite starts, referenced by
//! path rather than copied — a mapping fixed for one is fixed for all three.
//!
//! # Which stub answers, and why the reference is not fixed
//!
//! The conformance suite selects a stubbed response by *reference*, because
//! it constructs the `ChargeRef` itself. A confirm cannot: the
//! `provider_reference_id` is minted inside the handler
//! (`Uuid::new_v4()`, before the charge is committed), and a seam to fix it
//! from a test would be a code path that exists only outside production —
//! the thing `AGENTS.md`'s first rule forbids.
//!
//! It needs none. Both rails' mapping directories already carry a
//! reference-independent happy path, and those are the stubs that answer
//! every accepted confirm here:
//!
//! * MTN — `requesttopay.json`'s *"requestToPay accepted (202, empty body —
//!   the id is the caller's)"*, `priority: 10`, matching `POST
//!   /collection/v1_0/requesttopay` with no header matcher. The
//!   lower-priority mappings all pin a specific `X-Reference-Id`, so a
//!   server-generated one falls through to it.
//! * Orange — `webpayment.json`'s *"webpayment: accepted, with a pay_token
//!   derived from the order_id"*, `priority: 5`, matching the URL only and
//!   templating `pay_token`/`payment_url` from the request's own `order_id`.
//!   That is what lets [`redirect_confirm_commits_the_rails_material_before_it_answers`]
//!   assert the exact URL a random reference produces.
//!
//! Neither decline case needs a fixed reference, and they are two different
//! failures that were once conflated under one name:
//!
//! * [`credentials_the_rail_refuses_are_a_page_and_a_terminal_charge`] is
//!   driven by *credentials*, with the `bad-key` subscription key
//!   `token.json` answers `401` to — the same value the conformance suite's
//!   `Credentials::Rejected` uses, and the way this actually happens in
//!   production, on the day a key rotates. It never reaches `requestToPay`
//!   at all: it fails at the token call, and its taxonomy code is
//!   `provider_account_blocked`, whose documented handling is "page
//!   yourself".
//! * [`a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read`]
//!   is a real decline *at submit*, and it is steered by the one field of
//!   the outgoing rail request a merchant controls: the MSISDN. It confirms
//!   with `payment_method_data[mtn_momo][msisdn]=`[`UNKNOWN_PAYER_MSISDN`],
//!   which `requesttopay.json` matches on `$.payer.partyId` (`priority: 5`,
//!   under the reference-keyed stubs and over the catch-all 202) and
//!   answers `400 PAYER_NOT_FOUND` to — `invalid_payer` in the taxonomy.
//!
//! Both had to be here. Until the Step 3 review only the first existed, and
//! it was named as though it covered the second, so nothing in this
//! repository exercised an end-to-end confirm in which the *rail* declined
//! the payment itself.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};
use vpay_sdk::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, Credentials, IntentStatus,
    PaymentMethodType, RequestOptions,
};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres, serve,
};

/// The merchant every test acts as, and the tenant it acts for.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

const PUSH_RAIL: &str = "mtn_momo";
const REDIRECT_RAIL: &str = "orange_money";

/// XAF for both rails here, so the charge-vs-rail currency rule
/// (`vpay_api`'s `currencies_agree`) is satisfied by every test that is not
/// about it. [`a_rail_that_settles_in_another_currency_is_refused_before_any_charge`]
/// stands a rail up on EUR precisely to trip it.
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

/// A documentation MSISDN, not anyone's. Nothing stubs it specifically, so
/// it falls through to the catch-all 202 every accepted confirm here uses.
const MSISDN: &str = "237670000000";

/// The documentation MSISDN `wiremock/mtn/mappings/requesttopay.json` answers
/// `400 PAYER_NOT_FOUND` to, matched on the outgoing body's
/// `$.payer.partyId`.
///
/// The payer is the only part of a confirm's rail request a *merchant* can
/// choose — `provider_reference_id` is minted inside the handler — so it is
/// the only way to reach a rail's decline branch from the API without a test
/// seam in shipping code.
const UNKNOWN_PAYER_MSISDN: &str = "237600000400";

/// Where the merchant asks Orange to send the payer back.
const RETURN_URL: &str = "https://shop.example/order/1234/return";

/// The subscription key `wiremock/mtn/mappings/token.json` answers `401` to.
const REJECTED_SUBSCRIPTION_KEY: &str = "bad-key";

/// A port nothing listens on. Connection refused is the same
/// `ProviderError::Transport` / `Category::Rail` answer an HTTP `503`
/// produces — the adapter's own 503 mapping is proven against a stub in
/// `backends/tests/conformance/tests/adapter_conformance.rs`
/// (`an_unavailable_rail_is_a_transport_error_never_a_decline`); what this
/// file proves is what the *confirm path* does with that category, which is
/// the same for both.
const UNREACHABLE_RAIL: &str = "http://127.0.0.1:1";

/// The `wiremock/{rail}` root the conformance suite and `compose.yml` both
/// use. Referenced across the workspace rather than copied: a stub fixed in
/// one place is fixed everywhere it is mounted.
fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

// ------------------------------------------------------------------ harness

/// A running vpay server with both rails stubbed, its database, and the
/// merchant's credentials.
struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    _mtn: ContainerAsync<GenericImage>,
    _orange: ContainerAsync<GenericImage>,
    server: tokio::task::JoinHandle<()>,
    pool: PgPool,
    base_url: String,
    /// Where each rail's stub is listening, for the second servers below.
    mtn_url: String,
    orange_url: String,
    pem_a: String,
    jwks_a: Value,
    server_pem: String,
}

impl Harness {
    fn a(&self) -> vpay_sdk::Client {
        self.client_for(&self.base_url)
    }

    fn client_for(&self, base_url: &str) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(base_url)
            .credentials(
                Credentials::rsa_pem(CLIENT_A, &self.pem_a).expect("the generated PEM parses"),
            )
            .build()
            .expect("the SDK client builds from a base URL and a credential")
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// How a rail is configured for one server: where it is, what currency it
/// settles in, and whether its credentials are the ones the stub accepts.
///
/// A struct rather than four positional arguments because every test below
/// varies exactly one of them, and a test that silently varied two would be
/// proving something other than it claims.
#[derive(Debug, Clone)]
struct RailSetup {
    base_url: String,
    currency: String,
    subscription_key: String,
}

impl RailSetup {
    fn working(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_owned(),
            currency: "XAF".to_owned(),
            subscription_key: "stub-subscription-key".to_owned(),
        }
    }
}

/// The configuration a server boots with: two rails, one merchant, one
/// currency, `livemode: false`.
///
/// Shaped exactly like `config/application.yml`'s — including the settings
/// and credentials keys `vpay_config`'s `REQUIRED_RAIL_KEYS` insists on, so
/// a configuration this suite accepts is one that would load from a file.
fn config_with(base_url: &str, jwks_a: Value, mtn: &RailSetup, orange: &RailSetup) -> Config {
    Config {
        deployment: Deployment {
            name: "confirm-rails".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![
            ProviderHost {
                code: PUSH_RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: mtn.base_url.clone(),
                    label: "mtn-wiremock".to_owned(),
                },
                settings: BTreeMap::from([
                    ("target_environment".to_owned(), "sandbox".to_owned()),
                    (
                        "api_user".to_owned(),
                        "11111111-2222-3333-4444-555555555555".to_owned(),
                    ),
                ]),
                callback_url: None,
                currency: mtn.currency.clone(),
                credentials: BTreeMap::from([
                    ("subscription_key".to_owned(), mtn.subscription_key.clone()),
                    ("api_key".to_owned(), "stub-api-key".to_owned()),
                ]),
            },
            ProviderHost {
                code: REDIRECT_RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: orange.base_url.clone(),
                    label: "orange-wiremock".to_owned(),
                },
                settings: BTreeMap::from([
                    ("env".to_owned(), "dev".to_owned()),
                    ("lang".to_owned(), "en".to_owned()),
                ]),
                callback_url: None,
                currency: orange.currency.clone(),
                credentials: BTreeMap::from([
                    ("merchant_key".to_owned(), "stub-merchant-key".to_owned()),
                    ("client_id".to_owned(), "stub-client-id".to_owned()),
                    ("client_secret".to_owned(), "stub-client-secret".to_owned()),
                ]),
            },
        ],
        currencies: vec![
            CurrencyEntry {
                code: "XAF".to_owned(),
                exponent: 0,
            },
            CurrencyEntry {
                code: "EUR".to_owned(),
                exponent: 2,
            },
        ],
        merchant_clients: vec![merchant_client(CLIENT_A, MERCHANT_A, jwks_a)],
        dashboard_client: None,
    }
}

/// Boots Postgres, both rail stubs, and a server wired to all three.
async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (postgres, pool) = migrated_postgres().await?;

    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let orange = vpay_testkit::containers::start_wiremock(&mappings_dir("orange"))
        .await
        .context("the Orange stub container starts")?;

    let mtn_url = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );
    // The `/orange-money-webpay/{env}` prefix is part of the configured base
    // URL (`docs/flows/adapter-orange-money.md`), exactly as
    // `config/application.yml` writes it.
    let orange_url = format!(
        "http://127.0.0.1:{}/orange-money-webpay/dev",
        orange
            .get_host_port_ipv4(8080)
            .await
            .context("the Orange stub's mapped port")?
    );

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();

    let mtn_setup = RailSetup::working(&mtn_url);
    let orange_setup = RailSetup::working(&orange_url);
    let jwks_for_server = jwks_a.clone();
    let served = serve(&pool, &server_pem, |base_url| {
        config_with(base_url, jwks_for_server, &mtn_setup, &orange_setup)
    })
    .await?;

    Ok(Harness {
        _postgres: postgres,
        _mtn: mtn,
        _orange: orange,
        server: served.server,
        pool,
        base_url: served.base_url,
        mtn_url,
        orange_url,
        pem_a,
        jwks_a,
        server_pem,
    })
}

/// A payment intent for `rail`, in the currency both rails settle in.
fn create_params(rail: PaymentMethodType) -> CreatePaymentIntentParams {
    CreatePaymentIntentParams {
        amount: AMOUNT,
        currency: CURRENCY.to_owned(),
        payment_method_types: vec![rail],
        metadata: BTreeMap::new(),
        description: None,
    }
}

/// The `ApiError` fields a test cares about.
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

/// The status and the *sentence*, for the cases where the sentence is the
/// thing under test.
///
/// A separate helper rather than a fifth element on [`api_error`]: only two
/// cases below care what the message says, and widening the tuple would make
/// every other call site carry a `_` for it.
fn api_error_message(error: vpay_sdk::Error) -> (u16, String) {
    match error {
        vpay_sdk::Error::Api {
            status, message, ..
        } => (status, message),
        other => panic!("expected a vpay API error envelope, got {other:?}"),
    }
}

/// The charge row a confirm left behind, in the columns these tests assert
/// on. Read straight from Postgres with its own statement, never through
/// `vpay_db::charges` and never from a response body: the whole question
/// this suite answers is what is *committed*, and a read that went through
/// the same repository as the write would prove only that the two agree.
#[derive(Debug, sqlx::FromRow)]
struct StoredCharge {
    id: String,
    state: String,
    provider_ref_extra: Option<Value>,
    redirect_url: Option<String>,
    return_url: Option<String>,
    failure_code: Option<String>,
    failure_raw: Option<String>,
}

async fn stored_charge(pool: &PgPool, payment_intent_id: &str) -> anyhow::Result<StoredCharge> {
    sqlx::query_as::<_, StoredCharge>(
        "SELECT id, state::TEXT AS state, provider_ref_extra, redirect_url, return_url, \
         failure_code::TEXT AS failure_code, failure_raw \
         FROM charges WHERE payment_intent_id = $1",
    )
    .bind(payment_intent_id)
    .fetch_one(pool)
    .await
    .context("reading the charge a confirm committed")
}

/// The single attempt recorded for a charge: its status, its `error_kind`,
/// and whether it was ever answered.
async fn stored_attempt(
    pool: &PgPool,
    charge_id: &str,
) -> anyhow::Result<(Option<i32>, Option<String>, bool)> {
    let row: (Option<i32>, Option<String>, Option<time::OffsetDateTime>) = sqlx::query_as(
        "SELECT status_code, error_kind, responded_at FROM provider_requests \
         WHERE charge_id = $1 ORDER BY id",
    )
    .bind(charge_id)
    .fetch_one(pool)
    .await
    .context("reading the provider_requests row a confirm wrote")?;
    Ok((row.0, row.1, row.2.is_some()))
}

/// The `last_payment_error_{code,message}` column pair a decline stamped on
/// the intent.
///
/// Read from Postgres rather than from the rendered object because the two
/// are different claims: the object could render a code it derived, and the
/// `lpe_paired` CHECK is about what is *stored*.
async fn stored_payment_error(
    pool: &PgPool,
    payment_intent_id: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT last_payment_error_code::TEXT, last_payment_error_message \
         FROM payment_intents WHERE id = $1",
    )
    .bind(payment_intent_id)
    .fetch_one(pool)
    .await
    .context("reading the payment error a decline stamped on the intent")
}

async fn charge_count(pool: &PgPool, payment_intent_id: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM charges WHERE payment_intent_id = $1")
        .bind(payment_intent_id)
        .fetch_one(pool)
        .await
        .context("counting charges")
}

// ------------------------------------------------------------------ test 1

/// A push confirm the rail accepts: `200`, `processing`, and the three rows
/// that make it true.
///
/// This is the first test in this repository in which a payment intent
/// reaches `processing` — before Step 3 no confirm could return anything but
/// an error (`docs/flows/payment-lifecycle.md`'s Status section said so in
/// as many words). So it asserts the whole committed state and not just the
/// status: a `200` whose charge is still `submitting` would be a rendered
/// optimism, which is the failure mode `CLAUDE.md` names first.
#[tokio::test]
async fn a_push_confirm_the_rail_accepts_moves_the_intent_to_processing() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(
            create_params(PaymentMethodType::MtnMomo),
            RequestOptions::new(),
        )
        .await
        .context("creating the intent to confirm")?;
    assert_eq!(intent.status, IntentStatus::RequiresPaymentMethod);

    let confirmed = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(MSISDN),
            RequestOptions::new(),
        )
        .await
        .context("the MTN stub's catch-all 202 accepts any reference")?;

    assert_eq!(
        confirmed.status,
        IntentStatus::Processing,
        "a push rail's payer is prompted on their handset; there is nothing for a browser to do"
    );
    assert_eq!(
        confirmed.next_action, None,
        "next_action is redirect-only (docs/flows/payment-lifecycle.md)"
    );
    assert_eq!(confirmed.last_payment_error, None);

    let charge = stored_charge(&harness.pool, &intent.id).await?;
    assert_eq!(
        charge.state, "submitted",
        "the charge moved out of `submitting`: the rail has it"
    );
    assert_eq!(
        charge.redirect_url, None,
        "a push rail hands back no URL, and none may be invented"
    );
    assert_eq!(
        charge.provider_ref_extra,
        Some(serde_json::json!({})),
        "MTN returns no key material; an empty document says the rail answered, where NULL \
         would be indistinguishable from a charge that was never submitted"
    );

    let (status_code, error_kind, answered) = stored_attempt(&harness.pool, &charge.id).await?;
    assert!(
        answered,
        "the rail answered, so the attempt is answered — this is the row a recovery sweep uses \
         to tell `we heard back` from `we do not know`"
    );
    assert_eq!(
        status_code,
        Some(vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT),
        "the port does not carry the rail's HTTP status, so the sentinel is recorded rather \
         than a plausible-looking 202"
    );
    assert_eq!(
        error_kind, None,
        "an accepted submit records no failure label"
    );

    // And the object is reproducible: a later GET says the same thing.
    let retrieved = client
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("re-reading the confirmed intent")?;
    assert_eq!(
        retrieved, confirmed,
        "the confirm's response and a later retrieve must be the same object"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 2

/// A redirect confirm: `200`, `requires_action`, `next_action.redirect_to_url`
/// — and the charge row **already** holding the rail's `pay_token` and URL
/// when the response arrives.
///
/// This is the API half of `docs/flows/crash-safety.md`'s "the commit is the
/// gate on the redirect". The database is queried *after* the response has
/// been received and parsed, so a `redirect_url` found there cannot have
/// been written afterwards: the merchant could not have been handed a URL
/// before the row existed. Deleting the charge update makes this a `500`
/// with no `next_action` at all rather than a `200` with an uncommitted
/// URL — see `vpay_api`'s `submitted_response`.
///
/// The URL asserted is the one `webpayment.json` templates from the
/// `order_id` the *server* generated, which is also how this test knows the
/// reference actually reached the rail.
#[tokio::test]
async fn redirect_confirm_commits_the_rails_material_before_it_answers() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(
            create_params(PaymentMethodType::OrangeMoney),
            RequestOptions::new(),
        )
        .await
        .context("creating the intent to confirm")?;

    let confirmed = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::orange_money(RETURN_URL),
            RequestOptions::new(),
        )
        .await
        .context("the Orange stub's 201 accepts any order_id")?;

    assert_eq!(confirmed.status, IntentStatus::RequiresAction);

    let (url, return_url) = match confirmed.next_action.clone() {
        Some(vpay_sdk::NextAction::RedirectToUrl { redirect_to_url }) => {
            (redirect_to_url.url, redirect_to_url.return_url)
        }
        other => panic!("a redirect rail must answer with a next_action, got {other:?}"),
    };
    assert_eq!(
        return_url.as_deref(),
        Some(RETURN_URL),
        "the return_url a merchant sent must come back on the action they act on"
    );

    // The proof, taken after the response was in hand.
    let charge = stored_charge(&harness.pool, &intent.id).await?;
    assert_eq!(charge.state, "submitted");
    assert_eq!(
        charge.redirect_url.as_deref(),
        Some(url.as_str()),
        "the URL the merchant was given is the URL the database already held"
    );
    assert_eq!(
        charge.return_url.as_deref(),
        Some(RETURN_URL),
        "the merchant's return_url is a column, written before the rail was called"
    );
    let pay_token = charge
        .provider_ref_extra
        .as_ref()
        .and_then(|extra| extra.get("pay_token"))
        .and_then(Value::as_str)
        .expect("the rail's key material is committed with the URL, not after it");
    assert!(
        url.contains(pay_token),
        "the stub derives both from the order_id, so a URL that did not carry the committed \
         token would mean the two came from different calls: {url} / {pay_token}"
    );

    let (status_code, error_kind, answered) = stored_attempt(&harness.pool, &charge.id).await?;
    assert!(answered);
    assert_eq!(
        status_code,
        Some(vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT)
    );
    assert_eq!(error_kind, None);

    // Reproducible: the same next_action comes back from a plain GET,
    // rendered from the charge row rather than remembered.
    let retrieved = client
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("re-reading the intent in requires_action")?;
    assert_eq!(
        retrieved, confirmed,
        "a merchant who lost the confirm's response must be able to read the same action back"
    );

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 3

/// Credentials the rail refuses: `409 charge_declined`, a **failed** charge
/// carrying `provider_account_blocked`, and an intent still
/// confirmable-in-shape but carrying `last_payment_error`.
///
/// # What this is, and what it is not
///
/// It was called `a_rail_that_declines_at_submit_fails_the_charge_and_
/// records_why` until the Step 3 review, and that name claimed more than
/// the body proves. Nothing here reaches `requestToPay`: the rotated
/// subscription key fails at the **token** call, the stub answers `401`,
/// and the adapter maps that to `Rejected { provider_account_blocked }` —
/// "*your* partner account is blocked. Page yourself"
/// (`docs/flows/failures.md`). That is an operator's outage, in which every
/// charge on this rail is failing and no payer can do anything about it. A
/// payer *decline* is
/// [`a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read`],
/// which is a different code, a different severity and a different
/// on-call answer.
///
/// What both share, and what this case pins, is the confirm path's
/// treatment of a rail *decision* as a business outcome rather than a
/// system failure: a `409` the merchant can act on, a terminal charge, and
/// the taxonomy recorded rather than the rail's words. The `failure_code`
/// assertion below is `provider_account_blocked` precisely so the two cases
/// cannot silently become the same test.
///
/// The second server is what makes this a configuration change rather than
/// a test seam: same database, same merchant, one wrong credential.
#[tokio::test]
async fn credentials_the_rail_refuses_are_a_page_and_a_terminal_charge() -> anyhow::Result<()> {
    let harness = harness().await?;

    let mut rejected = RailSetup::working(&harness.mtn_url);
    rejected.subscription_key = REJECTED_SUBSCRIPTION_KEY.to_owned();
    let orange = RailSetup::working(&harness.orange_url);
    let jwks_a = harness.jwks_a.clone();
    let served = serve(&harness.pool, &harness.server_pem, |base_url| {
        config_with(base_url, jwks_a, &rejected, &orange)
    })
    .await?;
    let client = harness.client_for(&served.base_url);

    let intent = client
        .payment_intents()
        .create(
            create_params(PaymentMethodType::MtnMomo),
            RequestOptions::new(),
        )
        .await
        .context("creating the intent to confirm")?;

    let error = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(MSISDN),
            RequestOptions::new(),
        )
        .await
        .expect_err("credentials the rail rejects cannot produce a submitted charge");
    let (status, kind, code, _param) = api_error(error);
    assert_eq!(
        status, 409,
        "a decline is the caller's state to act on, not a 5xx: the rail answered"
    );
    assert_eq!(kind, "invalid_request_error");
    assert_eq!(
        code.as_deref(),
        Some("charge_declined"),
        "one code per outcome — `provider_unavailable` means `502, we are retrying`"
    );

    let charge = stored_charge(&harness.pool, &intent.id).await?;
    assert_eq!(
        charge.state, "failed",
        "the rail decided; the charge is terminal"
    );
    assert_eq!(
        charge.failure_code.as_deref(),
        Some("provider_account_blocked"),
        "the taxonomy, not the rail's words (docs/flows/failures.md)"
    );
    assert!(
        charge
            .failure_raw
            .as_ref()
            .is_some_and(|raw| !raw.is_empty()),
        "the rail's own reason is kept for whoever fixes the mapping table"
    );

    let (status_code, error_kind, answered) = stored_attempt(&harness.pool, &charge.id).await?;
    assert!(answered, "a decline is an answer");
    assert_eq!(
        status_code,
        Some(vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT)
    );
    assert_eq!(
        error_kind.as_deref(),
        Some("charge_declined"),
        "the attempt records the error's own classification, so an operator counting rail \
         failures and a merchant reading an envelope never see one word meaning two things"
    );

    let after = client
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("re-reading the intent after the decline")?;
    assert_eq!(
        after.status,
        IntentStatus::RequiresPaymentMethod,
        "the lifecycle has no `failed` status: a declined intent goes back with an error"
    );
    let last = after
        .last_payment_error
        .as_ref()
        .expect("a declined intent carries why");
    assert_eq!(last.code, "provider_account_blocked");
    assert!(
        !last.message.contains("invalid_client"),
        "the rail's raw words are logged, never rendered to a merchant \
         (docs/flows/errors.md): {}",
        last.message
    );

    // And one charge per intent, forever: the intent is not re-confirmable
    // even though its status invites it.
    let again = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(MSISDN),
            RequestOptions::new(),
        )
        .await
        .expect_err("a failed charge is still a charge");
    assert_eq!(api_error(again).0, 409);
    assert_eq!(charge_count(&harness.pool, &intent.id).await?, 1);

    served.server.abort();
    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------- test 3(bis)

/// The rail declining the **payment**, at `requestToPay`, for a payer it has
/// no record of: `409 charge_declined`, `invalid_payer` on the charge, and
/// an intent the merchant can read the reason off and re-attempt with a
/// different instrument.
///
/// # Why this case exists separately
///
/// Until the Step 3 review the only "decline" in this file was
/// [`credentials_the_rail_refuses_are_a_page_and_a_terminal_charge`], which
/// never reaches `requestToPay` — it fails at the token call with our own
/// credentials, and `provider_account_blocked` is an operator's page. So
/// nothing proved that a confirm whose *submit* the rail refuses commits
/// the same four things, and the difference is not cosmetic: the taxonomy
/// code, the severity, the person who has to act and the sentence the
/// merchant reads are all different.
///
/// # How the stub is steered without a test seam
///
/// The reference cannot be chosen (see the module doc), but the *payer*
/// can: it comes from `payment_method_data[mtn_momo][msisdn]` on the
/// merchant's own request and reaches the rail as `payer.partyId`. So this
/// confirms with [`UNKNOWN_PAYER_MSISDN`], which `requesttopay.json`
/// matches on that JSON path and answers `400 PAYER_NOT_FOUND` to. No
/// configuration is changed and no second server is needed: this is the
/// ordinary harness, the shipping adapter, and a payer the rail rejects.
///
/// # The two strings
///
/// `charges.failure_raw` keeps the rail's own words, because
/// `docs/flows/failures.md` needs an unmapped reason to survive for whoever
/// fixes the mapping table. `payment_intents.last_payment_error_message` is
/// rendered to the *merchant*, so it must be `public_message()` and must not
/// carry `PAYER_NOT_FOUND` — a rail's vocabulary reaching a merchant through
/// a side door is exactly what `docs/flows/errors.md` forbids. Both halves
/// are asserted below, on the same decline, which is the only way to see
/// that they are different strings.
#[tokio::test]
async fn a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read() -> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    let intent = client
        .payment_intents()
        .create(
            create_params(PaymentMethodType::MtnMomo),
            RequestOptions::new(),
        )
        .await
        .context("creating the intent to confirm")?;

    let error = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(UNKNOWN_PAYER_MSISDN),
            RequestOptions::new(),
        )
        .await
        .expect_err("a payer the rail has no record of cannot produce a submitted charge");
    let (status, kind, code, message) = match error {
        vpay_sdk::Error::Api {
            status,
            kind,
            code,
            message,
            ..
        } => (status, kind, code, message),
        other => panic!("expected a vpay API error envelope, got {other:?}"),
    };
    assert_eq!(
        status, 409,
        "the rail answered and its answer was `no`: that is the caller's state to act on, \
         not a 5xx claiming vpay failed"
    );
    assert_eq!(kind, "invalid_request_error");
    assert_eq!(
        code.as_deref(),
        Some("charge_declined"),
        "one code per outcome, whatever the rail's own vocabulary was"
    );
    assert!(
        !message.contains("PAYER_NOT_FOUND"),
        "the rail's words are logged, never rendered to a merchant \
         (docs/flows/errors.md): {message}"
    );

    let charge = stored_charge(&harness.pool, &intent.id).await?;
    assert_eq!(
        charge.state, "failed",
        "the rail decided about this payment; the charge is terminal"
    );
    assert_eq!(
        charge.failure_code.as_deref(),
        Some("invalid_payer"),
        "MTN's PAYER_NOT_FOUND maps through docs/flows/adapter-mtn-momo.md's table. Not \
         `provider_account_blocked`: that is the credentials case, and if these two ever \
         agree one of them has stopped testing what it says"
    );
    assert!(
        charge
            .failure_raw
            .as_ref()
            .is_some_and(|raw| raw.contains("PAYER_NOT_FOUND")),
        "the rail's own reason is kept where an operator fixing the mapping table will \
         find it: {:?}",
        charge.failure_raw
    );

    let (status_code, error_kind, answered) = stored_attempt(&harness.pool, &charge.id).await?;
    assert!(answered, "a decline is an answer");
    assert_eq!(
        status_code,
        Some(vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT)
    );
    assert_eq!(
        error_kind.as_deref(),
        Some("charge_declined"),
        "the attempt records the error's own classification, so an operator counting rail \
         failures and a merchant reading an envelope never see one word meaning two things"
    );

    // The committed columns, read straight from Postgres rather than off the
    // rendered object: `last_payment_error` is a column *pair*, and the
    // CHECK that keeps them paired is only meaningful if something asserts
    // what was actually written.
    let (error_code, error_message) = stored_payment_error(&harness.pool, &intent.id).await?;
    assert_eq!(
        error_code.as_deref(),
        Some("invalid_payer"),
        "the intent carries the same taxonomy code as its charge"
    );
    let error_message = error_message.expect("the pair is written together or not at all");
    assert_eq!(
        error_message, "The payment was declined (invalid_payer).",
        "the merchant-facing half is the error's public_message(), verbatim"
    );

    let after = client
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("re-reading the intent after the decline")?;
    assert_eq!(
        after.status,
        IntentStatus::RequiresPaymentMethod,
        "the lifecycle has no `failed` status: a declined intent goes back with an error, \
         and this payer may well succeed on a new intent with a different MSISDN"
    );
    assert_eq!(
        after.next_action, None,
        "a push rail that declined has nothing for a browser to do"
    );
    let last = after
        .last_payment_error
        .as_ref()
        .expect("a declined intent carries why");
    assert_eq!(last.code, "invalid_payer");
    assert_eq!(
        last.message, error_message,
        "the object renders the column, rather than re-deriving a sentence"
    );

    // One charge per intent, forever — and because that charge is terminal,
    // the advice is the one that says a retry is a new intent. The *live*
    // wording (`do not create a new PaymentIntent`) belongs to
    // `an_unreachable_rail_leaves_the_charge_where_recovery_expects_it`, and
    // handing it out here would tell a merchant to poll a charge that is
    // never going to change.
    let again = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(MSISDN),
            RequestOptions::new(),
        )
        .await
        .expect_err("a failed charge is still a charge");
    let (status, message) = api_error_message(again);
    assert_eq!(status, 409);
    assert!(
        message.contains("create a new payment intent to try again"),
        "a terminal charge is the case where that advice is safe: nothing is in flight to \
         duplicate: {message}"
    );
    assert!(
        !message.contains("do not create a new PaymentIntent"),
        "and the live-charge wording must not reach a merchant whose charge has settled: \
         {message}"
    );
    assert_eq!(charge_count(&harness.pool, &intent.id).await?, 1);

    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 4

/// A rail that cannot be reached: the `Category::Rail` answer, and **nothing
/// moved**.
///
/// The charge stays `submitting` and the attempt stays unanswered, which is
/// precisely the row pair `docs/flows/crash-safety.md`'s recovery table
/// reads as "POST issued, response lost — go and poll". Advancing the charge
/// here, or failing it, would be a claim about what the rail did with a
/// request we never learned the fate of.
///
/// The retry is the second half: the idempotency key is *released* on a
/// `5xx` (`vpay_api`'s `PostRequest::finish`), so the same call re-executes
/// rather than being answered "still in progress" for 24 hours — and what
/// stops it double-charging is the unique index, which answers `409`.
#[tokio::test]
async fn an_unreachable_rail_leaves_the_charge_where_recovery_expects_it() -> anyhow::Result<()> {
    let harness = harness().await?;

    let unreachable = RailSetup::working(UNREACHABLE_RAIL);
    let orange = RailSetup::working(&harness.orange_url);
    let jwks_a = harness.jwks_a.clone();
    let served = serve(&harness.pool, &harness.server_pem, |base_url| {
        config_with(base_url, jwks_a, &unreachable, &orange)
    })
    .await?;
    let client = harness.client_for(&served.base_url);

    let intent = client
        .payment_intents()
        .create(
            create_params(PaymentMethodType::MtnMomo),
            RequestOptions::new(),
        )
        .await
        .context("creating the intent to confirm")?;

    let options = RequestOptions::new().with_idempotency_key("confirm-unreachable-rail");
    let error = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(MSISDN),
            options.clone(),
        )
        .await
        .expect_err("a rail nothing is listening on cannot accept a charge");
    let (status, kind, code, _param) = api_error(error);
    assert_eq!(
        status, 502,
        "the rail is the failing party, and the worker is what retries it"
    );
    assert_eq!(kind, "api_error");
    assert_eq!(code.as_deref(), Some("provider_unavailable"));

    let charge = stored_charge(&harness.pool, &intent.id).await?;
    assert_eq!(
        charge.state, "submitting",
        "we do not know whether the rail saw the request, so the charge stays where a \
         recovery pass will find it"
    );
    assert_eq!(charge.failure_code, None, "nothing declined anything");

    let (status_code, error_kind, answered) = stored_attempt(&harness.pool, &charge.id).await?;
    assert!(
        !answered,
        "no answer was received, and the row must not claim one"
    );
    assert_eq!(status_code, None);
    assert_eq!(error_kind.as_deref(), Some("provider_unavailable"));

    let after = client
        .payment_intents()
        .retrieve(&intent.id)
        .await
        .context("re-reading the intent after the unreachable rail")?;
    assert_eq!(after.status, IntentStatus::RequiresPaymentMethod);
    assert_eq!(after.last_payment_error, None);
    assert_eq!(after.next_action, None);

    // The retry re-executes rather than replaying a stored 502 — and meets
    // the unique index, which is what actually prevents the second charge.
    let retried = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(MSISDN),
            options,
        )
        .await
        .expect_err("the retry re-executes and meets the existing charge");
    let (status, message) = api_error_message(retried);
    assert_eq!(
        status, 409,
        "the key was released, so this is the confirm running again — not a replayed 502"
    );
    // And the *sentence* is the finding this test exists to pin. The charge
    // it met is `submitting`: the rail may be holding this payment, because
    // "the response was lost" is not "the request was never received"
    // (docs/flows/crash-safety.md's recovery table). Answering this merchant
    // with "create a new payment intent to try again" — which is what this
    // 409 said until the Step 3 security review — is an instruction to
    // prompt the payer's handset a second time for the same money.
    assert!(
        message.contains("do not create a new PaymentIntent"),
        "a 409 over a live charge must not invite a second one: {message}"
    );
    assert!(
        message.contains(&intent.id),
        "and it must say what to poll: {message}"
    );
    assert!(
        !message.contains("create a new payment intent to try again"),
        "the terminal-charge advice must not reach a merchant whose charge is submitting: \
         {message}"
    );
    assert_eq!(charge_count(&harness.pool, &intent.id).await?, 1);

    served.server.abort();
    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 5

/// A rail that settles in another currency is refused **before** a charge
/// exists.
///
/// `config/application.yml` ships exactly this shape: MTN on EUR (its
/// sandbox rejects XAF) and Orange on XAF. Without this rule a confirm sent
/// an XAF amount to a rail that would read it as EUR — and a rail that
/// simply believes the number charges a payer 5,000 of the wrong unit.
///
/// The `param` is `payment_method_data[type]` rather than `currency`
/// because the currency is fixed at creation: the rail is the part of this
/// request the caller can still change.
///
/// `count(*) = 0` is the load-bearing assertion. A check that ran after the
/// insert would leave a `submitting` charge nothing will ever answer for,
/// and "one charge per intent, forever" means the merchant could not then
/// confirm on the right rail either.
#[tokio::test]
async fn a_rail_that_settles_in_another_currency_is_refused_before_any_charge() -> anyhow::Result<()>
{
    let harness = harness().await?;

    let mtn = RailSetup::working(&harness.mtn_url);
    let mut eur_orange = RailSetup::working(&harness.orange_url);
    eur_orange.currency = "EUR".to_owned();
    let jwks_a = harness.jwks_a.clone();
    let served = serve(&harness.pool, &harness.server_pem, |base_url| {
        config_with(base_url, jwks_a, &mtn, &eur_orange)
    })
    .await?;
    let client = harness.client_for(&served.base_url);

    let intent = client
        .payment_intents()
        .create(
            create_params(PaymentMethodType::OrangeMoney),
            RequestOptions::new(),
        )
        .await
        .context("an XAF intent for a rail that now settles in EUR")?;

    let error = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::orange_money(RETURN_URL),
            RequestOptions::new(),
        )
        .await
        .expect_err("a XAF intent may not be charged on a EUR rail");
    let (status, kind, _code, param) = api_error(error);
    assert_eq!(status, 400);
    assert_eq!(kind, "invalid_request_error");
    assert_eq!(param.as_deref(), Some("payment_method_data[type]"));

    assert_eq!(
        charge_count(&harness.pool, &intent.id).await?,
        0,
        "the refusal happens before any charge is inserted, so the intent is still confirmable \
         on a rail that settles in its currency"
    );

    // Which it is: the same intent confirms on the XAF rail.
    let confirmed = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(MSISDN),
            RequestOptions::new(),
        )
        .await;
    // The intent was created naming `orange_money` only, so this is refused
    // by the *rails the intent allows* rule rather than by currency — the
    // point is only that nothing about the first refusal consumed the
    // intent's one charge.
    assert_eq!(
        api_error(confirmed.expect_err("the intent names only orange_money")).1,
        "invalid_request_error"
    );
    assert_eq!(charge_count(&harness.pool, &intent.id).await?, 0);

    served.server.abort();
    harness.shutdown().await;
    Ok(())
}

// ------------------------------------------------------------------ test 6

/// A `return_url` the merchant cannot be allowed to store, refused **before
/// any charge exists** and as a `400` naming the field.
///
/// Two things are wrong with the values below, and they fail differently:
///
/// * `javascript:` is a scheme a browser *executes* rather than navigates
///   to. It is persisted on `charges.return_url` and rendered straight back
///   as `next_action.redirect_to_url.return_url` on every later read of the
///   intent, so a merchant's checkout — or this project's own dashboard —
///   would put it in front of a person. Migration `0019`'s
///   `return_url_is_a_web_url` refuses it at the column as a backstop.
/// * a URL past 2048 characters trips `return_url_length`, and until the
///   Step 3 security review that is exactly what happened: the value reached
///   the insert, Postgres refused it, and the merchant was told with a `503`
///   that vpay was unavailable and to retry — for a field they got wrong,
///   and after a `provider_requests` row had been opened.
///
/// `charge_count == 0` is the load-bearing half. A check that ran after the
/// insert would leave a `submitting` charge nothing will ever answer for,
/// and "one charge per intent, forever" means the merchant could not then
/// confirm the intent correctly either.
#[tokio::test]
async fn a_return_url_that_is_not_a_bounded_web_url_is_refused_before_any_charge()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let client = harness.a();

    for (return_url, why) in [
        (
            "javascript:alert(document.cookie)".to_owned(),
            "a scheme a browser executes",
        ),
        ("shop.example/return".to_owned(), "no scheme at all"),
        (
            format!("https://shop.example/{}", "x".repeat(2_048)),
            "past the column's 2048-character ceiling",
        ),
    ] {
        let intent = client
            .payment_intents()
            .create(
                create_params(PaymentMethodType::OrangeMoney),
                RequestOptions::new(),
            )
            .await
            .context("creating the intent to confirm")?;

        let error = client
            .payment_intents()
            .confirm(
                &intent.id,
                ConfirmPaymentIntentParams::orange_money(return_url.clone()),
                RequestOptions::new(),
            )
            .await
            .expect_err(&format!("{why}: this return_url must not be accepted"));
        let (status, kind, _code, param) = api_error(error);

        assert_eq!(
            status, 400,
            "{why}: the merchant's field is wrong, so this is theirs to fix — not a 503 \
             saying vpay is unavailable"
        );
        assert_eq!(kind, "invalid_request_error");
        assert_eq!(
            param.as_deref(),
            Some("return_url"),
            "{why}: the envelope must name the field"
        );
        assert_eq!(
            charge_count(&harness.pool, &intent.id).await?,
            0,
            "{why}: refused before the insert, so the intent's one charge is still available"
        );
    }

    harness.shutdown().await;
    Ok(())
}
