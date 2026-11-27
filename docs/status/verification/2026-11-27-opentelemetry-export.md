# ANR-1168: OpenTelemetry export, and the dashboards

_Written 2026-11-27 on `claude/anr-1168-otel`, branched from `origin/master` (`0eccf9f0`). What changed is in
[infrastructure.md](../infrastructure.md) (the OpenTelemetry row),
[ADR-0039](../../adr/0039-opentelemetry-through-the-sdk-and-generated-dashboards.md) and
[`vpay-telemetry`'s README](../../../backends/crates/vpay-telemetry/README.md); this page is what was run, and what was not._

## What changed

- New crate `vpay-telemetry`: OTLP/HTTP JSON export of metrics, one server span per request and plain-worded logs
  through the OpenTelemetry SDK, off unless an endpoint is set. `vpay-server` (serve and worker) starts it.
- `vpay_core::metrics`: six new names (21 in all) and the `LABELS` list the OpenTelemetry bridge forwards.
- `vpay-db`: `Health::pool_stats`, `WebhookDeliveries::backlog`, and `gauges` (the samplers). The worker's gauge loop and a
  serve-mode task write them.
- `vpay-api`: the request span records the bounded route, method and status the metric labels already carry.
- `tools/dashboards/` and `deploy/dashboards/`: three Grafana dashboards and their README, generated; `just dashboards-check`.

## What was run

Toolchain `1.98.0` (`rust-toolchain.toml`), target dir on tmpfs.

| Check                                                                                                                                                                                                                                                                                                                                                         | Result                                                                                                                                                           |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all --check`                                                                                                                                                                                                                                                                                                                                     | clean                                                                                                                                                            |
| `cargo clippy -p vpay-telemetry -p vpay-core -p vpay-db -p vpay-worker -p vpay-api -p vpay-server --all-targets --locked -- -D warnings`                                                                                                                                                                                                                      | clean                                                                                                                                                            |
| `cargo test -p vpay-core --locked`                                                                                                                                                                                                                                                                                                                            | 99 passed, 0 failed, 0 ignored; doctests 73 passed, 0 failed, 0 ignored                                                                                          |
| `cargo test -p vpay-telemetry --locked`                                                                                                                                                                                                                                                                                                                       | 27 passed (22 unit, 3 dashboards, 1 export, 1 hang), 0 failed, 0 ignored; doctests 0                                                                             |
| `cargo test -p vpay-db --test repositories -- the_webhook_backlog the_pool_reading the_sampler_writes`                                                                                                                                                                                                                                                        | 3 passed, 0 failed, 146 not selected (the whole 149-test binary was **not** run); a real `postgres:16-alpine` container, the tag `vpay-testkit` pins             |
| `cargo xtask` gates: `verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity`, `verify-links`, `verify-npm-scope`, `verify-serde`, `verify-repositories`, `verify-toolchain`, `verify-migrations`, `verify-versions`, `verify-privacy-inventory`, `verify-doc-counts`, `verify-security-inventory`, `verify-mcp-parity`, `verify-adr-numbers` | all ok after the change was committed (`verify-links` reads tracked files)                                                                                       |
| `prettier --check .` (3.9.6)                                                                                                                                                                                                                                                                                                                                  | clean                                                                                                                                                            |
| `sh tools/dashboards/generate.sh`, then `git diff --exit-code deploy`                                                                                                                                                                                                                                                                                         | no difference                                                                                                                                                    |
| `cargo deny check` (0.22.1)                                                                                                                                                                                                                                                                                                                                   | `bans`, `licenses`, `sources` ok. **`advisories` fails, and fails identically on `origin/master`**: RUSTSEC-2026-0592 on `url 2.5.8`, not touched by this change |
| `cargo tree -p vpay-server -i aws-lc-rs` / `openssl` / `native-tls`                                                                                                                                                                                                                                                                                           | not in the graph. `tonic` is in it only through `testcontainers` (a dev dependency), as before                                                                   |

`check-schema` (needs the CrateStack CLI) and `verify-ui` were not run; nothing here touches a schema or the UI.

## What the new tests hold

- `tests/export.rs`: an in-process collector receives metrics, a span and a log; headers from the file; the resource; a
  metric name outside the vocabulary and a label key outside `LABELS` do **not** arrive, while the Prometheus listener
  still has both; no byte of a sentinel (a path with an id, a request id, a phone number, the header value) reaches the
  collector; a histogram of seconds carries the unit `s` and bounds up to 120.
- `src/pipeline_tests.rs`: only allowlisted fields leave, text is plain or `redacted`, children join their parent's trace,
  a 5xx marks its span, a foreign or verbose callsite is `Interest::never`, a full queue drops and counts.
- `src/settings.rs`: off unless an endpoint is set; the refusals name the variable and never its value; the extra `OTEL_*`
  variables the SDK would obey silently (`..._TEMPORALITY_PREFERENCE`, the per-signal `*_COMPRESSION`,
  `OTEL_SPAN_EVENT_COUNT_LIMIT`, `OTEL_SPAN_LINK_COUNT_LIMIT`, ...) are refused.
- `tests/hang.rs`: a collector that never answers costs the caller nothing and `shutdown` ends by its deadline.
- `tests/dashboards.rs`: every metric and label a panel queries is in the vocabulary, every name of the vocabulary has a
  panel, every panel is in the README, and the rail dashboard carries the note that a stub rail reports like a live one.
- `vpay-core`: `every_documented_label_is_in_the_label_list`, in both directions.

## What was **not** run

- Nothing against a real collector, Grafana or OpenObserve. Whether a store applies the OpenTelemetry-to-Prometheus
  naming the dashboards assume, and whether the JSON imports, is **unverified**.
- `vpay-server`'s own subprocess tests (`tests/cli.rs`, which boot the binary against Postgres) and the integration and
  conformance suites. `install_process_defaults` was compiled and clippy-checked but its telemetry branch has no
  subprocess test: the telemetry-off path is the one `tests/cli.rs` exercises.
- The Helm chart: nothing wires `OTEL_EXPORTER_OTLP_ENDPOINT` or the headers file into it.
- No span for a worker job or a rail call; no payment-intent counts by state (no index to count by).
