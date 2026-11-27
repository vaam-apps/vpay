# vpay-telemetry

OpenTelemetry for vpay: **metrics, traces and logs through the OpenTelemetry SDK over OTLP/HTTP with JSON bodies, off
unless an endpoint is set.** [ADR-0039](../../../docs/adr/0039-opentelemetry-through-the-sdk-and-generated-dashboards.md)
says why it is built this way. The dashboards that read what it exports are in
[`deploy/dashboards`](../../../deploy/dashboards/README.md).

Only `vpay-server` uses it, in its two long-running modes (`serve` and `worker`); `staff` and `callbacks` never open a
connection to a collector. Everything else records through the `metrics` and `tracing` facades as before.

## What exists, and what does not

_Verified 2026-11-27 by reading the `=0.20.4` sources of CrateStack in the cargo registry and the crates.io listing:_
CrateStack has no OpenTelemetry exporter. `cratestack-service` has a `tracing_subscriber` formatter and nothing else, so
this crate uses the SDK, as the owner decided ("we'll use opentelemetry libs", 2026-11-27). vpay's `/v1` is served by
its own router, whose request span (`vpay_api::make_request_span`) is the one span exported.

| Signal  | What goes out                                                                                                                               | Where it is made                                    |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| Metrics | The 21 names of `vpay_core::metrics::ALL`, with the label keys of `LABELS`                                                                  | `meter::Fanout`, the process's `metrics` recorder   |
| Traces  | One server span per HTTP request: `http.request.method`, `http.route`, `http.response.status_code`, `vpay.auth.method`; error status on 5xx | `tracing-opentelemetry`, cleaned in `filter::clean` |
| Logs    | `INFO`-and-above events of `vpay*` targets whose message is plain words                                                                     | `layer::LogLayer`                                   |

**Not built yet** (a follow-up, not a claim): spans for the worker's jobs, a span per rail call, a log of every event
with its fields (only the allowlisted ones leave, and today almost no event has one), the intent-level state counts (see
below), and the Helm chart's wiring of the collector address and the headers file.

## Configuration

The standard `OTEL_*` variables, a strict subset. **No endpoint, no telemetry**: nothing is built, no thread is started,
no connection is opened, and the series `vpay_telemetry_*` do not exist.

| Variable                                                                               | Meaning                                                                                                                                                                               |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `OTEL_EXPORTER_OTLP_ENDPOINT`                                                          | The collector's base URL; `/v1/traces`, `/v1/metrics`, `/v1/logs` are appended. `OTEL_EXPORTER_OTLP_{TRACES,METRICS,LOGS}_ENDPOINT` give a full URL for one signal                    |
| `VPAY_OTEL_HEADERS_FILE`                                                               | A file of `Name: value` lines sent with every export. https, or this machine only                                                                                                     |
| `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`                                        | `service.name` (default `vpay`); `deployment.environment.name` (or `deployment.environment`) and `service.instance.id` are read, other keys ignored. `service.version` is the release |
| `OTEL_TRACES_SAMPLER_ARG`, `OTEL_TRACES_SAMPLER`                                       | The share of traces kept, 0 to 1 (default 1), decided from the trace id at the root; `always_on`, `always_off`, `traceidratio` and the `parentbased_` forms                           |
| `OTEL_METRIC_EXPORT_INTERVAL`, `OTEL_BSP_SCHEDULE_DELAY`, `OTEL_EXPORTER_OTLP_TIMEOUT` | Milliseconds: 15000, 5000 and 10000 by default                                                                                                                                        |
| `OTEL_TRACES_EXPORTER`, `OTEL_METRICS_EXPORTER`, `OTEL_LOGS_EXPORTER`                  | `otlp` or `none`                                                                                                                                                                      |
| `OTEL_SDK_DISABLED`                                                                    | `true` turns everything off                                                                                                                                                           |
| `OTEL_EXPORTER_OTLP_CERTIFICATE`                                                       | A PEM CA bundle; the only trust for an `https` collector                                                                                                                              |

A signal is on when it has a URL and is not `none`.

**`service.instance.id`** is `OTEL_RESOURCE_ATTRIBUTES`' value when set, else the `HOSTNAME` environment variable (a
pod's name), else a UUID generated at start, so two replicas never share an identity and a metrics store never merges
their series. The UUID changes at every restart; set the attribute or `HOSTNAME` where series should survive one.

**Refused at start (exit 78), with a message that names the variable and never its value:** every variable of the
specification that the SDK would obey and this exporter does not set itself (`settings::UNSUPPORTED`: per-signal
`*_HEADERS`, `*_TIMEOUT`, `*_COMPRESSION` and `*_CERTIFICATE`, `OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE`,
`OTEL_SPAN_EVENT_COUNT_LIMIT`, `OTEL_SPAN_LINK_COUNT_LIMIT`, the client key and certificate, `OTEL_BLRP_*` and the other
`OTEL_BSP_*`, the attribute limits, `OTEL_PROPAGATORS`), a protocol other than `http/json`, `OTEL_EXPORTER_OTLP_HEADERS`
(a credential in an environment variable shows in a process listing: use the file), an endpoint with a user, a password
or a query, and headers over plain `http` to another host. The SDK reads some `OTEL_*` variables itself when a builder
option is unset, so each option it has is set explicitly in `export.rs` (temporality is `Cumulative`) and the rest are
refused. `settings::tests::what_is_not_honoured_is_refused_and_what_is_honoured_is_read` holds the list.

## What leaves the process

**Metrics.** `meter::Fanout` is the recorder of the process: the Prometheus exporter first (`/metrics` is unchanged, and
still has everything), and the `meter::Bridge` second. The bridge forwards a series only if its **name is in
`vpay_core::metrics::ALL`** and only the **label keys in `vpay_core::metrics::LABELS`**; a test
(`every_documented_label_is_in_the_label_list`) holds `LABELS` to the labels the vocabulary documents, so a label cannot be
added to the listener and leak to the collector by accident. Every value of those labels is a closed set (a route pattern,
a rail code, a state word, an error code); none is an id, an amount, a phone number or a secret. At most 4096 distinct series
are forwarded; the rest are dropped and counted (`vpay_telemetry_dropped_total{signal="metrics"}`).

**Spans.** `tracing-opentelemetry` records every field of a span, so **each finished span is cleaned at the exporter**
(`filter::clean`): only the fields of `filter::FIELDS` survive, whatever their value (the request span's `path`, `request_id`
and raw `method` never do); span events, links and the status message never leave; a text value is held to a plain alphabet
and 64 bytes (`filter::plain`, else `redacted`); the span name is held to the same. An inbound `traceparent` is **not** read:
a caller does not choose the trace of its request. Only spans and events whose target starts with `vpay`, at `INFO` or
above, are looked at, by a per-layer filter, so the `TRACE` callsites of sqlx, hyper and rustls are never dispatched
(`foreign_and_verbose_callsites_are_never_dispatched_*` hold it).

**Logs.** An `INFO`-or-above event of a `vpay*` target is exported only with a message of **words of letters only**
(`filter::plain_message`: `drain failed (unavailable).` passes; anything with a digit in it or over 128 bytes is exported
as `redacted`), only the allowlisted fields, and no `Debug` value. **Most of vpay's log lines carry ids, so most are
`redacted` today:** the log signal is honest and nearly empty until the messages are written for it.

**Recording never blocks.** Spans and logs wait in a bounded queue (4096 and a batch of 512); a full one drops and counts
(`vpay_telemetry_dropped_total{signal}`), and the SDK exports from threads of its own. A failed export is retried by the
exporter and then dropped, and counted (`vpay_telemetry_exports_total{signal,result}`). `Guard::shutdown(deadline)` flushes
on a thread of its own, so a collector that never answers costs the deadline and nothing more (`tests/hang.rs`).

## Wiring

```rust
let guard = vpay_telemetry::start(&settings)?;                       // builds the providers; installs nothing
metrics::set_global_recorder(guard.recorder(prometheus_recorder))?;  // the fan-out
let layers = guard.layer();                                          // None when no signal is on
// add `layers` beside the console layer, each with its own filter, never under a global one
guard.shutdown(Duration::from_secs(5)).await;                        // on every way out
```

`vpay-server`'s `install_process_defaults` is the one caller. A layer that is absent is **not** added as `None`: a `None`
layer has no filter and would make every callsite of every crate interesting to the registry.

## Layout

| File              | What                                                                                                                                      |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `src/settings.rs` | `Settings::from_lookup`: the variables, their refusals, the resource                                                                      |
| `src/filter.rs`   | The allowlist (`FIELDS`, `clean`), the text rules, and the wrappers around the SDK's processors and exporters that count, bound and clean |
| `src/layer.rs`    | `LogLayer`                                                                                                                                |
| `src/meter.rs`    | `Bridge` (the `metrics` facade into the SDK's instruments) and `Fanout` (the recorder)                                                    |
| `src/export.rs`   | `start`, `Guard::{layer, recorder, shutdown}`: the tracer, meter and logger providers over one `reqwest` client                           |
| `src/testkit.rs`  | Feature `testkit`: a collector in the process (`TestCollector`), in no image                                                              |

## Dependencies

_Verified 2026-11-27 by `cargo tree` on this workspace:_ `opentelemetry` 0.33.1, `opentelemetry_sdk` 0.33.1,
`opentelemetry-otlp` 0.33.1 (features `http-json` and `reqwest-blocking-client`: message types and a blocking client, no
`tonic`) and `tracing-opentelemetry` 0.34.0, which move together. The HTTP client is the workspace's `reqwest` 0.13 on
rustls with ring, built by this crate and handed to each exporter (the SDK's own client builder is not used), so there is
one `reqwest`, no `aws-lc-rs`, no OpenSSL and no `tonic`. `cargo deny check` is part of CI and decides; the result for this
change is in the ADR.

## Tests

`cargo test -p vpay-telemetry`: the settings and their refusals; the pipeline against in-memory exporters
(`src/pipeline_tests.rs`: the allowlist, parents, a 5xx marking its span, sampling, a full queue dropping and counting, and
that a foreign or verbose callsite is never dispatched); the exporter against a collector in the process (`tests/export.rs`:
what arrives, the headers, the resource, the exporter's own metrics, a name and a label outside the vocabulary not
arriving, and no byte of a sentinel); a collector that never answers (`tests/hang.rs`); and the dashboards against the
vocabulary (`tests/dashboards.rs`).
