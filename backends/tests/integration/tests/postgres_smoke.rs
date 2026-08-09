//! Integration tests against a real Postgres via testcontainers.
//!
//! STATUS: the container harness is not wired yet. This file documents intent
//! and is `#[ignore]`d so it cannot report false confidence.

#[test]
#[ignore = "not implemented: testcontainers harness — see docs/status.md"]
fn schema_migrates_cleanly_on_an_empty_database() {
    unreachable!("enable when migrations land")
}

#[test]
#[ignore = "not implemented: testcontainers harness — see docs/status.md"]
fn one_charge_per_intent_is_enforced_by_the_database() {
    unreachable!("enable when migrations land")
}
