//! The span and log layers against in-memory exporters: what the allowlist lets out, how a failure
//! marks a span, what the sampler keeps, and that a full queue drops and counts.

#![allow(clippy::indexing_slicing)]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use opentelemetry::Value;
use opentelemetry::logs::AnyValue;
use opentelemetry::logs::LoggerProvider;
use opentelemetry::trace::{SpanKind, Status, TracerProvider};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, InMemorySpanExporter, Sampler, SdkTracerProvider,
    SpanData,
};
use tracing::{Metadata, info, info_span};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;

use crate::filter::{BoundedSpans, CAP, GuardedSpans, Stats, Tally, ours};
use crate::layer::LogLayer;

fn spans_filter(meta: &Metadata<'_>) -> bool {
    ours(meta.target()) && *meta.level() <= CAP
}

struct Run {
    spans: Vec<SpanData>,
    logs: Vec<opentelemetry_sdk::logs::in_memory_exporter::LogDataWithResource>,
}

fn run(sampler: Sampler, f: impl FnOnce()) -> Run {
    let stats = Arc::new(Stats::default());
    let tally = Tally::new(stats);
    let span_exporter = InMemorySpanExporter::default();
    let guarded = GuardedSpans::new(span_exporter.clone(), Arc::new(AtomicUsize::new(0)), tally);
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(guarded)
        .with_sampler(sampler)
        .build();
    let log_exporter = InMemoryLogExporter::default();
    let logger_provider = SdkLoggerProvider::builder()
        .with_simple_exporter(log_exporter.clone())
        .build();
    let otel = tracing_opentelemetry::layer()
        .with_tracer(tracer_provider.tracer("test"))
        .with_location(false)
        .with_threads(false)
        .with_tracked_inactivity(false)
        .with_filter(filter_fn(spans_filter));
    let logs = LogLayer::new(logger_provider.logger("test")).with_filter(filter_fn(
        |meta: &Metadata<'_>| meta.is_event() && meta.target().starts_with("vpay"),
    ));
    tracing::subscriber::with_default(tracing_subscriber::registry().with(otel).with(logs), f);
    let _ = tracer_provider.force_flush();
    let _ = logger_provider.force_flush();
    Run {
        spans: span_exporter.get_finished_spans().unwrap_or_default(),
        logs: log_exporter.get_emitted_logs().unwrap_or_default(),
    }
}

fn keys(span: &SpanData) -> Vec<&str> {
    span.attributes.iter().map(|kv| kv.key.as_str()).collect()
}

#[test]
fn only_allowlisted_fields_leave_and_their_text_is_plain() {
    let r = run(Sampler::AlwaysOn, || {
        let span = info_span!(
            target: "vpay_api",
            "request",
            http.request.method = "POST",
            http.route = "/v1/payment_intents/{id}",
            path = "/v1/payment_intents/pi_secret_7",
            request_id = "req-secret-7",
            msisdn = "+237600000000",
            amount = 123_456_789_u64,
            auth.method = "has a space and a token abc",
        );
        let _in = span.enter();
        info!(target: "vpay_worker", provider = "mtn_momo", msisdn = "+237600000000", batch = 5_u64, "drain finished");
    });
    assert_eq!(r.spans.len(), 1);
    let span = &r.spans[0];
    assert_eq!(
        keys(span),
        ["http.request.method", "http.route", "vpay.auth.method"]
    );
    assert_eq!(
        span.attributes[2].value,
        Value::String("redacted".into()),
        "an allowed name with a sentence in it is redacted"
    );
    assert_eq!(span.span_kind, SpanKind::Server);
    assert!(span.events.events.is_empty() && span.links.links.is_empty());
    let text = format!("{span:?}");
    for secret in [
        "pi_secret_7",
        "req-secret-7",
        "+237600000000",
        "123456789",
        "abc",
    ] {
        assert!(!text.contains(secret), "{secret} left in {text}");
    }
    assert_eq!(r.logs.len(), 1);
    let log = &r.logs[0].record;
    assert_eq!(log.body(), Some(&AnyValue::String("drain finished".into())));
    let attributes: Vec<_> = log
        .attributes_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.clone()))
        .collect();
    assert_eq!(
        attributes,
        [(
            "vpay.provider".to_owned(),
            AnyValue::String("mtn_momo".into())
        )],
        "only the allowlisted field of the event, not the number"
    );
}

#[test]
fn a_log_in_a_recorded_span_carries_its_trace_and_span_ids() {
    let r = run(Sampler::AlwaysOn, || {
        let span = info_span!(target: "vpay_api", "request");
        let _in = span.enter();
        info!(target: "vpay_worker", "drain finished");
    });
    let ids = r.logs[0].record.trace_context().expect("trace context");
    assert_eq!(ids.span_id, r.spans[0].span_context.span_id());
    assert_eq!(ids.trace_id, r.spans[0].span_context.trace_id());
}

#[test]
fn children_join_their_parents_trace() {
    let r = run(Sampler::AlwaysOn, || {
        let root = info_span!(target: "vpay_api", "request");
        let _root = root.enter();
        let child = info_span!(target: "vpay_worker", "deliver", provider = "notchpay");
        let _child = child.enter();
    });
    assert_eq!(r.spans.len(), 2);
    let (child, root) = (&r.spans[0], &r.spans[1]);
    assert_eq!(child.span_context.trace_id(), root.span_context.trace_id());
    assert_eq!(child.parent_span_id, root.span_context.span_id());
    assert_eq!(root.status, Status::Unset);
    assert_eq!(keys(child), ["vpay.provider"]);
    assert_eq!(child.span_kind, SpanKind::Internal);
}

#[test]
fn a_5xx_status_marks_the_span_as_an_error() {
    let r = run(Sampler::AlwaysOn, || {
        let span = info_span!(target: "vpay_api", "request", http.response.status_code = tracing::field::Empty);
        span.record("http.response.status_code", 503_i64);
    });
    assert!(matches!(r.spans[0].status, Status::Error { .. }));
}

#[test]
fn a_foreign_target_is_not_looked_at_and_the_sample_ratio_holds() {
    let r = run(Sampler::AlwaysOn, || {
        let span = info_span!(target: "sqlx::query", "sqlx", db.statement = "SELECT 1");
        let _in = span.enter();
        info!(target: "other", "hello");
    });
    assert!(r.spans.is_empty() && r.logs.is_empty());
    let none = run(Sampler::TraceIdRatioBased(0.0), || {
        let span = info_span!(target: "vpay_api", "request");
        let _in = span.enter();
    });
    assert!(none.spans.is_empty());
}

#[test]
fn verbose_levels_are_not_dispatched() {
    let r = run(Sampler::AlwaysOn, || {
        let span = tracing::debug_span!(target: "vpay_worker", "db.query");
        drop(span.enter());
    });
    assert!(r.spans.is_empty());
}

#[test]
fn a_full_queue_drops_and_counts() {
    let stats = Arc::new(Stats::default());
    let tally = Tally::new(Arc::clone(&stats));
    let exporter = InMemorySpanExporter::default();
    // A queue that is already at its limit: every span finds it full.
    let pending = Arc::new(AtomicUsize::new(4));
    let guarded = GuardedSpans::new(exporter.clone(), Arc::clone(&pending), tally.clone());
    let batch = BatchSpanProcessor::builder(guarded)
        .with_batch_config(BatchConfigBuilder::default().build())
        .build();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(BoundedSpans::new(batch, pending, 4, tally))
        .build();
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer("test"))
        .with_filter(filter_fn(spans_filter));
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::with_local_recorder(&recorder, || {
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            for _ in 0..5 {
                let span = info_span!(target: "vpay_api", "request");
                drop(span.enter());
            }
        });
    });
    let _ = provider.force_flush();
    assert!(exporter.get_finished_spans().unwrap_or_default().is_empty());
    assert_eq!(stats.dropped.load(std::sync::atomic::Ordering::Relaxed), 5);
    assert!(
        handle
            .render()
            .contains("vpay_telemetry_dropped_total{signal=\"traces\"} 5"),
        "{}",
        handle.render()
    );
}

#[test]
fn a_full_log_queue_drops_and_counts() {
    use crate::filter::{BoundedLogs, GuardedLogs};
    use opentelemetry_sdk::logs::BatchLogProcessor;
    let stats = Arc::new(Stats::default());
    let tally = Tally::new(Arc::clone(&stats));
    let exporter = InMemoryLogExporter::default();
    let pending = Arc::new(AtomicUsize::new(4));
    let guarded = GuardedLogs::new(exporter.clone(), Arc::clone(&pending), tally.clone());
    let batch = BatchLogProcessor::builder(guarded).build();
    let provider = SdkLoggerProvider::builder()
        .with_log_processor(BoundedLogs::new(batch, pending, 4, tally))
        .build();
    let layer =
        LogLayer::new(provider.logger("test")).with_filter(filter_fn(|meta: &Metadata<'_>| {
            meta.is_event() && meta.target().starts_with("vpay")
        }));
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::with_local_recorder(&recorder, || {
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            for _ in 0..5 {
                info!(target: "vpay_worker", "drain finished");
            }
        });
    });
    let _ = provider.force_flush();
    assert!(exporter.get_emitted_logs().unwrap_or_default().is_empty());
    assert_eq!(stats.dropped.load(std::sync::atomic::Ordering::Relaxed), 5);
    assert!(
        handle
            .render()
            .contains("vpay_telemetry_dropped_total{signal=\"logs\"} 5"),
        "{}",
        handle.render()
    );
}

/// Records what the subscriber answers when a callsite registers.
struct Probe<S> {
    inner: S,
    seen: Arc<std::sync::Mutex<Vec<(String, tracing::Level, bool)>>>,
}

impl<S: tracing::Subscriber> tracing::Subscriber for Probe<S> {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        let interest = self.inner.register_callsite(meta);
        // Callsites are process-global and other tests register theirs through this dispatcher too: keep only the
        // ones this test makes, which carry a `probe` field.
        if meta.fields().field("probe").is_none() {
            return interest;
        }
        self.seen.lock().unwrap().push((
            meta.target().to_owned(),
            *meta.level(),
            interest.is_never(),
        ));
        interest
    }
    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        self.inner.enabled(meta)
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        self.inner.max_level_hint()
    }
    fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        self.inner.new_span(attrs)
    }
    fn record(&self, span: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        self.inner.record(span, values);
    }
    fn record_follows_from(&self, span: &tracing::span::Id, follows: &tracing::span::Id) {
        self.inner.record_follows_from(span, follows);
    }
    fn event(&self, event: &tracing::Event<'_>) {
        self.inner.event(event);
    }
    fn enter(&self, span: &tracing::span::Id) {
        self.inner.enter(span);
    }
    fn exit(&self, span: &tracing::span::Id) {
        self.inner.exit(span);
    }
    fn clone_span(&self, id: &tracing::span::Id) -> tracing::span::Id {
        self.inner.clone_span(id)
    }
    fn try_close(&self, id: tracing::span::Id) -> bool {
        self.inner.try_close(id)
    }
}

fn interests(guard: &crate::export::Guard) -> Vec<(String, tracing::Level, bool)> {
    let seen = Arc::new(std::sync::Mutex::default());
    let layer = guard.layer().expect("a layer when a signal is on");
    let dispatch = tracing::Dispatch::new(Probe {
        inner: tracing_subscriber::registry().with(layer),
        seen: Arc::clone(&seen),
    });
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::trace!(target: "hyper", probe = true, "foreign trace");
        tracing::info!(target: "sqlx::query", probe = true, "foreign info");
        tracing::debug!(target: "vpay_worker", probe = true, "our debug");
        tracing::trace!(target: "vpay_api", probe = true, "our trace");
        let _ = tracing::debug_span!(target: "vpay_worker", "debug span", probe = true);
        tracing::info!(target: "vpay_worker", probe = true, "our info");
    });
    let seen = seen.lock().unwrap();
    seen.iter().map(|(t, l, i)| (t.clone(), *l, *i)).collect()
}

fn guard_with(tracer: bool, logger: bool) -> crate::export::Guard {
    let providers = crate::export::Providers {
        tracer: tracer.then(|| {
            SdkTracerProvider::builder()
                .with_simple_exporter(InMemorySpanExporter::default())
                .build()
        }),
        meter: None,
        logger: logger.then(|| {
            SdkLoggerProvider::builder()
                .with_simple_exporter(InMemoryLogExporter::default())
                .build()
        }),
        bridge: None,
    };
    crate::export::Guard::for_test(providers)
}

fn assert_only_ours_at_info_are_dispatched(guard: &crate::export::Guard) {
    let seen = interests(guard);
    // The layers' filters hint a maximum level of INFO, so the `tracing` macros do not even register a callsite
    // above it (the verbose ones never reach `register_callsite`); the two INFO callsites do, and the foreign one
    // is `Interest::never`.
    assert!(seen.len() >= 2, "{seen:?}");
    assert!(
        seen.iter()
            .any(|(t, l, _)| t == "vpay_worker" && *l == tracing::Level::INFO),
        "{seen:?}"
    );
    for (target, level, never) in &seen {
        let ours = target.starts_with("vpay") && *level <= tracing::Level::INFO;
        if ours {
            assert!(!*never, "{target} {level} must be looked at: {seen:?}");
        } else {
            assert!(*never, "{target} {level} must be Interest::never: {seen:?}");
        }
    }
}

#[test]
fn foreign_and_verbose_callsites_are_never_dispatched_with_traces_and_logs_on() {
    assert_only_ours_at_info_are_dispatched(&guard_with(true, true));
}

#[test]
fn foreign_and_verbose_callsites_are_never_dispatched_with_only_traces_on() {
    assert_only_ours_at_info_are_dispatched(&guard_with(true, false));
}

#[test]
fn foreign_and_verbose_callsites_are_never_dispatched_with_only_logs_on() {
    assert_only_ours_at_info_are_dispatched(&guard_with(false, true));
}

#[test]
fn with_no_signal_on_there_is_no_layer_to_install() {
    assert!(
        guard_with(false, false)
            .layer::<tracing_subscriber::Registry>()
            .is_none(),
        "no layer means the process's own subscriber is untouched, not a layer that sees everything"
    );
}
