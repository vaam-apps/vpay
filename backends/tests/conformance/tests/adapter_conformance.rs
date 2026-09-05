//! The shared adapter conformance suite.
//!
//! ONE suite, parameterised over every adapter. Adding a rail means making this
//! pass — not writing a new suite. That is the real test of whether the
//! provider port is a port or just a folder.
//!
//! STATUS: every case in this file runs. Nothing here is `#[ignore]`d — the
//! wire-level cases were ignored only while `mtn_momo`'s and `orange_money`'s
//! `submit`/`query_status`/`parse_callback` were `NotImplemented` tokens, and
//! Step 3 built them. A test is ignored in this repo only while the behaviour
//! it describes is unbuilt (`just verify-ignored` holds the count at zero for
//! this suite), so a green run here means these assertions were made against a
//! real container, not skipped. `mtn_momo::refund` keeps its `NotImplemented`
//! token — Disbursements is a separate product — and
//! [`a_rail_without_the_refund_capability_answers_unsupported`] asserts exactly
//! that, rather than being ignored for it. See `docs/status.md`.
//!
//! The wire-level cases were written *before* the adapters, deliberately: this
//! file was the specification the MTN and Orange implementers coded against,
//! and a specification that is only written after the fact is a description.
//!
//! # How a rail is stubbed
//!
//! A real `wiremock/wiremock` container per rail, started by
//! `vpay_testkit::containers::start_wiremock` from the same
//! `wiremock/{rail}/mappings` directory `compose.yml` bind-mounts
//! (ADR-0006: a stub rail is a WireMock *host in configuration*, reached over
//! HTTP exactly as a real rail is). The Rust `wiremock` crate — an in-process
//! double that would replace the adapter's own transport — must never appear
//! in this package's manifest.
//!
//! # The mapping contract each rail must satisfy
//!
//! Every case below selects its stubbed response by *reference*: MTN matches
//! it on the `X-Reference-Id` header, Orange on the body's `order_id` (which
//! is `ChargeRef::reference_id` rendered as a string, per
//! `docs/flows/adapter-orange-money.md`). So both rails' mapping directories
//! stub the *same* UUIDs, and adding a rail means adding one mappings
//! directory rather than editing this file:
//!
//! | constant | what the rail's mappings must do |
//! |---|---|
//! | [`REF_ACCEPTED`] | accept the submit (MTN 202, Orange 201 with a `pay_token` + `payment_url`) |
//! | [`REF_DUPLICATE`] | report the reference as already existing, twice, and *name the same payment* both times — a push rail with no key material at all (MTN 409 `RESOURCE_ALREADY_EXIST`), a redirect rail with the token it already minted for that reference. A redirect rail's pair must be a WireMock **scenario**, so the second answer is served by a mapping that could have differed (Orange: `webpayment.json`'s `orange-duplicate-order`) |
//! | [`REF_UNKNOWN`] | answer the status query with 404 |
//! | [`REF_UNAVAILABLE`] | answer the status query 503 |
//! | [`REF_SLOW`] | answer the status query after a `fixedDelayMilliseconds` well above [`SHORT_REQUEST_TIMEOUT`] |
//! | [`REF_SCENARIO`] | a WireMock scenario: first status query `PENDING`, second `SUCCESSFUL` |
//! | [`REF_REDIRECT`] | answer the **submit** `307` with `Location: `[`REDIRECT_TARGET`], and stub that path with the rail's *accepted* answer |
//! | [`REF_HUGE`] | answer the status query `200` with a valid body padded past [`vpay_provider::http::MAX_RAIL_BODY_BYTES`] |
//! | each rail's decline table | answer the status query with that rail's documented failure reason |
//!
//! The account-holder cases (issue #47) are the one family that steers on a
//! **payer reference** rather than on a charge reference, because
//! `ProviderAdapter::account_holder_name` takes no charge at all — it is a
//! stateless read of a number. A rail that declares
//! [`Capabilities::supports_account_holder_lookup`] must stub, on whatever
//! endpoint its own protocol documents:
//!
//! | constant | what the rail's mappings must do |
//! |---|---|
//! | [`MSISDN_REGISTERED`] | name a holder |
//! | [`MSISDN_UNREGISTERED`] | report that it has no record — the rail's own "not found" |
//! | [`MSISDN_SLOW`] | answer after a `fixedDelayMilliseconds` well above [`SHORT_REQUEST_TIMEOUT`] |
//! | [`MSISDN_HUGE`] | answer with a valid body padded past [`vpay_provider::http::MAX_RAIL_BODY_BYTES`] |
//! | [`MSISDN_FULL_OF_PII`] | answer with a **named** holder buried in as much other personal data as the rail could plausibly carry |
//!
//! A rail that declares the capability `false` stubs none of them: every one
//! of those cases asserts [`ProviderError::Unsupported`] for it instead, out
//! of the same shared body, so "Orange has no such API" is checked rather
//! than skipped.
//!
//! and each rail supplies one `ProviderConfig` whose credentials are wrong, to
//! prove a credential failure is not reported as a payer's problem.
//!
//! # What a conformance charge stands for
//!
//! [`Rail::charge`] builds the [`ChargeRef`] every case operates on, and it is
//! deliberately not an *empty* charge: it stands in for a charge whose `submit`
//! has already run, because that is the only kind of charge a `query_status`
//! ever sees in production. So it carries the key material that rail's `submit`
//! would have returned, selected by capability exactly as `payer_ref` is:
//!
//! - a **push** rail is addressed by the reference we generated, so there is
//!   nothing else to carry; it needs a `payer_ref` instead, because it prompts
//!   the payer's own instrument.
//! - a **redirect** rail is addressed by the token *it* returned at submit —
//!   the rail will not answer for our reference alone — so `ref_extra` carries
//!   a `pay_token`. Handing a redirect rail a charge with none is not a
//!   "reference the rail has never heard of": it is key material we lost for a
//!   charge the rail may well have settled, which is why the adapters answer
//!   `ProviderError::Config` and never `ChargeStatus::NotFound`
//!   (`docs/flows/crash-safety.md`). Seeding it here keeps
//!   [`not_found_is_never_on_its_own_a_failure`] a test of the rail's 404 and
//!   not an accidental test of our own missing-token branch — that branch has
//!   its own unit test in the adapter crate.
//!
//! `return_url` is the one field [`Rail::charge`] deliberately does **not**
//! select by capability: every conformance charge carries one, push rail
//! included. The core fills it for any charge whose merchant sent one and
//! leaves it to the *adapter* to decide whether its rail has a use for it (D2
//! of `docs/plans/2026-09-04-step9-hosted-checkout.md`). A push rail's case
//! would be vacuous if the field were `None` — "MTN sent no return URL"
//! proves nothing when there was none to send — so
//! [`the_submit_tells_the_rail_where_to_send_the_payer_back`] hands both
//! rails the same URL and asserts that only the redirect rail's wire carries
//! it.
//!
//! Only `pay_token`. Orange's `submit` also returns a `notif_token`, but no
//! adapter call *reads* one from a `ChargeRef` — it arrives on the inbound
//! notification and is checked there, and `parse_callback` takes a body, not a
//! charge. Seeding one here would put a value in front of the assertions that
//! nothing under test consumes, which is how a suite starts describing a
//! contract it does not check.
//!
//! No stub selects on the token's *value* (see the header comment in
//! `wiremock/orange/mappings/transactionstatus.json`), so the seeded value only
//! has to be present and non-blank; it is derived from the reference so a
//! captured request body says which case sent it. A rail that did key its
//! status read on an opaque token would need its cases to `submit` first, and
//! the day one is added, this is the helper that has to change.

// `clippy.toml`'s `allow-expect-in-tests` covers `#[test]` bodies and
// `#[cfg(test)]` modules; the helpers below sit at the top level of an
// integration-test crate, which clippy does not treat as "in test". The same
// crate-level allow the integration suite carries, for the same reason: a
// failing assertion SHOULD panic, that is how a test reports (ADR-0007).
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rstest::rstest;
use testcontainers::{ContainerAsync, GenericImage};
use uuid::Uuid;
use vpay_core::{Currency, FailureCode, Money, ProviderFlow};
use vpay_provider::{
    Capabilities, ChargeRef, ChargeStatus, ProviderAdapter, ProviderConfig, ProviderError,
};
use vpay_testkit::containers::start_wiremock;

/// The two rails this workspace ships, as the *only* place a rail's name
/// appears in a test body. Everything else branches on capability values, per
/// ADR-0002.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RailUnderTest {
    MtnMomo,
    OrangeMoney,
}

fn adapters() -> Vec<Box<dyn ProviderAdapter>> {
    let http = vpay_provider::http::client().expect("the vendored-roots client builds");
    vec![
        Box::new(vpay_adapter_mtn_momo::Adapter::new(http.clone())),
        Box::new(vpay_adapter_orange_money::Adapter::new(http)),
    ]
}

// ---------------------------------------------------------------------------
// Capability-level cases. These run: they need no rail, only the port.
// ---------------------------------------------------------------------------

/// Proves no adapter advertises partial refunds without refunds — a pair the
/// core branches on and would never be able to satisfy. Needs no rail: it is a
/// statement about what the port *declares*, and it is checked for every
/// adapter in the workspace, so a rail added tomorrow is covered without
/// touching this file.
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

/// Proves the code a `ProviderConfig` selects an adapter by is unambiguous.
/// Two rails sharing one would route a merchant's charges to whichever the
/// registry happened to insert last, silently and per-deployment.
#[test]
fn adapter_codes_are_unique() {
    let mut codes: Vec<_> = adapters().iter().map(|a| a.code()).collect();
    codes.sort_unstable();
    let before = codes.len();
    codes.dedup();
    assert_eq!(before, codes.len(), "duplicate adapter codes: {codes:?}");
}

/// The *declaration* half of the refund contract: a rail with no refund API
/// must not advertise partial refunds either, so the core's capability branch
/// (ADR-0002) is the only thing that ever has to decide.
///
/// The *behavioural* half — that calling `refund` on such a rail answers
/// `Unsupported` — is
/// [`a_rail_without_the_refund_capability_answers_unsupported`], which needs a
/// configured rail and so sits with the wire-level cases.
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

/// Proves `parse_callback` fails closed on a body carrying no identifiers.
/// `{}` is the shape of an unauthenticated POST from anyone who can reach the
/// callback URL, and an `Ok` would hand the reconciler a charge reference the
/// adapter invented. Either a real parse error or a `NotImplemented` token is
/// acceptable — what is refused is a plausible success.
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
// Wire-level cases, against a real WireMock container per rail.
//
// These were `#[ignore]`d while the adapters were `NotImplemented` tokens, so
// a green `cargo nextest run` never implied they had passed. Step 3 removed
// the tokens, so the same change removed the `#[ignore]`s and dropped this
// suite's `expected_ignored` to zero: an ignored test here would now be
// hiding a regression rather than declaring an absence, and `just
// verify-ignored` fails if the count and the recipe disagree.
//
// Each case runs twice, once per rail, against that rail's own stub container.
// Nothing below branches on `RailUnderTest` inside a test body — rail
// differences are either capability values (`flow`, `supports_refunds`) or
// table data (`documented_declines`, `documented_callback_body`). If a case
// ever needs `if rail == MtnMomo` to pass, the port has leaked and that is the
// finding, not the fix.
// ---------------------------------------------------------------------------

/// A submit the rail accepts.
const REF_ACCEPTED: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0202);
/// A reference the rail has already seen — a duplicate submit.
const REF_DUPLICATE: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00dd);
/// A reference the rail has no record of.
const REF_UNKNOWN: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0404);
/// A reference whose status query the rail answers 503.
const REF_UNAVAILABLE: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_05aa);
/// A reference whose status query the rail answers *slowly*.
const REF_SLOW: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0560);
/// A reference under a WireMock scenario: `PENDING`, then `SUCCESSFUL`.
const REF_SCENARIO: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0ce0);
/// A reference whose **submit** the rail answers with a `307` redirect.
const REF_REDIRECT: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0302);
/// A reference whose status query the rail answers with an oversized body.
const REF_HUGE: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0b16);

// The account-holder family (issue #47). Digits only, all of them, and that
// is not cosmetic: `GET /v1/account_holders` validates Cameroon E.164
// server-side, so a hex-lettered steering number of the kind
// `requesttopay.json` uses could never reach this port method from the API.
// A conformance suite that steered on one would be exercising a path no
// caller has.

/// A number the rail knows and will name a holder for.
const MSISDN_REGISTERED: &str = "237600000200";
/// A number the rail has no record of.
const MSISDN_UNREGISTERED: &str = "237600000404";
/// A number whose lookup the rail answers *slowly* — past
/// [`SHORT_REQUEST_TIMEOUT`].
const MSISDN_SLOW: &str = "237600000560";
/// A number whose lookup the rail answers with an oversized body.
const MSISDN_HUGE: &str = "237600000616";
/// A number whose holder the rail describes in far more detail than a name.
const MSISDN_FULL_OF_PII: &str = "237600000700";

/// The name [`MSISDN_REGISTERED`]'s holder is registered under, as the
/// mapping spells it.
const REGISTERED_HOLDER_NAME: &str = "David Mbarga";

/// Everything [`MSISDN_FULL_OF_PII`]'s body carries **besides** the name,
/// each of which must be absent from the returned value and from every log
/// line the call produced.
///
/// A list rather than one string, because the failure this guards against is
/// partial: an adapter that projected `given_name`, `family_name` *and*
/// `birthdate` would pass a test that only looked for an address.
const PERSONAL_DATA_THE_RAIL_SENDS: [&str; 8] = [
    "1988-04-17",
    "en_CM",
    "FEMALE",
    "ACTIVE",
    "mtn-customer-9f2c41",
    "amina.nkeng@example.test",
    "12 Avenue Kennedy, Yaounde",
    "CM-ID-0099887766",
];

/// The name that body *does* carry, kept apart from the list above because
/// the two assertions differ: this one must reach the caller and must not
/// reach a log.
const PII_HOLDER_NAME: &str = "Amina Nkeng";

/// Where the core says this charge's payer goes when a rail's page is done
/// with them — the merchant's own site here, which is the case that closes
/// browser-checkout's D4.
///
/// A URL on nobody's real domain, and deliberately *not* under the stub's own
/// origin: it has to be visibly not-vpay's, so a rail (or an adapter) that
/// substituted the callback URL or the base URL would fail on the value
/// rather than pass on a coincidence.
const RETURN_URL: &str = "https://shop.example/order/1234/return";

/// Where [`REF_REDIRECT`]'s `Location` points, on the rail's own stub.
///
/// Both rails' mappings stub this path with the answer that means *accepted*,
/// so a followed redirect fails [`redirects_are_refused_and_never_followed`]
/// twice over: once because `submit` would return `Ok`, and once because the
/// request would appear in the stub's journal.
const REDIRECT_TARGET: &str = "/__never-followed/00000000-0000-0000-0000-000000000302";

/// The request deadline the timeout case asks for. Two orders of magnitude
/// below the delay its mapping must impose, so the assertion is about the
/// deadline firing and not about a slow CI machine.
const SHORT_REQUEST_TIMEOUT: Duration = Duration::from_millis(100);

/// Everything a case needs to talk to one rail: the adapter, a
/// [`ProviderConfig`] pointing at that rail's freshly started stub, and the
/// container itself — which must stay alive for the duration of the test,
/// because dropping it stops and removes the stub.
struct Rail {
    adapter: Box<dyn ProviderAdapter>,
    config: ProviderConfig,
    /// `http://127.0.0.1:{mapped port}` — the stub's origin, which is *not*
    /// derivable from `config.base_url` (Orange's carries a path prefix).
    /// Only [`requests_recorded_for`] uses it, to reach the admin API.
    stub_origin: String,
    /// Held, not read. `ContainerAsync::drop` stops the container.
    _container: ContainerAsync<GenericImage>,
}

impl Rail {
    /// A charge on this rail as it looks *after* `submit` — see "What a
    /// conformance charge stands for" in the module doc for why that is the
    /// right stand-in, and why the redirect branch below is not optional.
    fn charge(&self, reference_id: Uuid) -> ChargeRef {
        let flow = self.adapter.capabilities().flow;
        ChargeRef {
            reference_id,
            amount: Money::new(5_000, self.config.currency).expect("non-negative"),
            payer_ref: match flow {
                // A push rail prompts a payer's own instrument, so it needs
                // one; a redirect rail authenticates the payer itself and may
                // never learn who they are.
                ProviderFlow::Push => Some("237600000000".to_owned()),
                ProviderFlow::Redirect => None,
            },
            ref_extra: match flow {
                // Symmetrically: a push rail's status read is addressed by the
                // reference we generated and carries no rail key material at
                // all, so an empty map is the truthful state.
                ProviderFlow::Push => BTreeMap::new(),
                // A redirect rail's status read is addressed by the token the
                // rail handed back at submit, so a charge that reached the
                // point of being queryable has one.
                ProviderFlow::Redirect => {
                    BTreeMap::from([("pay_token".to_owned(), format!("pay-{reference_id}"))])
                }
            },
            // Not selected by flow, unlike the two fields above — see "What a
            // conformance charge stands for" in the module doc.
            return_url: Some(RETURN_URL.to_owned()),
        }
    }
}

/// Where a rail's WireMock mappings live: the same directories `compose.yml`
/// bind-mounts, so a mapping fixed for the compose stack is fixed for CI too.
fn mappings_dir(rail: RailUnderTest) -> PathBuf {
    let dir = match rail {
        RailUnderTest::MtnMomo => "mtn",
        RailUnderTest::OrangeMoney => "orange",
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("wiremock")
        .join(dir)
}

/// Starts `rail`'s stub and builds the adapter and configuration that point at
/// it.
///
/// `credentials` is a parameter rather than a constant so one shared helper
/// serves both the "everything is configured correctly" cases and the
/// bad-credentials case — the latter is the same rail, same stub, one wrong
/// value, which is exactly how it happens in production.
async fn start(rail: RailUnderTest, credentials: Credentials, request_timeout: Duration) -> Rail {
    let container = start_wiremock(&mappings_dir(rail))
        .await
        .expect("the rail stub starts");
    let port = container
        .get_host_port_ipv4(8080)
        .await
        .expect("the stub's mapped host port");
    let host = format!("http://127.0.0.1:{port}");

    // The process's ordinary client, built exactly as a binary builds it at
    // boot: the *production* defaults, and deliberately not `request_timeout`.
    //
    // That is what makes `an_unavailable_rail_is_a_transport_error_never_a_decline`
    // a real gate. With the short deadline on the client, a rail adapter that
    // ignored `ProviderConfig::request_timeout` entirely still passed —
    // reqwest's client-level timeout fired for it, and the case proved
    // nothing about the adapter. Orange did ignore it, on both its token call
    // and every payment call, and this suite said it was fine. With the short
    // deadline on the `ProviderConfig` alone, the only thing that can make
    // that case pass is the adapter applying the configured deadline per
    // request.
    let http = vpay_provider::http::client_with_timeouts(
        vpay_provider::DEFAULT_CONNECT_TIMEOUT,
        vpay_provider::DEFAULT_REQUEST_TIMEOUT,
    )
    .expect("the vendored-roots client builds");

    let (adapter, config): (Box<dyn ProviderAdapter>, ProviderConfig) = match rail {
        RailUnderTest::MtnMomo => (
            Box::new(vpay_adapter_mtn_momo::Adapter::new(http)),
            ProviderConfig {
                base_url: host.clone(),
                callback_url: format!("{host}/provider/mtn_momo/callback"),
                // EUR: MTN's sandbox rejects XAF (docs/flows/money.md), and
                // the stub mirrors the sandbox.
                currency: Currency::Eur,
                settings: BTreeMap::from([
                    ("target_environment".to_owned(), "sandbox".to_owned()),
                    (
                        "api_user".to_owned(),
                        "11111111-2222-3333-4444-555555555555".to_owned(),
                    ),
                ]),
                credentials: match credentials {
                    Credentials::Valid => BTreeMap::from([
                        (
                            "subscription_key".to_owned(),
                            "stub-subscription-key".to_owned(),
                        ),
                        ("api_key".to_owned(), "stub-api-key".to_owned()),
                    ]),
                    // The value the MTN mappings answer 401 to.
                    Credentials::Rejected => BTreeMap::from([
                        ("subscription_key".to_owned(), "bad-key".to_owned()),
                        ("api_key".to_owned(), "stub-api-key".to_owned()),
                    ]),
                },
                connect_timeout: vpay_provider::DEFAULT_CONNECT_TIMEOUT,
                request_timeout,
            },
        ),
        RailUnderTest::OrangeMoney => (
            Box::new(vpay_adapter_orange_money::Adapter::new(http)),
            ProviderConfig {
                // The `/orange-money-webpay/{env}` prefix is part of the
                // configured base URL (docs/flows/adapter-orange-money.md);
                // the token endpoint sits at the host root, which the adapter
                // derives.
                base_url: format!("{host}/orange-money-webpay/dev"),
                callback_url: format!("{host}/provider/orange_money/callback"),
                currency: Currency::Xaf,
                settings: BTreeMap::from([
                    ("env".to_owned(), "dev".to_owned()),
                    ("lang".to_owned(), "en".to_owned()),
                ]),
                credentials: BTreeMap::from([
                    ("merchant_key".to_owned(), "stub-merchant-key".to_owned()),
                    (
                        "client_id".to_owned(),
                        match credentials {
                            Credentials::Valid => "stub-client-id".to_owned(),
                            // The value the Orange mappings answer 401 to.
                            Credentials::Rejected => "expired-client-id".to_owned(),
                        },
                    ),
                    ("client_secret".to_owned(), "stub-client-secret".to_owned()),
                ]),
                connect_timeout: vpay_provider::DEFAULT_CONNECT_TIMEOUT,
                request_timeout,
            },
        ),
    };

    Rail {
        adapter,
        config,
        stub_origin: host,
        _container: container,
    }
}

/// How many requests this rail's stub has recorded for `path`.
///
/// WireMock's own request journal, over its admin API — the only witness
/// available for a request the code under test was supposed **not** to make.
/// Asserting on the adapter's return value alone cannot distinguish "the
/// redirect was refused" from "the redirect was followed and the answer was
/// then rejected for some other reason"; the journal can.
///
/// Every case starts its own container, so the journal is empty at the top of
/// a test and needs no reset.
///
/// The count is dug out of the JSON by hand rather than with `serde_json`:
/// this package deliberately carries the smallest dev-dependency set that
/// still lets it talk to a real rail stub, and the response body is
/// `{"count": N, "requestJournalDisabled": false}` — one integer, after one
/// key.
async fn requests_recorded_for(rail: &Rail, path: &str) -> usize {
    requests_matching(rail, &format!(r#"{{"method":"ANY","url":"{path}"}}"#)).await
}

/// How many requests this rail's stub has recorded matching an arbitrary
/// WireMock request pattern.
///
/// [`requests_recorded_for`]'s general form, extracted when
/// [`the_submit_tells_the_rail_where_to_call_back`] needed to ask about a
/// *header* on one rail and a *body field* on the other. The pattern itself
/// is rail-specific data ([`callback_url_pattern`]) for the same reason
/// [`documented_declines`] is: one shared assertion, one table row per rail,
/// and no `if rail == …` in a test body.
///
/// Same hand-rolled count extraction as its caller, and for the same reason
/// — see that function.
async fn requests_matching(rail: &Rail, pattern: &str) -> usize {
    let http = vpay_provider::http::client().expect("the vendored-roots client builds");
    let body = pattern.to_owned();
    let text = http
        .post(format!("{}/__admin/requests/count", rail.stub_origin))
        .body(body)
        .send()
        .await
        .expect("the stub's admin API answers")
        .text()
        .await
        .expect("the count response is readable");

    let (_, after) = text
        .split_once("\"count\"")
        .unwrap_or_else(|| panic!("no count in the admin response: {text}"));
    let digits: String = after
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("count is not a number in: {text}"))
}

/// Whether the configuration handed to the adapter is the one the rail
/// accepts.
#[derive(Debug, Clone, Copy)]
enum Credentials {
    Valid,
    Rejected,
}

/// The declines each rail documents, as (reference, expected code) pairs.
///
/// Rail-specific *data*, one shared test body — the line that keeps this a
/// port test. The codes are `docs/flows/adapter-mtn-momo.md`'s and
/// `docs/flows/adapter-orange-money.md`'s mapping tables; a rail that grows a
/// documented reason grows a row here and a mapping in its own directory.
fn documented_declines(rail: RailUnderTest) -> Vec<(Uuid, FailureCode)> {
    match rail {
        RailUnderTest::MtnMomo => vec![
            (
                Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0f01),
                FailureCode::InsufficientFunds,
            ),
            (
                Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0f02),
                FailureCode::PayerTimeout,
            ),
            (
                Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0f03),
                FailureCode::ProviderAccountBlocked,
            ),
        ],
        RailUnderTest::OrangeMoney => vec![(
            Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0f01),
            FailureCode::PayerTimeout,
        )],
    }
}

/// The notification body each rail is documented to POST to its callback URL.
///
/// Rail-specific data again: the bodies are transcribed from each rail's flow
/// doc, and the shared assertion is that neither one yields a *status* — only
/// identifiers.
fn documented_callback_body(rail: RailUnderTest, reference: Uuid) -> Vec<u8> {
    match rail {
        RailUnderTest::MtnMomo => format!(
            r#"{{"externalId":"{reference}","amount":"5000","currency":"EUR",
                "payer":{{"partyIdType":"MSISDN","partyId":"237600000000"}},
                "status":"SUCCESSFUL","financialTransactionId":"1234567890"}}"#
        )
        .into_bytes(),
        RailUnderTest::OrangeMoney => format!(
            r#"{{"order_id":"{reference}","status":"SUCCESS","txnid":"stub-txn",
                "notif_token":"stub-notif-token","pay_token":"stub-pay-token"}}"#
        )
        .into_bytes(),
    }
}

/// Where each rail is documented to carry vpay's callback URL on a `submit`,
/// as a WireMock request pattern over that rail's submit endpoint.
///
/// Rail-specific *data*, one shared test body — the same line
/// [`documented_declines`] and [`documented_callback_body`] hold. Both rails'
/// protocol is push-then-callback and both carry the URL per request, but
/// they carry it in different *places*: MTN in the `X-Callback-Url` header
/// (`docs/flows/adapter-mtn-momo.md`), Orange in the request body's
/// `notif_url` (`docs/flows/adapter-orange-money.md`). A `Capabilities` value
/// could say *whether* a rail takes a per-request callback URL
/// (`delivers_callbacks`, which both set) but not *where* it puts it — that
/// is wire shape, and wire shape belongs to the adapter and to the mapping
/// directory. So it is a table row, exactly as a decline vocabulary is.
///
/// The pattern asserts the URL vpay was configured with arrived **verbatim**,
/// not merely that some URL did: `ProviderConfig::callback_url` is what
/// `vpay_config::ProviderHost::effective_callback_url` derives, and an
/// adapter that sent its `base_url`, an empty string or a hard-coded
/// constant would satisfy a presence check and be exactly as broken.
fn callback_url_pattern(rail: RailUnderTest, callback_url: &str) -> String {
    match rail {
        RailUnderTest::MtnMomo => format!(
            r#"{{"method":"POST","urlPath":"/collection/v1_0/requesttopay",
                 "headers":{{"X-Callback-Url":{{"equalTo":"{callback_url}"}}}}}}"#
        ),
        RailUnderTest::OrangeMoney => format!(
            r#"{{"method":"POST","urlPathPattern":"/orange-money-webpay/[^/]+/v1/webpayment",
                 "bodyPatterns":[{{"matchesJsonPath":
                   {{"expression":"$.notif_url","equalTo":"{callback_url}"}}}}]}}"#
        ),
    }
}

/// Where each rail is documented to carry the **payer's** return URL on a
/// `submit`, as a WireMock request pattern — or `None` for a rail that has no
/// browser step and must carry it nowhere.
///
/// Rail-specific *data*, the same line [`callback_url_pattern`] holds, and the
/// two are not the same question: that one is where the rail talks to *us*
/// (`notif_url` / `X-Callback-Url`, one value per deployment), this one is
/// where the rail sends the *payer* (one value per charge, D2 of
/// `docs/plans/2026-09-04-step9-hosted-checkout.md`). A rail could get one
/// right and the other wrong.
///
/// `None` is MTN's answer and it is not "unknown": a push rail prompts a
/// handset and never opens a browser, so there is no field for a return URL
/// to be right or wrong in. The absence is asserted by the shared body's
/// journal check rather than by a pattern, because a pattern can only say
/// what a request *does* contain.
///
/// Orange's pattern pins **both** `return_url` and `cancel_url` to the same
/// value, which is what the adapter sends: vpay cannot yet tell "the payer
/// paid" from "the payer gave up" on the way back — the outcome comes from
/// the authenticated status query — so two different URLs would be a
/// distinction nothing checks.
fn return_url_pattern(rail: RailUnderTest, return_url: &str) -> Option<String> {
    match rail {
        RailUnderTest::MtnMomo => None,
        RailUnderTest::OrangeMoney => Some(format!(
            r#"{{"method":"POST","urlPathPattern":"/orange-money-webpay/[^/]+/v1/webpayment",
                 "bodyPatterns":[
                   {{"matchesJsonPath":{{"expression":"$.return_url","equalTo":"{return_url}"}}}},
                   {{"matchesJsonPath":{{"expression":"$.cancel_url","equalTo":"{return_url}"}}}}]}}"#
        )),
    }
}

/// The stub's whole request journal, as text.
///
/// [`requests_matching`] can only ask what a request *contains*; this is what
/// a case needs to assert a value reached the rail **nowhere at all** — not in
/// a body, not in a header, not in a query string. The journal is the only
/// witness for a negative, exactly as it is for the redirect that must never
/// be followed.
///
/// Returned unparsed on purpose: the assertion is a substring search for one
/// URL, and this package deliberately carries no JSON dependency (see
/// [`requests_matching`]).
async fn recorded_requests(rail: &Rail) -> String {
    let http = vpay_provider::http::client().expect("the vendored-roots client builds");
    http.get(format!("{}/__admin/requests", rail.stub_origin))
        .send()
        .await
        .expect("the stub's admin API answers")
        .text()
        .await
        .expect("the journal is readable")
}

/// Every `submit` tells the rail where to send the **payer** — and only a rail
/// that has a browser to send them in.
///
/// # Why this is a conformance case
///
/// Because "the payer came back to the wrong place" is invisible from inside
/// an adapter and invisible from the payment's own outcome. The charge
/// settles either way: the status query is what moves money, and a payer
/// stranded on Orange's thank-you screen has still paid. What breaks is the
/// merchant's order flow, and it breaks in production, in a browser, for
/// somebody who has already been charged.
///
/// Until 2026-09-04 `vpay-adapter-orange-money` read `return_url` and
/// `cancel_url` from *deployment* settings, falling back to the notification
/// endpoint — one answer per deployment to a per-charge question. The
/// merchant's own `return_url` was validated, stored on `charges`, echoed
/// back to them on `next_action`, and never sent to the rail that would act
/// on it (`docs/flows/browser-checkout.md`'s D4). Every wire assertion in
/// this suite passed throughout.
///
/// # The two halves, and why the second one is the sharp one
///
/// [`return_url_pattern`] pins the exact value on the rail that carries it.
/// The journal check then says the value appears **nowhere** on a rail whose
/// flow gives it no browser — a stronger statement than any request pattern
/// can make, and the reason [`Rail::charge`] fills `return_url` on a push
/// charge too. `flow` is a capability value, so the branch is the one
/// ADR-0002 allows; the rail's name appears nowhere in this body.
///
/// The mappings hold the same contract from the other direction:
/// `webpayment.json` **requires** an `http(s)` `return_url` and `cancel_url`
/// on every accepted submit, so an adapter that stopped sending them gets a
/// 404 rather than an accepted payment.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn the_submit_tells_the_rail_where_to_send_the_payer_back(
    #[case] rail_under_test: RailUnderTest,
) {
    let rail = start(rail_under_test, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_ACCEPTED);
    assert_eq!(
        charge.return_url.as_deref(),
        Some(RETURN_URL),
        "the case is vacuous unless the core actually filled the field"
    );

    rail.adapter
        .submit(&charge, &rail.config)
        .await
        .expect("an accepted submit must be Ok");

    if let Some(pattern) = return_url_pattern(rail_under_test, RETURN_URL) {
        assert_eq!(
            requests_matching(&rail, &pattern).await,
            1,
            "{}: the rail received no submit carrying {RETURN_URL} where its protocol \
             documents one; a payer this rail redirects would come back somewhere this \
             merchant did not choose",
            rail.adapter.code(),
        );
    }

    let carries_the_payer = matches!(rail.adapter.capabilities().flow, ProviderFlow::Redirect);
    assert_eq!(
        recorded_requests(&rail).await.contains(RETURN_URL),
        carries_the_payer,
        "{}: a rail with flow {:?} must {} the payer's return URL, and the journal says \
         otherwise",
        rail.adapter.code(),
        rail.adapter.capabilities().flow,
        if carries_the_payer {
            "carry"
        } else {
            "never see"
        },
    );
}

/// Every `submit` tells the rail where to call back, with the URL this
/// deployment was configured with.
///
/// # Why this is a conformance case and not an adapter unit test
///
/// Because the failure it exists to catch is invisible from inside an
/// adapter. Both rails' documented protocol is push-then-callback; polling
/// alone settles a payment perfectly well (Step 4 proves it), so an adapter
/// that quietly stopped sending its callback URL would pass every other
/// assertion in this suite, settle every payment in
/// `backends/tests/integration`, and be discovered only by an MTN sandbox
/// registration failing or by a production deployment whose settlements were
/// all ten seconds late. The only witness is what the rail *received*, which
/// is what WireMock's request journal is.
///
/// The mappings assert the same thing a second way and from the other
/// direction: `requesttopay.json`'s catch-all and `webpayment.json`'s both
/// **require** the callback URL to match, so an adapter that stopped sending
/// it fails to match any mapping and gets a 404 rather than an accepted
/// submit. That belt is deliberate — this case is the braces, and it is the
/// half that names the exact value.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn the_submit_tells_the_rail_where_to_call_back(#[case] rail_under_test: RailUnderTest) {
    let rail = start(rail_under_test, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_ACCEPTED);

    // The path `vpay_api::provider_callback` mounts, which is also what
    // `ProviderHost::effective_callback_url` derives — `start` builds this
    // config the same way, so a change to either end shows up here as a
    // mismatch rather than as a silent agreement.
    assert!(
        rail.config
            .callback_url
            .ends_with(&format!("/provider/{}/callback", rail.adapter.code())),
        "the configuration under test must carry the route vpay actually mounts, got {}",
        rail.config.callback_url
    );

    rail.adapter
        .submit(&charge, &rail.config)
        .await
        .expect("an accepted submit must be Ok");

    let pattern = callback_url_pattern(rail_under_test, &rail.config.callback_url);
    assert_eq!(
        requests_matching(&rail, &pattern).await,
        1,
        "{}: the rail received no submit carrying {} where its protocol documents one; \
         a rail told nothing can only ever be settled by the poll ladder",
        rail.adapter.code(),
        rail.config.callback_url
    );
}

/// Proves an accepted submit is `Ok` on both rails, and that the shape of what
/// comes back follows the *declared flow* rather than the rail's name: a push
/// rail returns no redirect URL, and a redirect rail returns one together with
/// the `pay_token` that addresses it, in the same value — so a caller cannot
/// end up holding the URL without the token
/// (`docs/flows/crash-safety.md`). This is the adapter-level half of the
/// guarantee whose other half is the `confirm` handler's transaction boundary;
/// see the note at the foot of this file.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn submit_returns_a_reference_and_a_flow_shaped_result(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_ACCEPTED);

    let submitted = rail
        .adapter
        .submit(&charge, &rail.config)
        .await
        .expect("an accepted submit must be Ok");

    // The one place the two flows differ, expressed as a branch on a
    // *capability* rather than on a rail name.
    match rail.adapter.capabilities().flow {
        ProviderFlow::Push => assert_eq!(
            submitted.redirect_url, None,
            "a push rail prompts the payer on their handset; there is nowhere to redirect"
        ),
        ProviderFlow::Redirect => {
            assert!(
                submitted.redirect_url.is_some(),
                "a redirect rail must return the hosted page to send the payer to"
            );
            assert!(
                submitted.ref_extra.contains_key("pay_token"),
                "the key material must come back in the same value as the URL, so a caller \
                 cannot hold one without the other (docs/flows/crash-safety.md)"
            );
        }
    }
}

/// Proves a second submit of the same reference — what a crash between
/// persisting the reference and reading the rail's answer leaves behind — is
/// reported as submitted, not as an error and not as a decline, **and that
/// the second answer names the same payment as the first**.
///
/// Both rails must land on `Ok`; that is what makes recovery safe to
/// re-submit the reference it already durably holds. But `Ok` twice is not
/// on its own evidence, and on the redirect rail it was not: the second half
/// of the guarantee is that the retry does not create a *second* payment on
/// the rail under the same reference. Each flow states that differently, and
/// each is asserted below on capability rather than on a rail name:
///
/// * **push** — the rail is addressed by the reference we already hold, so
///   its 409 `RESOURCE_ALREADY_EXIST` carries no key material and none may
///   be invented. An empty `ref_extra` is the whole of the correct answer.
/// * **redirect** — the rail is addressed by the token *it* minted, so the
///   duplicate must come back with the token already minted for this
///   `order_id`. A second, different `pay_token` would mean two hosted pages
///   for one reference: the first one, whose URL the merchant may already
///   have handed the payer, is then a page we can no longer poll for.
///
/// # What makes the redirect half decisive
///
/// Nothing, while `REF_DUPLICATE` fell through to `webpayment.json`'s
/// priority-5 catch-all: that mapping templates `pay_token` from the
/// request's own `order_id`, so two submits of one reference agreed by
/// construction and this case could not have failed however the adapter
/// behaved — the Orange half was the accepted case run twice. It now has a
/// WireMock *scenario* of its own, which advances state on the first submit
/// and serves the second from a separate mapping that could answer anything.
/// It answers `pay-dup-1` both times on purpose, and returning `pay-dup-2`
/// from the `resubmitted` mapping fails the assertion below — checked by
/// hand, and recorded in that mapping's `metadata`.
///
/// The push half is weaker on purpose, because the port makes it so: a
/// `Submitted` carries no HTTP status, so nothing observable here separates
/// MTN's `409 RESOURCE_ALREADY_EXIST` from the catch-all `202` — deleting
/// the 409 mapping would leave this case green. What the 409 *means* is
/// pinned where the status is still visible, by
/// `vpay_adapter_mtn_momo`'s `a_duplicate_reference_is_a_success_not_an_error`
/// over `submit_outcome`. This case's job on a push rail is the other half:
/// that whatever the rail answered, no key material was invented for a
/// charge addressed solely by our own reference.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn duplicate_submit_reports_submitted_not_an_error(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_DUPLICATE);

    // Idempotency on our reference is what makes a same-reference retry safe
    // after a crash: the rail says "I already have that one", and that is a
    // success, not a decline.
    let first = rail
        .adapter
        .submit(&charge, &rail.config)
        .await
        .expect("the first submit must be Ok");
    let second = rail
        .adapter
        .submit(&charge, &rail.config)
        .await
        .expect("a duplicate submission must be reported as Submitted, never as an error");

    match rail.adapter.capabilities().flow {
        ProviderFlow::Push => {
            assert!(
                second.ref_extra.is_empty(),
                "a push rail's duplicate answer carries no key material — the reference we \
                 generated is the whole address, and inventing a value here would put \
                 something in `provider_ref_extra` that names nothing: {:?}",
                second.ref_extra
            );
            assert_eq!(
                second.redirect_url, None,
                "there is nowhere to send a payer whose handset is already being prompted"
            );
        }
        ProviderFlow::Redirect => {
            let first_token = first
                .ref_extra
                .get("pay_token")
                .expect("a redirect rail's accepted submit returns the token that addresses it");
            let second_token = second
                .ref_extra
                .get("pay_token")
                .expect("a duplicate submit must still return the token, not an empty map");
            assert_eq!(
                first_token, second_token,
                "a repeated order_id must re-issue the token already minted for it. Two \
                 different tokens would be two hosted pages for one reference, and the first \
                 — whose URL the merchant may already have given the payer — would be one we \
                 can no longer poll for (docs/flows/crash-safety.md)"
            );
        }
    }
}

/// Proves a 404 on a status read becomes [`ChargeStatus::NotFound`] and never
/// a failure. The distinction the recovery story rests on: a push rail can
/// answer 404 for a charge it is about to accept, so failing here would
/// abandon a payment that is still in flight.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn not_found_is_never_on_its_own_a_failure(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_UNKNOWN);

    let status = rail
        .adapter
        .query_status(&charge, &rail.config)
        .await
        .expect("a rail with no record of a reference must answer, not error");

    // The distinction the whole recovery story rests on: "I have no record"
    // is not "it failed". A push rail can answer 404 for a charge it is about
    // to accept (docs/flows/crash-safety.md), and failing the charge here
    // would lose a payment that is still in flight.
    assert_eq!(status, ChargeStatus::NotFound);
    assert!(
        !matches!(status, ChargeStatus::Failed { .. }),
        "NotFound must never be reported as a failure"
    );
}

/// Proves each rail's *documented* decline reasons land on the taxonomy code
/// its flow doc promises, and that the rail's own words survive in `raw` for
/// an operator to read. One shared body over per-rail data
/// ([`documented_declines`]) — the line that keeps this a port test instead of
/// two rail suites in one file.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn a_declined_charge_maps_to_the_documented_failure_code(
    #[case] rail_under_test: RailUnderTest,
) {
    let rail = start(rail_under_test, Credentials::Valid, Duration::from_secs(10)).await;

    for (reference, expected) in documented_declines(rail_under_test) {
        let charge = rail.charge(reference);
        let status = rail
            .adapter
            .query_status(&charge, &rail.config)
            .await
            .expect("a documented decline is an answer, not a transport failure");

        match status {
            ChargeStatus::Failed { code, raw } => {
                assert_eq!(code, expected, "reference {reference} mapped to {code}");
                assert!(
                    !raw.is_empty(),
                    "the rail's own reason must be carried through for an operator, \
                     even though the taxonomy is what the merchant sees"
                );
            }
            other => panic!("reference {reference} expected a decline, got {other:?}"),
        }
    }
}

/// Proves both ways a rail can be unreachable — an explicit 503, and an answer
/// slower than the configured request deadline — surface as
/// [`ProviderError::Transport`] classified `Category::Rail`, which is what
/// makes the worker retry rather than the merchant start a new intent. A rail
/// that is down, reported as a decline, would fail live charges and page
/// nobody.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn an_unavailable_rail_is_a_transport_error_never_a_decline(#[case] rail: RailUnderTest) {
    use vpay_core::{Category, Classify as _};

    let rail = start(rail, Credentials::Valid, SHORT_REQUEST_TIMEOUT).await;

    for (reference, what) in [(REF_UNAVAILABLE, "a 503"), (REF_SLOW, "a timeout")] {
        let charge = rail.charge(reference);
        let error = rail
            .adapter
            .query_status(&charge, &rail.config)
            .await
            .expect_err(&format!("{what} must be an error, not a status"));

        // The distinction that decides whether the worker retries or the
        // merchant starts a new intent. A rail that is down must never look
        // like a payer who was declined.
        assert!(
            matches!(error, ProviderError::Transport { .. }),
            "{what} must be a Transport error, got {error:?}"
        );
        assert_eq!(error.category(), Category::Rail);
    }
}

/// Proves credentials the rail rejects come back as
/// `Rejected { ProviderAccountBlocked }` at `Severity::Page`. Same rail, same
/// stub, one wrong value — which is exactly how it happens in production, on
/// the day a key rotates. Reported as an ordinary decline, a total outage
/// would sit unnoticed among payers' insufficient funds.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn bad_credentials_are_not_reported_as_a_payer_problem(#[case] rail: RailUnderTest) {
    use vpay_core::{Classify as _, Severity};

    let rail = start(rail, Credentials::Rejected, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_ACCEPTED);

    let error = rail
        .adapter
        .submit(&charge, &rail.config)
        .await
        .expect_err("credentials the rail rejects cannot produce a submitted charge");

    // Every charge on this rail is failing and no payer can fix it, so it
    // pages (docs/flows/failures.md). Reporting it as `InsufficientFunds` or
    // as a generic decline would bury an outage among ordinary declines.
    match &error {
        ProviderError::Rejected { code, .. } => {
            assert_eq!(*code, FailureCode::ProviderAccountBlocked);
            assert_eq!(error.severity(), Severity::Page);
        }
        other => panic!("expected a ProviderAccountBlocked rejection, got {other:?}"),
    }
}

/// Proves each rail's documented notification body parses to the reference it
/// is about, and that it yields *no status* — structurally, because
/// `CallbackRef` has no such field. "Callbacks are hints": only the
/// authenticated status query moves money, and the type assertion below is
/// what stops compiling the day someone adds a status field.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn a_callback_body_round_trips_to_identifiers_only(#[case] rail_under_test: RailUnderTest) {
    // No container: parsing a callback body is pure, and must stay pure — the
    // port's `parse_callback` is synchronous precisely so an adapter cannot
    // reach the network while "parsing" an unauthenticated request.
    let http = vpay_provider::http::client().expect("the vendored-roots client builds");
    let adapter: Box<dyn ProviderAdapter> = match rail_under_test {
        RailUnderTest::MtnMomo => Box::new(vpay_adapter_mtn_momo::Adapter::new(http)),
        RailUnderTest::OrangeMoney => Box::new(vpay_adapter_orange_money::Adapter::new(http)),
    };

    let parsed = adapter
        .parse_callback(&documented_callback_body(rail_under_test, REF_ACCEPTED))
        .expect("a documented notification body must parse");

    assert_eq!(
        parsed.reference_id, REF_ACCEPTED,
        "a callback must identify the charge it is about"
    );
    // That `CallbackRef` carries no status is structural — it has no such
    // field — and this line is what would stop compiling if one were added.
    // "Callbacks are hints"; only the authenticated status query moves money.
    let _: &vpay_provider::RefExtra = &parsed.ref_extra;
}

/// The behavioural half of the refund contract, on a configured rail.
///
/// Proves a rail with no refund API answers the permanent
/// [`ProviderError::Unsupported`] — not `NotImplemented`, because there is
/// nothing to build — and that a rail which *does* advertise refunds never
/// answers `Unsupported`. `mtn_momo::refund` is still an unbuilt
/// `NotImplemented` token (Disbursements is a separate product, see
/// `docs/status.md`); this case is what keeps that token honest, which is why
/// it runs rather than being `#[ignore]`d for it.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn a_rail_without_the_refund_capability_answers_unsupported(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_ACCEPTED);
    let amount = charge.amount;

    let outcome = rail.adapter.refund(&charge, amount, &rail.config).await;

    if rail.adapter.capabilities().supports_refunds {
        // A rail that *can* refund must not claim the operation is
        // unsupported — whatever else it answers while the call is unbuilt.
        assert!(
            !matches!(outcome, Err(ProviderError::Unsupported)),
            "a rail advertising supports_refunds must not answer Unsupported"
        );
    } else {
        // `Unsupported`, not `NotImplemented`: there is nothing to build. The
        // rail has no refund API, the capability says so, and the core is
        // meant to have branched before it ever called (ADR-0002).
        assert!(
            matches!(outcome, Err(ProviderError::Unsupported)),
            "a rail with no refund API must answer Unsupported, not NotImplemented: {outcome:?}"
        );
    }
}

/// Proves `query_status` reports what the rail says *now* and never caches its
/// first answer: one charge, two calls, `Pending` then a success, through a
/// WireMock scenario. The poll ladder in `docs/flows/reconciler.md` is built
/// entirely on that property.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn pending_then_successful_walks_the_scenario(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_SCENARIO);

    let first = rail
        .adapter
        .query_status(&charge, &rail.config)
        .await
        .expect("the first status query answers");
    let second = rail
        .adapter
        .query_status(&charge, &rail.config)
        .await
        .expect("the second status query answers");

    // `query_status` must stay callable and must report what the rail says
    // *now*, not a cached first answer — the poll ladder in
    // docs/flows/reconciler.md is built entirely on that.
    assert_eq!(first, ChargeStatus::Pending);
    assert!(
        matches!(second, ChargeStatus::Succeeded { .. }),
        "the scenario's second answer must be a success: {second:?}"
    );
}

/// Proves a rail's `3xx` is **refused**, and — decisively — that the
/// `Location` is never requested.
///
/// The finding this pins: reqwest follows up to ten redirects by default and
/// strips only `Authorization`, `Cookie` and `Proxy-Authorization` on a
/// cross-host hop. Every header a rail adapter actually authenticates with is
/// therefore replayed — MTN's `Ocp-Apim-Subscription-Key`,
/// `X-Target-Environment`, `X-Reference-Id`, `X-Callback-Url` — and a 307 or
/// 308 replays the request *body*, which on Orange's `webpayment` carries
/// `merchant_key`. A rail answering `302 Location: https://attacker.example/`
/// would have been handed a merchant's rail credentials and the identity of a
/// live charge.
///
/// Two independent assertions, because the return value alone is not enough:
/// a followed redirect could plausibly still end in an error, and the
/// assertion would pass while the credentials had already left. So the stubs
/// answer the `Location` with the rail's **accepted** status, which makes a
/// followed redirect an `Ok(Submitted)` the `expect_err` catches, and the
/// stub's own request journal is then asked whether the path was ever
/// requested at all. The journal is the decisive one.
///
/// `Malformed`, and nothing else. This accepted `Malformed | Transport`
/// until the Step 3 review, on the reasoning that where a 3xx lands is a
/// detail of each adapter's own table. Both adapters now answer it from an
/// explicit `status.is_redirection()` arm, on every call, so the
/// alternative bought nothing and cost the assertion its edge: `Transport`
/// is the *retryable* category (`docs/flows/errors.md`), and a rail
/// answering 3xx is a rail to stop and look at, not one to poll again on a
/// ladder. Accepting both classifications meant this case could not tell
/// the two apart — including for a rail added tomorrow, which is the whole
/// point of a conformance suite.
///
/// What that does **not** claim, because it is not true of either adapter
/// today: that deleting the arm would make this case fail. MTN's
/// `submit_outcome` falls through to a `Malformed` catch-all, so the
/// classification survives the deletion; Orange's `webpayment` falls
/// through to `Rejected { provider_error }`, which this case has always
/// caught. The narrowing is about what the suite *permits* a rail to do,
/// and it was checked to be non-vacuous by temporarily answering
/// `Transport` from MTN's `submit_outcome` redirect arm and watching the
/// `mtn_momo` half of this case fail on exactly that value.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn redirects_are_refused_and_never_followed(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_REDIRECT);

    let error = rail
        .adapter
        .submit(&charge, &rail.config)
        .await
        .expect_err("a redirect is not an accepted charge; the rail pointed elsewhere");

    assert!(
        matches!(error, ProviderError::Malformed { .. }),
        "a 3xx must not be read as a decline, an acceptance, or a retryable transport \
         failure. `Transport` is the category the worker retries on a ladder, and a rail \
         answering redirects is one to stop and look at — its charge's fate is unknown, \
         which is what `Malformed` says: {error:?}"
    );
    assert_eq!(
        requests_recorded_for(&rail, REDIRECT_TARGET).await,
        0,
        "the Location was requested: the rail's credentials and this charge's identity \
         were replayed at whatever host answered it"
    );
}

/// Proves a rail cannot decide how much memory this process allocates.
///
/// `Response::text()`/`bytes()` read to end of stream. One worker task per
/// charge, each willing to buffer whatever a rail sends, is a memory
/// exhaustion whose size the peer chooses — and the peer here is a host
/// reached over the internet, which on a bad day is a load balancer's error
/// page and on a worse one is not the rail at all.
///
/// The stubbed body is a *valid, successful* status padded past the cap, so
/// the case cannot pass for the wrong reason: without the bound the adapter
/// would parse it and answer `Succeeded`. The assertion on the message is
/// what makes this a test of the **cap** rather than of any old parse
/// failure — the error has to name the limit that was hit.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn an_oversized_rail_body_is_refused_at_the_cap(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    let charge = rail.charge(REF_HUGE);

    let error = rail
        .adapter
        .query_status(&charge, &rail.config)
        .await
        .expect_err("a body past the cap must not be buffered and parsed");

    let message = error.to_string();
    match &error {
        ProviderError::Malformed { .. } => assert!(
            message.contains(&vpay_provider::http::MAX_RAIL_BODY_BYTES.to_string()),
            "the refusal must name the cap it hit, so an operator can tell it from any \
             other parse failure: {message}"
        ),
        other => panic!("expected Malformed naming the body cap, got {other:?}"),
    }
}

// --------------------------------------------------------------------------
// Account-holder lookups (issue #47).
//
// Five cases, each parameterised over both rails out of ONE body. Nothing
// below branches on `RailUnderTest`: the branch is
// `capabilities().supports_account_holder_lookup`, a capability *value*, so
// "Orange has no such API" is asserted rather than skipped, and a third rail
// added tomorrow is covered by declaring the flag and adding one mappings
// file. The pattern is exactly the one
// `a_rail_without_the_refund_capability_answers_unsupported` already sets.
// --------------------------------------------------------------------------

/// Asserts the shared refusal a rail without the capability must produce,
/// and answers whether the caller should go on.
///
/// A helper rather than a copied four-line `if` in each case below, because
/// the refusal is the *same* contract every time and a copy is what
/// eventually says `NotImplemented` in one case and `Unsupported` in
/// another. Returns `true` when this rail does claim the capability and the
/// case's real assertions should run.
///
/// `Unsupported` and never `NotImplemented`: there is nothing for Orange to
/// build. Its Web Payment product documents no account-holder API, so the
/// capability is a permanent answer the core branches on (ADR-0002), and a
/// `NotImplemented` token would claim work someone owes.
async fn claims_account_holder_lookup(rail: &Rail) -> bool {
    if rail.adapter.capabilities().supports_account_holder_lookup {
        return true;
    }
    let outcome = rail
        .adapter
        .account_holder_name(MSISDN_REGISTERED, &rail.config)
        .await;
    assert!(
        matches!(outcome, Err(ProviderError::Unsupported)),
        "{}: a rail with no account-holder API must answer Unsupported, not NotImplemented \
         and certainly not Ok(None): {outcome:?}",
        rail.adapter.code(),
    );
    false
}

/// A number the rail knows comes back as a **name**, projected out of
/// whatever else the rail's body carried.
///
/// The `Ok(Some(..))` half of the three-way answer. The stub sends MTN's
/// documented six-field body (`wiremock/mtn/mappings/basicuserinfo.json`),
/// four fields of which have no home in the adapter's wire type — so a rail
/// that grew a seventh field would be dropped by the same rule, without an
/// edit here.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn an_account_holder_lookup_returns_a_name_and_nothing_else(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    if !claims_account_holder_lookup(&rail).await {
        return;
    }

    let holder = rail
        .adapter
        .account_holder_name(MSISDN_REGISTERED, &rail.config)
        .await
        .expect("a number the rail knows is an answer, not a failure")
        .expect("and the rail named its holder");

    assert_eq!(holder.name(), REGISTERED_HOLDER_NAME);

    // The struct has one field and its `Debug` redacts it, so there is no
    // rendering in which a birthdate, a locale or a gender could appear.
    // Asserted rather than assumed: this is the whole privacy claim.
    let rendered = format!("{holder:?}");
    for dropped in ["1970-01-01", "fr_CM", "MALE", "ACTIVE"] {
        assert!(
            !rendered.contains(dropped),
            "{}: {dropped} survived the projection into {rendered}",
            rail.adapter.code(),
        );
    }
    assert!(
        !rendered.contains(REGISTERED_HOLDER_NAME),
        "{}: AccountHolder's Debug must redact the name, so a `{{:?}}` cannot leak it: \
         {rendered}",
        rail.adapter.code(),
    );
}

/// A number the rail has no record of is `Ok(None)` — **an answer**, not an
/// error, and not a fabricated name.
///
/// The distinction the whole method exists for: `Ok(None)` is a fact about
/// the *number*, which a caller may act on (issue #47's nominated-refund
/// name match refuses on it). Every other non-answer is an `Err`, and
/// [`a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder`]
/// is the case that pins the other side of it.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn a_number_the_rail_has_no_record_of_is_not_an_error(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    if !claims_account_holder_lookup(&rail).await {
        return;
    }

    let outcome = rail
        .adapter
        .account_holder_name(MSISDN_UNREGISTERED, &rail.config)
        .await;

    match outcome {
        Ok(None) => {}
        Ok(Some(holder)) => panic!(
            "{}: the rail has no record of {MSISDN_UNREGISTERED} and the adapter named \
             somebody anyway: {holder:?}",
            rail.adapter.code(),
        ),
        Err(error) => panic!(
            "{}: 'no such holder' is an answer, not a failure — a caller cannot tell this \
             apart from an outage if it arrives as an error: {error:?}",
            rail.adapter.code(),
        ),
    }
}

/// A rail that cannot be *asked* is an error carrying a readable source
/// chain — never `Ok(None)`.
///
/// # Why this is the sharp one
///
/// `Ok(None)` reaches issue #47's caller as "this number is not registered",
/// which refuses a buyer's nominated refund destination on the strength of a
/// lookup that never happened. Reported honestly as a transport failure the
/// same caller refuses too — but a support ticket about it says "vpay could
/// not reach MTN" instead of "your account does not exist", and only one of
/// those two is fixable.
///
/// Steered by a **timeout** rather than a 503 on purpose: a rail that
/// answered badly produces a `Transport` with no `source`, while a deadline
/// that fires produces one carrying the `reqwest` error, and the assertion
/// below walks it with [`vpay_core::error::source_chain`]. `reqwest`'s own
/// `Display` for a timeout stops at "error sending request for url (…)" —
/// the word *timeout* is one link further down, which is exactly the
/// diagnosis an operator needs and exactly what flattening the error into a
/// string throws away.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder(
    #[case] rail: RailUnderTest,
) {
    use vpay_core::{Category, Classify as _};

    let rail = start(rail, Credentials::Valid, SHORT_REQUEST_TIMEOUT).await;
    if !claims_account_holder_lookup(&rail).await {
        return;
    }

    let error = rail
        .adapter
        .account_holder_name(MSISDN_SLOW, &rail.config)
        .await
        .expect_err("a deadline that fires is a failure, and must not be Ok(None)");

    assert!(
        matches!(error, ProviderError::Transport { .. }),
        "{}: a rail we could not finish talking to is a Transport error: {error:?}",
        rail.adapter.code(),
    );
    // `Category::Rail` is what makes the `/v1` boundary answer 502 rather
    // than a 200 with nulls — the derivation ADR-0011 puts in one place.
    assert_eq!(error.category(), Category::Rail);

    let chain = vpay_core::error::source_chain(&error);
    assert!(
        !chain.is_empty(),
        "{}: the reqwest error must stay reachable through source(), not be flattened into \
         the message: {error} / chain {chain:?}",
        rail.adapter.code(),
    );
}

/// A lookup body past the cap is refused at the cap, naming it.
///
/// The same bound `an_oversized_rail_body_is_refused_at_the_cap` proves for
/// a status query, on the one endpoint a **merchant** can provoke directly:
/// `GET /v1/account_holders` turns one inbound request into one rail call,
/// so without the bound the rail would choose how much memory an
/// unauthenticated-to-it peer allocates, once per request rather than once
/// per charge.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn an_oversized_account_holder_body_is_refused_at_the_cap(#[case] rail: RailUnderTest) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    if !claims_account_holder_lookup(&rail).await {
        return;
    }

    let error = rail
        .adapter
        .account_holder_name(MSISDN_HUGE, &rail.config)
        .await
        .expect_err("a body past the cap must not be buffered and parsed");

    let message = error.to_string();
    match &error {
        ProviderError::Malformed { .. } => assert!(
            message.contains(&vpay_provider::http::MAX_RAIL_BODY_BYTES.to_string()),
            "the refusal must name the cap it hit: {message}"
        ),
        other => panic!("expected Malformed naming the body cap, got {other:?}"),
    }
}

/// **The privacy case.** A rail body full of a real person's details yields
/// a name, and puts none of it — the name included — in a log line.
///
/// # Why the assertion is on captured output and not on the call site
///
/// "We did not log it" is only checkable by looking at what was logged.
/// A test that asserted the adapter calls no `tracing` macro would pass the
/// day someone logged the body from a different macro, from a helper, or
/// from `reqwest` itself; a scoped subscriber over the call is the only
/// witness that cannot be talked around. `with_max_level(TRACE)` so a
/// *negative* assertion cannot pass because an event was filtered out, and
/// `with_ansi(false)` so a colour escape cannot sit between a field name and
/// its value and defeat a `contains`.
///
/// # The two halves
///
/// The name must reach the caller — the route needs it — and must not reach
/// the log. Everything else in the body must reach neither. The stub sends
/// five fields MTN does not even document (`sub`, `email`, `address`,
/// `national_id`, `phone_number`) precisely so the projection is proven
/// against a rail that changed, not only against the one that was written
/// down.
#[rstest]
#[case::mtn_momo(RailUnderTest::MtnMomo)]
#[case::orange_money(RailUnderTest::OrangeMoney)]
#[tokio::test]
async fn an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing(
    #[case] rail: RailUnderTest,
) {
    let rail = start(rail, Credentials::Valid, Duration::from_secs(10)).await;
    if !claims_account_holder_lookup(&rail).await {
        return;
    }

    let sink = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();

    // `set_default` and not `with_default`: the call is `async`, and a
    // closure returning a future would install the subscriber only while the
    // future is *built*. The guard has to be held across the `.await`.
    let guard = tracing::subscriber::set_default(subscriber);
    let outcome = rail
        .adapter
        .account_holder_name(MSISDN_FULL_OF_PII, &rail.config)
        .await;
    drop(guard);

    let holder = outcome
        .expect("a number the rail knows is an answer")
        .expect("and the rail named its holder");
    assert_eq!(
        holder.name(),
        PII_HOLDER_NAME,
        "{}: the name is the one thing this call is for",
        rail.adapter.code(),
    );

    for leaked in PERSONAL_DATA_THE_RAIL_SENDS {
        assert!(
            !format!("{holder:?}").contains(leaked),
            "{}: {leaked} reached the returned value",
            rail.adapter.code(),
        );
    }

    let logged = sink.contents();
    assert!(
        !logged.is_empty(),
        "{}: nothing was captured at all, so every assertion below would pass vacuously — \
         the adapter logs at least one debug line per rail call",
        rail.adapter.code(),
    );
    for leaked in PERSONAL_DATA_THE_RAIL_SENDS {
        assert!(
            !logged.contains(leaked),
            "{}: {leaked} reached a log line:\n{logged}",
            rail.adapter.code(),
        );
    }
    assert!(
        !logged.contains(PII_HOLDER_NAME),
        "{}: the holder's NAME reached a log line — the one thing \
         docs/flows/account-holder-lookup.md forbids outright:\n{logged}",
        rail.adapter.code(),
    );
    assert!(
        !logged.contains(MSISDN_FULL_OF_PII),
        "{}: the payer's number reached a log line unmasked; only `/v1`'s masked form may \
         be logged:\n{logged}",
        rail.adapter.code(),
    );
}

/// An in-memory `tracing` sink, for the case above.
///
/// The same shape `vpay_api`'s `test_log` module uses, and it is duplicated
/// rather than shared for a structural reason: that one is `#[cfg(test)]`
/// inside `vpay-api`, which is a *library*, so nothing outside that crate's
/// own test build can name it. Lifting it into `vpay-testkit` was the
/// alternative and is worse — the testkit is a dependency of shipping
/// crates' dev-builds and this is nine lines of `io::Write`.
///
/// The `Arc<Mutex<..>>` is not incidental: `MakeWriter` hands out a fresh
/// writer per event, so the buffer has to be shared by handle for the caller
/// to read back what every event wrote.
#[derive(Clone, Default)]
struct CapturedLog(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl CapturedLog {
    /// Everything written so far, lossily decoded. A poisoned mutex is read
    /// through rather than treated as a failure: a panic on another thread
    /// is a test failure whose own assertion reports it far better than
    /// "poisoned mutex" would, and losing the captured output at that moment
    /// would hide the log line that explains it.
    fn contents(&self) -> String {
        self.0.lock().map_or_else(
            |poisoned| String::from_utf8_lossy(&poisoned.into_inner()).into_owned(),
            |bytes| String::from_utf8_lossy(&bytes).into_owned(),
        )
    }
}

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.0.lock() {
            Ok(mut sink) => sink.write(buf),
            Err(poisoned) => poisoned.into_inner().write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// `redirect_rails_commit_ref_extra_before_returning_a_url` used to live here
// and has been deleted rather than ported. It asserted an ordering this layer
// cannot observe: whether `provider_ref_extra` was *committed* before a
// `redirect_url` reached a caller is a property of the `confirm` handler's
// transaction boundary, not of an adapter — an adapter has no database and
// returns both values in one `Submitted`. Its real home is
// `backends/tests/integration/tests/payment_intents.rs`, asserting that after
// a confirm answering `next_action.redirect_to_url` the `charges` row already
// carries a non-NULL `provider_ref_extra`, and that a confirm whose post-submit
// UPDATE fails answers 500 with no `next_action`. The adapter-level half of the
// guarantee is `submit_returns_a_reference_and_a_flow_shaped_result` above:
// the rail cannot hand back a URL without the token beside it.

/// Every other wire-level case above steers by *reference*: `Rail::charge`
/// hands `query_status` a manufactured `ChargeRef` with a fixed placeholder
/// payer (`237600000000`) and lets the reference pick which stubbed answer
/// comes back. MTN's three documentation-MSISDN scenarios
/// (`requesttopay-scenario.json`'s `mtn-e2e-poll`, `demo-outcomes.json`'s
/// `mtn-demo-decline` and `mtn-demo-expiry`) steer the opposite way — on the
/// **submit**'s body, `$.payer.partyId` — because that is the one field of an
/// end-to-end confirm a payer actually controls; MTN's status query is a `GET`
/// carrying no body, so nothing about *it* can be steered. Every other case in
/// this file is therefore silent about whether that steering still works,
/// which is what this one exists to check, and for real: `submit` is called
/// here, not skipped.
///
/// The three MSISDNs are `237600000ce0`, `237600000f01` and `237600000f02` —
/// except this case sends their **digits-only twins**, `237600000100`,
/// `237600000101` and `237600000102`. The hex originals carry a letter so a
/// WireMock mapping can key on them, which is exactly why a real payer can
/// never send one: `frontends/apps/checkout/src/lib/msisdn.ts` validates
/// Cameroon E.164 (`237` + `6` + eight digits) and refuses a letter as it
/// refuses any other non-digit (`docs/plans/step9-notes/lane-3.md` #4c). The
/// three mappings above now match *either* PartyId, by regex, into the exact
/// same scenario — so this case is the proof that the number a real phone
/// form accepts reaches the identical walk the hex documentation number
/// always has, not a second, differently-tested path. The full steering
/// table for both families is `docs/plans/step9-notes/lane-2b.md`.
///
/// MTN only, deliberately not parameterised over `RailUnderTest`: Orange is a
/// redirect rail whose submit body carries no MSISDN at all (`order_id`
/// selects everything), so there is nothing on that rail for a payer's typed
/// phone number to steer.
#[rstest]
#[case::settles("237600000100", None)]
#[case::insufficient_funds("237600000101", Some(FailureCode::InsufficientFunds))]
#[case::payer_timeout("237600000102", Some(FailureCode::PayerTimeout))]
#[tokio::test]
async fn a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin(
    #[case] msisdn: &str,
    #[case] expected_decline: Option<FailureCode>,
) {
    let rail = start(
        RailUnderTest::MtnMomo,
        Credentials::Valid,
        Duration::from_secs(10),
    )
    .await;

    // Not `rail.charge(reference)`: that helper hard-codes the placeholder
    // payer this case exists to steer away from. The reference is a fresh
    // random UUID, exactly as `vpay_api`'s confirm handler mints one before
    // committing a charge (the reason the *reference* cannot be the steering
    // field the three mappings above document).
    let charge = ChargeRef {
        reference_id: Uuid::new_v4(),
        amount: Money::new(5_000, rail.config.currency).expect("non-negative"),
        payer_ref: Some(msisdn.to_owned()),
        ref_extra: BTreeMap::new(),
        return_url: Some(RETURN_URL.to_owned()),
    };

    let submitted = rail
        .adapter
        .submit(&charge, &rail.config)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "msisdn {msisdn}: MTN accepts every one of these submits \
             — the steering happens after, on query_status — got {error:?}"
            )
        });
    assert!(
        submitted.redirect_url.is_none(),
        "msisdn {msisdn}: a push rail's submit carries no redirect"
    );

    match expected_decline {
        None => {
            let first = rail
                .adapter
                .query_status(&charge, &rail.config)
                .await
                .unwrap_or_else(|error| {
                    panic!("msisdn {msisdn}: the first status query answers, got {error:?}")
                });
            assert_eq!(first, ChargeStatus::Pending, "msisdn {msisdn}: first query");
            let second = rail
                .adapter
                .query_status(&charge, &rail.config)
                .await
                .expect("the second status query answers");
            assert!(
                matches!(second, ChargeStatus::Succeeded { .. }),
                "msisdn {msisdn}: expected to settle on the second query, got {second:?}"
            );
        }
        Some(expected) => {
            let status = rail
                .adapter
                .query_status(&charge, &rail.config)
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "msisdn {msisdn}: a documented decline is an answer, not a transport \
                         failure: {error:?}"
                    )
                });
            match status {
                ChargeStatus::Failed { code, raw } => {
                    assert_eq!(code, expected, "msisdn {msisdn} mapped to {code}");
                    assert!(
                        !raw.is_empty(),
                        "msisdn {msisdn}: the rail's own reason must be carried through for an \
                         operator, even though the taxonomy is what the merchant sees"
                    );
                }
                other => panic!("msisdn {msisdn}: expected a decline, got {other:?}"),
            }
        }
    }
}
