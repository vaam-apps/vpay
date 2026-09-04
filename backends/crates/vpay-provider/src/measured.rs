//! [`Measured`]: the one place a rail call is counted and timed.
//!
//! # Why a decorator and not instrumentation inside the adapters
//!
//! ADR-0002's rule is that rail-specific code lives inside
//! `backends/crates/vpay-adapter-*` and nowhere else. Its corollary, which
//! this module is an instance of, is that *cross-rail* concerns must not be
//! written inside an adapter either: two adapters each holding their own
//! copy of "start a timer, classify the error, increment a counter" is two
//! copies that can disagree about what an operation is called, and a third
//! rail is a third copy someone has to remember to write. The port is the
//! only place that sees every rail and every call, so it is where the
//! measurement belongs.
//!
//! [`Measured`] therefore implements [`ProviderAdapter`] by delegating to
//! another one. It is applied by `vpay_api::v1::boot::adapters_by_code`,
//! which is the single funnel every rail call in this workspace passes
//! through — both binaries build their adapter map with it, and so does the
//! integration suite. Wrapping there rather than in each binary's
//! `adapters()` list is deliberate: `adapters()` is duplicated per binary
//! (Step 2's D6, so a worker's capabilities are not a function of the API
//! server's crate) and a metric mounted in a duplicated list is a metric one
//! of the copies eventually loses.
//!
//! # This is not a test double, and could not be one
//!
//! It has no behaviour of its own: every method forwards, the return value
//! is the inner adapter's unchanged, and it cannot be constructed without a
//! real adapter to wrap. ADR-0006 is about a *substitute* for a rail
//! compiled into a shipping process; this is the shipping process's own
//! observability, on the real rail's real answer.
//!
//! # What is counted, and what is not
//!
//! One increment per **port call**, not per HTTP request: Orange Money's
//! `submit` mints an access token and then posts the payment, and that is
//! one `submit`. A call that never reaches the socket — a rail missing a
//! credential, a push charge with no payer — is also counted, carrying that
//! refusal's `error_kind`, because "calls to this rail are failing" is true
//! whether the failure was local or remote and an operator seeing a rate
//! climb needs to be told either way.
//!
//! `parse_callback` is **not** counted: it parses bytes that have already
//! arrived, touches no rail, and cannot fail slowly. Including it would put
//! a pure function into a series an operator reads as rail traffic.

use std::time::Instant;

use async_trait::async_trait;
use vpay_core::Money;
use vpay_core::error::Classify as _;
use vpay_core::metrics::{
    PROVIDER_REQUEST_DURATION_SECONDS, PROVIDER_REQUESTS_TOTAL, provider_operation,
};

use crate::{
    CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderAdapter, ProviderConfig,
    ProviderError, Submitted,
};

/// A [`ProviderAdapter`] that counts and times every call it forwards.
///
/// See the module header for why the measurement lives here rather than in
/// the adapters, and for the one thing it deliberately does not count.
#[derive(Debug)]
pub struct Measured {
    inner: Box<dyn ProviderAdapter>,
}

impl Measured {
    /// Wraps `inner`, returning it as a plain [`ProviderAdapter`] again.
    ///
    /// The return type is `Box<dyn ProviderAdapter>` rather than
    /// `Box<Measured>` on purpose: a caller that could name the wrapper's
    /// type could branch on whether an adapter is wrapped, which is the
    /// beginning of a code path that exists only when metrics are on.
    /// Callers see a rail adapter; that is all there is to see.
    #[must_use]
    pub fn wrap(inner: Box<dyn ProviderAdapter>) -> Box<dyn ProviderAdapter> {
        Box::new(Self { inner })
    }

    /// Times `call`, then records the pair of series for `operation`.
    ///
    /// `error_kind` is `""` on success and otherwise
    /// [`Classify::code`](vpay_core::error::Classify::code) — the same
    /// vocabulary the `provider_requests.error_kind` column stores, so the
    /// PromQL an alert runs and the SQL an operator runs use one set of
    /// words. An empty string rather than a missing label, because a
    /// Prometheus series with a different label *set* is a different series:
    /// `sum(rate(vpay_provider_requests_total[5m]))` has to see successes
    /// and failures as one denominator, which is exactly what
    /// `VpayProviderErrorRateHigh` divides by.
    ///
    /// The duration is recorded on both paths — a rail that times out is the
    /// slowest thing it ever does, and dropping those samples would make the
    /// histogram claim the rail is fast at the moment it stops answering —
    /// but carries no `error_kind` label, for the reason
    /// [`PROVIDER_REQUEST_DURATION_SECONDS`]'s own documentation gives.
    async fn measure<T>(
        &self,
        operation: &'static str,
        call: impl Future<Output = Result<T, ProviderError>>,
    ) -> Result<T, ProviderError> {
        let started = Instant::now();
        let result = call.await;
        let elapsed = started.elapsed();
        let error_kind = match &result {
            Ok(_) => "",
            Err(error) => error.code(),
        };
        let provider = self.inner.code();
        metrics::counter!(
            PROVIDER_REQUESTS_TOTAL,
            "provider" => provider,
            "operation" => operation,
            "error_kind" => error_kind,
        )
        .increment(1);
        metrics::histogram!(
            PROVIDER_REQUEST_DURATION_SECONDS,
            "provider" => provider,
            "operation" => operation,
        )
        .record(elapsed.as_secs_f64());
        result
    }
}

#[async_trait]
impl ProviderAdapter for Measured {
    /// The wrapped rail's own code, so the map key and every `provider`
    /// label are the same string the adapter chose for itself.
    fn code(&self) -> &'static str {
        self.inner.code()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn submit(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        self.measure(
            provider_operation::SUBMIT,
            self.inner.submit(charge, config),
        )
        .await
    }

    async fn query_status(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError> {
        self.measure(
            provider_operation::QUERY_STATUS,
            self.inner.query_status(charge, config),
        )
        .await
    }

    /// Forwarded unmeasured — see the module header.
    fn parse_callback(&self, body: &[u8]) -> Result<CallbackRef, ProviderError> {
        self.inner.parse_callback(body)
    }

    async fn refund(
        &self,
        charge: &ChargeRef,
        amount: Money,
        config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        self.measure(
            provider_operation::REFUND,
            self.inner.refund(charge, amount, config),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use metrics_exporter_prometheus::PrometheusBuilder;
    use uuid::Uuid;
    use vpay_core::{Currency, FailureCode, ProviderFlow};

    use super::*;

    /// A rail that answers whatever the test asked it to, and reaches no
    /// network.
    ///
    /// **Not a test double of an adapter**, for the same reason
    /// `vpay_api::v1::boot`'s `TestRail` is not: what is under test here is
    /// the *decorator*, and the decorator's contract is "whatever the inner
    /// adapter returned, plus two series". Proving that needs an inner
    /// adapter whose answers the test chose, which no real rail can offer;
    /// the real-rail half is `worker_e2e`'s scrape, which asserts
    /// `vpay_provider_requests_total{provider="mtn_momo"}` after the MTN
    /// adapter has really spoken HTTP to a WireMock container. It is
    /// `#[cfg(test)]`, so no shipping binary can reach it (ADR-0006).
    #[derive(Debug)]
    struct Answering {
        code: &'static str,
        error: Option<FailureCode>,
    }

    #[async_trait]
    impl ProviderAdapter for Answering {
        fn code(&self) -> &'static str {
            self.code
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                flow: ProviderFlow::Push,
                supports_refunds: false,
                supports_partial_refunds: false,
                delivers_callbacks: false,
                requires_ip_allowlist: false,
            }
        }

        async fn submit(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<Submitted, ProviderError> {
            match self.error {
                Some(code) => Err(ProviderError::Rejected {
                    code,
                    message: "the rail's own words".to_owned(),
                }),
                None => Ok(Submitted {
                    ref_extra: BTreeMap::new(),
                    redirect_url: None,
                }),
            }
        }

        async fn query_status(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<ChargeStatus, ProviderError> {
            Err(ProviderError::transport("no rail here".to_owned()))
        }

        fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
            Err(ProviderError::Unsupported)
        }
    }

    fn charge() -> ChargeRef {
        ChargeRef {
            reference_id: Uuid::nil(),
            amount: Money::new(5_000, Currency::Xaf).expect("5000 is non-negative"),
            payer_ref: None,
            ref_extra: BTreeMap::new(),
            return_url: None,
        }
    }

    fn config() -> ProviderConfig {
        ProviderConfig {
            base_url: "https://rail.example".to_owned(),
            callback_url: "https://vpay.example/provider/x/callback".to_owned(),
            currency: Currency::Xaf,
            settings: BTreeMap::new(),
            credentials: BTreeMap::new(),
            connect_timeout: crate::DEFAULT_CONNECT_TIMEOUT,
            request_timeout: crate::DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Renders a real Prometheus scrape of whatever `body` recorded.
    ///
    /// The **shipping** exporter rather than a debugging recorder, and a
    /// *local* recorder rather than the global one: the global can only be
    /// installed once per process, so a unit test that installed it would
    /// make every other test in this binary depend on the order it ran in.
    /// Asserting on the rendered text is also what makes these tests fail
    /// for the same reason a dashboard would be empty — a label spelled
    /// wrongly is a different line here, not a different struct field.
    fn scrape_of(body: impl FnOnce()) -> String {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, body);
        handle.render()
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime builds")
            .block_on(future)
    }

    /// A successful call carries `error_kind=""` — present and empty, not
    /// absent — so successes and failures share one denominator.
    #[test]
    fn a_successful_call_is_counted_with_an_empty_error_kind() {
        let adapter = Measured::wrap(Box::new(Answering {
            code: "mtn_momo",
            error: None,
        }));

        let scrape = scrape_of(|| {
            let submitted = block_on(adapter.submit(&charge(), &config()));
            assert!(submitted.is_ok(), "the inner adapter's answer is forwarded");
        });

        assert!(
            scrape.contains(
                r#"vpay_provider_requests_total{provider="mtn_momo",operation="submit",error_kind=""} 1"#
            ),
            "{scrape}"
        );
        assert!(
            scrape.contains(
                r#"vpay_provider_request_duration_seconds_count{provider="mtn_momo",operation="submit"} 1"#
            ),
            "the duration histogram must observe the same call: {scrape}"
        );
    }

    /// A failure carries the error's `Classify::code`, which is the same
    /// word `provider_requests.error_kind` stores and the same word
    /// `VpayProviderErrorRateHigh` selects on.
    #[test]
    fn a_failed_call_is_counted_under_the_errors_classify_code() {
        let adapter = Measured::wrap(Box::new(Answering {
            code: "orange_money",
            error: Some(FailureCode::InsufficientFunds),
        }));

        let scrape = scrape_of(|| {
            let submitted = block_on(adapter.submit(&charge(), &config()));
            assert!(
                submitted.is_err(),
                "the inner adapter's answer is forwarded"
            );
            // A second operation, so the `operation` label is proven to
            // separate two series rather than being written once and reused.
            let queried = block_on(adapter.query_status(&charge(), &config()));
            assert!(queried.is_err());
        });

        assert!(
            scrape.contains(
                r#"vpay_provider_requests_total{provider="orange_money",operation="submit",error_kind="charge_declined"} 1"#
            ),
            "{scrape}"
        );
        assert!(
            scrape.contains(
                r#"vpay_provider_requests_total{provider="orange_money",operation="query_status",error_kind="provider_unavailable"} 1"#
            ),
            "{scrape}"
        );
    }

    /// A callback is parsed, not fetched, so it must not appear in a series
    /// an operator reads as rail traffic.
    #[test]
    fn parsing_a_callback_is_not_counted_as_a_rail_call() {
        let adapter = Measured::wrap(Box::new(Answering {
            code: "mtn_momo",
            error: None,
        }));

        let scrape = scrape_of(|| {
            let parsed = adapter.parse_callback(b"{}");
            assert!(parsed.is_err(), "the inner adapter's answer is forwarded");
        });

        assert!(
            !scrape.contains("vpay_provider_requests_total"),
            "parse_callback reaches no rail and must record nothing: {scrape}"
        );
    }

    /// The `operation` labels are the port's method names. `vpay-core`
    /// cannot depend on this crate to spell them, so this is what keeps the
    /// two in step: renaming a method without renaming the constant leaves
    /// this assertion naming a method that no longer exists.
    #[test]
    fn the_operation_labels_are_the_ports_own_method_names() {
        assert_eq!(provider_operation::SUBMIT, "submit");
        assert_eq!(provider_operation::QUERY_STATUS, "query_status");
        assert_eq!(provider_operation::REFUND, "refund");
    }
}
