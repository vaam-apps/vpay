//! Starts the OpenTelemetry SDK: a tracer provider, a meter provider and a logger provider, each exporting over
//! OTLP/HTTP with JSON bodies through one caller-supplied `reqwest` client (rustls on ring; the CA bundle of
//! `OTEL_EXPORTER_OTLP_CERTIFICATE` is then the only trust), and installs the `tracing` subscriber that feeds them.
//!
//! Recording never blocks: spans and logs wait in a bounded queue (a full one drops and counts, [`crate::filter`]),
//! and the SDK exports from threads of its own. A failed export is retried by the exporter and then dropped: the
//! telemetry is never the reason a request waits.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use opentelemetry::metrics::MeterProvider;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, logs::LoggerProvider};
use opentelemetry_otlp::{
    LogExporter, MetricExporter, Protocol, SpanExporter, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::Resource as SdkResource;
use opentelemetry_sdk::logs::{
    BatchConfigBuilder as LogBatchConfig, BatchLogProcessor, SdkLoggerProvider,
};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider, Temporality};
use opentelemetry_sdk::trace::{
    BatchConfigBuilder as SpanBatchConfig, BatchSpanProcessor, Sampler, SdkTracerProvider,
};
use reqwest::header::{HeaderName, HeaderValue};
use tracing::Metadata;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::registry::LookupSpan;

use crate::filter::{
    BoundedLogs, BoundedSpans, CAP, GuardedLogs, GuardedMetrics, GuardedSpans, Tally, ours,
};
use crate::layer::LogLayer;
use crate::meter::Bridge;
use crate::settings::{Resource, Settings};

pub use crate::filter::Stats;

/// Spans (and, apart, logs) that may wait for export, in the queue and in flight; a new one past it is dropped.
const QUEUE: usize = 4096;
/// The most items in one export request.
const BATCH: usize = 512;
/// The name of the instrumentation scope.
const SCOPE: &str = "vpay";

/// Why telemetry did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// A header of `VPAY_OTEL_HEADERS_FILE` is not a valid HTTP header.
    #[error("VPAY_OTEL_HEADERS_FILE holds a header that is not valid")]
    Header,
    /// The HTTP client could not be built.
    #[error("the OTLP client could not be built")]
    Client,
    /// `OTEL_EXPORTER_OTLP_CERTIFICATE` does not hold a PEM certificate.
    #[error("OTEL_EXPORTER_OTLP_CERTIFICATE does not hold a PEM certificate")]
    Certificate,
    /// An OTLP exporter could not be built.
    #[error("an OTLP exporter could not be built")]
    Exporter,
}

impl vpay_core::error::Classify for StartError {
    fn category(&self) -> vpay_core::error::Category {
        match self {
            Self::Header | Self::Certificate => vpay_core::error::Category::Configuration,
            Self::Client | Self::Exporter => vpay_core::error::Category::Internal,
        }
    }
}

fn client(settings: &Settings) -> Result<reqwest::blocking::Client, StartError> {
    // reqwest is built with no crypto provider of its own: rustls uses the process default, ring.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(settings.timeout);
    if let Some(pem) = &settings.ca_pem {
        // The CA bundle is the only trust, as `OTEL_EXPORTER_OTLP_CERTIFICATE` says.
        let certificates =
            reqwest::Certificate::from_pem_bundle(pem).map_err(|_| StartError::Certificate)?;
        builder = builder.tls_certs_only(certificates);
    }
    builder.build().map_err(|_| StartError::Client)
}

fn headers(settings: &Settings) -> Result<HashMap<String, String>, StartError> {
    let mut out = HashMap::new();
    for (name, value) in &settings.headers {
        HeaderName::from_bytes(name.as_bytes()).map_err(|_| StartError::Header)?;
        HeaderValue::from_str(value).map_err(|_| StartError::Header)?;
        out.insert(name.clone(), value.clone());
    }
    Ok(out)
}

fn resource(resource: &Resource) -> SdkResource {
    let mut attributes = vec![
        KeyValue::new("service.name", resource.service_name.clone()),
        KeyValue::new("service.version", resource.service_version.clone()),
    ];
    if let Some(environment) = &resource.environment {
        attributes.push(KeyValue::new(
            "deployment.environment.name",
            environment.clone(),
        ));
    }
    if let Some(instance) = &resource.instance {
        attributes.push(KeyValue::new("service.instance.id", instance.clone()));
    }
    // No detectors: the resource is exactly what the settings say, never read from the environment by the SDK.
    SdkResource::builder_empty()
        .with_attributes(attributes)
        .build()
}

/// What `build` made, to be installed on the thread that called `start`.
pub(crate) struct Providers {
    pub(crate) tracer: Option<SdkTracerProvider>,
    pub(crate) meter: Option<SdkMeterProvider>,
    pub(crate) logger: Option<SdkLoggerProvider>,
    pub(crate) bridge: Option<Arc<Bridge>>,
}

impl Providers {
    /// Shuts the three providers down at once, each with the same `deadline`: the whole costs one deadline, not
    /// three in a row.
    fn shutdown(&self, deadline: Duration) {
        std::thread::scope(|scope| {
            if let Some(p) = &self.tracer {
                scope.spawn(move || p.shutdown_with_timeout(deadline));
            }
            if let Some(p) = &self.logger {
                scope.spawn(move || p.shutdown_with_timeout(deadline));
            }
            if let Some(p) = &self.meter {
                scope.spawn(move || p.shutdown_with_timeout(deadline));
            }
        });
    }
}

#[allow(clippy::too_many_lines)]
fn build(settings: &Settings, tally: &Tally) -> Result<Providers, StartError> {
    let http = client(settings)?;
    let headers = headers(settings)?;
    let resource = resource(&settings.resource);

    let tracer = match settings
        .traces_url
        .as_ref()
        .filter(|_| settings.signals.traces)
    {
        None => None,
        Some(url) => {
            let exporter = SpanExporter::builder()
                .with_http()
                .with_http_client(http.clone())
                .with_endpoint(url.as_str())
                .with_protocol(Protocol::HttpJson)
                .with_timeout(settings.timeout)
                .with_headers(headers.clone())
                .build()
                .map_err(|_| StartError::Exporter)?;
            let pending = Arc::new(AtomicUsize::new(0));
            let exporter = GuardedSpans::new(exporter, Arc::clone(&pending), tally.clone());
            let config = SpanBatchConfig::default()
                .with_max_queue_size(QUEUE + BATCH)
                .with_max_export_batch_size(BATCH)
                .with_scheduled_delay(settings.batch_delay)
                .build();
            let batch = BatchSpanProcessor::builder(exporter)
                .with_batch_config(config)
                .build();
            let sampler = if settings.sample_ratio >= 1.0 {
                Sampler::AlwaysOn
            } else {
                Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(settings.sample_ratio)))
            };
            Some(
                SdkTracerProvider::builder()
                    .with_span_processor(BoundedSpans::new(batch, pending, QUEUE, tally.clone()))
                    .with_sampler(sampler)
                    .with_resource(resource.clone())
                    .build(),
            )
        }
    };

    let logger = match settings.logs_url.as_ref().filter(|_| settings.signals.logs) {
        None => None,
        Some(url) => {
            let exporter = LogExporter::builder()
                .with_http()
                .with_http_client(http.clone())
                .with_endpoint(url.as_str())
                .with_protocol(Protocol::HttpJson)
                .with_timeout(settings.timeout)
                .with_headers(headers.clone())
                .build()
                .map_err(|_| StartError::Exporter)?;
            let pending = Arc::new(AtomicUsize::new(0));
            let exporter = GuardedLogs::new(exporter, Arc::clone(&pending), tally.clone());
            let config = LogBatchConfig::default()
                .with_max_queue_size(QUEUE + BATCH)
                .with_max_export_batch_size(BATCH)
                .with_scheduled_delay(settings.batch_delay)
                .build();
            let batch = BatchLogProcessor::builder(exporter)
                .with_batch_config(config)
                .build();
            Some(
                SdkLoggerProvider::builder()
                    .with_log_processor(BoundedLogs::new(batch, pending, QUEUE, tally.clone()))
                    .with_resource(resource.clone())
                    .build(),
            )
        }
    };

    let mut bridge = None;
    let meter = match settings
        .metrics_url
        .as_ref()
        .filter(|_| settings.signals.metrics)
    {
        None => None,
        Some(url) => {
            let exporter = MetricExporter::builder()
                .with_http()
                .with_http_client(http)
                // Explicit: the exporter would otherwise read OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE.
                .with_temporality(Temporality::Cumulative)
                .with_endpoint(url.as_str())
                .with_protocol(Protocol::HttpJson)
                .with_timeout(settings.timeout)
                .with_headers(headers)
                .build()
                .map_err(|_| StartError::Exporter)?;
            let reader = PeriodicReader::builder(GuardedMetrics::new(exporter, tally.clone()))
                .with_interval(settings.metric_interval)
                .build();
            let provider = SdkMeterProvider::builder()
                .with_reader(reader)
                .with_resource(resource)
                .build();
            bridge = Some(Arc::new(Bridge::new(provider.meter(SCOPE))));
            Some(provider)
        }
    };

    Ok(Providers {
        tracer,
        meter,
        logger,
        bridge,
    })
}

/// The running exporter. Dropping it without [`Guard::shutdown`] (an error return that skipped it) still flushes, but
/// blocks the dropping thread for at most two seconds; a caller that can await should call [`Guard::shutdown`] on its
/// normal way out: it flushes the queues and the metrics on a thread of its own, with the deadline it chooses.
pub struct Guard {
    /// `None` once shut down, so that dropping a shut-down guard does nothing.
    providers: Option<Providers>,
    stats: Arc<Stats>,
}

/// How long dropping a guard that was not shut down waits for the final flush.
const DROP_FLUSH: Duration = Duration::from_secs(2);

impl std::fmt::Debug for Guard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard")
            .field(
                "traces",
                &self.providers.as_ref().is_some_and(|p| p.tracer.is_some()),
            )
            .field(
                "metrics",
                &self.providers.as_ref().is_some_and(|p| p.meter.is_some()),
            )
            .field(
                "logs",
                &self.providers.as_ref().is_some_and(|p| p.logger.is_some()),
            )
            .finish_non_exhaustive()
    }
}

impl Guard {
    #[cfg(test)]
    pub(crate) fn for_test(providers: Providers) -> Self {
        Self {
            providers: Some(providers),
            stats: Arc::new(Stats::default()),
        }
    }

    /// What the exporter did.
    #[must_use]
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Spans and logs dropped because the queue was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.stats.dropped.load(Ordering::Relaxed)
    }

    /// Flushes everything queued and the metrics one last time, waiting at most `deadline`. The flush runs on a
    /// thread of its own, so a collector that never answers costs the deadline and nothing more.
    pub async fn shutdown(mut self, deadline: Duration) {
        let Some(providers) = self.providers.take() else {
            return;
        };
        let (done, wait) = tokio::sync::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("vpay-telemetry-shutdown".to_owned())
            .spawn(move || {
                providers.shutdown(deadline);
                let _ = done.send(());
            });
        if spawned.is_ok() {
            let _ = tokio::time::timeout(deadline, wait).await;
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // A way out that skipped `shutdown` (an error returned with `?`): flush anyway, bounded.
        let Some(providers) = self.providers.take() else {
            return;
        };
        let (done, wait) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("vpay-telemetry-drop".to_owned())
            .spawn(move || {
                providers.shutdown(DROP_FLUSH);
                let _ = done.send(());
            });
        if spawned.is_ok() {
            let _ = wait.recv_timeout(DROP_FLUSH);
        }
    }
}

fn spans_and_logs(meta: &Metadata<'_>) -> bool {
    ours(meta.target()) && *meta.level() <= CAP
}

impl Guard {
    /// The `tracing` layers of these providers, to be added to the process's subscriber: the spans layer when traces
    /// are on and the log layer when logs are on. `None` when neither is. Each has its own filter, so a callsite of
    /// any other target, or below `INFO`, is `Interest::never` for it and is never dispatched.
    ///
    /// The layers filter themselves: add them *beside* the process's own layers, each with its own filter, never
    /// under a global one (a global `EnvFilter` set to `warn` would silence the spans).
    #[must_use]
    pub fn layer<S>(&self) -> Option<Box<dyn Layer<S> + Send + Sync>>
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
    {
        let providers = self.providers.as_ref()?;
        if providers.tracer.is_none() && providers.logger.is_none() {
            return None;
        }
        // An absent layer is `None` *inside* its filter, never a `None` layer: a `None` layer has no filter and would
        // leave every callsite of every crate interesting to the registry.
        let spans_on = providers.tracer.is_some();
        let spans = providers
            .tracer
            .as_ref()
            .map(|provider| {
                tracing_opentelemetry::layer()
                    .with_tracer(provider.tracer(SCOPE))
                    .with_location(false)
                    .with_threads(false)
                    .with_level(false)
                    .with_target(false)
                    .with_tracked_inactivity(false)
            })
            .with_filter(
                filter_fn(move |meta: &Metadata<'_>| spans_on && spans_and_logs(meta))
                    .with_max_level_hint(CAP),
            );
        let logs_on = providers.logger.is_some();
        let logs = providers
            .logger
            .as_ref()
            .map(|provider| LogLayer::new(provider.logger(SCOPE)))
            .with_filter(
                filter_fn(move |meta: &Metadata<'_>| {
                    logs_on && meta.is_event() && ours(meta.target()) && *meta.level() <= CAP
                })
                .with_max_level_hint(CAP),
            );
        Some(Box::new(spans.and_then(logs)))
    }

    /// The recorder to install as the process's `metrics` recorder: `primary` (the Prometheus exporter) and, when
    /// metrics are on, the bridge into OpenTelemetry, which sees only the vocabulary of `vpay_core::metrics`.
    #[must_use]
    pub fn recorder<R>(&self, primary: R) -> crate::meter::Fanout<R>
    where
        R: metrics::Recorder + 'static,
    {
        crate::meter::Fanout::new(
            primary,
            self.providers.as_ref().and_then(|p| p.bridge.clone()),
        )
    }
}

/// Starts exporting: builds the providers and the exporters, on a thread of their own. It installs nothing:
/// the caller adds [`Guard::layer`] to its subscriber and [`Guard::recorder`] as its `metrics` recorder. Call it once.
///
/// # Errors
///
/// [`StartError`] for an invalid header, or a client or an exporter that cannot be built.
pub fn start(settings: &Settings) -> Result<Guard, StartError> {
    let stats = Arc::new(Stats::default());
    let tally = Tally::new(Arc::clone(&stats));
    // The blocking HTTP client and the SDK's threads are built off any async runtime this is called inside.
    let settings_for_build = settings.clone();
    let tally_for_build = tally.clone();
    let providers = std::thread::Builder::new()
        .name("vpay-telemetry-start".to_owned())
        .spawn(move || build(&settings_for_build, &tally_for_build))
        .map_err(|_| StartError::Client)?
        .join()
        .map_err(|_| StartError::Client)??;
    Ok(Guard {
        providers: Some(providers),
        stats,
    })
}
