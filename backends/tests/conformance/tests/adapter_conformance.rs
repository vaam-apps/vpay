//! The shared adapter conformance suite.
//!
//! ONE suite, parameterised over every adapter. Adding a rail means making this
//! pass — not writing a new suite. That is the real test of whether the
//! provider port is a port or just a folder.
//!
//! STATUS: only capability-level cases run today. The wire-level cases are
//! `#[ignore]`d with a reason until the adapters exist, so a green run never
//! overstates coverage. See docs/status.md.

use vpay_provider::{Capabilities, ProviderAdapter, ProviderError};

fn adapters() -> Vec<Box<dyn ProviderAdapter>> {
    vec![
        Box::new(vpay_adapter_mtn_momo::Adapter::new()),
        Box::new(vpay_adapter_orange_money::Adapter::new()),
    ]
}

#[test]
fn every_adapter_declares_coherent_capabilities() {
    for a in adapters() {
        let c: Capabilities = a.capabilities();
        assert!(
            c.is_coherent(),
            "{}: partial refunds without refunds",
            a.code()
        );
    }
}

#[test]
fn adapter_codes_are_unique() {
    let mut codes: Vec<_> = adapters().iter().map(|a| a.code()).collect();
    codes.sort_unstable();
    let before = codes.len();
    codes.dedup();
    assert_eq!(before, codes.len(), "duplicate adapter codes: {codes:?}");
}

#[test]
fn refund_is_refused_when_the_capability_is_absent() {
    for a in adapters() {
        if !a.capabilities().supports_refunds {
            // Orange has no refund API; the capability flag is what makes the
            // core refuse, with no rail-specific branch anywhere.
            assert!(!a.capabilities().supports_partial_refunds);
        }
    }
}

#[test]
fn unimplemented_operations_never_fabricate_success() {
    for a in adapters() {
        match a.parse_callback(b"{}") {
            Err(ProviderError::NotImplemented(_)) => {}
            Err(_) => {}
            Ok(_) => panic!("{}: parse_callback returned Ok from a stub", a.code()),
        }
    }
}

// ---------------------------------------------------------------------------
// Wire-level cases. Ignored until the adapters are built, so `cargo nextest run`
// is green without implying these passed.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "not implemented: submit() — see docs/status.md"]
fn duplicate_submit_reports_submitted_not_an_error() {
    unreachable!("enable when submit() lands")
}

#[test]
#[ignore = "not implemented: query_status() — see docs/status.md"]
fn not_found_is_never_on_its_own_a_failure() {
    unreachable!("enable when query_status() lands")
}

#[test]
#[ignore = "not implemented: redirect flow — see docs/status.md"]
fn redirect_rails_commit_ref_extra_before_returning_a_url() {
    unreachable!("enable when the Orange adapter lands")
}
