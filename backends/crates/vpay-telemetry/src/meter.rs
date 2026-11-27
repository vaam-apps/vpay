//! vpay's `metrics` facade as OpenTelemetry instruments.
//!
//! vpay records through the `metrics` crate (`vpay_core::metrics`), and a binary installs one recorder. [`Fanout`]
//! is that recorder when telemetry is on: it hands every registration to the Prometheus exporter (the `/metrics`
//! route keeps working exactly as before) and to the [`Bridge`], which adds each increment and observation to the
//! SDK's instrument of the same name.
//!
//! **What the bridge lets through is closed at both ends.** A metric whose name is not in
//! [`vpay_core::metrics::ALL`] is not forwarded, and a label whose key is not in [`vpay_core::metrics::LABELS`] is
//! dropped from the series (the series is still counted, without it). The vocabulary holds no id, amount, phone
//! number or secret, and a test holds it to that. The number of distinct series per process is capped
//! ([`MAX_SERIES`]); a series past the cap is not forwarded and its registration is counted in `vpay_telemetry_dropped_total{signal="metrics"}`,
//! straight on the OpenTelemetry instrument (never through the facade, which would register another series).
//! A handle is made once per series and handed back on every later registration of it.
//! Names, units and descriptions are the facade's own (`describe_*`), so the dashboards' names do not depend on this
//! file.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use metrics::{
    Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SharedString, Unit,
};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{
    Counter as OtelCounter, Gauge as OtelGauge, Histogram as OtelHistogram, Meter,
};

use vpay_core::metrics::{ALL, LABELS, TELEMETRY_DROPPED_TOTAL};

/// The most distinct series (a name with its labels) the bridge will hold.
pub const MAX_SERIES: usize = 4096;

/// Seconds, for every histogram whose name ends in `_seconds`: a payment rail is slow in a way an HTTP route is not,
/// so the upper buckets reach a minute.
const SECONDS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

/// The OpenTelemetry unit of a facade unit. A count is *no* unit: a backend that turns an OpenTelemetry name into a
/// Prometheus one appends the unit it knows (`s` becomes `_seconds`) and would turn a unit of `count` into a
/// `_count` suffix, which is the name of a histogram's own series. Seconds are the only unit this vocabulary has.
fn otel_unit(unit: Option<Unit>) -> &'static str {
    match unit {
        Some(Unit::Seconds) => "s",
        _ => "",
    }
}

fn attributes(key: &Key) -> Vec<KeyValue> {
    key.labels()
        .filter(|label| LABELS.contains(&label.key()))
        .map(|label| KeyValue::new(label.key().to_owned(), label.value().to_owned()))
        .collect()
}

struct Descriptions {
    by_name: HashMap<String, (Option<Unit>, SharedString)>,
}

/// The forwarding half: OpenTelemetry instruments made on one meter, one per metric name.
pub struct Bridge {
    meter: Meter,
    described: Mutex<Descriptions>,
    counters: Mutex<HashMap<String, OtelCounter<u64>>>,
    gauges: Mutex<HashMap<String, OtelGauge<f64>>>,
    histograms: Mutex<HashMap<String, OtelHistogram<f64>>>,
    handles: Mutex<SeriesHandles>,
}

/// The facade handles already made, one per series (a name with its labels). The facade asks the recorder for a
/// handle on every `counter!`, `gauge!` or `histogram!` call, and a gauge's `increment` and a counter's `absolute`
/// keep their state in the handle: giving back the same handle for the same series is what makes them right across
/// calls. Their number is the cap on series.
#[derive(Default)]
struct SeriesHandles {
    counters: HashMap<Key, Counter>,
    gauges: HashMap<Key, Gauge>,
    histograms: HashMap<Key, Histogram>,
}

impl SeriesHandles {
    fn len(&self) -> usize {
        self.counters.len() + self.gauges.len() + self.histograms.len()
    }
}

impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge").finish_non_exhaustive()
    }
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Bridge {
    /// A bridge that makes its instruments on `meter`.
    #[must_use]
    pub fn new(meter: Meter) -> Self {
        // The counter of dropped series is made here and counted on directly, never through the facade: a drop
        // counted through `metrics::counter!` would register a series, which is the thing that was just refused.
        // `counter()` finds this same instrument by name for the other signals' drops.
        let dropped = meter
            .u64_counter(TELEMETRY_DROPPED_TOTAL)
            .with_description(
                "Spans, logs and metric series the OpenTelemetry exporter dropped, by signal.",
            )
            .build();
        Self {
            meter,
            described: Mutex::new(Descriptions {
                by_name: HashMap::new(),
            }),
            counters: Mutex::new(HashMap::from([(
                TELEMETRY_DROPPED_TOTAL.to_owned(),
                dropped,
            )])),
            gauges: Mutex::default(),
            histograms: Mutex::default(),
            handles: Mutex::default(),
        }
    }

    /// Counts a series refused at the cap (one per registration of it), straight on the instrument.
    fn count_dropped_series(&self) {
        if let Some(instrument) = locked(&self.counters).get(TELEMETRY_DROPPED_TOTAL) {
            instrument.add(1, &[KeyValue::new("signal", "metrics")]);
        }
    }

    fn describe(&self, name: &KeyName, unit: Option<Unit>, description: SharedString) {
        locked(&self.described)
            .by_name
            .insert(name.as_str().to_owned(), (unit, description));
    }

    fn description(&self, name: &str) -> (String, String) {
        let described = locked(&self.described);
        let (unit, text) = described
            .by_name
            .get(name)
            .map_or((None, ""), |(unit, text)| (*unit, text.as_ref()));
        (otel_unit(unit).to_owned(), text.to_owned())
    }

    /// Whether a series of this key may be forwarded: its name is in the vocabulary.
    fn in_vocabulary(key: &Key) -> bool {
        ALL.contains(&key.name())
    }

    fn counter(&self, key: &Key) -> Counter {
        if !Self::in_vocabulary(key) {
            return Counter::noop();
        }
        let mut handles = locked(&self.handles);
        if let Some(handle) = handles.counters.get(key) {
            return handle.clone();
        }
        if handles.len() >= MAX_SERIES {
            drop(handles);
            self.count_dropped_series();
            return Counter::noop();
        }
        let name = key.name().to_owned();
        let instrument = {
            let mut counters = locked(&self.counters);
            counters
                .entry(name.clone())
                .or_insert_with(|| {
                    let (unit, text) = self.description(&name);
                    self.meter
                        .u64_counter(name.clone())
                        .with_description(text)
                        .with_unit(unit)
                        .build()
                })
                .clone()
        };
        let handle = Counter::from_arc(Arc::new(CounterHandle {
            instrument,
            attributes: attributes(key),
            last_absolute: AtomicU64::new(0),
        }));
        handles.counters.insert(key.clone(), handle.clone());
        handle
    }

    fn gauge(&self, key: &Key) -> Gauge {
        if !Self::in_vocabulary(key) {
            return Gauge::noop();
        }
        let mut handles = locked(&self.handles);
        if let Some(handle) = handles.gauges.get(key) {
            return handle.clone();
        }
        if handles.len() >= MAX_SERIES {
            drop(handles);
            self.count_dropped_series();
            return Gauge::noop();
        }
        let name = key.name().to_owned();
        let instrument = {
            let mut gauges = locked(&self.gauges);
            gauges
                .entry(name.clone())
                .or_insert_with(|| {
                    let (unit, text) = self.description(&name);
                    self.meter
                        .f64_gauge(name.clone())
                        .with_description(text)
                        .with_unit(unit)
                        .build()
                })
                .clone()
        };
        let handle = Gauge::from_arc(Arc::new(GaugeHandle {
            instrument,
            attributes: attributes(key),
            current: AtomicU64::new(0f64.to_bits()),
        }));
        handles.gauges.insert(key.clone(), handle.clone());
        handle
    }

    fn histogram(&self, key: &Key) -> Histogram {
        if !Self::in_vocabulary(key) {
            return Histogram::noop();
        }
        let mut handles = locked(&self.handles);
        if let Some(handle) = handles.histograms.get(key) {
            return handle.clone();
        }
        if handles.len() >= MAX_SERIES {
            drop(handles);
            self.count_dropped_series();
            return Histogram::noop();
        }
        let name = key.name().to_owned();
        let instrument = {
            let mut histograms = locked(&self.histograms);
            histograms
                .entry(name.clone())
                .or_insert_with(|| {
                    let (unit, text) = self.description(&name);
                    let builder = self
                        .meter
                        .f64_histogram(name.clone())
                        .with_description(text)
                        .with_unit(unit);
                    if name.ends_with("_seconds") {
                        builder.with_boundaries(SECONDS.to_vec()).build()
                    } else {
                        builder.build()
                    }
                })
                .clone()
        };
        let handle = Histogram::from_arc(Arc::new(HistogramHandle {
            instrument,
            attributes: attributes(key),
        }));
        handles.histograms.insert(key.clone(), handle.clone());
        handle
    }
}

struct CounterHandle {
    instrument: OtelCounter<u64>,
    attributes: Vec<KeyValue>,
    last_absolute: AtomicU64,
}

impl CounterFn for CounterHandle {
    fn increment(&self, value: u64) {
        self.instrument.add(value, &self.attributes);
    }

    fn absolute(&self, value: u64) {
        // An absolute counter is a running total: forward what it grew by since the last call.
        let before = self.last_absolute.fetch_max(value, Ordering::Relaxed);
        if value > before {
            self.instrument.add(value - before, &self.attributes);
        }
    }
}

struct GaugeHandle {
    instrument: OtelGauge<f64>,
    attributes: Vec<KeyValue>,
    current: AtomicU64,
}

impl GaugeHandle {
    fn update(&self, change: impl Fn(f64) -> f64) {
        let mut seen = self.current.load(Ordering::Relaxed);
        loop {
            let next = change(f64::from_bits(seen));
            match self.current.compare_exchange_weak(
                seen,
                next.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.instrument.record(next, &self.attributes);
                    return;
                }
                Err(now) => seen = now,
            }
        }
    }
}

impl GaugeFn for GaugeHandle {
    #[expect(
        clippy::float_arithmetic,
        reason = "a gauge's own delta, as the `metrics` facade defines it; the workspace rule is about money, which no gauge holds"
    )]
    fn increment(&self, value: f64) {
        self.update(|current| current + value);
    }

    #[expect(
        clippy::float_arithmetic,
        reason = "a gauge's own delta, as the `metrics` facade defines it; the workspace rule is about money, which no gauge holds"
    )]
    fn decrement(&self, value: f64) {
        self.update(|current| current - value);
    }

    fn set(&self, value: f64) {
        self.update(|_| value);
    }
}

struct HistogramHandle {
    instrument: OtelHistogram<f64>,
    attributes: Vec<KeyValue>,
}

impl HistogramFn for HistogramHandle {
    fn record(&self, value: f64) {
        self.instrument.record(value, &self.attributes);
    }
}

/// The process's `metrics` recorder: `primary` always, and the OpenTelemetry [`Bridge`] when metrics are on.
pub struct Fanout<R> {
    primary: R,
    bridge: Option<Arc<Bridge>>,
}

impl<R> std::fmt::Debug for Fanout<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fanout")
            .field("bridge", &self.bridge.is_some())
            .finish_non_exhaustive()
    }
}

impl<R> Fanout<R> {
    /// `primary` and, when given, `bridge`.
    #[must_use]
    pub const fn new(primary: R, bridge: Option<Arc<Bridge>>) -> Self {
        Self { primary, bridge }
    }
}

struct Both<T>(T, T);

impl CounterFn for Both<Counter> {
    fn increment(&self, value: u64) {
        self.0.increment(value);
        self.1.increment(value);
    }
    fn absolute(&self, value: u64) {
        self.0.absolute(value);
        self.1.absolute(value);
    }
}

impl GaugeFn for Both<Gauge> {
    fn increment(&self, value: f64) {
        self.0.increment(value);
        self.1.increment(value);
    }
    fn decrement(&self, value: f64) {
        self.0.decrement(value);
        self.1.decrement(value);
    }
    fn set(&self, value: f64) {
        self.0.set(value);
        self.1.set(value);
    }
}

impl HistogramFn for Both<Histogram> {
    fn record(&self, value: f64) {
        self.0.record(value);
        self.1.record(value);
    }
}

impl<R: Recorder> Recorder for Fanout<R> {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        if let Some(bridge) = &self.bridge {
            bridge.describe(&key, unit, description.clone());
        }
        self.primary.describe_counter(key, unit, description);
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        if let Some(bridge) = &self.bridge {
            bridge.describe(&key, unit, description.clone());
        }
        self.primary.describe_gauge(key, unit, description);
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        if let Some(bridge) = &self.bridge {
            bridge.describe(&key, unit, description.clone());
        }
        self.primary.describe_histogram(key, unit, description);
    }

    fn register_counter(&self, key: &Key, metadata: &Metadata<'_>) -> Counter {
        let first = self.primary.register_counter(key, metadata);
        match &self.bridge {
            Some(bridge) => Counter::from_arc(Arc::new(Both(first, bridge.counter(key)))),
            None => first,
        }
    }

    fn register_gauge(&self, key: &Key, metadata: &Metadata<'_>) -> Gauge {
        let first = self.primary.register_gauge(key, metadata);
        match &self.bridge {
            Some(bridge) => Gauge::from_arc(Arc::new(Both(first, bridge.gauge(key)))),
            None => first,
        }
    }

    fn register_histogram(&self, key: &Key, metadata: &Metadata<'_>) -> Histogram {
        let first = self.primary.register_histogram(key, metadata);
        match &self.bridge {
            Some(bridge) => Histogram::from_arc(Arc::new(Both(first, bridge.histogram(key)))),
            None => first,
        }
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use vpay_core::metrics::{HTTP_REQUESTS_TOTAL, JOBS_OLDEST_CLAIMABLE_AGE_SECONDS};

    struct Rig {
        provider: SdkMeterProvider,
        exporter: InMemoryMetricExporter,
        fanout: Fanout<metrics_exporter_prometheus::PrometheusRecorder>,
    }

    fn rig() -> Rig {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone())
            .with_interval(std::time::Duration::from_secs(3600))
            .build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        let bridge = Arc::new(Bridge::new(provider.meter("test")));
        let primary = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        Rig {
            provider,
            exporter,
            fanout: Fanout::new(primary, Some(bridge)),
        }
    }

    impl Rig {
        /// The data points of the newest export of `name`: its attributes as `k=v,...` and its value.
        fn points(&self, name: &str) -> Vec<(String, f64)> {
            let _ = self.provider.force_flush();
            let all = self.exporter.get_finished_metrics().unwrap();
            let Some(last) = all.last() else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for scope in last.scope_metrics() {
                for metric in scope.metrics().filter(|m| m.name() == name) {
                    match metric.data() {
                        AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                            for p in sum.data_points() {
                                out.push((attrs(p.attributes()), p.value() as f64));
                            }
                        }
                        AggregatedMetrics::F64(MetricData::Gauge(gauge)) => {
                            for p in gauge.data_points() {
                                out.push((attrs(p.attributes()), p.value()));
                            }
                        }
                        other => panic!("unexpected data for {name}: {other:?}"),
                    }
                }
            }
            out
        }
    }

    fn attrs<'a>(it: impl Iterator<Item = &'a KeyValue>) -> String {
        let mut v: Vec<String> = it.map(|kv| format!("{}={}", kv.key, kv.value)).collect();
        v.sort();
        v.join(",")
    }

    #[test]
    fn series_past_the_cap_are_dropped_and_counted_without_recursing() {
        let rig = rig();
        let extra = 50;
        metrics::with_local_recorder(&rig.fanout, || {
            for i in 0..(MAX_SERIES + extra) {
                metrics::counter!(HTTP_REQUESTS_TOTAL, "route" => format!("/r{i}")).increment(1);
            }
            // A series already held still works at the cap, and a refused one stays refused.
            metrics::counter!(HTTP_REQUESTS_TOTAL, "route" => "/r0").increment(1);
            metrics::counter!(HTTP_REQUESTS_TOTAL, "route" => format!("/r{}", MAX_SERIES + 1))
                .increment(1);
        });
        let requests = rig.points(HTTP_REQUESTS_TOTAL);
        // The SDK has a cardinality limit of its own per instrument (2000, plus an overflow series), lower than ours.
        assert!(requests.len() <= MAX_SERIES, "the cap holds");
        assert!(
            requests.iter().any(|(a, v)| a == "route=/r0" && *v >= 2.0),
            "a held series still counts at the cap"
        );
        let dropped = rig.points(TELEMETRY_DROPPED_TOTAL);
        assert_eq!(
            dropped,
            vec![("signal=metrics".to_owned(), (extra + 1) as f64)],
            "{extra} refused series and one registered again, counted and exported"
        );
    }

    #[test]
    fn a_gauge_increment_accumulates_across_calls() {
        let rig = rig();
        metrics::with_local_recorder(&rig.fanout, || {
            metrics::gauge!(JOBS_OLDEST_CLAIMABLE_AGE_SECONDS).set(10.0);
            metrics::gauge!(JOBS_OLDEST_CLAIMABLE_AGE_SECONDS).increment(2.5);
            metrics::gauge!(JOBS_OLDEST_CLAIMABLE_AGE_SECONDS).increment(2.5);
            metrics::gauge!(JOBS_OLDEST_CLAIMABLE_AGE_SECONDS).decrement(1.0);
        });
        assert_eq!(
            rig.points(JOBS_OLDEST_CLAIMABLE_AGE_SECONDS),
            vec![(String::new(), 14.0)]
        );
    }

    #[test]
    fn a_counter_absolute_forwards_only_growth_across_calls() {
        let rig = rig();
        metrics::with_local_recorder(&rig.fanout, || {
            metrics::counter!(HTTP_REQUESTS_TOTAL).absolute(5);
            metrics::counter!(HTTP_REQUESTS_TOTAL).absolute(5);
            metrics::counter!(HTTP_REQUESTS_TOTAL).absolute(8);
            // A lower total is a reset of the source, not a negative increment.
            metrics::counter!(HTTP_REQUESTS_TOTAL).absolute(3);
        });
        assert_eq!(rig.points(HTTP_REQUESTS_TOTAL), vec![(String::new(), 8.0)]);
    }
}
