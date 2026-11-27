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
//! ([`MAX_SERIES`]); a series past the cap is not forwarded and is counted in `vpay_telemetry_dropped_total`.
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
    series: Mutex<HashMap<Key, ()>>,
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
        Self {
            meter,
            described: Mutex::new(Descriptions {
                by_name: HashMap::new(),
            }),
            counters: Mutex::default(),
            gauges: Mutex::default(),
            histograms: Mutex::default(),
            series: Mutex::default(),
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

    /// Whether this series may be forwarded: its name is in the vocabulary, and the cap is not spent.
    fn admit(&self, key: &Key) -> bool {
        if !ALL.contains(&key.name()) {
            return false;
        }
        let mut series = locked(&self.series);
        if series.contains_key(key) {
            return true;
        }
        if series.len() >= MAX_SERIES {
            drop(series);
            // The vocabulary's own counter, through the facade: it is one series of a closed name.
            metrics::counter!(TELEMETRY_DROPPED_TOTAL, "signal" => "metrics").increment(1);
            return false;
        }
        series.insert(key.clone(), ());
        true
    }

    fn counter(&self, key: &Key) -> Counter {
        if !self.admit(key) {
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
        Counter::from_arc(Arc::new(CounterHandle {
            instrument,
            attributes: attributes(key),
            last_absolute: AtomicU64::new(0),
        }))
    }

    fn gauge(&self, key: &Key) -> Gauge {
        if !self.admit(key) {
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
        Gauge::from_arc(Arc::new(GaugeHandle {
            instrument,
            attributes: attributes(key),
            current: AtomicU64::new(0f64.to_bits()),
        }))
    }

    fn histogram(&self, key: &Key) -> Histogram {
        if !self.admit(key) {
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
        Histogram::from_arc(Arc::new(HistogramHandle {
            instrument,
            attributes: attributes(key),
        }))
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
