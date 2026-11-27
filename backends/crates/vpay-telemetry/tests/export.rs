//! The exporter against a collector in the process: what is recorded arrives as OTLP/JSON, with the headers from
//! the file, the resource, and nothing the allowlists drop. One test: it installs the process's recorder and
//! subscriber.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::time::Duration;

use tracing::{info, info_span};
use tracing_subscriber::layer::SubscriberExt;
use vpay_core::metrics::{
    HTTP_REQUESTS_TOTAL, JOBS_OLDEST_CLAIMABLE_AGE_SECONDS, PROVIDER_REQUEST_DURATION_SECONDS,
};
use vpay_telemetry::testkit::TestCollector;
use vpay_telemetry::{Settings, start};

#[tokio::test]
async fn spans_metrics_and_logs_reach_the_collector() {
    let collector = TestCollector::start().await;
    let vars: BTreeMap<&str, String> = BTreeMap::from([
        ("OTEL_EXPORTER_OTLP_ENDPOINT", collector.endpoint()),
        ("OTEL_BSP_SCHEDULE_DELAY", "50".to_owned()),
        ("OTEL_METRIC_EXPORT_INTERVAL", "50".to_owned()),
        (
            "OTEL_RESOURCE_ATTRIBUTES",
            "deployment.environment.name=ci,service.instance.id=pod-7".to_owned(),
        ),
        ("VPAY_OTEL_HEADERS_FILE", "headers".to_owned()),
    ]);
    let settings = Settings::from_lookup(
        |name| vars.get(name).cloned(),
        "9.9.9",
        |_| Some("X-Collector-Key: k-12345\n".to_owned()),
    )
    .unwrap()
    .unwrap();
    let guard = start(&settings).expect("starts");

    // What a binary does: the fan-out recorder (Prometheus first) and the layers beside its own.
    let prometheus = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let scrape = prometheus.handle();
    metrics::set_global_recorder(guard.recorder(prometheus)).expect("one recorder");
    vpay_core::metrics::describe_all();
    let layer = guard.layer().expect("signals are on");
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
        .expect("one subscriber");

    metrics::counter!(HTTP_REQUESTS_TOTAL, "route" => "/v1/payment_intents/{id}", "method" => "GET", "status" => "200")
        .increment(4);
    // A label key the vocabulary does not name, on a name it does: the series goes out without the label.
    metrics::counter!(HTTP_REQUESTS_TOTAL, "route" => "/healthz", "method" => "GET", "status" => "200", "msisdn" => "+237600000000")
        .increment(1);
    // A name outside the vocabulary never goes out (it is still on the Prometheus listener).
    metrics::counter!("vpay_not_in_the_vocabulary_total", "route" => "x").increment(9);
    metrics::gauge!(JOBS_OLDEST_CLAIMABLE_AGE_SECONDS).set(12.5);
    metrics::histogram!(PROVIDER_REQUEST_DURATION_SECONDS, "provider" => "mtn_momo", "operation" => "submit")
        .record(0.31);
    {
        let span = info_span!(
            target: "vpay_api",
            "request",
            http.request.method = "GET",
            path = "/v1/payment_intents/pi_must_not_leave",
            request_id = "req-must-not-leave",
        );
        let _in = span.enter();
        info!(target: "vpay_api", "a log line");
    }
    assert!(
        collector
            .wait_for(Duration::from_secs(5), |c| {
                !c.spans().is_empty() && !c.metrics().is_empty() && !c.logs().is_empty()
            })
            .await
    );
    // The exporter counts a request when the collector has answered it, a moment after the collector has the body.
    assert!(
        collector
            .wait_for(Duration::from_secs(5), |_| {
                guard
                    .stats()
                    .exported
                    .load(std::sync::atomic::Ordering::Relaxed)
                    >= 3
            })
            .await,
        "a request of each signal is counted"
    );
    guard.shutdown(Duration::from_secs(5)).await;

    let span = &collector.spans()[0];
    assert_eq!(span["name"], "request");
    assert_eq!(span["kind"], 2);
    let attributes: Vec<_> = span["attributes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(attributes, ["http.request.method"], "only the allowlist");
    assert_eq!(collector.logs()[0]["body"]["stringValue"], "a log line");

    let names: Vec<String> = collector
        .metrics()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_owned())
        .collect();
    for want in [
        HTTP_REQUESTS_TOTAL,
        JOBS_OLDEST_CLAIMABLE_AGE_SECONDS,
        PROVIDER_REQUEST_DURATION_SECONDS,
    ] {
        assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
    }
    assert!(
        !names
            .iter()
            .any(|n| n == "vpay_not_in_the_vocabulary_total")
    );
    // The exporter's own metrics: what it exported is counted, and the final flush carries them.
    assert!(
        names.iter().any(|n| n == "vpay_telemetry_exports_total"),
        "the exporter counts its own requests: {names:?}"
    );
    // The Prometheus listener still has everything, the name outside the vocabulary included.
    let exposition = scrape.render();
    assert!(exposition.contains("vpay_not_in_the_vocabulary_total"));
    assert!(exposition.contains("msisdn"), "the listener is unchanged");

    for resource in collector.resources() {
        let text = resource.to_string();
        for want in ["vpay", "9.9.9", "ci", "pod-7"] {
            assert!(text.contains(want), "{want} in {text}");
        }
    }
    assert!(
        collector
            .headers()
            .iter()
            .all(|h| h.get("x-collector-key").is_some_and(|v| v == "k-12345"))
    );
    let everything = collector.bodies().join("");
    for secret in [
        "pi_must_not_leave",
        "req-must-not-leave",
        "msisdn",
        "237600000000",
        "k-12345",
    ] {
        assert!(
            !everything.contains(secret),
            "{secret} reached the collector"
        );
    }
    // The histogram carries seconds-shaped buckets and the unit the Prometheus name expects.
    let histogram = collector
        .metrics()
        .into_iter()
        .find(|m| m["name"] == PROVIDER_REQUEST_DURATION_SECONDS)
        .unwrap();
    assert_eq!(histogram["unit"], "s");
    assert!(
        histogram["histogram"]["dataPoints"][0]["explicitBounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b == 60.0)
    );
}
