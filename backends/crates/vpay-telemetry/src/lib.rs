//! OpenTelemetry for vpay: traces, metrics and logs through the OpenTelemetry SDK over OTLP/HTTP with JSON bodies,
//! **off unless an endpoint is set** (`OTEL_EXPORTER_OTLP_ENDPOINT`, [`Settings`]).
//!
//! - Metrics: [`meter::Fanout`] is the process's `metrics` recorder. It feeds the Prometheus exporter as before and,
//!   when metrics are on, the SDK's instruments, for the names and labels of `vpay_core::metrics` and nothing else.
//! - Spans: `tracing-opentelemetry` turns the request spans of vpay into OpenTelemetry spans, and
//!   [`filter::clean`] holds each finished span to an allowlist of field names ([`filter::FIELDS`]) at the exporter.
//! - Logs: [`layer::LogLayer`] exports the `INFO`-and-above events of this repository through the same allowlist,
//!   with a message of plain words only.
//! - [`export::start`] builds the SDK's providers over OTLP/HTTP; [`Guard::layer`] and [`Guard::recorder`] are what
//!   the binary installs.
//!
//! The crate holds no secret: headers come from a file, and no error or `Debug` shows one.

pub mod export;
pub mod filter;
pub mod layer;
pub mod meter;
#[cfg(test)]
mod pipeline_tests;
pub mod settings;
#[cfg(feature = "testkit")]
pub mod testkit;

pub use export::{Guard, StartError, Stats, start};
pub use meter::{Bridge, Fanout};
pub use settings::{Resource, Settings, SettingsError};
