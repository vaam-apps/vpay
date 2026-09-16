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
//! `parse_destination` is not counted for the same reason, and is forwarded
//! for a reason of its own — it is the one port method whose *default* body
//! is a refusal, so a decorator that forgot to forward it would compile and
//! would answer `Unsupported` for every rail. See the method.

use std::time::Instant;

use async_trait::async_trait;
use vpay_core::Money;
use vpay_core::error::Classify as _;
use vpay_core::metrics::{
    PROVIDER_REQUEST_DURATION_SECONDS, PROVIDER_REQUESTS_TOTAL, provider_operation,
};

use crate::{
    AccountHolder, CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderAdapter,
    ProviderConfig, ProviderError, RefundTarget, Refunded, Submitted,
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

    /// Forwarded unmeasured, and — the part that matters here — forwarded at
    /// all.
    ///
    /// [`ProviderAdapter::parse_destination`] has a default body answering
    /// [`ProviderError::Unsupported`], which is right for an
    /// [`Origin`](crate::RefundDestination::Origin) rail and catastrophic for
    /// a decorator: `Measured` wraps every adapter this workspace resolves
    /// (`vpay_api::v1::boot::adapters_by_code` is the single funnel), so an
    /// omitted forward here would not fail to compile — it would make every
    /// refund on every rail answer "this rail has no such API" in production
    /// while each adapter's own unit tests, which hold the adapter unwrapped,
    /// stayed green. `a_defaulted_method_is_not_silently_answered_by_the_wrapper`
    /// is what catches it.
    ///
    /// Unmeasured for [`parse_callback`](Measured::parse_callback)'s reason:
    /// it parses values that have already arrived, touches no rail, and
    /// cannot fail slowly. And nothing is derived from the parsed
    /// [`RefundTarget`] for a label — see `refund` below.
    fn parse_destination(
        &self,
        raw: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<RefundTarget, ProviderError> {
        self.inner.parse_destination(raw)
    }

    /// Forwarded with the destination untouched: this decorator counts and
    /// times, and a layer that looked inside a [`RefundTarget`] — even to
    /// derive a label from it — would put a payee's phone number in a metric
    /// series. `error_kind` stays the only dimension.
    async fn refund(
        &self,
        charge: &ChargeRef,
        amount: Money,
        destination: Option<&RefundTarget>,
        config: &ProviderConfig,
    ) -> Result<Refunded, ProviderError> {
        self.measure(
            provider_operation::REFUND,
            self.inner.refund(charge, amount, destination, config),
        )
        .await
    }

    /// Measured like every other call that reaches a rail — and the `Ok`
    /// value is forwarded untouched, so `Some` and `None` are indistinguishable
    /// in the series.
    ///
    /// That is deliberate and not an omission: `error_kind` is the only
    /// dimension here, and splitting `found` from `not_found` on a *rail*
    /// series would be answering a question about the `/v1` route from the
    /// wrong seam. `vpay_account_holder_lookups_total` is where those two
    /// are told apart, and it is emitted by the handler, which is also the
    /// only layer that knows a merchant asked.
    async fn account_holder_name(
        &self,
        msisdn: &str,
        config: &ProviderConfig,
    ) -> Result<Option<AccountHolder>, ProviderError> {
        self.measure(
            provider_operation::ACCOUNT_HOLDER_NAME,
            self.inner.account_holder_name(msisdn, config),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

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
                supports_account_holder_lookup: false,
                refund_destination: crate::RefundDestination::Origin,
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
        assert_eq!(
            provider_operation::ACCOUNT_HOLDER_NAME,
            "account_holder_name"
        );
    }

    /// The identity read is a call to the rail, so it is on the same series
    /// as every other one — and the wrapper must not be able to tell `Some`
    /// from `None` there. `Answering` inherits the port's default, which is
    /// `Unsupported`, so this also pins that a *default* method is measured:
    /// the decorator forwards to the trait, not to an override that may not
    /// exist.
    #[test]
    fn an_account_holder_lookup_is_counted_as_a_rail_call() {
        let adapter = Measured::wrap(Box::new(Answering {
            code: "mtn_momo",
            error: None,
        }));

        let scrape = scrape_of(|| {
            let looked_up = block_on(adapter.account_holder_name("237600000000", &config()));
            assert!(
                matches!(looked_up, Err(ProviderError::Unsupported)),
                "the inner adapter's answer is forwarded"
            );
        });

        assert!(
            scrape.contains(
                r#"vpay_provider_requests_total{provider="mtn_momo",operation="account_holder_name",error_kind="operation_unsupported_by_rail"} 1"#
            ),
            "{scrape}"
        );
    }

    /// An inner adapter that records the `destination` it was handed, so the
    /// decorator's forwarding can be observed rather than read.
    ///
    /// Its own type rather than a third field on [`Answering`]: every other
    /// case here builds `Answering` by struct literal, and what this proves
    /// is about one argument of one method. It declares
    /// [`RefundDestination::Required`](crate::RefundDestination::Required)
    /// because that is the only declaration for which a `destination` is
    /// ever `Some` (RFC-0003 section 1) - the value the argument exists for.
    #[derive(Debug)]
    struct RecordingRefund {
        /// Shared with the test, which cannot reach the stub again: the
        /// adapter is moved into a `Box<dyn ProviderAdapter>` by
        /// [`Measured::wrap`] and never comes back out.
        seen: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait]
    impl ProviderAdapter for RecordingRefund {
        fn code(&self) -> &'static str {
            "mtn_momo"
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                flow: ProviderFlow::Push,
                supports_refunds: true,
                supports_partial_refunds: false,
                delivers_callbacks: false,
                requires_ip_allowlist: false,
                supports_account_holder_lookup: false,
                refund_destination: crate::RefundDestination::Required,
            }
        }

        async fn submit(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<Submitted, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn query_status(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<ChargeStatus, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        /// A `Required` rail's parser, spelled as simply as one can be: it
        /// answers `Ok` for a key nobody else in this module uses.
        ///
        /// It exists so the wrapper's forward can be told apart from the
        /// port's default, which answers `Err(Unsupported)`. A stub that
        /// also defaulted would make the two indistinguishable and the test
        /// below unfalsifiable.
        fn parse_destination(
            &self,
            raw: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<RefundTarget, ProviderError> {
            raw.get("msisdn")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ProviderError::malformed("recording: no msisdn".to_owned()))
                .and_then(|msisdn| {
                    RefundTarget::mobile_money(msisdn).map_err(|invalid| {
                        ProviderError::malformed(format!("recording: {invalid}"))
                    })
                })
        }

        async fn refund(
            &self,
            _charge: &ChargeRef,
            _amount: Money,
            destination: Option<&RefundTarget>,
            _config: &ProviderConfig,
        ) -> Result<Refunded, ProviderError> {
            self.seen
                .lock()
                .expect("no test panics while holding this lock")
                .push(destination.map(|target| target.msisdn().to_owned()));
            Err(ProviderError::NotImplemented("recording::refund"))
        }
    }

    /// The destination reaches the inner adapter **unchanged**, and never
    /// reaches a metric.
    ///
    /// Two claims in one case because they are the two halves of the same
    /// argument about this decorator: it must pass the payee through, and it
    /// must not read it.
    ///
    /// Measured on 2026-09-15, before this test existed: replacing the
    /// forwarded `destination` with `None` in [`Measured::refund`] left
    /// `vpay-provider`, both adapter crates and the conformance suite green
    /// (199 tests, 0 failures), because no adapter reads the argument yet.
    /// `Measured` is what `vpay_api::v1::boot::adapters_by_code` wraps every
    /// shipping adapter in, so that mutation would have addressed every
    /// refund on a `Required` rail to nobody, in production, silently.
    ///
    /// The second half is the privacy one: a payee's phone number in a
    /// metric label is a high-cardinality series that outlives any erasure
    /// request, and its retention is RFC-0003 open question 2, undecided.
    /// `error_kind` stays the only dimension.
    #[test]
    fn the_destination_reaches_the_inner_adapter_and_never_a_metric() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let adapter = Measured::wrap(Box::new(RecordingRefund {
            seen: Arc::clone(&seen),
        }));
        let destination =
            RefundTarget::mobile_money("+237600000200").expect("a documentation MSISDN");

        let scrape = scrape_of(|| {
            let refunded =
                block_on(adapter.refund(&charge(), charge().amount, Some(&destination), &config()));
            assert!(
                matches!(refunded, Err(ProviderError::NotImplemented(_))),
                "the inner adapter's answer is forwarded: {refunded:?}"
            );
        });

        assert_eq!(
            seen.lock().expect("the stub released the lock").as_slice(),
            [Some("237600000200".to_owned())],
            "the inner adapter must be handed the destination it was called with"
        );
        assert!(
            scrape.contains(
                r#"vpay_provider_requests_total{provider="mtn_momo",operation="refund",error_kind="not_implemented"} 1"#
            ),
            "the call is still counted on the refund series: {scrape}"
        );
        assert!(
            !scrape.contains("237600000200"),
            "a payee's number must never reach a metric label: {scrape}"
        );
    }

    /// The wrapper forwards [`ProviderAdapter::parse_destination`] rather
    /// than inheriting the port's default.
    ///
    /// `parse_destination` is the only port method whose default body is a
    /// *refusal* an adapter is expected to override, which makes a missing
    /// forward in this decorator uniquely dangerous: it compiles, every
    /// adapter's own unit tests keep passing because they hold the adapter
    /// unwrapped, and every refund in production — `Measured` wraps every
    /// adapter `vpay_api::v1::boot::adapters_by_code` resolves — answers
    /// "this rail has no such API" for a rail that plainly does.
    ///
    /// The decisive mutation: delete `Measured::parse_destination` and this
    /// case fails on `Unsupported`.
    ///
    /// The second assertion is `refund`'s privacy half, on a method that is
    /// *un*measured: no series at all may be emitted here, and in particular
    /// none carrying the number.
    #[test]
    fn a_defaulted_method_is_not_silently_answered_by_the_wrapper() {
        let adapter = Measured::wrap(Box::new(RecordingRefund {
            seen: Arc::new(Mutex::new(Vec::new())),
        }));
        let mut raw = serde_json::Map::new();
        raw.insert(
            "msisdn".to_owned(),
            serde_json::Value::String("+237600000200".to_owned()),
        );

        let mut parsed = None;
        let scrape = scrape_of(|| parsed = Some(adapter.parse_destination(&raw)));

        let parsed = parsed.expect("the closure ran");
        assert_eq!(
            parsed
                .as_ref()
                .map(|target| target.msisdn().to_owned())
                .map_err(|error| format!("{error}")),
            Ok("237600000200".to_owned()),
            "the inner adapter's parser must be the one that answered, not the port's default"
        );
        assert!(
            !scrape.contains("parse_destination") && !scrape.contains("237600000200"),
            "parsing a merchant's parameters reaches no rail and must emit no series: {scrape}"
        );
    }
}
