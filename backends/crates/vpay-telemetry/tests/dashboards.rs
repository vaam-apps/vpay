//! The dashboards in `deploy/dashboards` are held to the service: every metric a panel queries is one the service
//! exports (as a Prometheus backend names it), every label is one the vocabulary records, every panel with a query
//! says what it watches and is described in the README, and the README lists every file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;
use vpay_core::metrics::{ALL, LABELS};

/// What a Prometheus backend adds or a query computes, beside the labels the code records.
const EXTRA_LABELS: [&str; 5] = ["job", "instance", "le", "class", "__rate_interval"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../deploy/dashboards")
}

fn dashboards() -> Vec<(String, Value)> {
    let mut found: Vec<_> = std::fs::read_dir(root())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| {
            let text = std::fs::read_to_string(e.path()).unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                serde_json::from_str(&text).unwrap(),
            )
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The series a backend exposes for the vocabulary: a histogram is three of them.
fn exposed() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for name in ALL {
        if name.ends_with("_duration_seconds") {
            for suffix in ["bucket", "count", "sum"] {
                names.insert(format!("{name}_{suffix}"));
            }
        } else {
            names.insert((*name).to_owned());
        }
    }
    names
}

fn tokens(expr: &str) -> BTreeSet<String> {
    expr.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

fn recorded(label: &str) -> bool {
    LABELS.contains(&label) || EXTRA_LABELS.contains(&label)
}

/// The label names a PromQL expression groups by, or matches on inside a selector (`name{label="x"}`).
fn labels_of(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    for group in expr.split("by (").skip(1) {
        let inside = group.split(')').next().unwrap();
        out.extend(inside.split(',').map(|l| l.trim().to_owned()));
    }
    // A selector is a `{` right after a metric name; `${1}` in a replacement string is not one.
    let bytes = expr.as_bytes();
    for (at, byte) in bytes.iter().enumerate() {
        if *byte != b'{' || at == 0 {
            continue;
        }
        let before = bytes[at - 1];
        if !(before.is_ascii_alphanumeric() || before == b'_') {
            continue;
        }
        let inside = expr[at + 1..].split('}').next().unwrap();
        for part in inside.split(',') {
            let label: String = part
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            out.push(label);
        }
    }
    out
}

#[test]
fn every_panel_queries_what_the_service_exports_and_is_described() {
    let exposed = exposed();
    let readme = std::fs::read_to_string(root().join("README.md")).unwrap();
    let found = dashboards();
    assert_eq!(found.len(), 3, "service, rails, platform");
    let mut uids = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut queried = BTreeSet::new();
    for (file, dashboard) in &found {
        assert!(
            uids.insert(dashboard["uid"].as_str().unwrap().to_owned()),
            "{file}: uid is unique"
        );
        assert!(readme.contains(file.as_str()), "{file} is in the README");
        let panels = dashboard["panels"].as_array().unwrap();
        assert!(
            panels.iter().any(|p| p["type"] != "row"),
            "{file} has panels"
        );
        for panel in panels.iter().filter(|p| p["type"] != "row") {
            let title = panel["title"].as_str().unwrap();
            assert!(
                seen.insert(title.to_owned()),
                "{file}: panel title {title:?} is unique"
            );
            assert!(
                readme.contains(&format!("`{title}`")),
                "{file}: panel {title:?} is not in the README"
            );
            if panel["type"] == "text" {
                // A note queries nothing; it must still say something.
                assert!(
                    panel["options"]["content"]
                        .as_str()
                        .is_some_and(|c| c.len() > 40),
                    "{title:?}"
                );
                continue;
            }
            assert!(
                panel["description"].as_str().is_some_and(|d| d.len() > 20),
                "{file}: {title:?} says what it watches"
            );
            let targets = panel["targets"].as_array().unwrap();
            assert!(!targets.is_empty(), "{title:?} has a query");
            for target in targets {
                let expr = target["expr"].as_str().unwrap();
                let names: Vec<_> = tokens(expr)
                    .into_iter()
                    .filter(|t| t.starts_with("vpay_"))
                    .collect();
                assert!(
                    !names.is_empty(),
                    "{title:?}: {expr} queries no metric of the service"
                );
                for name in names {
                    assert!(
                        exposed.contains(&name),
                        "{title:?} queries {name}, which the service does not export"
                    );
                    queried.insert(name);
                }
                for label in labels_of(expr) {
                    assert!(
                        recorded(&label),
                        "{title:?}: label {label:?} is not recorded by the service"
                    );
                }
            }
        }
    }
    // The other direction, for what the card names: the dashboards must reach these.
    for name in [
        "vpay_http_requests_total",
        "vpay_provider_requests_total",
        "vpay_provider_request_duration_seconds_bucket",
        "vpay_charge_transitions_total",
        "vpay_webhook_deliveries_total",
        "vpay_webhook_deliveries_pending",
        "vpay_webhook_oldest_pending_overdue_seconds",
        "vpay_db_pool_connections",
        "vpay_db_pool_max_connections",
        "vpay_jobs_oldest_claimable_age_seconds",
        "vpay_telemetry_exports_total",
    ] {
        assert!(queried.contains(name), "no panel reads {name}");
    }
}

#[test]
fn the_rail_dashboard_says_what_a_rail_panel_does_not() {
    let (_, rails) = dashboards()
        .into_iter()
        .find(|(file, _)| file == "vpay-rails.json")
        .unwrap();
    let note = rails["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["type"] == "text")
        .expect("a note");
    let content = note["options"]["content"].as_str().unwrap();
    assert!(
        content.contains("stub") && content.contains("docs/status.md"),
        "the note must say a stub rail reports like a live one, and where the truth is: {content}"
    );
}

#[test]
fn every_name_of_the_vocabulary_has_a_panel() {
    // Every name of the vocabulary is read by some panel. A metric nobody can see is a metric nobody keeps true.
    let mut queried = BTreeSet::new();
    for (_, dashboard) in dashboards() {
        for panel in dashboard["panels"].as_array().unwrap() {
            for target in panel["targets"].as_array().into_iter().flatten() {
                queried.extend(tokens(target["expr"].as_str().unwrap()));
            }
        }
    }
    let exposed = exposed();
    for name in ALL {
        let read = exposed
            .iter()
            .filter(|e| e.starts_with(name))
            .any(|e| queried.contains(e));
        assert!(read, "{name} is in the vocabulary and no panel reads it");
    }
}
