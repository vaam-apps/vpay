//! A merchant's confirm, settled by the worker — the whole path, in one
//! process, with nothing simulated.
//!
//! `confirm_rails.rs` ends where Step 3 ended: the rail has the request and
//! the intent is `processing`. This file is what Step 4 added — the part where
//! the payer approves (or does not) and vpay finds out.
//!
//! ```text
//!   vpay_sdk::Client                     (the artefact a merchant integrates)
//!     -> POST /v1/payment_intents/{id}/confirm   (the real axum router)
//!        -> charge row + poll job, one transaction
//!        -> MTN adapter -> HTTP -> WireMock container
//!   vpay_worker::run_loop                (the loop vpay-worker-bin runs)
//!     -> claim -> query_status -> HTTP -> WireMock container
//!     -> settlement transaction: charge, intent, events
//!   vpay_sdk::Client
//!     -> GET /v1/payment_intents/{id}  until  status == succeeded
//! ```
//!
//! # How a real confirm reaches a PENDING → SUCCESSFUL walk
//!
//! It cannot be steered by reference. `confirm` mints `provider_reference_id`
//! with `Uuid::new_v4()` before committing the charge, and a seam to fix it
//! from a test would be a code path that exists only outside production —
//! AGENTS.md's first rule. MTN's status query is a `GET` carrying no body, so
//! it cannot be steered by a payer either.
//!
//! What *is* steerable is the submit: the MSISDN comes from the merchant's own
//! request body. So [`SETTLING_MSISDN`] enters a WireMock **scenario** on the
//! POST (`requesttopay-scenario.json`, `mtn-e2e-poll`, priority 5) and the
//! scenario's state — not the request — decides what the following status
//! queries answer. The same technique, and the same documentation-MSISDN
//! block, as the payer-decline mapping `confirm_rails.rs` uses.
//!
//! # The decline case is driven differently, and says so
//!
//! A decline needs the rail to answer `FAILED / NOT_ENOUGH_FUNDS`, which
//! `requesttopay-status.json` keys on the reference `…0f01`. Rather than add a
//! second MSISDN-keyed scenario for one case, that test performs a real
//! confirm and then rewrites `charges.provider_reference_id` to `…0f01`
//! *before the worker polls*. That is a test writing stored state — the same
//! thing `worker_recovery.rs` does throughout — and not a code seam: the
//! reference is opaque to everything except the rail, so the test is saying
//! "suppose the rail had assigned this charge the reference it declines".
//!
//! # No test doubles
//!
//! Real Postgres, real WireMock (the shared
//! `backends/tests/conformance/wiremock` tree), the shipping adapters, the
//! shipping router, the shipping SDK and the shipping loop.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;
use vpay_api::op::MerchantOp;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_api::resource_auth::{JwtValidator, MerchantJwtValidator, Surface};
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};
use vpay_db::Repositories;
use vpay_sdk::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, Credentials, IntentStatus,
    PaymentMethodType, RequestOptions,
};
use vpay_worker::{Adapters, Drain, RailConfigs, RecoveryPolicy};

mod support;

use support::{
    confirmed_intent, crashed_charge, ensure_crypto_provider_installed, generate_key,
    merchant_client, migrated_postgres, rail_configs, reconcile_from_config, router_deps,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const RAIL: &str = "mtn_momo";
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

/// The documentation MSISDN that enters `requesttopay-scenario.json`'s
/// `mtn-e2e-poll` scenario on the POST, making the two status queries that
/// follow answer `PENDING` then `SUCCESSFUL`.
///
/// The same value `examples/merchant-demo` uses, and for the same reason: it
/// is the only way an end-to-end confirm can reach a settling walk.
const SETTLING_MSISDN: &str = "237600000ce0";

/// A documentation MSISDN nothing stubs, so a confirm with it falls through to
/// the catch-all 202 and leaves the `mtn-e2e-poll` scenario untouched.
const PLAIN_MSISDN: &str = "237670000000";

/// The reference `requesttopay-status.json` answers
/// `FAILED / NOT_ENOUGH_FUNDS` to — `insufficient_funds` in the taxonomy.
const DECLINED_REF: Uuid = Uuid::from_u128(0x0f01);

/// This test binary's Prometheus recorder, installed on first use.
///
/// A `LazyLock` rather than a per-test install because
/// `metrics::set_global_recorder` can only ever succeed once in a process.
/// Under `cargo nextest` — which is what `just test-rust` and CI run — every
/// test gets its own process, so "once per process" is also "once per test"
/// and the counters below start at zero for the test that reads them. Under
/// a plain `cargo test`, which shares one process across the binary, the
/// exact-count assertion in
/// `a_decline_after_submission_returns_the_intent_to_requires_payment_method`
/// would be reading a total across whatever else had run first; that is a
/// property of the runner, stated here rather than papered over with a
/// `contains`-style assertion that would also pass against a counter that
/// never moved.
static METRICS: LazyLock<PrometheusHandle> = LazyLock::new(|| {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::set_global_recorder(recorder).expect("this test binary installs exactly one recorder");
    vpay_core::metrics::describe_all();
    handle
});

/// Serves the **shipping** observability router
/// (`vpay_api::observability`, the same function both `main.rs` files call)
/// on an ephemeral port, rendering [`METRICS`].
///
/// Deliberately the real router over a real socket rather than
/// `PrometheusHandle::render()` read directly: the thing under test is what
/// a Prometheus scrape of a vpay pod would actually receive, and that
/// includes the route being mounted and the handler returning it.
async fn serve_metrics() -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let handle = METRICS.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding the observability listener")?;
    let addr = listener.local_addr().context("reading the bound address")?;
    let task = tokio::spawn(async move {
        let _ = vpay_api::observability::serve(
            listener,
            move || handle.render(),
            std::future::pending::<()>(),
        )
        .await;
    });
    Ok((addr, task))
}

/// The reference whose status query takes two seconds
/// (`fixedDelayMilliseconds: 2000`). What the drain case blocks its handlers
/// on.
const SLOW_REF: Uuid = Uuid::from_u128(0x0560);

/// The `financialTransactionId` the `mtn-e2e-poll` scenario returns.
/// Deliberately distinct from every other stub's, so an assertion on
/// `charges.provider_txn_id` cannot pass by reading the wrong mapping's
/// answer.
const SETTLED_TXN_ID: &str = "e2e-1234567892";

/// How long the merchant-facing poll waits for `succeeded`.
///
/// The ladder's first rung is ten seconds (`vpay_worker::poll_delay`) and the
/// scenario answers `PENDING` first, so the earliest possible settlement is
/// about eleven seconds in. This is that plus a wide margin for container
/// scheduling, not a guess: it is a *ceiling* on a wait that normally ends
/// well before it.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long [`wait_for_fanout`] waits for the outbox drain.
///
/// The drain reschedules itself every five seconds when the backlog is empty
/// (`vpay_worker::webhooks`), so an event written just after a pass is picked
/// up by the next one. Four times that, for the same container-scheduling
/// margin [`SETTLE_TIMEOUT`] carries.
const FANOUT_TIMEOUT: Duration = Duration::from_secs(20);

fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

// ------------------------------------------------------------------ harness

struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    _mtn: ContainerAsync<GenericImage>,
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    /// The plain `sqlx` pool, for the fixtures that read or force schema
    /// state no repository method owns.
    pool: PgPool,
    base_url: String,
    pem_a: String,
    adapters: Arc<Adapters>,
    rails: Arc<RailConfigs>,
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

    /// Runs the loop until `shutdown` fires — the same function
    /// `vpay-worker-bin`'s `main` calls, with the same arguments it builds.
    async fn run_loop(
        &self,
        policy: RecoveryPolicy,
        concurrency: usize,
        grace: Duration,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> vpay_worker::LoopReport {
        vpay_worker::run_loop(
            Arc::clone(&self.repositories),
            Arc::clone(&self.adapters),
            Arc::clone(&self.rails),
            policy,
            support::no_webhook_endpoints(),
            support::default_egress_policy(),
            concurrency,
            grace,
            "worker-e2e-suite".to_owned(),
            shutdown,
        )
        .await
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// One rail, on XAF, `livemode: false` — the shape `config/application.yml`
/// has.
fn config_with(base_url: &str, mtn_url: &str, jwks_a: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "worker-e2e".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![ProviderHost {
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
        }],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_ascii_uppercase(),
            exponent: 0,
        }],
        merchant_clients: vec![merchant_client(CLIENT_A, MERCHANT_A, jwks_a)],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
    }
}

/// Postgres, the MTN stub, and a vpay server booted in `vpay-server`'s own
/// order: announce the signing key, run boot step 4, bind, serve.
async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (postgres, repositories, pool) = migrated_postgres().await?;
    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let mtn_url = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .context("binding an ephemeral loopback port")?;
    let bound = listener.local_addr().context("reading the bound port")?;
    let base_url = format!("http://{bound}");
    let issuer = format!("{base_url}/v1/oauth");

    let signing_key =
        LoadedSigningKey::from_pem(&server_pem, &issuer).context("loading the signing key")?;
    signing_key
        .ensure_active_in_database(repositories.as_ref())
        .await
        .context("announcing the signing key in oauth_signing_keys")?;

    let config = config_with(&base_url, &mtn_url, jwks_a);
    reconcile_from_config(repositories.as_ref(), &config).await?;

    let merchant_op = Arc::new(MerchantOp::new(
        &config,
        signing_key.clone(),
        Arc::clone(&repositories),
    ));
    let merchant_validator = MerchantJwtValidator(
        JwtValidator::new(
            format!("{base_url}/v1/oauth/jwks.json"),
            Duration::from_secs(300),
            merchant_op.issuer(),
            Surface::Merchant,
        )
        .expect("the vendored-roots JWKS client builds"),
    );

    let deps = router_deps(
        Arc::clone(&repositories),
        merchant_op,
        merchant_validator,
        &config,
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, vpay_api::router(deps)).await;
    });

    Ok(Harness {
        _postgres: postgres,
        _mtn: mtn,
        server,
        repositories,
        pool,
        base_url,
        pem_a,
        adapters: Arc::new(support::adapters_by_code()),
        rails: Arc::new(rail_configs(&config)),
    })
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

// ------------------------------------------------------------------ reading

#[derive(Debug, sqlx::FromRow)]
struct StoredCharge {
    id: String,
    state: String,
    provider_txn_id: Option<String>,
    failure_code: Option<String>,
}

async fn stored_charge(pool: &PgPool, payment_intent_id: &str) -> anyhow::Result<StoredCharge> {
    sqlx::query_as::<_, StoredCharge>(
        "SELECT id, state::TEXT AS state, provider_txn_id, failure_code::TEXT AS failure_code \
         FROM charges WHERE payment_intent_id = $1",
    )
    .bind(payment_intent_id)
    .fetch_one(pool)
    .await
    .context("reading the charge")
}

/// `payment_intents.amount_received` — the column that is **not** on the wire.
///
/// `PaymentIntentObject` does not carry it (`vpay_api::model`), so a merchant
/// cannot see it and this suite has to read the row. That is worth stating
/// rather than hiding: the settlement transaction writes it
/// (`vpay_db::settlement::apply_succeeded`), `docs/runbooks` reconcile against
/// it, and nothing on `/v1` exposes it yet.
async fn stored_intent(pool: &PgPool, id: &str) -> anyhow::Result<(String, i64, Option<String>)> {
    sqlx::query_as::<_, (String, i64, Option<String>)>(
        "SELECT status::TEXT, amount_received, last_payment_error_code::TEXT \
         FROM payment_intents WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .context("reading the intent")
}

async fn events_for(pool: &PgPool, object_id: &str) -> anyhow::Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT type::TEXT, fanout_state::TEXT FROM events WHERE object_id = $1 ORDER BY seq",
    )
    .bind(object_id)
    .fetch_all(pool)
    .await
    .context("reading the events table")
}

/// Just the event types, in `seq` order.
///
/// Separate from [`events_for`] because `fanout_state` stopped being a
/// constant the moment the loop grew a fan-out drain (Step 5): the same
/// `run_loop` that settles the charge also drains the outbox, so an assertion
/// on the pair is a race between two of this loop's own jobs. What each of
/// these cases is about is that the settlement emitted *one* event of the
/// *right type*; the state that follows is [`wait_for_fanout`]'s assertion,
/// and the deliveries it produces are `webhooks.rs`'s.
async fn event_types_for(pool: &PgPool, object_id: &str) -> anyhow::Result<Vec<String>> {
    Ok(events_for(pool, object_id)
        .await?
        .into_iter()
        .map(|(kind, _)| kind)
        .collect())
}

/// Waits for the fan-out drain to mark every event on `object_id` `done`.
///
/// This is the seam between Step 4's settlement and Step 5's outbox, and the
/// only place this suite asserts it: `run_loop` seeds `fanout:events`
/// alongside `sweep:expired` and `scan:live`, so an event this loop wrote
/// must be drained by this same loop without anything else running. A
/// deployment where that seeding was dropped settles payments and tells no
/// merchant about any of them — and every other assertion in this file would
/// still pass.
///
/// This harness registers **no** endpoints, so `done` here means "fanned out
/// to zero endpoints", which is the documented behaviour for a merchant who
/// has configured none. The delivery half is proven in `webhooks.rs` against
/// a real receiver.
async fn wait_for_fanout(pool: &PgPool, object_id: &str) -> anyhow::Result<()> {
    let deadline = Instant::now() + FANOUT_TIMEOUT;
    loop {
        let states: Vec<String> = events_for(pool, object_id)
            .await?
            .into_iter()
            .map(|(_, state)| state)
            .collect();
        if !states.is_empty() && states.iter().all(|state| state == "done") {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "the fan-out drain did not mark the events on {object_id} done within \
             {FANOUT_TIMEOUT:?}; states were {states:?}. The usual cause is that \
             `run_loop` no longer seeds the `fanout:events` singleton."
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The `events.data` snapshot: the wire object as it stood at emit time.
///
/// Read as JSON rather than as columns because what matters is the *body*.
/// `data` holds the `payment_intent` object itself (migration 0018: "the full
/// wire object as it was at emit time"), and Step 5 delivers it verbatim to a
/// merchant's Stripe-shaped handler — so an object snapshotted before the
/// transition it announces would tell every merchant that a succeeded payment
/// is still `processing`, and nothing would ever correct it.
async fn event_data(pool: &PgPool, object_id: &str) -> anyhow::Result<Value> {
    sqlx::query_scalar::<_, Value>(
        "SELECT data FROM events WHERE object_id = $1 ORDER BY seq LIMIT 1",
    )
    .bind(object_id)
    .fetch_one(pool)
    .await
    .context("reading the event's data snapshot")
}

/// The `jobs` row a confirm left behind, or `None` once the worker has
/// finished it.
async fn poll_job(pool: &PgPool, charge_id: &str) -> anyhow::Result<Option<(String, String)>> {
    sqlx::query_as::<_, (String, String)>("SELECT kind, dedupe_key FROM jobs WHERE dedupe_key = $1")
        .bind(vpay_worker::jobs::poll_dedupe_key(charge_id))
        .fetch_optional(pool)
        .await
        .context("reading the poll job")
}

/// Polls `GET /v1/payment_intents/{id}` through the SDK until it is not
/// `processing`, exactly as a merchant integration would.
///
/// Deliberately through the merchant's own client and not the database: what
/// this suite claims is that a *merchant* sees the payment settle, and a read
/// through the same pool the worker wrote to would prove only that vpay agrees
/// with itself.
async fn wait_for_settlement(
    client: &vpay_sdk::Client,
    pool: &PgPool,
    intent_id: &str,
) -> anyhow::Result<vpay_sdk::PaymentIntent> {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    loop {
        let intent = client
            .payment_intents()
            .retrieve(intent_id)
            .await
            .map_err(|error| anyhow::anyhow!("retrieving the intent: {error}"))?;
        if intent.status != IntentStatus::Processing {
            return Ok(intent);
        }
        if Instant::now() >= deadline {
            // The `jobs` row is where a stalled ladder says why: `last_error`
            // carries the failure that rescheduled it, and `attempts` says
            // whether anything claimed it at all. Without this the timeout
            // reads as "the worker did nothing", which is only one of the
            // several things it can mean.
            let queued: Vec<(String, i32, Option<String>)> = sqlx::query_as(
                "SELECT dedupe_key, attempts, last_error FROM jobs ORDER BY dedupe_key",
            )
            .fetch_all(pool)
            .await
            .unwrap_or_default();
            anyhow::bail!(
                "the intent was still `processing` after {SETTLE_TIMEOUT:?}; nothing drove \
                 the charge to a terminal state. jobs: {queued:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

// -------------------------------------------------------------------- cases

/// A merchant confirms, the payer approves, and the merchant's next `GET`
/// says `succeeded`.
///
/// Everything between those two sentences is the worker. The assertions are
/// the whole of what a settlement is supposed to commit, and every one of them
/// is a row the previous step could not have written:
///
/// * the intent, over the wire, is `succeeded` — what the merchant sees;
/// * `amount_received == amount` — the column nothing wrote before Step 4;
/// * the charge is `succeeded` and carries the rail's own `provider_txn_id` —
///   migration 0021's column, the field `docs/runbooks/unresolved-charges.md`
///   reconciles by name;
/// * exactly one `events` row, `payment_intent.succeeded`, `fanout_state`
///   still `pending` — Step 5 delivers it; this step only has to emit it, once,
///   inside the same transaction as the rows above;
/// * the job is gone, because the work is done.
///
/// It also pins the *cross-crate* spelling the confirm path duplicates: the
/// row `vpay_api`'s `insert_charge` wrote must carry the `kind` and
/// `dedupe_key` `vpay_worker::jobs` names, or the worker would never claim it
/// and this test would time out instead of settling.
///
/// And it is the regression test for a bug this suite found: `run_loop` used
/// to start its grace clock at *boot* rather than when the shutdown signal
/// arrived, so it aborted every in-flight task `grace` seconds after starting.
/// The settlement here lands at about eleven seconds and the grace period
/// below is ten, so a loop with that bug abandons the poll ladder one rung
/// short and this case fails — which is exactly how it was found.
#[tokio::test]
async fn a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it() -> anyhow::Result<()>
{
    let h = harness().await?;
    let client = h.client();

    // Installed *before* the create, because the charge's whole state
    // machine is what the assertions at the end are about: the confirm
    // opens the charge (`` -> submitting`) and submits it
    // (`submitting -> submitted`) inside vpay-api, and only the last edge
    // belongs to the worker. `metrics::counter!` with no recorder is a
    // silent no-op, so a recorder installed later would render a document
    // missing exactly the half this test is uniquely able to see.
    let (metrics_addr, metrics_task) = serve_metrics().await?;

    let created = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .map_err(|e| anyhow::anyhow!("creating the intent: {e}"))?;
    let confirmed = client
        .payment_intents()
        .confirm(
            &created.id,
            ConfirmPaymentIntentParams::mtn_momo(SETTLING_MSISDN),
            RequestOptions::new(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("confirming the intent: {e}"))?;
    assert_eq!(
        confirmed.status,
        IntentStatus::Processing,
        "the rail accepted the charge; a push confirm has exactly one success state"
    );

    // Read before the worker starts, because the worker deletes it.
    let charge_before = stored_charge(&h.pool, &created.id).await?;
    assert_eq!(charge_before.state, "submitted");
    let job = poll_job(&h.pool, &charge_before.id).await?.context(
        "the confirm committed a charge with no poll job; all three of \
                  crash-safety.md's kill points would leave that charge undriven",
    )?;
    assert_eq!(
        job,
        (
            vpay_worker::JobKind::PollCharge.as_wire_str().to_owned(),
            vpay_worker::jobs::poll_dedupe_key(&charge_before.id)
        ),
        "vpay-api spells the job kind and dedupe key by hand (it cannot depend on \
         vpay-worker without a cycle); this is where the two spellings are compared"
    );

    // The loop the binary runs, stopped once the merchant has seen the answer.
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let repositories = Arc::clone(&h.repositories);
    let adapters = Arc::clone(&h.adapters);
    let rails = Arc::clone(&h.rails);
    let worker = tokio::spawn(async move {
        vpay_worker::run_loop(
            repositories,
            adapters,
            rails,
            RecoveryPolicy::default(),
            support::no_webhook_endpoints(),
            support::default_egress_policy(),
            2,
            Duration::from_secs(10),
            "worker-e2e-suite".to_owned(),
            async move {
                let _ = stopped.await;
            },
        )
        .await
    });

    let settled = wait_for_settlement(&client, &h.pool, &created.id).await?;
    assert_eq!(
        settled.status,
        IntentStatus::Succeeded,
        "the rail reported the payment as made; the merchant must see `succeeded`"
    );
    assert_eq!(settled.amount, AMOUNT);

    let (status, amount_received, _) = stored_intent(&h.pool, &created.id).await?;
    assert_eq!(status, "succeeded");
    assert_eq!(
        amount_received, AMOUNT,
        "amount_received must be the amount that settled; the column existed since \
         migration 0003 and nothing wrote it before this step"
    );

    let charge_after = stored_charge(&h.pool, &created.id).await?;
    assert_eq!(charge_after.state, "succeeded");
    assert_eq!(
        charge_after.provider_txn_id.as_deref(),
        Some(SETTLED_TXN_ID),
        "the rail's own identifier for the money movement must be recorded, and it must be \
         the one the e2e scenario returned rather than another stub's"
    );

    assert_eq!(
        event_types_for(&h.pool, &created.id).await?,
        vec!["payment_intent.succeeded".to_owned()],
        "exactly one event, emitted in the settlement transaction"
    );
    // And the loop that settled it drains its own outbox — see
    // `wait_for_fanout` for why this is asserted here and not in `webhooks.rs`.
    wait_for_fanout(&h.pool, &created.id).await?;

    // The body, not just the type. Step 5 delivers this object verbatim, so a
    // snapshot taken *before* the transition — the natural mistake, since
    // `apply_succeeded` takes `event_data` as an input and cannot render it
    // from its own result — would ship `"status": "processing"` inside a
    // `payment_intent.succeeded` webhook to every merchant, forever, with
    // nothing to correct it.
    let data = event_data(&h.pool, &created.id).await?;
    assert_eq!(
        data.get("status").and_then(Value::as_str),
        Some("succeeded"),
        "the event's object must be the post-transition snapshot; got {data}"
    );
    assert_eq!(
        data.get("object").and_then(Value::as_str),
        Some("payment_intent"),
        "the snapshot must be the wire object a merchant's SDK switches on, not a \
         hand-rolled body: {data}"
    );
    assert_eq!(
        data.get("id").and_then(Value::as_str),
        Some(created.id.as_str()),
        "and it must be *this* intent"
    );
    assert_eq!(
        data.get("last_payment_error"),
        Some(&Value::Null),
        "a succeeded payment carries no error; the field is present-and-null because \
         Stripe-shaped objects always carry every documented key (vpay_api::model)"
    );

    assert!(
        poll_job(&h.pool, &charge_before.id).await?.is_none(),
        "a settled charge's poll job must be deleted, not left to run forever"
    );

    let _ = stop.send(());
    let report = worker.await.context("the worker task panicked")?;
    assert_eq!(
        report.drain,
        Drain::Clean,
        "the loop had nothing in flight and must drain cleanly"
    );
    assert_eq!(
        report.released, 0,
        "a clean drain leaves no lease to release"
    );

    // The metrics half, and the only place in this workspace where the two
    // rail-facing series are asserted against a *real* adapter speaking real
    // HTTP to a real container. `vpay_provider::measured`'s unit tests prove
    // the decorator's labels; this proves the shipping path produces them —
    // that `adapters_by_code` really wraps, that the MTN adapter really
    // reaches the stub, and that a settlement really moves the charge.
    let scrape = reqwest::get(format!("http://{metrics_addr}/metrics"))
        .await
        .context("scraping /metrics off the observability listener")?
        .text()
        .await
        .context("reading the scrape body")?;

    // At least one successful status query: the ladder polls until the stub
    // answers SUCCESSFUL, and how many rungs that takes is the stub's
    // business, not this assertion's.
    assert!(
        scrape.lines().any(|line| {
            line.starts_with(concat!(
                r#"vpay_provider_requests_total{provider="mtn_momo","#,
                r#"operation="query_status",error_kind=""}"#
            ))
        }),
        "the MTN adapter's status queries must be counted through \
         vpay_provider::Measured — is adapters_by_code still wrapping?\n{scrape}"
    );
    assert!(
        scrape.contains(concat!(
            r#"vpay_provider_requests_total{provider="mtn_momo","#,
            r#"operation="submit",error_kind=""} 1"#
        )),
        "exactly one submit reached the rail, and the confirm is what made it:\n{scrape}"
    );

    // **The charge's whole walk, edge by edge, each exactly once.** This is
    // the assertion that says the `from` label follows the row rather than
    // the caller's intention: the last two edges are `submitted → pending`
    // and `pending → succeeded`, because the `mtn-e2e-poll` scenario answers
    // `PENDING` on the first status query and `SUCCESSFUL` on the second. A
    // `from` taken from what the settlement *meant* to move would have read
    // `submitted → succeeded` and this list would be wrong in a way no
    // unit test can see — `vpay_db::settlement`'s previous-state sub-select
    // is what makes it right.
    //
    // Four boundaries wrote these four rows: `vpay-api`'s create, its
    // confirm, the worker's ladder rung, and the settlement transaction.
    for edge in [
        r#"vpay_charge_transitions_total{provider="mtn_momo",from="",to="submitting"} 1"#,
        r#"vpay_charge_transitions_total{provider="mtn_momo",from="submitting",to="submitted"} 1"#,
        r#"vpay_charge_transitions_total{provider="mtn_momo",from="submitted",to="pending"} 1"#,
        r#"vpay_charge_transitions_total{provider="mtn_momo",from="pending",to="succeeded"} 1"#,
    ] {
        assert!(
            scrape.contains(edge),
            "missing charge transition `{edge}`; the settled edge is the one \
             vpay_db::settlement's RETURNING sub-select supplies `from` for:\n{scrape}"
        );
    }

    // The confirm went through the real router, so the HTTP middleware is on
    // this scrape too — labelled with the route *pattern*, never the id.
    assert!(
        scrape.contains(concat!(
            r#"vpay_http_requests_total{route="/v1/payment_intents/{id}/confirm","#,
            r#"method="POST",status="200"} 1"#
        )),
        "the confirm must be counted under its route pattern:\n{scrape}"
    );
    assert!(
        !scrape.contains(&created.id),
        "a payment intent id must never reach a metric label:\n{scrape}"
    );

    metrics_task.abort();
    h.shutdown().await;
    Ok(())
}

/// A decline the rail reports *after* it accepted the submit.
///
/// This is the transition `payment-lifecycle.md` describes and that nothing
/// implemented before Step 4: the charge was submitted successfully, the payer
/// then had no balance, and the intent goes **back** to
/// `requires_payment_method` carrying `last_payment_error` — a status a
/// merchant can act on, rather than an intent stuck in `processing` forever.
///
/// The confirm is real; only the reference is rewritten, so that the rail's
/// reference-keyed decline stub answers. See this file's header for why that
/// is stored state and not a code seam.
#[tokio::test]
async fn a_decline_after_submission_returns_the_intent_to_requires_payment_method()
-> anyhow::Result<()> {
    let h = harness().await?;
    let client = h.client();

    let created = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .map_err(|e| anyhow::anyhow!("creating the intent: {e}"))?;
    client
        .payment_intents()
        .confirm(
            &created.id,
            ConfirmPaymentIntentParams::mtn_momo(PLAIN_MSISDN),
            RequestOptions::new(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("confirming the intent: {e}"))?;

    let charge = stored_charge(&h.pool, &created.id).await?;
    sqlx::query("UPDATE charges SET provider_reference_id = $2 WHERE id = $1")
        .bind(&charge.id)
        .bind(DECLINED_REF)
        .execute(&h.pool)
        .await
        .context("pointing the charge at the rail's decline stub")?;

    // Installed *before* the job runs: `metrics::counter!` with no recorder
    // is a silent no-op, so a recorder installed afterwards would render an
    // empty document and this test would pass by asserting nothing.
    let (metrics_addr, metrics_task) = serve_metrics().await?;

    // One step of the real loop is all this needs: the rail answers with a
    // terminal status on the first poll.
    let endpoints = support::no_webhook_endpoints();
    let egress = support::default_egress_policy();
    let settled = vpay_worker::run_once(
        h.repositories.as_ref(),
        &h.adapters,
        &h.rails,
        &RecoveryPolicy::default(),
        &vpay_worker::WebhookContext {
            endpoints: &endpoints,
            egress,
        },
        "worker-e2e-suite",
    )
    .await?
    .context("the confirm enqueued no job")?;
    assert_eq!(
        settled.disposition,
        vpay_worker::Disposition::Finished,
        "a declined charge is resolved, so the queue is done with it: a retry is a new \
         PaymentIntent, not a re-run of this job"
    );
    assert!(
        settled.error.is_none(),
        "a rail's decline is an answer, not a job failure: {:?}",
        settled.error
    );

    let after = client
        .payment_intents()
        .retrieve(&created.id)
        .await
        .map_err(|e| anyhow::anyhow!("retrieving the intent: {e}"))?;
    assert_eq!(
        after.status,
        IntentStatus::RequiresPaymentMethod,
        "payment-lifecycle.md: a rail-reported failure returns the intent to a status the \
         merchant can act on"
    );
    let error = after.last_payment_error.context(
        "a declined intent must carry last_payment_error, or the merchant is told \
                  nothing about why",
    )?;
    assert_eq!(error.code, "insufficient_funds");
    assert!(
        !error.message.to_ascii_lowercase().contains("not_enough"),
        "the rail's raw vocabulary belongs in charges.failure_raw, not in a merchant's \
         message (docs/flows/errors.md); got: {}",
        error.message
    );

    let charge_after = stored_charge(&h.pool, &created.id).await?;
    assert_eq!(charge_after.state, "failed");
    assert_eq!(
        charge_after.failure_code.as_deref(),
        Some("insufficient_funds")
    );

    assert_eq!(
        events_for(&h.pool, &created.id).await?,
        vec![(
            "payment_intent.payment_failed".to_owned(),
            "pending".to_owned()
        )],
        "exactly one event, emitted in the settlement transaction, and still awaiting \
         fan-out: a decline is as much a webhook as a success, and an event the settlement \
         wrote as anything but `pending` would never be delivered at all"
    );
    // `fanout_state` is deterministic *here* and nowhere else in this file:
    // this case drives a single `run_once`, which claims exactly one job and
    // never seeds a singleton, so no `fan_out_events` pass can have run. The
    // cases that call `run_loop` do seed one, and there the pair is a race
    // between two of the loop's own jobs — which is why they assert through
    // `event_types_for` and `wait_for_fanout` instead.

    // The metrics half: exactly one job was claimed and exactly one was
    // settled, and a Prometheus scrape of this process says so.
    //
    // This test is where the assertion belongs rather than the settling one
    // above, because `run_once` runs *one* job against a table holding
    // exactly one row — no poll ladder, no seeded singletons — so the counts
    // are `1` and not "at least 1". An assertion that only checked the name
    // appeared would also pass against a counter that was registered and
    // never incremented.
    let scrape = reqwest::get(format!("http://{metrics_addr}/metrics"))
        .await
        .context("scraping /metrics off the observability listener")?
        .text()
        .await
        .context("reading the scrape body")?;

    assert!(
        scrape.contains(r#"vpay_jobs_claimed_total{kind="poll_charge"} 1"#),
        "the claim point in vpay_worker::run_once did not reach the recorder:\n{scrape}"
    );
    assert!(
        scrape.contains(r#"vpay_jobs_completed_total{kind="poll_charge",outcome="terminal"} 1"#),
        "a declined charge is Disposition::Finished, which is the `terminal` outcome label \
         (vpay_core::metrics::job_outcome):\n{scrape}"
    );
    // `describe_all()` ran too, so a scrape carries help text and not just
    // bare series — the thing that makes these names readable in a
    // dashboard nobody wrote yet.
    assert!(
        scrape.contains("# HELP vpay_jobs_claimed_total"),
        "vpay_core::metrics::describe_all() did not reach the recorder:\n{scrape}"
    );
    // The rail-facing half of the same run. The recorder went in after the
    // confirm here (this test rewrites the charge's reference in between),
    // so what it can see is the *worker's* status query and the settlement
    // it caused — which is precisely the decline path, and the `to="failed"`
    // edge the success case above cannot produce.
    assert!(
        scrape.contains(concat!(
            r#"vpay_provider_requests_total{provider="mtn_momo","#,
            r#"operation="query_status",error_kind=""} 1"#
        )),
        "one status query, answered: {scrape}"
    );
    assert!(
        scrape.contains(
            r#"vpay_charge_transitions_total{provider="mtn_momo",from="submitted",to="failed"} 1"#
        ),
        "a rail-reported decline is a charge transition and must be counted with the state \
         the charge came from:\n{scrape}"
    );

    metrics_task.abort();
    h.shutdown().await;
    Ok(())
}

/// A drain that runs out of grace hands every lease back before it exits.
///
/// The load is fifty jobs whose handler blocks on a rail that takes two
/// seconds to answer (`fixedDelayMilliseconds`, the `…0560` stub), against a
/// one-second grace period: the tasks in flight when the signal arrives cannot
/// possibly finish, which is the case this exists for.
///
/// Without `jobs::release_all` those rows stay leased until a reaper frees
/// them. The loop reaps at boot and then every half-lease, so the wait is
/// bounded — but it is still up to one and a half leases of live charges
/// going undriven because a pod was rolled, and if no worker is running at
/// all (a rolling restart between the two) it is unbounded. **That is what
/// the last assertion catches**, and it is the reason the release is on the
/// timed-out path specifically: a clean drain settles every claimed job and so
/// has no lease left over, which is why `released` is zero there.
#[tokio::test]
async fn a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds() -> anyhow::Result<()>
{
    const JOBS: usize = 50;
    const CONCURRENCY: usize = 4;
    const GRACE: Duration = Duration::from_secs(1);

    let h = harness().await?;

    for _ in 0..JOBS {
        let intent = confirmed_intent(
            h.repositories.as_ref(),
            MERCHANT_A,
            RAIL,
            AMOUNT,
            &CURRENCY.to_ascii_uppercase(),
        )
        .await?;
        let charge = crashed_charge(
            h.repositories.as_ref(),
            &intent,
            RAIL,
            SLOW_REF,
            AMOUNT,
            &CURRENCY.to_ascii_uppercase(),
            Some(PLAIN_MSISDN),
        )
        .await?;
        // Past the recovery branch, so the handler goes straight to the slow
        // status query rather than concluding anything from an absent attempt
        // row.
        h.repositories
            .set_live_state(&charge, "submitting", "submitted")
            .await?;
    }

    // Long enough for the first `CONCURRENCY` claims to be in flight against
    // the two-second rail, short enough that the whole case stays quick.
    let shutdown = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let started = Instant::now();
    let report = h
        .run_loop(RecoveryPolicy::default(), CONCURRENCY, GRACE, shutdown)
        .await;
    let elapsed = started.elapsed();

    assert_eq!(
        report.drain,
        Drain::TimedOut,
        "handlers blocked on a two-second rail cannot finish inside a one-second grace; a \
         clean drain here would mean the grace period is not bounding anything"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "the drain must be bounded by the grace period, not by the rail: took {elapsed:?}"
    );
    assert!(
        report.released >= 1,
        "the timed-out drain released nothing, so the jobs it cut off are still leased"
    );

    let leased: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE locked_by IS NOT NULL")
        .fetch_one(&h.pool)
        .await
        .context("counting leased jobs")?;
    assert_eq!(
        leased, 0,
        "{leased} job(s) are still leased by a process that has exited; nothing will claim \
         them until some worker's lease reaper frees them, and every one of them is a live \
         charge"
    );

    // The work itself was not lost: the rows are still there, claimable, with
    // their attempt counters incremented by the claim that was cut off.
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'poll_charge'")
        .fetch_one(&h.pool)
        .await
        .context("counting poll jobs")?;
    assert_eq!(
        queued,
        i64::try_from(JOBS).expect("50 fits in an i64"),
        "a cut-off drain must not delete work"
    );
    h.shutdown().await;
    Ok(())
}
