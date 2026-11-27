//! A collector that accepts and never answers: recording stays instant, `shutdown` returns by its deadline, and
//! the export that cannot finish is not waited for. One test: it installs the process's subscriber.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use tracing::info_span;
use vpay_telemetry::{Settings, start};

#[tokio::test]
async fn a_collector_that_never_answers_costs_telemetry_and_never_the_caller() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hold = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let vars: BTreeMap<&str, String> = BTreeMap::from([
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            format!("http://127.0.0.1:{port}"),
        ),
        ("OTEL_BSP_SCHEDULE_DELAY", "20".to_owned()),
        ("OTEL_METRIC_EXPORT_INTERVAL", "20".to_owned()),
    ]);
    let settings = Settings::from_lookup(|n| vars.get(n).cloned(), "1", |_| None)
        .unwrap()
        .unwrap();
    let guard = start(&settings).unwrap();

    // The layers are what a binary installs; the guard counts what they drop.
    let layer = guard.layer().expect("signals are on");
    tracing::subscriber::set_global_default(tracing_subscriber::layer::SubscriberExt::with(
        tracing_subscriber::registry(),
        layer,
    ))
    .unwrap();

    let began = Instant::now();
    for _ in 0..10_000 {
        let span = info_span!(target: "vpay_api", "request");
        drop(span.enter());
    }
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "recording never waits for the collector"
    );

    assert!(
        guard.dropped() > 0,
        "a queue that cannot drain drops and counts"
    );

    let stopping = Instant::now();
    guard.shutdown(Duration::from_millis(500)).await;
    assert!(
        stopping.elapsed() < Duration::from_secs(3),
        "shutdown ends by its deadline"
    );
    hold.abort();
}
