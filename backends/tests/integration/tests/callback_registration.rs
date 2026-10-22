//! RFC-0013's callback registration pass, against a real Postgres and a
//! WireMock Notch Pay (RFC-0013 § 10, "Integration cases").
//!
//! # What this file proves
//!
//! - The pass creates vpay's endpoint once, and the next pass finds it by its
//!   description and creates nothing.
//! - **Two passes racing on one database make one create.** The stub's empty
//!   list answers after a second, so two passes started together both reach
//!   the list before either creates; the registration lock is what leaves
//!   one of them `locked`. With the lock removed this case makes two creates
//!   — measured by hand on 2026-10-22, recorded in
//!   `docs/status/verification/2026-10-22-rfc-0013-callback-registration.md`.
//! - `check`, from configuration or from `--check`, never writes; `off` asks
//!   nothing; a rail that is told its URL per request is never asked.
//! - The `ensure_callbacks` job, claimed off the real queue, registers the
//!   endpoint and reschedules itself — and reschedules, never dead-letters,
//!   when the keys are refused or the rail cannot be reached.
//!
//! # What it does not
//!
//! - A foreign endpoint at vpay's URL. Notch Pay answers `"url": null` for
//!   every endpoint (2026-10-21), so the adapter cannot see one, and a stub
//!   that echoed URLs would test a Notch Pay that does not exist.
//! - A create whose answer is lost. The adapter never retries a create within
//!   a pass by construction (`vpay-adapter-notchpay/src/callbacks.rs`), and
//!   WireMock cannot drop an answer after recording the request.
//!
//! Every case starts its own WireMock, so each journal and each scenario
//! state belongs to one case.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use serde_json::Value;
use testcontainers::{ContainerAsync, GenericImage};
use vpay_core::Currency;
use vpay_provider::{EndpointState, ProviderAdapter, ProviderConfig};
use vpay_worker::callbacks::{ENSURE_INTERVAL, RETRY_SOON};
use vpay_worker::{
    CallbackPlan, Disposition, JobKind, Outcome, RailCheck, RailConfigs, RailPass, RecoveryPolicy,
    Registration,
};

mod support;

use support::migrated_postgres;

/// The deployment name and callback URL the stub's list answers for — see
/// the "list webhook endpoints: the integration suite's endpoint" mapping in
/// `backends/tests/conformance/wiremock/notchpay/mappings/webhooks.json`.
const DEPLOYMENT: &str = "callbacks-suite";
const CALLBACK_URL: &str = "https://vpay.invalid/provider/notchpay/callback";
const DESCRIPTION: &str = "vpay:callbacks-suite:notchpay:default url:e54195acc071987c";
const ENDPOINT_ID: &str = "ep_test.vpaySuite0000001";

fn mappings_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/wiremock/notchpay")
}

/// A Notch Pay stub and a migrated database, and the two maps the pass reads.
struct Harness {
    repositories: Arc<dyn vpay_db::Repositories>,
    adapters: vpay_worker::Adapters,
    stub: String,
    _wiremock: ContainerAsync<GenericImage>,
    _postgres: ContainerAsync<testcontainers_modules::postgres::Postgres>,
}

impl Harness {
    async fn start() -> anyhow::Result<Self> {
        // `requests` below builds a plain client for WireMock's admin API.
        support::ensure_crypto_provider_installed();
        let (postgres, repositories, _pool) = migrated_postgres().await?;
        let wiremock = vpay_testkit::containers::start_wiremock(&mappings_dir())
            .await
            .context("the Notch Pay stub starts")?;
        let port = wiremock.get_host_port_ipv4(8080).await?;
        let stub = format!("http://127.0.0.1:{port}");

        let http = vpay_provider::http::client_with_timeouts(
            vpay_provider::DEFAULT_CONNECT_TIMEOUT,
            vpay_provider::DEFAULT_REQUEST_TIMEOUT,
        )
        .context("the vendored-roots client builds")?;
        // The real adapters, measured, as `vpay-server` builds them. MTN is
        // here so the pass has a `PerRequest` rail to leave alone.
        let adapters = vpay_api::v1::boot::adapters_by_code(vec![
            Box::new(vpay_adapter_notchpay::Adapter::new(http.clone())) as Box<dyn ProviderAdapter>,
            Box::new(vpay_adapter_mtn_momo::Adapter::new(http)),
        ]);
        Ok(Self {
            repositories,
            adapters,
            stub,
            _wiremock: wiremock,
            _postgres: postgres,
        })
    }

    /// Notch Pay at the stub, with `public_key`; MTN at an address nothing
    /// listens on, so a call to it would fail loudly.
    fn rails(&self, public_key: &str) -> RailConfigs {
        self.rails_at(&self.stub, public_key)
    }

    fn rails_at(&self, base_url: &str, public_key: &str) -> RailConfigs {
        BTreeMap::from([
            (
                "notchpay".to_owned(),
                ProviderConfig {
                    base_url: base_url.to_owned(),
                    callback_url: CALLBACK_URL.to_owned(),
                    currency: Currency::Xaf,
                    settings: BTreeMap::from([("channel".to_owned(), "cm.mobile".to_owned())]),
                    credentials: BTreeMap::from([
                        ("public_key".to_owned(), public_key.to_owned()),
                        (
                            "private_key".to_owned(),
                            "stub-private-key-test_0002".to_owned(),
                        ),
                    ]),
                    connect_timeout: Duration::from_secs(2),
                    request_timeout: Duration::from_secs(10),
                },
            ),
            (
                "mtn_momo".to_owned(),
                ProviderConfig {
                    base_url: "http://127.0.0.1:1".to_owned(),
                    callback_url: "https://vpay.invalid/provider/mtn_momo/callback".to_owned(),
                    currency: Currency::Eur,
                    settings: BTreeMap::new(),
                    credentials: BTreeMap::new(),
                    connect_timeout: Duration::from_secs(2),
                    request_timeout: Duration::from_secs(10),
                },
            ),
        ])
    }

    async fn pass(&self, plan: &CallbackPlan, check_only: bool) -> Vec<RailPass> {
        vpay_worker::run_pass(
            self.repositories.as_ref(),
            &self.adapters,
            &self.rails(VALID_KEY),
            plan,
            check_only,
        )
        .await
    }

    /// How many requests the stub recorded for `method` on `/webhooks`.
    async fn requests(&self, method: &str) -> anyhow::Result<u64> {
        let body: Value = reqwest::Client::new()
            .post(format!("{}/__admin/requests/count", self.stub))
            .json(&serde_json::json!({ "method": method, "urlPath": "/webhooks" }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        body.get("count")
            .and_then(Value::as_u64)
            .context("WireMock's count answer carries `count`")
    }
}

const VALID_KEY: &str = "stub-public-key-test_0001";

fn plan(registration: Registration) -> CallbackPlan {
    CallbackPlan {
        deployment: DEPLOYMENT.to_owned(),
        rails: BTreeMap::from([
            ("notchpay".to_owned(), registration),
            // Configured, and `ensure`: the pass must still never ask it,
            // because its adapter declares `PerRequest`.
            ("mtn_momo".to_owned(), Registration::Ensure),
        ]),
    }
}

/// The one rail the pass answered about, which must be Notch Pay.
fn only(passes: Vec<RailPass>) -> RailPass {
    let rails: Vec<&str> = passes.iter().map(|pass| pass.rail.as_str()).collect();
    assert_eq!(
        rails,
        ["notchpay"],
        "only the rail that registers its URL is asked; mtn_momo is told its URL per request"
    );
    passes.into_iter().next().expect("one pass")
}

fn state(pass: &RailPass) -> &EndpointState {
    match &pass.check {
        RailCheck::Answered(report) => &report.state,
        other => panic!("the rail did not answer: {other:?}"),
    }
}

#[tokio::test]
async fn the_pass_creates_vpays_endpoint_once_and_then_finds_it() -> anyhow::Result<()> {
    let h = Harness::start().await?;

    let first = only(h.pass(&plan(Registration::Ensure), false).await);
    assert_eq!(first.marker, "vpay:callbacks-suite:notchpay:default");
    assert_eq!(
        state(&first),
        &EndpointState::Created {
            endpoint_id: ENDPOINT_ID.to_owned()
        }
    );

    // What was sent is the contract, read from the stub's journal rather
    // than from the adapter: the URL, the marker with the URL's tag, and the
    // five payment events.
    let journal: Value = reqwest::Client::new()
        .post(format!("{}/__admin/requests/find", h.stub))
        .json(&serde_json::json!({ "method": "POST", "urlPath": "/webhooks" }))
        .send()
        .await?
        .json()
        .await?;
    let body: Value = serde_json::from_str(
        journal
            .pointer("/requests/0/body")
            .and_then(Value::as_str)
            .context("the create is in the journal")?,
    )?;
    assert_eq!(body.get("url"), Some(&Value::from(CALLBACK_URL)));
    assert_eq!(body.get("description"), Some(&Value::from(DESCRIPTION)));
    assert_eq!(
        body.get("events"),
        Some(&serde_json::json!([
            "payment.created",
            "payment.complete",
            "payment.failed",
            "payment.canceled",
            "payment.expired"
        ]))
    );

    let second = only(h.pass(&plan(Registration::Ensure), false).await);
    assert_eq!(
        state(&second),
        &EndpointState::Present {
            endpoint_id: ENDPOINT_ID.to_owned(),
            repaired: false,
        },
        "the second pass must recognise the endpoint by its description, since Notch Pay \
         answers url null"
    );
    assert!(second.has_endpoint());
    assert_eq!(h.requests("POST").await?, 1, "one create across two passes");
    Ok(())
}

/// RFC-0013 § 4: two passes that both list before either creates would both
/// create on Notch Pay. The lock is what stops it.
#[tokio::test]
async fn two_racing_passes_create_one_endpoint() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let plan = plan(Registration::Ensure);

    let (a, b) = tokio::join!(h.pass(&plan, false), h.pass(&plan, false));
    let mut states: Vec<&str> = [only(a), only(b)]
        .iter()
        .map(|pass| pass.state().expect("ensure is never off"))
        .collect();
    states.sort_unstable();

    assert_eq!(
        h.requests("POST").await?,
        1,
        "two racing passes, one create"
    );
    // Locked when the two overlapped, which the stub's one-second list
    // makes the expected case; present if the second began after the first
    // committed. Either is correct, and neither creates.
    assert!(
        states == ["created", "locked"] || states == ["created", "present"],
        "one pass creates and the other does not: {states:?}"
    );
    Ok(())
}

#[tokio::test]
async fn check_never_writes_and_off_asks_nothing() -> anyhow::Result<()> {
    let h = Harness::start().await?;

    let configured = only(h.pass(&plan(Registration::Check), false).await);
    assert_eq!(state(&configured), &EndpointState::Missing);
    assert!(
        !configured.has_endpoint(),
        "the command exits non-zero on this"
    );

    let forced = only(h.pass(&plan(Registration::Ensure), true).await);
    assert_eq!(
        state(&forced),
        &EndpointState::Missing,
        "--check wins over ensure"
    );
    assert_eq!(h.requests("POST").await?, 0, "check mode never creates");
    assert_eq!(h.requests("GET").await?, 2, "and it did list, twice");

    let off = only(h.pass(&plan(Registration::Off), false).await);
    assert!(matches!(off.check, RailCheck::Off));
    assert_eq!(off.state(), None, "an off rail is not counted");
    assert_eq!(h.requests("GET").await?, 2, "off asks nothing");
    Ok(())
}

/// The job, claimed off the real queue: seeded by `seed_singletons`,
/// dispatched by `run_once`, and back on the clock an hour out.
#[tokio::test]
async fn the_seeded_job_registers_the_endpoint_and_reschedules_hourly() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    vpay_worker::seed_singletons(h.repositories.as_ref()).await?;

    let rails = h.rails(VALID_KEY);
    let plan = plan(Registration::Ensure);
    let endpoints = support::no_webhook_endpoints();
    let mut settled_kind = None;
    // Six singletons, all due now; the others find nothing to do here.
    for _ in 0..6 {
        let settled = vpay_worker::run_once(
            h.repositories.as_ref(),
            &h.adapters,
            &rails,
            &RecoveryPolicy::default(),
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                egress: support::default_egress_policy(),
            },
            &plan,
            "callbacks-suite",
        )
        .await?
        .context("six singletons were seeded")?;
        if settled.kind == JobKind::EnsureCallbacks.as_wire_str() {
            assert_eq!(
                settled.disposition,
                Disposition::Rescheduled(ENSURE_INTERVAL)
            );
            settled_kind = Some(settled.kind);
        }
    }
    assert!(
        settled_kind.is_some(),
        "ensure_callbacks was seeded and ran"
    );
    assert_eq!(
        h.requests("POST").await?,
        1,
        "the job registered the endpoint"
    );
    Ok(())
}

/// RFC-0013 § 2's table: whatever the rail answers, the job reschedules —
/// at the normal interval after a refusal, sooner after silence — and never
/// dead-letters, because a parked singleton could never be seeded again.
#[tokio::test]
async fn the_job_reschedules_after_a_refusal_and_sooner_after_silence() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let plan = plan(Registration::Ensure);

    let refused = vpay_worker::handle_ensure_callbacks(
        h.repositories.as_ref(),
        &h.adapters,
        &h.rails("bad-key"),
        &plan,
    )
    .await?;
    assert_eq!(
        refused,
        Outcome::RescheduleAfter(ENSURE_INTERVAL),
        "a refused key is a person's to fix; asking every five minutes repeats the alert"
    );

    let silent = vpay_worker::handle_ensure_callbacks(
        h.repositories.as_ref(),
        &h.adapters,
        &h.rails_at("http://127.0.0.1:1", VALID_KEY),
        &plan,
    )
    .await?;
    assert_eq!(silent, Outcome::RescheduleAfter(RETRY_SOON));

    let passes = vpay_worker::run_pass(
        h.repositories.as_ref(),
        &h.adapters,
        &h.rails("bad-key"),
        &plan,
        false,
    )
    .await;
    let pass = only(passes);
    assert!(
        matches!(
            &pass.check,
            RailCheck::Provider(vpay_provider::ProviderError::Rejected {
                code: vpay_core::FailureCode::ProviderAccountBlocked,
                ..
            })
        ),
        "a 401 is the account's keys refused: {:?}",
        pass.check
    );
    assert_eq!(h.requests("POST").await?, 0, "nothing was created");
    Ok(())
}
