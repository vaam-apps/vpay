//! What leaves the process, held at the exit: the allowlist of field names, the plain-text rules, and the
//! exporters and processors that wrap the SDK's so that nothing else gets through.
//!
//! **An attribute leaves only if its field name is in [`FIELDS`].** The `tracing-opentelemetry` layer records every
//! field of a span as an attribute; [`clean`] runs on each finished span, in the exporter that wraps the OTLP one,
//! and drops every other attribute, whatever its value. A text value is held to a plain alphabet and a length
//! ([`plain`]), so a field that is allowed for its name cannot carry a sentence. Span events and links never leave
//! (a response of 500 or more is the span's error status), the
//! span's name is held to the same alphabet, and an inbound `traceparent` is never read (the process extracts
//! nothing). Only spans and events whose target is this repository's are looked at ([`ours`]).
//!
//! The exporters also own the exporter's own metrics: a request the collector accepted or refused is counted
//! (`vpay_telemetry_exports_total`), and a span or log that found the bounded queue full is dropped and counted
//! (`vpay_telemetry_dropped_total`).

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use opentelemetry::InstrumentationScope;
use opentelemetry::trace::{SpanKind, Status};
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{
    BatchLogProcessor, LogBatch, LogExporter, LogProcessor, SdkLogRecord,
};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::trace::{BatchSpanProcessor, SpanData, SpanExporter, SpanProcessor};
use tracing::Level;

/// The fields that may be exported, and the attribute each becomes. Everything else is dropped: the request span's
/// `path` and `request_id`, a raw `method` (a caller can send any token), every error text.
pub const FIELDS: [(&str, &str); 9] = [
    ("http.request.method", "http.request.method"),
    ("http.route", "http.route"),
    ("http.response.status_code", "http.response.status_code"),
    ("auth.method", "vpay.auth.method"),
    ("provider", "vpay.provider"),
    ("operation", "vpay.operation"),
    ("outcome", "vpay.outcome"),
    ("state", "vpay.state"),
    ("kind", "vpay.job.kind"),
];

/// The attribute a field name exports as, when it is allowed.
#[must_use]
pub fn allowed(field: &str) -> Option<&'static str> {
    FIELDS
        .iter()
        .find(|(raw, _)| *raw == field)
        .map(|(_, name)| *name)
}

const MAX_TEXT: usize = 64;

/// A text value held to a plain alphabet and [`MAX_TEXT`] bytes; anything else is `redacted`.
#[must_use]
pub fn plain(text: &str) -> String {
    let fits = text.len() <= MAX_TEXT
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./$:-{}".contains(&b));
    if fits {
        text.to_owned()
    } else {
        "redacted".to_owned()
    }
}

/// A log's message: words made of letters only (hyphens and underscores between letters, punctuation at their ends),
/// 128 bytes at most. Anything else, an id with a digit in it among them, is `redacted`: so `info!("account {id}")`
/// does not export its id, though a message of letters alone is the code's own to keep static (README).
#[must_use]
pub fn plain_message(text: &str) -> String {
    let word_ok = |word: &str| {
        let word = word.trim_matches(|c: char| ",.:;()'\"".contains(c));
        word.chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '_')
    };
    if text.len() <= 128 && text.split_whitespace().all(word_ok) {
        text.to_owned()
    } else {
        "redacted".to_owned()
    }
}

/// The most verbose level looked at: nothing below `INFO` is a signal of this service, and a `TRACE` callsite of
/// sqlx, hyper or rustls must not be dispatched for it.
pub const CAP: Level = Level::INFO;

/// Whether a span or event of this target is looked at.
#[must_use]
pub fn ours(target: &str) -> bool {
    target.starts_with("vpay")
}

fn kind_of(name: &str) -> SpanKind {
    if name == "request" {
        SpanKind::Server
    } else {
        SpanKind::Internal
    }
}

fn put(attributes: &mut Vec<KeyValue>, name: &'static str, value: Value) {
    match attributes.iter_mut().find(|kv| kv.key.as_str() == name) {
        Some(slot) => slot.value = value,
        None => attributes.push(KeyValue::new(name, value)),
    }
}

/// An attribute value that may leave: text held to [`plain`], numbers and flags as they are, nothing else (no list).
fn sanitised(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => Some(Value::String(plain(text.as_str()).into())),
        Value::I64(n) => Some(Value::I64(*n)),
        Value::F64(x) => Some(Value::F64(*x)),
        Value::Bool(b) => Some(Value::Bool(*b)),
        _ => None,
    }
}

/// A finished span as it may leave: the allowlisted attributes, no events, no links, no status message.
#[must_use]
pub fn clean(mut span: SpanData) -> SpanData {
    let mut attributes: Vec<KeyValue> = Vec::new();
    let mut error = false;
    for kv in &span.attributes {
        let Some(name) = allowed(kv.key.as_str()) else {
            continue;
        };
        let Some(value) = sanitised(&kv.value) else {
            continue;
        };
        if name == "http.response.status_code" {
            error |= matches!(value, Value::I64(code) if code >= 500);
        }
        put(&mut attributes, name, value);
    }
    span.attributes = attributes;
    span.dropped_attributes_count = 0;
    span.events.events.clear();
    span.events.dropped_count = 0;
    span.links.links.clear();
    span.links.dropped_count = 0;
    span.status = if error {
        Status::error("")
    } else {
        Status::Unset
    };
    span.span_kind = kind_of(&span.name);
    span.parent_span_is_remote = false;
    let name = plain(&span.name);
    if name != span.name {
        span.name = Cow::Owned(name);
    }
    span
}

/// What the exporter did, in counts.
#[derive(Debug, Default)]
pub struct Stats {
    /// Requests the collector accepted.
    pub exported: AtomicU64,
    /// Requests that failed after every attempt.
    pub failed: AtomicU64,
    /// Spans and logs dropped because the queue was full.
    pub dropped: AtomicU64,
}

/// The exporter's own telemetry: counts, a metric, and one rate-limited line on stderr.
#[derive(Clone)]
pub struct Tally {
    stats: Arc<Stats>,
    last_warning: Arc<Mutex<Option<Instant>>>,
}

impl std::fmt::Debug for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tally")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Tally {
    /// A tally recording into `stats` and `metrics`.
    #[must_use]
    pub fn new(stats: Arc<Stats>) -> Self {
        Self {
            stats,
            last_warning: Arc::default(),
        }
    }

    fn exported(&self, signal: &'static str, ok: bool) {
        let (counter, result) = if ok {
            (&self.stats.exported, "ok")
        } else {
            (&self.stats.failed, "failed")
        };
        counter.fetch_add(1, Ordering::Relaxed);
        metrics::counter!(
            vpay_core::metrics::TELEMETRY_EXPORTS_TOTAL,
            "signal" => signal,
            "result" => result,
        )
        .increment(1);
        if !ok {
            self.warn();
        }
    }

    fn dropped(&self, signal: &'static str) {
        self.stats.dropped.fetch_add(1, Ordering::Relaxed);
        metrics::counter!(vpay_core::metrics::TELEMETRY_DROPPED_TOTAL, "signal" => signal)
            .increment(1);
    }

    fn warn(&self) {
        let mut last = self
            .last_warning
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.is_none_or(|at| at.elapsed() >= Duration::from_secs(60)) {
            *last = Some(Instant::now());
            // Stderr, not `tracing`: the exporter must not feed itself. No URL, no header, no body.
            eprintln!("vpay: telemetry export failed; dropping, will keep trying");
        }
    }
}

/// The span exporter that cleans, counts and releases the queue's room.
#[derive(Debug)]
pub struct GuardedSpans<E> {
    inner: E,
    pending: Arc<AtomicUsize>,
    tally: Tally,
}

impl<E> GuardedSpans<E> {
    /// Wraps `inner`; `pending` is the count the bounded processor ([`BoundedSpans`]) shares.
    pub fn new(inner: E, pending: Arc<AtomicUsize>, tally: Tally) -> Self {
        Self {
            inner,
            pending,
            tally,
        }
    }
}

impl<E: SpanExporter> SpanExporter for GuardedSpans<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let n = batch.len();
        let result = self
            .inner
            .export(batch.into_iter().map(clean).collect())
            .await;
        self.pending.fetch_sub(
            n.min(self.pending.load(Ordering::Relaxed)),
            Ordering::Relaxed,
        );
        self.tally.exported("traces", result.is_ok());
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// The batch processor behind a bounded queue of our own: when `limit` spans wait or are in flight, a new one is
/// dropped and counted, so the SDK's own queue never overflows silently and recording never blocks.
#[derive(Debug)]
pub struct BoundedSpans {
    inner: BatchSpanProcessor,
    pending: Arc<AtomicUsize>,
    limit: usize,
    tally: Tally,
}

impl BoundedSpans {
    /// Wraps `inner`; `pending` is shared with the [`GuardedSpans`] that feeds it.
    #[must_use]
    pub fn new(
        inner: BatchSpanProcessor,
        pending: Arc<AtomicUsize>,
        limit: usize,
        tally: Tally,
    ) -> Self {
        Self {
            inner,
            pending,
            limit,
            tally,
        }
    }
}

impl SpanProcessor for BoundedSpans {
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &opentelemetry::Context) {
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, span: SpanData) {
        if self.pending.fetch_add(1, Ordering::Relaxed) >= self.limit {
            self.pending.fetch_sub(1, Ordering::Relaxed);
            self.tally.dropped("traces");
            return;
        }
        self.inner.on_end(span);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// The log exporter that counts and releases the queue's room. Logs are filtered where they are made
/// ([`crate::layer::LogLayer`]), so nothing is rewritten here.
#[derive(Debug)]
pub struct GuardedLogs<E> {
    inner: E,
    pending: Arc<AtomicUsize>,
    tally: Tally,
}

impl<E> GuardedLogs<E> {
    /// Wraps `inner`; `pending` is shared with the [`BoundedLogs`] that feeds it.
    pub fn new(inner: E, pending: Arc<AtomicUsize>, tally: Tally) -> Self {
        Self {
            inner,
            pending,
            tally,
        }
    }
}

impl<E: LogExporter> LogExporter for GuardedLogs<E> {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let n = batch.iter().count();
        let result = self.inner.export(batch).await;
        self.pending.fetch_sub(
            n.min(self.pending.load(Ordering::Relaxed)),
            Ordering::Relaxed,
        );
        self.tally.exported("logs", result.is_ok());
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// The batch log processor behind a bounded queue of our own, as [`BoundedSpans`].
#[derive(Debug)]
pub struct BoundedLogs {
    inner: BatchLogProcessor,
    pending: Arc<AtomicUsize>,
    limit: usize,
    tally: Tally,
}

impl BoundedLogs {
    /// Wraps `inner`; `pending` is shared with the [`GuardedLogs`] that feeds it.
    #[must_use]
    pub fn new(
        inner: BatchLogProcessor,
        pending: Arc<AtomicUsize>,
        limit: usize,
        tally: Tally,
    ) -> Self {
        Self {
            inner,
            pending,
            limit,
            tally,
        }
    }
}

impl LogProcessor for BoundedLogs {
    fn emit(&self, data: &mut SdkLogRecord, scope: &InstrumentationScope) {
        if self.pending.fetch_add(1, Ordering::Relaxed) >= self.limit {
            self.pending.fetch_sub(1, Ordering::Relaxed);
            self.tally.dropped("logs");
            return;
        }
        self.inner.emit(data, scope);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// The metrics exporter that counts what the collector accepted and refused.
pub struct GuardedMetrics<E> {
    inner: E,
    tally: Tally,
}

impl<E> std::fmt::Debug for GuardedMetrics<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedMetrics").finish_non_exhaustive()
    }
}

impl<E> GuardedMetrics<E> {
    /// Wraps `inner`.
    pub fn new(inner: E, tally: Tally) -> Self {
        Self { inner, tally }
    }
}

impl<E: PushMetricExporter> PushMetricExporter for GuardedMetrics<E> {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        let result = self.inner.export(metrics).await;
        self.tally.exported("metrics", result.is_ok());
        result
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn temporality(&self) -> Temporality {
        self.inner.temporality()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_message_of_letters_passes_and_one_with_an_id_is_redacted() {
        assert_eq!(
            plain_message("drain failed (unavailable)."),
            "drain failed (unavailable)."
        );
        for bad in [
            "charge ch_42 refused",
            "token Bearer abc.def.1",
            &"x".repeat(200),
        ] {
            assert_eq!(plain_message(bad), "redacted", "{bad}");
        }
    }

    #[test]
    fn text_is_plain_or_redacted() {
        assert_eq!(
            plain("/v1/payment_intents/{id}"),
            "/v1/payment_intents/{id}"
        );
        assert_eq!(plain("has a space and an id ch_1"), "redacted");
        assert_eq!(plain(&"a".repeat(65)), "redacted");
    }

    #[test]
    fn only_allowlisted_fields_have_a_name() {
        assert_eq!(allowed("http.route"), Some("http.route"));
        assert_eq!(allowed("auth.method"), Some("vpay.auth.method"));
        for denied in ["path", "request_id", "method", "msisdn", "error", "owner"] {
            assert_eq!(allowed(denied), None, "{denied}");
        }
    }
}
