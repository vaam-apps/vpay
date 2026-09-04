//! `POST /provider/{code}/callback`, end to end: the rail tells us, and the
//! payment settles *now* instead of at the poll ladder's next rung.
//!
//! ```text
//!   vpay_sdk::Client
//!     -> POST /v1/payment_intents/{id}/confirm      (the real axum router)
//!        -> MTN/Orange adapter -> HTTP -> WireMock container
//!           carrying X-Callback-Url / notif_url = {public_base_url}/provider/{code}/callback
//!   vpay_worker::run_once                           (the real loop body)
//!     -> poll #1 -> the rail says PENDING -> reschedule onto the ladder
//!   (the job is put out at a later rung: a callback deliberately does not
//!    accelerate a poll already due inside PULL_FORWARD_FLOOR)
//!   vpay_worker::run_once                           -> None: nothing is runnable
//!   the rail                                        (this test, as the rail)
//!     -> POST {the very URL the rail was handed}, with the rail's documented body
//!   vpay_worker::run_once                           -> poll #2 -> settled
//! ```
//!
//! # What each step of that is evidence for, and what it is not
//!
//! The two `run_once` calls either side of the callback are the whole proof
//! and are the reason this suite steps the loop rather than running it. The
//! second one answering **`None`** is the assertion that matters most: the
//! queue's only job is parked out on the ladder, `jobs::claim`'s predicate is
//! `run_at <= now()`, and so *nothing this process can do* settles that
//! charge until the ladder comes round. The `Some` that follows the callback
//! therefore cannot have come from the ladder — the wall clock between the
//! two is asserted to be well under the rung — and the only thing that
//! changed in between was one `UPDATE jobs SET run_at = now()`.
//!
//! # Why the headline case parks the job past the first rung
//!
//! Since Step 8's review the pull-forward refuses a job **already due within
//! `vpay_api::provider_callback::PULL_FORWARD_FLOOR`** — ten seconds, the
//! ladder's own fastest rung — so that an unauthenticated caller cannot spend
//! a rail request on a charge the queue was about to ask about anyway. A poll
//! rescheduled onto the ladder's *first* rung is exactly that job, and
//! `a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run` is
//! the case that asserts it is left alone. The headline case therefore stages
//! the state a callback is actually for: a charge sitting further out on the
//! ladder, which is where every charge is after its first couple of rungs.
//!
//! It is deliberately **not** evidence that the rails call this URL. Nothing
//! in this repository has ever called MTN or Orange
//! (`docs/flows/adapter-mtn-momo.md` §"Not proven"), so what is proven here
//! is the half vpay owns: the URL vpay hands the rail is the URL vpay serves,
//! and a POST of the rail's *documented* body to it does what the design says.
//! The bodies below are transcribed from the flow docs, exactly as the
//! conformance suite's `documented_callback_body` transcribes them, and a
//! body faithful to those documents but not to the rail would pass.
//!
//! # Why the callback URL is read back off the rail's own journal
//!
//! [`callback_url_the_rail_was_told`] digs the `X-Callback-Url` header (MTN)
//! or the `notif_url` field (Orange) out of WireMock's request journal, and
//! the test POSTs to *that string*. Constructing the URL from `base_url` in
//! the test would prove only that the test can do string formatting; taking
//! it from what the rail received joins three things that live in three
//! crates and do not compile against each other —
//! `vpay_config::ProviderHost::effective_callback_url`, each adapter's
//! `submit`, and `vpay_api::provider_callback`'s mount point. If any of them
//! moves, this suite POSTs somewhere that answers 404 and fails.
//!
//! # How a real confirm reaches a PENDING -> SUCCESSFUL walk on both rails
//!
//! By rewriting `charges.provider_reference_id` to [`SCENARIO_REF`] after the
//! confirm and before the worker polls — the technique `worker_e2e.rs`
//! documents for its decline case, and for the same reason. A confirm cannot
//! choose its reference (`vpay_api` mints it with `Uuid::new_v4()` before
//! committing the charge, and a seam to fix it from a test would be a code
//! path that exists only outside production — AGENTS.md's first rule), and
//! *both* rails' pending-then-successful scenarios are keyed on that fixed
//! reference. MTN's e2e MSISDN steer (`worker_e2e.rs`'s `SETTLING_MSISDN`)
//! has no Orange counterpart at all — Orange's submit body carries no payer —
//! so a rewrite is the only technique that is the same on both rails, and a
//! suite whose two halves used different techniques would be two suites.
//!
//! The reference is opaque to everything except the rail, so this is a test
//! writing stored state, not a code seam: it says "suppose the rail had
//! assigned this charge the reference it walks a scenario for".
//!
//! # No test doubles
//!
//! Real Postgres, real WireMock (the shared
//! `backends/tests/conformance/wiremock` tree), the shipping adapters, the
//! shipping router, the shipping SDK and the shipping loop body.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use rstest::rstest;
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};
use vpay_db::Repositories;
use vpay_sdk::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, Credentials, IntentStatus,
    PaymentMethodType, RequestOptions,
};
use vpay_worker::{Adapters, Disposition, RailConfigs, RecoveryPolicy, Settled};

mod support;

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres,
    rail_configs, serve,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

const PUSH_RAIL: &str = "mtn_momo";
const REDIRECT_RAIL: &str = "orange_money";

const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

/// A documentation MSISDN nothing stubs specifically, so every confirm here
/// falls through to `requesttopay.json`'s catch-all `202`.
const MSISDN: &str = "237670000000";

/// Where a merchant asks Orange to send the payer back. Never fetched; it is
/// a column and a rendered field.
const RETURN_URL: &str = "https://shop.example/return";

/// The reference both rails stub a `PENDING` -> `SUCCESSFUL` scenario for —
/// `mtn-pending-then-successful` and `orange-pending-then-successful`, both
/// `priority: 1` and both keyed on this exact UUID. See the module docs for
/// why a charge is moved onto it rather than confirmed onto it.
const SCENARIO_REF: Uuid = Uuid::from_u128(0x0ce0);

/// The ladder's first rung (`vpay_worker::poll_delay(0)`), written out here
/// because this suite asserts *against* it rather than using it: the whole
/// claim is that a callback settles a charge in less time than this.
///
/// Transcribed rather than imported so the assertion is about the documented
/// ladder (`docs/flows/reconciler.md`: "10s, 20s, 30s, …") and not about
/// whatever number the implementation currently returns — the same reason
/// `vpay_worker`'s own `the_delivery_ladder_is_the_documented_one` writes its
/// rungs out.
const FIRST_RUNG: Duration = Duration::from_secs(10);

/// How far short of [`FIRST_RUNG`] the whole callback round trip must land.
///
/// Not a timeout: the sequence it bounds is one HTTP POST to loopback, one
/// job claim and one status query against a container on the same host,
/// which is milliseconds. Eight seconds is a ceiling wide enough that a
/// loaded CI machine does not flake it and tight enough that it could not be
/// met by the ladder, whose next rung is [`FIRST_RUNG`] away and has not
/// arrived.
const CALLBACK_BUDGET: Duration = Duration::from_secs(8);

/// Where the headline case parks the poll job before the callback arrives.
///
/// Thirty seconds — `poll_delay(2)`, the ladder's third rung. Past
/// [`FIRST_RUNG`], because a callback deliberately no longer accelerates a
/// job due inside `vpay_api::provider_callback::PULL_FORWARD_FLOOR`, and a
/// real charge reaches this rung by its third poll.
///
/// Written by moving `run_at` directly rather than by walking the ladder:
/// each rung costs a poll, and each poll consumes one of the rail scenario's
/// two answers — walking to it would settle the charge before the callback
/// could be tested at all. What is under test here is the route, not the
/// ladder (`vpay_worker`'s own suites own that).
const LATER_RUNG: Duration = Duration::from_secs(30);

fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

// ------------------------------------------------------------------ harness

struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    _mtn: ContainerAsync<GenericImage>,
    _orange: ContainerAsync<GenericImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    pool: PgPool,
    base_url: String,
    /// Each rail stub's origin, for the admin API — not derivable from the
    /// configured `base_url`, because Orange's carries a path prefix.
    mtn_origin: String,
    orange_origin: String,
    pem_a: String,
    adapters: Arc<Adapters>,
    rails: Arc<RailConfigs>,
    /// The plain outbound client this suite uses *as a rail*: it POSTs the
    /// callback exactly as MTN's or Orange's backend would, over a real
    /// socket to the real router. The shipping client from
    /// `vpay_provider::http`, not `reqwest::Client::new()`, for the reason
    /// `support::webhook_client` gives.
    http: reqwest::Client,
}

impl Harness {
    fn client(&self) -> vpay_sdk::Client {
        vpay_sdk::Client::builder(&self.base_url)
            .credentials(
                Credentials::rsa_pem(CLIENT_A, &self.pem_a).expect("the generated PEM parses"),
            )
            .build()
            .expect("the SDK client builds from a base URL and a credential")
    }

    /// Claims and runs exactly one job, or `None` when the queue has nothing
    /// runnable **right now**.
    ///
    /// `run_once` is the loop's own body — the same function
    /// `vpay_worker::run_loop` calls — so this drives the shipping
    /// claim/settle protocol one step at a time. The `Option` is the point
    /// here rather than an inconvenience: `None` is this suite's evidence
    /// that a job parked at the next rung is genuinely unreachable, which is
    /// what makes the `Some` after a callback mean something.
    async fn step(&self) -> anyhow::Result<Option<Settled>> {
        let endpoints = support::no_webhook_endpoints();
        vpay_worker::run_once(
            self.repositories.as_ref(),
            &self.adapters,
            &self.rails,
            &RecoveryPolicy::default(),
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                egress: support::default_egress_policy(),
            },
            "provider-callback-suite",
        )
        .await
        .context("running one job")
    }

    /// The same, asserting there was a job to run.
    async fn step_one(&self) -> anyhow::Result<Settled> {
        self.step()
            .await?
            .context("the queue had no runnable job when one was expected")
    }

    fn stub_origin(&self, rail: &str) -> &str {
        match rail {
            PUSH_RAIL => &self.mtn_origin,
            REDIRECT_RAIL => &self.orange_origin,
            other => panic!("no stub for rail {other}"),
        }
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// Both rails on XAF, `livemode: false` — the shape `config/application.yml`
/// has, including the settings and credentials keys
/// `vpay_config::REQUIRED_RAIL_KEYS` insists on.
///
/// `callback_url` is `None` on both, which is the load-bearing part of this
/// fixture: it makes `ProviderHost::effective_callback_url` *derive*
/// `{public_base_url}/provider/{code}/callback`, and `public_base_url` is the
/// address this suite's own server is listening on. So the URL each adapter
/// sends its rail is a URL that resolves to the router under test. An
/// override here would make every assertion below pass against a route
/// nothing mounts.
fn config_with(base_url: &str, jwks_a: Value, mtn_url: &str, orange_url: &str) -> Config {
    Config {
        deployment: Deployment {
            name: "provider-callback".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        webhooks: vpay_config::WebhookPolicy::default(),
        providers: vec![
            ProviderHost {
                code: PUSH_RAIL.to_owned(),
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
                ]),
                callback_url: None,
                currency: CURRENCY.to_ascii_uppercase(),
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
                    url: orange_url.to_owned(),
                    label: "orange-wiremock".to_owned(),
                },
                settings: BTreeMap::from([
                    ("env".to_owned(), "dev".to_owned()),
                    ("lang".to_owned(), "en".to_owned()),
                ]),
                callback_url: None,
                currency: CURRENCY.to_ascii_uppercase(),
                credentials: BTreeMap::from([
                    ("merchant_key".to_owned(), "stub-merchant-key".to_owned()),
                    ("client_id".to_owned(), "stub-client-id".to_owned()),
                    ("client_secret".to_owned(), "stub-client-secret".to_owned()),
                ]),
            },
        ],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_ascii_uppercase(),
            exponent: 0,
        }],
        merchant_clients: vec![merchant_client(CLIENT_A, MERCHANT_A, jwks_a)],
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (postgres, repositories, pool) = migrated_postgres().await?;

    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let orange = vpay_testkit::containers::start_wiremock(&mappings_dir("orange"))
        .await
        .context("the Orange stub container starts")?;

    let mtn_origin = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );
    let orange_origin = format!(
        "http://127.0.0.1:{}",
        orange
            .get_host_port_ipv4(8080)
            .await
            .context("the Orange stub's mapped port")?
    );
    // The `/orange-money-webpay/{env}` prefix is part of the configured base
    // URL (`docs/flows/adapter-orange-money.md`), exactly as
    // `config/application.yml` writes it.
    let orange_base = format!("{orange_origin}/orange-money-webpay/dev");

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();

    let jwks_for_server = jwks_a.clone();
    let mtn_for_server = mtn_origin.clone();
    let orange_for_server = orange_base.clone();
    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(
            base_url,
            jwks_for_server,
            &mtn_for_server,
            &orange_for_server,
        )
    })
    .await?;

    // The same configuration `serve` booted, rebuilt so the worker half of
    // this suite polls the rails at the hosts the server submitted to. The
    // base URL is only knowable after the listener is bound, which is why it
    // is built twice rather than passed in.
    let config = config_with(&served.base_url, jwks_a, &mtn_origin, &orange_base);

    Ok(Harness {
        _postgres: postgres,
        _mtn: mtn,
        _orange: orange,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
        mtn_origin,
        orange_origin,
        pem_a,
        adapters: Arc::new(support::adapters_by_code()),
        rails: Arc::new(rail_configs(&config)),
        // The suite's own client, for the POSTs a rail would make; delivery
        // egress is the worker's concern (`support::default_egress_policy`).
        http: reqwest::Client::new(),
    })
}

fn create_params(rail: PaymentMethodType) -> CreatePaymentIntentParams {
    CreatePaymentIntentParams {
        amount: AMOUNT,
        currency: CURRENCY.to_owned(),
        payment_method_types: vec![rail],
        metadata: BTreeMap::new(),
        description: None,
    }
}

// -------------------------------------------------------------- rail tables
//
// Everything below that differs per rail is a table row, never an `if rail ==
// …` in a test body — the same rule the conformance suite holds itself to
// (ADR-0002). Three things differ, and all three are *wire shape*, which is
// the one thing a `Capabilities` value cannot express: how a merchant
// confirms, where the rail carries our callback URL on a submit, and what its
// notification body looks like.

/// How a merchant confirms an intent on this rail through the SDK.
fn confirm_params(rail: &str) -> ConfirmPaymentIntentParams {
    match rail {
        PUSH_RAIL => ConfirmPaymentIntentParams::mtn_momo(MSISDN),
        REDIRECT_RAIL => ConfirmPaymentIntentParams::orange_money(RETURN_URL),
        other => panic!("no confirm shape for rail {other}"),
    }
}

fn payment_method_type(rail: &str) -> PaymentMethodType {
    match rail {
        PUSH_RAIL => PaymentMethodType::MtnMomo,
        REDIRECT_RAIL => PaymentMethodType::OrangeMoney,
        other => panic!("no payment method type for rail {other}"),
    }
}

/// The notification body this rail is documented to POST to its callback URL,
/// naming `reference`.
///
/// Transcribed from `docs/flows/adapter-mtn-momo.md` and
/// `docs/flows/adapter-orange-money.md`, the same source the conformance
/// suite's `documented_callback_body` transcribes from — and deliberately
/// **carrying a terminal `status`**, because the point of the assertions
/// below is that vpay reads identifiers out of these and nothing else. A body
/// that said `SUCCESSFUL` and settled a charge on its own would be a rail
/// telling vpay where the money went over an unauthenticated request.
///
/// Orange's `pay_token` here is deliberately *not* the token the stub minted
/// at submit: if the route ever wrote `CallbackRef::ref_extra` onto the
/// charge, the following status query would carry this made-up token, and
/// `a_callback_writes_no_charge_or_intent_state` would see it.
fn documented_callback_body(rail: &str, reference: Uuid) -> String {
    match rail {
        PUSH_RAIL => format!(
            r#"{{"financialTransactionId":"1234567890","externalId":"{reference}",
                 "amount":"5000","currency":"XAF",
                 "payer":{{"partyIdType":"MSISDN","partyId":"{MSISDN}"}},
                 "payeeNote":"vpay","status":"SUCCESSFUL"}}"#
        ),
        REDIRECT_RAIL => format!(
            r#"{{"order_id":"{reference}","status":"SUCCESS","txnid":"stub-txn",
                 "notif_token":"a-token-this-deployment-never-issued",
                 "pay_token":"a-pay-token-this-deployment-never-issued"}}"#
        ),
        other => panic!("no documented callback body for rail {other}"),
    }
}

/// A body this rail's `parse_callback` must refuse.
///
/// **Orange's names a real charge and MTN's cannot**, and that asymmetry is
/// the rails', not this suite's. MTN's `parse_callback` recovers the
/// reference from `referenceId` *or* `externalId` and fails only when neither
/// is a UUID, so a refused MTN body is by construction one that names nothing.
/// Orange's additionally requires a `notif_token` — "refusing to parse an
/// unverifiable callback" — so its refused body can carry a genuine
/// `order_id`, which makes the Orange half the decisive one for
/// [`an_unparseable_callback_body_is_refused_and_moves_no_job`]: it is the
/// only one where a route that ignored the parse failure would have had a
/// charge to act on.
fn refused_callback_body(rail: &str, reference: Uuid) -> String {
    match rail {
        PUSH_RAIL => r#"{"externalId":"not-a-uuid","status":"SUCCESSFUL"}"#.to_owned(),
        REDIRECT_RAIL => format!(r#"{{"order_id":"{reference}","status":"SUCCESS"}}"#),
        other => panic!("no refused callback body for rail {other}"),
    }
}

/// The WireMock request pattern selecting this rail's `submit`, and where in
/// the recorded request its callback URL sits.
///
/// `Header` on MTN (`X-Callback-Url`), `BodyField` on Orange (`notif_url`).
enum CallbackCarrier {
    Header(&'static str),
    BodyField(&'static str),
}

fn submit_journal_query(rail: &str) -> (&'static str, CallbackCarrier) {
    match rail {
        PUSH_RAIL => (
            r#"{"method":"POST","urlPath":"/collection/v1_0/requesttopay"}"#,
            CallbackCarrier::Header("X-Callback-Url"),
        ),
        REDIRECT_RAIL => (
            r#"{"method":"POST","urlPathPattern":"/orange-money-webpay/[^/]+/v1/webpayment"}"#,
            CallbackCarrier::BodyField("notif_url"),
        ),
        other => panic!("no submit journal query for rail {other}"),
    }
}

// ------------------------------------------------------------------ reading

/// The URL this rail's stub was actually handed on the submit it just
/// received.
///
/// WireMock's request journal over its admin API — the only witness for what
/// left this process. See the module docs for why the test POSTs to this
/// string rather than to one it built itself.
async fn callback_url_the_rail_was_told(harness: &Harness, rail: &str) -> anyhow::Result<String> {
    let (pattern, carrier) = submit_journal_query(rail);
    let found: Value = harness
        .http
        .post(format!(
            "{}/__admin/requests/find",
            harness.stub_origin(rail)
        ))
        .body(pattern)
        .send()
        .await
        .context("the stub's admin API answers")?
        .json()
        .await
        .context("the journal response is JSON")?;

    let request = found
        .get("requests")
        .and_then(Value::as_array)
        .and_then(|requests| requests.first())
        .with_context(|| format!("{rail}: the stub recorded no submit at all"))?;

    let url = match carrier {
        // Case-insensitively, because HTTP header names are: `hyper` writes
        // the lower-case spelling `HeaderName::from_static` requires, so
        // WireMock's journal records `x-callback-url` and a lookup for the
        // documented `X-Callback-Url` finds nothing. Matching on the
        // documented spelling and comparing without case is what keeps this
        // assertion about the header MTN documents rather than about
        // whichever casing our client happens to emit.
        CallbackCarrier::Header(name) => request
            .get("headers")
            .and_then(Value::as_object)
            .and_then(|headers| {
                headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .and_then(|(_, value)| value.as_str())
            })
            .map(str::to_owned)
            .with_context(|| format!("{rail}: the submit carried no {name} header"))?,
        CallbackCarrier::BodyField(field) => {
            let body: Value = serde_json::from_str(
                request
                    .get("body")
                    .and_then(Value::as_str)
                    .with_context(|| format!("{rail}: the submit had no body"))?,
            )
            .with_context(|| format!("{rail}: the submit body is not JSON"))?;
            body.get(field)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .with_context(|| format!("{rail}: the submit body carried no {field}"))?
        }
    };
    Ok(url)
}

#[derive(Debug, sqlx::FromRow)]
struct StoredCharge {
    id: String,
    state: String,
    /// The reference vpay generated and the rail was submitted under — the
    /// identifier a callback names, and the only one it can.
    provider_reference_id: Uuid,
    provider_txn_id: Option<String>,
    provider_ref_extra: Option<Value>,
}

async fn stored_charge(pool: &PgPool, payment_intent_id: &str) -> anyhow::Result<StoredCharge> {
    sqlx::query_as::<_, StoredCharge>(
        "SELECT id, state::TEXT AS state, provider_reference_id, provider_txn_id, \
         provider_ref_extra FROM charges WHERE payment_intent_id = $1",
    )
    .bind(payment_intent_id)
    .fetch_one(pool)
    .await
    .context("reading the charge")
}

async fn intent_status(pool: &PgPool, id: &str) -> anyhow::Result<String> {
    sqlx::query_scalar::<_, String>("SELECT status::TEXT FROM payment_intents WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("reading the intent status")
}

/// Every job row as `(dedupe_key, run_at)`, ordered — the whole queue, so a
/// "nothing was enqueued" assertion can be about the table rather than about
/// the one row a test remembered to look at.
async fn queue_snapshot(pool: &PgPool) -> anyhow::Result<Vec<(String, time::OffsetDateTime)>> {
    sqlx::query_as::<_, (String, time::OffsetDateTime)>(
        "SELECT dedupe_key, run_at FROM jobs WHERE run_at < 'infinity' ORDER BY dedupe_key",
    )
    .fetch_all(pool)
    .await
    .context("reading the queue")
}

async fn poll_run_at(pool: &PgPool, charge_id: &str) -> anyhow::Result<time::OffsetDateTime> {
    sqlx::query_scalar::<_, time::OffsetDateTime>("SELECT run_at FROM jobs WHERE dedupe_key = $1")
        .bind(vpay_worker::jobs::poll_dedupe_key(charge_id))
        .fetch_one(pool)
        .await
        .context("reading the poll job's run_at")
}

/// Parks this charge's poll job [`LATER_RUNG`] out, as the ladder would have
/// after a couple of rungs.
async fn park_the_poll_at_a_later_rung(pool: &PgPool, charge_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE jobs SET run_at = now() + ($2::BIGINT * INTERVAL '1 second') WHERE dedupe_key = $1",
    )
    .bind(vpay_worker::jobs::poll_dedupe_key(charge_id))
    .bind(i64::try_from(LATER_RUNG.as_secs()).unwrap_or(i64::MAX))
    .execute(pool)
    .await
    .context("parking the poll job at a later rung")?;
    Ok(())
}

/// Moves this charge onto the reference both rails walk a scenario for. See
/// the module docs for why.
async fn point_the_charge_at_the_scenario(pool: &PgPool, charge_id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE charges SET provider_reference_id = $2 WHERE id = $1")
        .bind(charge_id)
        .bind(SCENARIO_REF)
        .execute(pool)
        .await
        .context("pointing the charge at the scenario reference")?;
    Ok(())
}

/// Creates and confirms one intent on `rail`, and returns
/// `(intent_id, charge)` once the rail has answered.
async fn confirmed_charge(harness: &Harness, rail: &str) -> anyhow::Result<(String, StoredCharge)> {
    let client = harness.client();
    let intent = client
        .payment_intents()
        .create(
            create_params(payment_method_type(rail)),
            RequestOptions::new(),
        )
        .await
        .map_err(|error| anyhow::anyhow!("creating the intent: {error}"))?;

    let confirmed = client
        .payment_intents()
        .confirm(&intent.id, confirm_params(rail), RequestOptions::new())
        .await
        .map_err(|error| anyhow::anyhow!("confirming on {rail}: {error}"))?;
    assert!(
        matches!(
            confirmed.status,
            IntentStatus::Processing | IntentStatus::RequiresAction
        ),
        "{rail}: an accepted confirm leaves the intent live, got {:?}",
        confirmed.status
    );

    let charge = stored_charge(&harness.pool, &intent.id).await?;
    Ok((intent.id, charge))
}

// -------------------------------------------------------------------- cases

/// **The lane's headline claim**: a callback settles a charge before the poll
/// ladder's next rung would have fired.
///
/// The sequence and what each step rules out is in the module docs. In short:
/// the queue is measurably *empty of runnable work* immediately before the
/// callback and measurably settling immediately after it, with less than
/// [`CALLBACK_BUDGET`] of wall clock in between and the next rung
/// [`FIRST_RUNG`] away — so nothing but the callback can account for it.
///
/// Removing the `pull_forward_in_tx` call from
/// `vpay_api::provider_callback::callback` fails this at the
/// `run_at` assertion and again at the `step_one` after it, because the job
/// stays parked and the claim finds nothing.
///
/// The job is parked at [`LATER_RUNG`] rather than left on the rung poll #1
/// put it on: a callback no longer accelerates a job due inside
/// `PULL_FORWARD_FLOOR`, which is what
/// `a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run`
/// asserts and this case would otherwise trip over.
#[rstest]
#[case::mtn_momo(PUSH_RAIL)]
#[case::orange_money(REDIRECT_RAIL)]
#[tokio::test]
async fn a_callback_settles_the_charge_before_the_ladders_next_rung_would_have_fired(
    #[case] rail: &str,
) -> anyhow::Result<()> {
    let harness = harness().await?;
    let (intent_id, charge) = confirmed_charge(&harness, rail).await?;

    // The join between three crates: the URL the rail was handed is the URL
    // this router mounts. Asserted before it is used, so a mismatch reads as
    // a mismatch rather than as a 404 from a POST later on.
    let callback_url = callback_url_the_rail_was_told(&harness, rail).await?;
    assert_eq!(
        callback_url,
        format!("{}/provider/{rail}/callback", harness.base_url),
        "{rail}: the rail was told to call a URL this deployment does not serve"
    );

    point_the_charge_at_the_scenario(&harness.pool, &charge.id).await?;

    // Poll #1: the rail says PENDING, so the job goes back on the ladder.
    let first = harness.step_one().await?;
    assert!(
        matches!(first.disposition, Disposition::Rescheduled(_)),
        "{rail}: the first poll must leave the charge live, got {:?}",
        first.disposition
    );

    let first_rung_at = poll_run_at(&harness.pool, &charge.id).await?;
    let now = time::OffsetDateTime::now_utc();
    assert!(
        first_rung_at - now >= time::Duration::seconds(8),
        "{rail}: the ladder should have parked the poll about {FIRST_RUNG:?} out; run_at is \
         {first_rung_at} and now is {now}"
    );

    // Out to the third rung, which is where a callback is worth something.
    park_the_poll_at_a_later_rung(&harness.pool, &charge.id).await?;
    let parked_at = poll_run_at(&harness.pool, &charge.id).await?;
    let now = time::OffsetDateTime::now_utc();
    assert!(
        parked_at - now >= time::Duration::seconds(25),
        "{rail}: the poll should be parked about {LATER_RUNG:?} out; run_at is {parked_at} \
         and now is {now}"
    );

    // And it really is unreachable: this is the assertion the whole case
    // rests on, because it means nothing else in this process could settle
    // the charge before the rung arrives.
    assert!(
        harness.step().await?.is_none(),
        "{rail}: a job parked at the next rung must not be claimable"
    );

    let started = Instant::now();
    let response = harness
        .http
        .post(&callback_url)
        .header("content-type", "application/json")
        .body(documented_callback_body(rail, SCENARIO_REF))
        .send()
        .await
        .context("POSTing the rail's documented notification")?;
    assert_eq!(
        response.status().as_u16(),
        202,
        "{rail}: an accepted notification is a 202"
    );

    let pulled_to = poll_run_at(&harness.pool, &charge.id).await?;
    assert!(
        pulled_to <= time::OffsetDateTime::now_utc(),
        "{rail}: the callback must make the poll claimable now; run_at is still {pulled_to}"
    );

    // Poll #2: the scenario's second answer settles the charge.
    let second = harness.step_one().await?;
    assert!(
        matches!(second.disposition, Disposition::Finished),
        "{rail}: the poll a callback pulled forward must settle the charge, got {:?} ({:?})",
        second.disposition,
        second.error
    );

    let elapsed = started.elapsed();
    assert!(
        elapsed < CALLBACK_BUDGET,
        "{rail}: the callback round trip took {elapsed:?}, which is not decisively less than \
         the {LATER_RUNG:?} the job was parked at"
    );

    let settled = stored_charge(&harness.pool, &intent_id).await?;
    assert_eq!(settled.state, "succeeded", "{rail}");
    assert!(
        settled.provider_txn_id.is_some(),
        "{rail}: the rail's own identifier comes from the authenticated status query"
    );
    assert_eq!(
        intent_status(&harness.pool, &intent_id).await?,
        "succeeded",
        "{rail}"
    );

    harness.shutdown().await;
    Ok(())
}

/// The floor the route enforces **is** the poll ladder's fastest rung.
///
/// `vpay_api` cannot call `vpay_worker::poll_delay`: the dependency runs the
/// other way (`vpay-worker` links `vpay-api`), so the number is written out
/// in `provider_callback` and this suite — which links both — is the join.
/// Without it the two are free to drift, and drift in the wrong direction
/// means either an anonymous caller can accelerate a poll the queue was
/// about to make anyway (floor too small) or a rail's callback stops being
/// worth anything for several rungs (floor too large).
///
/// [`FIRST_RUNG`] is in the assertion as well, so the number is also checked
/// against the ladder `docs/flows/reconciler.md` documents rather than only
/// against the one the code currently returns.
#[test]
fn the_pull_forward_floor_is_the_poll_ladders_first_rung() {
    assert_eq!(
        vpay_api::provider_callback::PULL_FORWARD_FLOOR,
        vpay_worker::poll_delay(0),
        "the callback route's floor has drifted from the ladder's first rung"
    );
    assert_eq!(
        vpay_api::provider_callback::PULL_FORWARD_FLOOR,
        FIRST_RUNG,
        "and from the ladder `docs/flows/reconciler.md` documents"
    );
}

/// A callback about a charge the queue is **already about to ask about**
/// moves nothing — and is still answered `202`.
///
/// This is the abuse bound, asserted where it can fail. The route is
/// unauthenticated, and before Step 8's review every POST naming a live
/// charge turned into one real, authenticated `query_status` within about a
/// second, whatever the queue was already going to do — so a caller who knew
/// one reference could drive rail traffic at their own rate. The floor
/// removes the cheapest version of that: a poll due inside
/// `PULL_FORWARD_FLOOR` is left exactly where the ladder put it.
///
/// One rail rather than both: what is under test is
/// `pull_forward_in_tx`'s guard, which is rail-agnostic by construction —
/// the reference is the only thing that reaches it, and
/// `pull_forward_moves_a_job_past_the_floor_and_leaves_near_leased_parked_and_due_alone`
/// (in `vpay-db`) is the exhaustive statement of the same guard. What is
/// added here is that the *route* passes the floor at all, which no
/// single-crate test can see.
#[tokio::test]
async fn a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run() -> anyhow::Result<()>
{
    let harness = harness().await?;
    let (_intent_id, charge) = confirmed_charge(&harness, PUSH_RAIL).await?;
    point_the_charge_at_the_scenario(&harness.pool, &charge.id).await?;

    // Poll #1: PENDING, so the ladder puts the job on its first rung — which
    // is the floor.
    let first = harness.step_one().await?;
    assert!(
        matches!(first.disposition, Disposition::Rescheduled(_)),
        "the first poll must leave the charge live, got {:?}",
        first.disposition
    );
    let parked_at = poll_run_at(&harness.pool, &charge.id).await?;

    let callback_url = callback_url_the_rail_was_told(&harness, PUSH_RAIL).await?;
    let response = harness
        .http
        .post(&callback_url)
        .header("content-type", "application/json")
        .body(documented_callback_body(PUSH_RAIL, SCENARIO_REF))
        .send()
        .await
        .context("POSTing the rail's documented notification")?;
    assert_eq!(
        response.status().as_u16(),
        202,
        "a rail must still be told its notification was accepted; answering anything else \
         buys a retry loop"
    );

    assert_eq!(
        poll_run_at(&harness.pool, &charge.id).await?,
        parked_at,
        "a poll already due inside the floor must be left where the ladder put it"
    );
    assert!(
        harness.step().await?.is_none(),
        "the callback made claimable work out of a charge the ladder was about to ask \
         about anyway — which is one rail request per POST, at the caller's rate"
    );

    harness.shutdown().await;
    Ok(())
}

/// **Callbacks are hints** (AGENTS.md), asserted where it can fail: the
/// notification says `SUCCESSFUL`/`SUCCESS`, and nothing about the charge or
/// the intent moves until the *authenticated* status query answers.
///
/// The `ref_extra` half is the sharper one. Orange's `parse_callback` returns
/// a `notif_token` and a `pay_token` in `CallbackRef::ref_extra`; this body
/// carries values no rail ever issued for this charge, and a route that
/// merged them onto the row would both corrupt the key material the next
/// status query is addressed by and take rail material from an
/// unauthenticated request.
#[rstest]
#[case::mtn_momo(PUSH_RAIL)]
#[case::orange_money(REDIRECT_RAIL)]
#[tokio::test]
async fn a_callback_writes_no_charge_or_intent_state(#[case] rail: &str) -> anyhow::Result<()> {
    let harness = harness().await?;
    let (intent_id, charge) = confirmed_charge(&harness, rail).await?;
    let before_status = intent_status(&harness.pool, &intent_id).await?;

    let callback_url = callback_url_the_rail_was_told(&harness, rail).await?;
    let response = harness
        .http
        .post(&callback_url)
        .body(documented_callback_body(rail, charge.provider_reference_id))
        .send()
        .await
        .context("POSTing the rail's documented notification")?;
    assert_eq!(response.status().as_u16(), 202);

    let after = stored_charge(&harness.pool, &intent_id).await?;
    assert_eq!(
        after.state, charge.state,
        "{rail}: a callback carrying a terminal status must not move the charge"
    );
    assert_eq!(
        after.provider_txn_id, None,
        "{rail}: only a status query names the rail's transaction"
    );
    assert_eq!(
        after.provider_ref_extra, charge.provider_ref_extra,
        "{rail}: rail key material from an unauthenticated request must never reach the row"
    );
    assert_eq!(
        intent_status(&harness.pool, &intent_id).await?,
        before_status,
        "{rail}: a callback must not move the intent either"
    );

    harness.shutdown().await;
    Ok(())
}

/// Refusal 1 of 3: a rail code this deployment links no adapter for is a
/// `404`, and it enqueues nothing.
///
/// The body is a *valid* MTN notification naming a real charge, so the only
/// thing that can produce the 404 is the code — a route that resolved the
/// adapter after parsing, or not at all, would have had everything it needed
/// to act.
#[tokio::test]
async fn an_unlinked_rail_code_is_a_404_and_enqueues_nothing() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (_intent_id, charge) = confirmed_charge(&harness, PUSH_RAIL).await?;
    let before = queue_snapshot(&harness.pool).await?;

    for code in ["wave", "mtn-momo", "MTN_MOMO", "orange_money2"] {
        let response = harness
            .http
            .post(format!("{}/provider/{code}/callback", harness.base_url))
            .body(documented_callback_body(
                PUSH_RAIL,
                charge.provider_reference_id,
            ))
            .send()
            .await
            .context("POSTing to an unlinked rail code")?;
        assert_eq!(
            response.status().as_u16(),
            404,
            "{code}: an unlinked rail code is not a route"
        );
    }

    assert_eq!(
        queue_snapshot(&harness.pool).await?,
        before,
        "a 404 must leave the queue exactly as it found it"
    );

    harness.shutdown().await;
    Ok(())
}

/// Refusal 2 of 3, and **the guard-failure proof**: a body this rail's
/// adapter refuses is a `400`, and nothing is enqueued or moved.
///
/// # What makes it decisive, and how it was checked
///
/// The Orange case posts `{"order_id": "<a real reference>", "status":
/// "SUCCESS"}` — a body that names a genuine charge of this deployment's and
/// is refused only because it carries no `notif_token`
/// (`vpay_adapter_orange_money::Adapter::parse_callback`: "refusing to parse
/// an unverifiable callback"). So a route that acted on an unparseable body
/// would have had a charge to act on, which is exactly the failure this test
/// has to be able to see.
///
/// Verified by hand, and recorded in `docs/plans/step8-notes/lane-c.md`:
/// making `notif_token` optional in that adapter makes the body parse, the
/// route enqueues and pulls forward, and the `run_at` assertion below fails.
/// Restored afterwards. The MTN half cannot be made decisive the same way and
/// says so — see [`refused_callback_body`].
#[rstest]
#[case::mtn_momo(PUSH_RAIL)]
#[case::orange_money(REDIRECT_RAIL)]
#[tokio::test]
async fn an_unparseable_callback_body_is_refused_and_moves_no_job(
    #[case] rail: &str,
) -> anyhow::Result<()> {
    let harness = harness().await?;
    let (_intent_id, charge) = confirmed_charge(&harness, rail).await?;

    // Put the poll job on the ladder, so "nothing moved" is a statement about
    // a `run_at` that a pull-forward *would* have changed. Straight from the
    // confirm the job sits at `now()`, where a pull-forward is a no-op and
    // this assertion would hold for the wrong reason.
    point_the_charge_at_the_scenario(&harness.pool, &charge.id).await?;
    harness.step_one().await?;
    let before = queue_snapshot(&harness.pool).await?;
    assert!(
        poll_run_at(&harness.pool, &charge.id).await? > time::OffsetDateTime::now_utc(),
        "{rail}: the poll must be parked in the future for this assertion to mean anything"
    );

    let callback_url = callback_url_the_rail_was_told(&harness, rail).await?;
    for body in [
        refused_callback_body(rail, SCENARIO_REF),
        "not json at all".to_owned(),
        String::new(),
    ] {
        let response = harness
            .http
            .post(&callback_url)
            .body(body.clone())
            .send()
            .await
            .context("POSTing a body the rail's adapter refuses")?;
        assert_eq!(
            response.status().as_u16(),
            400,
            "{rail}: a body that is not a notification from this rail is a 400; body was {body}"
        );
    }

    assert_eq!(
        queue_snapshot(&harness.pool).await?,
        before,
        "{rail}: a refused body must not enqueue a job and must not move one"
    );
    assert!(
        harness.step().await?.is_none(),
        "{rail}: and the parked poll must still be unclaimable"
    );

    harness.shutdown().await;
    Ok(())
}

/// Refusal 3 of 3: a well-formed notification naming a reference this
/// deployment never generated is accepted anyway — `202`, no job, one `info`
/// line.
///
/// A `404` here would buy a retry loop no amount of retrying can fix (both
/// rails retry a non-2xx) and would turn this route into an oracle for which
/// references exist. See the module docs of `vpay_api::provider_callback`.
///
/// The reference is also posted to the *other* rail's callback path, which
/// must answer the same 202: `Charges::get_by_provider_reference` is scoped
/// by `provider_code`, so a charge on MTN is not reachable through Orange's
/// path even by someone who knows its reference.
#[rstest]
#[case::mtn_momo(PUSH_RAIL, REDIRECT_RAIL)]
#[case::orange_money(REDIRECT_RAIL, PUSH_RAIL)]
#[tokio::test]
async fn a_reference_this_deployment_has_no_charge_for_is_accepted_anyway(
    #[case] rail: &str,
    #[case] other_rail: &str,
) -> anyhow::Result<()> {
    let harness = harness().await?;
    let (_intent_id, charge) = confirmed_charge(&harness, rail).await?;
    let before = queue_snapshot(&harness.pool).await?;

    let callback_url = callback_url_the_rail_was_told(&harness, rail).await?;
    let unknown = Uuid::new_v4();
    let response = harness
        .http
        .post(&callback_url)
        .body(documented_callback_body(rail, unknown))
        .send()
        .await
        .context("POSTing a notification for an unknown reference")?;
    assert_eq!(
        response.status().as_u16(),
        202,
        "{rail}: a rail must not be told to retry a reference we will never know"
    );

    // The same charge's *real* reference, through the other rail's path.
    let crossed = harness
        .http
        .post(format!(
            "{}/provider/{other_rail}/callback",
            harness.base_url
        ))
        .body(documented_callback_body(
            other_rail,
            charge.provider_reference_id,
        ))
        .send()
        .await
        .context("POSTing one rail's reference to the other rail's path")?;
    assert_eq!(
        crossed.status().as_u16(),
        202,
        "{other_rail}: a charge on another rail is not reachable here, and saying so plainly \
         would be an oracle"
    );

    assert_eq!(
        queue_snapshot(&harness.pool).await?,
        before,
        "neither an unknown reference nor another rail's reference may enqueue anything"
    );

    harness.shutdown().await;
    Ok(())
}
