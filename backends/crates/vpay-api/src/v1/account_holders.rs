//! `GET /v1/account_holders` — "whose mobile-money account is this number?"
//!
//! The merchant-facing half of issue #47. It answers with a **name**, or
//! with the fact that the rail has no record, and it can tell the two apart
//! from "we could not ask" — which is the whole reason it exists. Its
//! caller is an integrator refusing a nominated refund destination whose
//! holder they cannot match, and a refusal on a lookup that never happened
//! is a different thing from a refusal on a number nobody holds.
//!
//! [docs/flows/account-holder-lookup.md](../../../../../docs/flows/account-holder-lookup.md)
//! is this route's flow doc and carries the policy: what may be logged, what
//! is deliberately not stored, and the one abuse question left open.
//!
//! # This route is materially different from the rest of `/v1`
//!
//! Everything else on this surface is about the merchant's own objects. This
//! is a lookup of **a third party** by phone number, so four rules apply
//! here and nowhere else, and each one is a thing this module does rather
//! than a sentence somewhere:
//!
//! 1. **A name, and nothing else.** MTN's `basicuserinfo` body carries a
//!    birthdate, a locale and a gender too; the projection happens in the
//!    adapter's wire type, which has no field for them, so nothing this
//!    module could render even exists. See
//!    `vpay_adapter_mtn_momo::wire::BasicUserInfo`.
//! 2. **Nothing is persisted** — not the name, not the number, not the fact
//!    that the question was asked. There is no repository call in this file
//!    and no migration behind it. That is a decision with a cost, and the
//!    flow doc records both halves: it also means a merchant enumerating the
//!    number space leaves no record in vpay.
//! 3. **The log carries a masked number and never a name.** One line, at
//!    `info`, with [`masked`]'s `+2376••••000` shape and the outcome.
//!    `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing`
//!    in `backends/tests/conformance` asserts the adapter half against
//!    captured `tracing` output; [`a_lookup_logs_a_masked_number_and_never_a_name`]
//!    asserts this half the same way.
//! 4. **The metric's labels carry neither.** One counter, one label, four
//!    constants ([`vpay_core::metrics::account_holder_outcome`]) — a
//!    Prometheus label is retained and shipped wherever the scrape goes, so
//!    a number in one would be the record rule 2 exists not to keep.
//!
//! # What this route deliberately does not do
//!
//! **No rate limit.** Issue #47 §3 asks for one, and it is not here: rate
//! limiting in this deployment is an ingress concern
//! (`docs/flows/provider-port.md` records the same gap for the callback
//! route) and inventing a per-merchant bucket in one handler would be a
//! control nothing else on this surface has. It is a **reserved decision**
//! for the maintainer, written down in the flow doc rather than defaulted
//! to here.
//!
//! **No scope of its own.** §3 also asks for one, and this route is served
//! under [`crate::v1::SCOPE_PAYMENTS_READ`] like every other `GET`. Adding
//! `identity:read` means minting it, checking it, and documenting it in
//! three places that fail *silently* when they disagree (see
//! `SCOPE_PAYMENTS_WRITE`'s own doc comment) — and it would refuse every
//! existing merchant credential on the day it landed. Also a reserved
//! decision, also in the flow doc.
//!
//! **No audit log.** §3 asks for that too, and it contradicts rule 2 above:
//! a per-merchant, per-MSISDN audit trail *is* a stored record of who asked
//! about whom. Which of the two wins is exactly the sort of choice this
//! repository does not let an implementer make quietly.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use vpay_core::metrics::{ACCOUNT_HOLDER_LOOKUPS_TOTAL, account_holder_outcome};
use vpay_provider::ProviderAdapter;

use crate::error::ApiError;
use crate::form::VpayQuery;
use crate::model::{AccountHolderObject, AccountHolderTag};
use crate::v1::{MerchantScope, ResourceConfig};

/// Cameroon's country calling code.
///
/// The market this deployment serves, and the same rule
/// `frontends/apps/checkout/src/lib/msisdn.ts` enforces in the payer's
/// browser — see [`canonical_msisdn`] for why the two are separate
/// implementations rather than one.
const CM_COUNTRY_CODE: &str = "237";

/// Every Cameroon mobile number begins with this digit.
const CM_MOBILE_PREFIX: char = '6';

/// Digits in a Cameroon national mobile number, the leading `6` included.
const CM_NATIONAL_DIGITS: usize = 9;

/// The longest input [`canonical_msisdn`] will even look at.
///
/// A query parameter is caller-controlled and this endpoint is reached
/// before anything else validates its length. The bound is generous — a
/// fully separated `+237 6 71 23 45 67` is 20 characters — and exists so a
/// megabyte of digits is refused by a length check rather than by a loop.
const MAX_MSISDN_INPUT_CHARS: usize = 32;

/// The parameter a merchant sends the number as, and the name a refusal
/// echoes into the envelope's `param`.
const MSISDN_PARAM: &str = "msisdn";

/// The parameter naming the rail, and the name a refusal echoes.
const PAYMENT_METHOD_TYPE_PARAM: &str = "payment_method_type";

/// `GET /v1/account_holders`'s query parameters.
///
/// Both are `Option<String>` and neither is typed further, for
/// `crate::v1::events::ListParams`' reason: the wire carries strings, and
/// letting serde reject a shape produces a sentence about the request
/// instead of one naming the parameter an SDK should point at.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct RetrieveParams {
    msisdn: Option<String>,
    payment_method_type: Option<String>,
}

/// `GET /v1/account_holders?msisdn=…&payment_method_type=…`.
///
/// # Why `MerchantScope` is extracted and then unused
///
/// There is nothing to scope: no query is run, no row is read, and the
/// answer is a property of the rail rather than of a tenant. Binding it
/// anyway is what makes the *authentication* boundary structural rather than
/// remembered — the extractor fails closed with a paging 500 when the
/// middleware is not mounted in front of the route
/// (`MerchantScope::from_request_parts`), so a future refactor that dropped
/// the layer would fail here instead of serving an unauthenticated identity
/// lookup. It is also the value an audit log would be keyed on the day the
/// reserved decision in this module's header is taken.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] naming `payment_method_type` for a missing
/// one, for a rail this deployment does not offer, and for a rail whose
/// [`vpay_provider::Capabilities::supports_account_holder_lookup`] is false;
/// naming `msisdn` for a missing or malformed number.
/// [`ApiError::Internal`] for a configured rail with no linked adapter,
/// which boot refuses (`crate::boot::boot_seeds`) and is therefore an
/// invariant failure rather than a merchant's mistake. Otherwise whatever
/// the rail's [`vpay_provider::ProviderError`] classifies to — a 502 for a
/// rail that could not be reached, never a `200`.
pub(crate) async fn retrieve(
    State(config): State<Arc<ResourceConfig>>,
    State(adapters): State<Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>>,
    _scope: MerchantScope,
    VpayQuery(params): VpayQuery<RetrieveParams>,
) -> Result<Response, ApiError> {
    let code = params
        .payment_method_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            count(account_holder_outcome::UNSUPPORTED);
            ApiError::invalid_param(
                PAYMENT_METHOD_TYPE_PARAM,
                "Name the rail to ask, as `payment_method_type` — the same value \
                 `payment_method_types` on a PaymentIntent takes.",
            )
        })?;

    // A rail this deployment does not offer, or offers with `enabled: false`,
    // is indistinguishable to a merchant from one that cannot answer the
    // question, and both are their request to fix. `enabled_rail` rather than
    // `rail`: a rail an operator has switched off must not still be reachable
    // for identity lookups.
    let rail = config.enabled_rail(code).ok_or_else(|| {
        count(account_holder_outcome::UNSUPPORTED);
        unsupported_rail()
    })?;

    // Boot refuses a configured rail with no linked adapter
    // (`ConfigError::ProviderWithoutAdapter`, exit 78), so a miss here is our
    // invariant failing rather than anything a caller sent.
    let adapter = adapters.get(rail.code()).ok_or_else(|| {
        ApiError::Internal(format!(
            "rail {} is configured but no adapter is linked; boot is supposed to have \
             refused to start",
            rail.code()
        ))
    })?;

    // **The capability value, never the rail's code** (ADR-0002). A third
    // rail added tomorrow is refused or served by declaring the flag, with no
    // arm here.
    if !adapter.capabilities().supports_account_holder_lookup {
        count(account_holder_outcome::UNSUPPORTED);
        return Err(unsupported_rail());
    }

    let msisdn = params
        .msisdn
        .as_deref()
        .and_then(canonical_msisdn)
        .ok_or_else(|| {
            count(account_holder_outcome::ERROR);
            ApiError::invalid_param(
                MSISDN_PARAM,
                "`msisdn` must be a Cameroon mobile number in E.164 — `+2376XXXXXXXX`, \
                 `2376XXXXXXXX` or the national `6XXXXXXXX`.",
            )
        })?;

    let outcome = adapter
        .account_holder_name(&msisdn, &rail.provider_config())
        .await;

    // Matched rather than `?`d, so the counter sees the failure too: "a
    // merchant is asking a rail that is not answering" is precisely the rate
    // an operator wants, and a `?` here would leave it invisible.
    let holder = match outcome {
        Ok(holder) => holder,
        Err(error) => {
            count(account_holder_outcome::ERROR);
            // The masked number and the rail, at `warn` — the log line that
            // correlates this refusal with the rail's own. The error itself
            // is rendered by `ApiError::into_response`, at its own severity,
            // which is where its `Display` and source chain reach the log.
            tracing::warn!(
                rail = %rail.code(),
                msisdn = %masked(&msisdn),
                "account-holder lookup failed at the rail"
            );
            return Err(error.into());
        }
    };

    count(if holder.is_some() {
        account_holder_outcome::FOUND
    } else {
        account_holder_outcome::NOT_FOUND
    });

    // **The number is masked and the name is absent.** `found = true/false`
    // is the whole of what an operator learns about the answer, which is
    // enough to read a rate off a log and not enough to rebuild the lookup.
    tracing::info!(
        rail = %rail.code(),
        msisdn = %masked(&msisdn),
        found = holder.is_some(),
        "account-holder lookup served"
    );

    crate::v1::payment_intents::json_response(
        StatusCode::OK,
        &AccountHolderObject {
            object: AccountHolderTag,
            payment_method_type: rail.code().to_owned(),
            verified: holder.is_some(),
            name: holder.map(|holder| holder.name().to_owned()),
        },
    )
}

/// The refusal a rail that cannot be asked produces, in one place.
///
/// One function so the two paths that reach it — an unknown or disabled
/// rail, and a rail whose capability is false — answer byte for byte the
/// same thing. Telling them apart would let a merchant enumerate which rails
/// a deployment has configured but switched off, and neither is actionable
/// differently: the fix is the same, name a rail that can answer.
///
/// A `400 invalid_request_error` naming the parameter, which is what a
/// Stripe SDK reads to point at a form field —
/// **not** a `409`. `ProviderError::Unsupported` classifies to `Conflict`
/// (409) and would be the honest answer if the rail had been *called*; it is
/// not called, because the capability check is what ADR-0002 asks the core
/// to do first, and at that point the wrong thing is the merchant's
/// parameter.
fn unsupported_rail() -> ApiError {
    ApiError::invalid_param(
        PAYMENT_METHOD_TYPE_PARAM,
        "This payment method cannot look up an account holder on this deployment. Send a \
         `payment_method_type` whose rail exposes one.",
    )
}

/// Counts one lookup outcome.
///
/// A function rather than a `counter!` at four call sites, so the label can
/// only ever be one of [`account_holder_outcome`]'s constants and so there
/// is one place to look when asking what this series can contain.
fn count(outcome: &'static str) {
    metrics::counter!(ACCOUNT_HOLDER_LOOKUPS_TOTAL, "outcome" => outcome).increment(1);
}

/// The canonical `2376XXXXXXXX` — twelve digits, no `+` — or `None`.
///
/// # Why this is not shared with the checkout page's validator
///
/// `frontends/apps/checkout/src/lib/msisdn.ts` implements the identical
/// rule, and the two are deliberately separate rather than one crossing the
/// language boundary: the browser's is a *form affordance* that also
/// formats for display, and this one is a **trust boundary** — the page can
/// be bypassed entirely by a merchant calling `/v1` directly, which is the
/// ordinary way this route is used. Sharing would mean the server trusted a
/// client-side rule, which is the shape of the bug, not the fix. What is
/// shared is the specification: `237` + `6` + eight digits, the three input
/// spellings a payer types, and the same separator set.
///
/// # Why it is validated at all, when the rail would refuse a bad number
///
/// Because a rail refusing it is a rail *call*, on our credentials, for an
/// input we could see was not a phone number — and because an unvalidated
/// value is what makes "enumerate the number space" cheap in ways E.164 does
/// not. It is also the only thing standing between a caller and an arbitrary
/// path segment on MTN's API; the adapter escapes it as well
/// (`vpay_provider::http::path_segment`), belt and braces.
///
/// The **twelve-digit, no-`+`** form is what the rail receives:
/// `payer.partyId` is `237600000000` in `vpay-adapter-mtn-momo` and in every
/// conformance mapping.
fn canonical_msisdn(input: &str) -> Option<String> {
    /// Separators a caller may send, matching the checkout page's set: ASCII
    /// space, tab, hyphen, dot, parentheses, and the two spaces a phone
    /// keypad or a French locale inserts (U+00A0, U+202F). Written as
    /// escapes because a literal one is invisible in a diff.
    const SEPARATORS: [char; 8] = [' ', '\t', '-', '.', '(', ')', '\u{00a0}', '\u{202f}'];

    let trimmed = input.trim();
    if trimmed.chars().count() > MAX_MSISDN_INPUT_CHARS {
        return None;
    }

    let mut digits = String::with_capacity(CM_COUNTRY_CODE.len() + CM_NATIONAL_DIGITS);
    for (index, character) in trimmed.chars().enumerate() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else if character == '+' && index == 0 {
            // A leading `+` only, exactly as the page allows: a second one
            // is not punctuation in a phone number, it is a different string.
        } else if SEPARATORS.contains(&character) {
            // Dropped.
        } else {
            // A letter, punctuation, anything else: not a phone number. The
            // hex steering numbers the WireMock mappings key on
            // (`237600000f01`) are refused here for exactly this reason, and
            // that is why the account-holder mappings use digits-only ones.
            return None;
        }
    }

    let national = if digits.len() == CM_NATIONAL_DIGITS {
        digits.as_str()
    } else if digits.len() == CM_COUNTRY_CODE.len() + CM_NATIONAL_DIGITS
        && digits.starts_with(CM_COUNTRY_CODE)
    {
        &digits[CM_COUNTRY_CODE.len()..]
    } else {
        return None;
    };

    if !national.starts_with(CM_MOBILE_PREFIX) {
        return None;
    }
    Some(format!("{CM_COUNTRY_CODE}{national}"))
}

/// A canonical MSISDN in the shape `charges.payer_ref_masked` is documented
/// to hold: `+2376••••000`.
///
/// Country code, the leading `6` every Cameroon mobile number starts with,
/// four bullets, and the last three digits — enough for a merchant reading
/// their own logs to tell two of their payers apart, and not enough to
/// reconstruct a number. The literal shape is the one
/// `backends/crates/vpay-db/tests/repositories.rs` pins for that column.
///
/// **Nothing writes that column yet.** The confirm path stores `None`
/// (`crate::v1::payment_intents`'s `open_attempt`), so this is the first
/// producer of the shape in the workspace and the two are not yet wired
/// together — a gap, named in `docs/status.md` rather than closed here,
/// because writing the column is a change to the charge path and not to this
/// route.
///
/// The bullet count is **fixed at four** and is not the number of digits
/// hidden; a mask whose length revealed the input's length would be a small
/// oracle for free.
fn masked(canonical: &str) -> String {
    /// How many trailing digits stay visible.
    const VISIBLE_TAIL: usize = 3;

    let head: String = canonical.chars().take(CM_COUNTRY_CODE.len() + 1).collect();
    let digits: Vec<char> = canonical.chars().collect();
    let tail: String = digits
        .iter()
        .skip(digits.len().saturating_sub(VISIBLE_TAIL))
        .collect();
    format!("+{head}••••{tail}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::body::to_bytes;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};
    use vpay_core::{Money, ProviderFlow};
    use vpay_provider::{
        AccountHolder, CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderConfig,
        ProviderError, Submitted,
    };

    use super::*;

    /// What the rail under test should answer.
    ///
    /// An enum rather than three fixture types, so every case below reads as
    /// one call with the rail's answer named in it.
    #[derive(Debug, Clone, Copy)]
    enum Answer {
        /// A named holder.
        Holder,
        /// The rail has no record.
        NoRecord,
        /// The rail could not be finished with.
        Unreachable,
        /// The rail has no such API at all — the port's default.
        NoSuchApi,
    }

    /// A rail that answers whatever the case asked it to, and reaches no
    /// network.
    ///
    /// **Not a test double of an adapter**, on `v1::boot`'s `TestRail`'s
    /// terms and `vpay_provider::measured`'s `Answering`'s: what is under
    /// test here is the *handler*, whose contract is "whatever the port
    /// answered, rendered and counted this way", and proving that needs a
    /// port answer the test chose. `vpay-api` links no adapter crate at all
    /// (ADR-0002), so a real rail is not available to it even in principle;
    /// the real-rail half is
    /// `backends/tests/integration/tests/account_holders.rs`, which drives
    /// this same handler over a socket with the shipping MTN adapter talking
    /// HTTP to a WireMock container. This type is `#[cfg(test)]`, so no
    /// shipping binary can reach it (ADR-0006).
    ///
    /// `Unsupported` on every *other* method rather than a `NotImplemented`
    /// token, for `TestRail`'s stated reason: a fixture has no business
    /// adding a row to `docs/status.md`.
    #[derive(Debug)]
    struct AnsweringRail {
        code: &'static str,
        answer: Answer,
    }

    #[async_trait::async_trait]
    impl ProviderAdapter for AnsweringRail {
        fn code(&self) -> &'static str {
            self.code
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                flow: ProviderFlow::Push,
                supports_refunds: false,
                supports_partial_refunds: false,
                delivers_callbacks: true,
                requires_ip_allowlist: false,
                supports_account_holder_lookup: !matches!(self.answer, Answer::NoSuchApi),
            }
        }

        async fn submit(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<Submitted, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn query_status(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<ChargeStatus, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn refund(
            &self,
            _charge: &ChargeRef,
            _amount: Money,
            _config: &ProviderConfig,
        ) -> Result<Submitted, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn account_holder_name(
            &self,
            _msisdn: &str,
            _config: &ProviderConfig,
        ) -> Result<Option<AccountHolder>, ProviderError> {
            match self.answer {
                Answer::Holder => Ok(Some(AccountHolder::new(HOLDER_NAME))),
                Answer::NoRecord => Ok(None),
                Answer::Unreachable => Err(ProviderError::transport(
                    "test_rail: the request to the rail failed".to_owned(),
                )),
                // Reaching this at all would mean the handler skipped the
                // capability check ADR-0002 asks it to make first.
                Answer::NoSuchApi => Err(ProviderError::Unsupported),
            }
        }
    }

    /// The rail every case configures, and the number every case sends.
    const RAIL: &str = "test_rail";
    const MSISDN: &str = "237600000200";
    const HOLDER_NAME: &str = "David Mbarga";

    fn resource_config(enabled: bool) -> Arc<ResourceConfig> {
        let config = Config {
            deployment: Deployment {
                name: "account-holder-tests".to_owned(),
                livemode: false,
                public_base_url: "http://localhost:8080".to_owned(),
            },
            providers: vec![ProviderHost {
                code: RAIL.to_owned(),
                enabled,
                host: HostEntry {
                    url: "http://127.0.0.1:1".to_owned(),
                    label: "unreachable".to_owned(),
                },
                settings: BTreeMap::new(),
                callback_url: None,
                currency: "XAF".to_owned(),
                credentials: BTreeMap::new(),
            }],
            currencies: vec![CurrencyEntry {
                code: "xaf".to_owned(),
                exponent: 0,
            }],
            merchant_clients: Vec::new(),
            webhooks: vpay_config::WebhookPolicy::default(),
            checkout: vpay_config::CheckoutConfig::default(),
            dashboard_client: None,
        };
        Arc::new(
            ResourceConfig::from_config(&config).expect("the fixture rail projects onto the port"),
        )
    }

    fn adapters(answer: Answer) -> Arc<BTreeMap<String, Box<dyn ProviderAdapter>>> {
        Arc::new(BTreeMap::from([(
            RAIL.to_owned(),
            Box::new(AnsweringRail { code: RAIL, answer }) as Box<dyn ProviderAdapter>,
        )]))
    }

    /// Calls the shipping handler directly. The extractors are constructed
    /// rather than run, because what is under test is the handler body — the
    /// extractors have their own tests (`crate::form`,
    /// `MerchantScope::from_request_parts`) and routing one request through
    /// axum would prove those again instead of this.
    async fn lookup(
        answer: Answer,
        enabled: bool,
        msisdn: Option<&str>,
        payment_method_type: Option<&str>,
    ) -> Result<Response, ApiError> {
        retrieve(
            State(resource_config(enabled)),
            State(adapters(answer)),
            MerchantScope::for_payer("acme-cameroon-tenant".to_owned()),
            VpayQuery(RetrieveParams {
                msisdn: msisdn.map(ToOwned::to_owned),
                payment_method_type: payment_method_type.map(ToOwned::to_owned),
            }),
        )
        .await
    }

    async fn body_of(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("the handler's body is small and complete");
        serde_json::from_slice(&bytes).expect("every /v1 response is JSON")
    }

    /// A rail that names a holder renders the four documented keys, and the
    /// name reaches the merchant.
    #[tokio::test]
    async fn a_number_the_rail_knows_answers_with_the_name() {
        let response = lookup(Answer::Holder, true, Some(MSISDN), Some(RAIL))
            .await
            .expect("a rail that answered is a 200");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_of(response).await,
            serde_json::json!({
                "object": "account_holder",
                "payment_method_type": RAIL,
                "name": HOLDER_NAME,
                "verified": true,
            })
        );
    }

    /// The distinction the route exists for, from the merchant's side: a
    /// number with no holder is a `200` whose `name` is null, and a rail
    /// that could not be asked is **not**.
    #[tokio::test]
    async fn a_number_the_rail_does_not_know_is_a_200_with_a_null_name() {
        let response = lookup(Answer::NoRecord, true, Some(MSISDN), Some(RAIL))
            .await
            .expect("'no record' is an answer");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_of(response).await,
            serde_json::json!({
                "object": "account_holder",
                "payment_method_type": RAIL,
                "name": null,
                "verified": false,
            })
        );
    }

    /// **A rail that cannot be asked is never a `200` with nulls.** The
    /// status is derived from the error's `Category` (ADR-0011), never
    /// chosen here, so this asserts the derivation reached the boundary
    /// rather than that someone wrote `502`.
    #[tokio::test]
    async fn a_rail_that_cannot_be_asked_is_a_classified_error_and_never_a_null_name() {
        use vpay_core::{Category, Classify as _};

        let error = lookup(Answer::Unreachable, true, Some(MSISDN), Some(RAIL))
            .await
            .expect_err("an unreachable rail must not render an object");

        assert_eq!(error.category(), Category::Rail);
        assert_eq!(Category::Rail.http_status(), 502);

        let response = axum::response::IntoResponse::into_response(error);
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = body_of(response).await;
        assert_eq!(
            body.pointer("/error/type")
                .and_then(serde_json::Value::as_str),
            Some("api_error")
        );
        assert!(
            body.get("name").is_none() && body.get("verified").is_none(),
            "a failure must not be rendered as an account_holder at all: {body}"
        );
    }

    /// Refused on the **capability value**, and the envelope names the
    /// parameter an SDK should point at.
    #[tokio::test]
    async fn a_rail_that_has_no_such_api_is_a_400_naming_the_parameter() {
        let error = lookup(Answer::NoSuchApi, true, Some(MSISDN), Some(RAIL))
            .await
            .expect_err("a rail with no account-holder API cannot answer");

        let response = axum::response::IntoResponse::into_response(error);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = body_of(response).await;
        assert_eq!(
            body.pointer("/error/type")
                .and_then(serde_json::Value::as_str),
            Some("invalid_request_error")
        );
        assert_eq!(
            body.pointer("/error/param")
                .and_then(serde_json::Value::as_str),
            Some(PAYMENT_METHOD_TYPE_PARAM)
        );
        assert!(
            !body
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains(RAIL),
            "the refusal must not name the rail's code — it is decided on the capability \
             value, and echoing the code invites a caller to branch on it: {body}"
        );
    }

    /// A rail an operator has switched off answers exactly as a rail with no
    /// such API does — byte for byte, so a merchant cannot tell which
    /// deployments have which rails configured-but-disabled.
    #[tokio::test]
    async fn a_disabled_or_unknown_rail_is_the_same_refusal_as_an_incapable_one() {
        let disabled = lookup(Answer::Holder, false, Some(MSISDN), Some(RAIL))
            .await
            .expect_err("a disabled rail cannot be asked");
        let unknown = lookup(Answer::Holder, true, Some(MSISDN), Some("no_such_rail"))
            .await
            .expect_err("an unknown rail cannot be asked");
        let incapable = lookup(Answer::NoSuchApi, true, Some(MSISDN), Some(RAIL))
            .await
            .expect_err("an incapable rail cannot be asked");

        let mut bodies = Vec::new();
        for error in [disabled, unknown, incapable] {
            let response = axum::response::IntoResponse::into_response(error);
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            bodies.push(body_of(response).await);
        }
        assert!(
            bodies.windows(2).all(|pair| pair.first() == pair.last()),
            "the three refusals must be byte for byte the same envelope: {bodies:?}"
        );
    }

    /// Both parameters are required, and each refusal names itself.
    #[tokio::test]
    async fn a_missing_or_malformed_parameter_names_itself() {
        for (msisdn, rail, param) in [
            (Some(MSISDN), None, PAYMENT_METHOD_TYPE_PARAM),
            (Some(MSISDN), Some("   "), PAYMENT_METHOD_TYPE_PARAM),
            (None, Some(RAIL), MSISDN_PARAM),
            (Some(""), Some(RAIL), MSISDN_PARAM),
            (Some("not-a-number"), Some(RAIL), MSISDN_PARAM),
            // A hex WireMock steering number: refused like any other
            // non-number, which is what keeps stub-specific behaviour
            // unreachable through this route.
            (Some("237600000f01"), Some(RAIL), MSISDN_PARAM),
            // Right shape, wrong country and wrong mobile prefix.
            (Some("234600000200"), Some(RAIL), MSISDN_PARAM),
            (Some("237700000200"), Some(RAIL), MSISDN_PARAM),
        ] {
            let error = lookup(Answer::Holder, true, msisdn, rail)
                .await
                .err()
                .unwrap_or_else(|| {
                    panic!("{msisdn:?}/{rail:?} must be refused before the rail is called")
                });
            let response = axum::response::IntoResponse::into_response(error);
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{msisdn:?}/{rail:?}"
            );
            let body = body_of(response).await;
            assert_eq!(
                body.pointer("/error/type")
                    .and_then(serde_json::Value::as_str),
                Some("invalid_request_error"),
                "{msisdn:?}/{rail:?}"
            );
            assert_eq!(
                body.pointer("/error/param")
                    .and_then(serde_json::Value::as_str),
                Some(param),
                "{msisdn:?}/{rail:?}"
            );
        }
    }

    /// **The log line an operator sees.** Asserted against captured
    /// `tracing` output rather than against the arguments a macro was called
    /// with, for `crate::test_log`'s reason: only the sink can say what was
    /// actually written.
    ///
    /// Three things must hold at once — the masked number is there (so the
    /// line is useful), the whole number is not, and the holder's name is
    /// not. A line with none of the three would pass a naive "does not
    /// contain the name" check while being useless, which is why the
    /// positive assertion is here too.
    #[tokio::test(flavor = "current_thread")]
    async fn a_lookup_logs_a_masked_number_and_never_a_name() {
        let sink = crate::test_log::CapturedLog::default();
        let guard = tracing::subscriber::set_default(crate::test_log::captured_log_subscriber(
            sink.clone(),
        ));
        let response = lookup(Answer::Holder, true, Some(MSISDN), Some(RAIL))
            .await
            .expect("a rail that answered is a 200");
        drop(guard);

        // The name did reach the merchant — otherwise the assertions below
        // would be about a route that returned nothing.
        assert_eq!(
            body_of(response)
                .await
                .get("name")
                .and_then(serde_json::Value::as_str),
            Some(HOLDER_NAME)
        );

        let logged = sink.contents();
        assert!(
            logged.contains("+2376••••200"),
            "the masked number must be there, or the line tells an operator nothing:\n{logged}"
        );
        assert!(
            !logged.contains(MSISDN),
            "the payer's number reached a log line unmasked:\n{logged}"
        );
        assert!(
            !logged.contains(HOLDER_NAME),
            "the holder's NAME reached a log line — the one thing \
             docs/flows/account-holder-lookup.md forbids outright:\n{logged}"
        );
    }

    /// One counter, one label, four values — and **no label carrying the
    /// number or the name**, which is the property that makes the metric
    /// safe to ship to wherever a scrape goes.
    ///
    /// Rendered through the shipping Prometheus exporter rather than a
    /// debugging recorder, and through a *local* recorder rather than the
    /// global one (which can be installed only once per process): asserting
    /// on the rendered text is what makes this fail for the same reason a
    /// dashboard would be wrong.
    #[tokio::test(flavor = "current_thread")]
    async fn every_outcome_is_counted_and_no_label_carries_the_number_or_the_name() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        // `enter()` rather than `with_local_recorder`, because the calls are
        // `async`: a closure returning a future would install the recorder
        // only while the future is built. The guard is held across the
        // awaits, on a current-thread runtime.
        {
            let _guard = metrics::set_default_local_recorder(&recorder);
            let _ = lookup(Answer::Holder, true, Some(MSISDN), Some(RAIL)).await;
            let _ = lookup(Answer::NoRecord, true, Some(MSISDN), Some(RAIL)).await;
            let _ = lookup(Answer::NoSuchApi, true, Some(MSISDN), Some(RAIL)).await;
            let _ = lookup(Answer::Unreachable, true, Some(MSISDN), Some(RAIL)).await;
        }

        let scrape = handle.render();
        for outcome in [
            account_holder_outcome::FOUND,
            account_holder_outcome::NOT_FOUND,
            account_holder_outcome::UNSUPPORTED,
            account_holder_outcome::ERROR,
        ] {
            assert!(
                scrape.contains(&format!(
                    "vpay_account_holder_lookups_total{{outcome=\"{outcome}\"}} 1"
                )),
                "{outcome} was not counted exactly once:\n{scrape}"
            );
        }
        assert!(
            !scrape.contains(MSISDN) && !scrape.contains(HOLDER_NAME),
            "no label may carry the number looked up or the name returned:\n{scrape}"
        );
        // And the rail's own code is not a label either: it would make the
        // series cardinality a function of how many rails a deployment
        // offers, and the question this metric answers is about the route.
        assert!(
            !scrape.contains(&format!("provider=\"{RAIL}\"")),
            "unexpected label on the lookup series:\n{scrape}"
        );
        // A rail's answer is never mistaken for a decline: the failure arm
        // increments `error` and not `not_found`.
        assert!(
            !scrape.contains(&format!(
                "vpay_account_holder_lookups_total{{outcome=\"{}\"}} 2",
                account_holder_outcome::NOT_FOUND
            )),
            "an unreachable rail was counted as a missing holder:\n{scrape}"
        );
    }

    /// The three spellings a caller may send, and the one canonical answer —
    /// the same table `frontends/apps/checkout/src/lib/msisdn.ts`'s own tests
    /// hold, which is what keeps the two implementations one rule.
    #[test]
    fn every_spelling_a_caller_sends_canonicalises_to_the_form_the_rail_takes() {
        for input in [
            "237600000200",
            "+237600000200",
            "+237 6 00 00 02 00",
            "237-600-000-200",
            "600000200",
            "+237(6)00.00.02.00",
            "  237600000200  ",
            // The two non-breaking spaces a keypad or a French locale emits.
            "+237\u{00a0}600000200",
            "+237\u{202f}600000200",
        ] {
            assert_eq!(
                canonical_msisdn(input).as_deref(),
                Some("237600000200"),
                "{input:?}"
            );
        }
    }

    /// The refusals. `237600000f01` is in this list on purpose: it is a
    /// WireMock steering number, it carries a hex letter, and a validator
    /// that admitted it would let a merchant drive stub-specific behaviour
    /// through a production-shaped route.
    #[test]
    fn anything_that_is_not_a_cameroon_mobile_number_is_refused() {
        for input in [
            "",
            "   ",
            "237600000f01",
            "07700900123",
            // Right length, wrong country.
            "234600000200",
            // Right country, not a mobile prefix.
            "237700000200",
            // A digit short, and a digit long.
            "23760000020",
            "2376000002000",
            // A second `+`, and one that is not leading.
            "++237600000200",
            "237+600000200",
            "237600000200; DROP TABLE charges",
            "../../v1_0/token",
            "600000200@example.test",
        ] {
            assert_eq!(canonical_msisdn(input), None, "{input:?}");
        }
    }

    /// A caller-controlled query parameter is refused by a length check
    /// rather than by a loop over a megabyte of digits.
    #[test]
    fn an_absurdly_long_input_is_refused_without_being_walked() {
        let long = "2".repeat(MAX_MSISDN_INPUT_CHARS + 1);
        assert_eq!(canonical_msisdn(&long), None);
        assert_eq!(canonical_msisdn(&"6".repeat(100_000)), None);
    }

    /// The mask is the shape `charges.payer_ref_masked` is documented to
    /// hold, and it hides the middle of the number rather than a count of
    /// its digits.
    #[test]
    fn the_mask_is_the_documented_shape_and_reveals_only_the_ends() {
        assert_eq!(masked("237600000000"), "+2376••••000");
        assert_eq!(masked("237671234567"), "+2376••••567");

        let rendered = masked("237671234567");
        assert!(
            !rendered.contains("71234"),
            "the middle of the number must not survive: {rendered}"
        );
        assert_eq!(
            rendered.matches('•').count(),
            4,
            "a fixed four bullets, not one per hidden digit — a mask whose length \
             revealed the input's length would be an oracle: {rendered}"
        );
    }

    /// The wire shape, pinned as bytes rather than as a struct, because it
    /// is the contract both SDKs decode. Four keys, `name` present-and-null
    /// on the not-found answer.
    #[test]
    fn the_object_renders_the_four_documented_keys_in_both_answers() {
        let found = AccountHolderObject {
            object: AccountHolderTag,
            payment_method_type: "mtn_momo".to_owned(),
            name: Some("David Mbarga".to_owned()),
            verified: true,
        };
        assert_eq!(
            serde_json::to_value(&found).expect("a wire DTO always serialises"),
            serde_json::json!({
                "object": "account_holder",
                "payment_method_type": "mtn_momo",
                "name": "David Mbarga",
                "verified": true,
            })
        );

        let absent = AccountHolderObject {
            object: AccountHolderTag,
            payment_method_type: "mtn_momo".to_owned(),
            name: None,
            verified: false,
        };
        let rendered = serde_json::to_value(&absent).expect("a wire DTO always serialises");
        assert_eq!(
            rendered,
            serde_json::json!({
                "object": "account_holder",
                "payment_method_type": "mtn_momo",
                "name": null,
                "verified": false,
            })
        );
        // Present *and* null, not omitted: both SDKs model `name` as a
        // required nullable field, so a dropped key is a decode failure in a
        // merchant's own client.
        assert!(
            rendered
                .as_object()
                .is_some_and(|map| map.contains_key("name")),
            "`name` must be present and null, never absent: {rendered}"
        );
    }
}
