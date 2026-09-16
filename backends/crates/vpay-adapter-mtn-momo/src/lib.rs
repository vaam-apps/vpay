//! MTN MoMo Cameroon adapter: a push rail, in the sense
//! `docs/flows/provider-port.md` gives the word — the payer is prompted on
//! their own handset, we supply the reference (`X-Reference-Id`) so a submit
//! is idempotent on an id that exists in our database *before* the call, and
//! status stays queryable by that same reference indefinitely, which is what
//! the poll ladder is built on.
//!
//! The wire, the failure mapping and the environment values are in
//! `docs/flows/adapter-mtn-momo.md`; the `mapping` module transcribes its table.
//! `docs/reference/rails.md` has the three answers this adapter reads against
//! the grain — a 409 is a success, a 500 is not automatically retryable, a
//! 404 is not a failure.
//!
//! [`Adapter::refund`] is MTN's **Disbursements** `transfer` call as of
//! 2026-09-15 (RFC-0003 § 5) and is no longer
//! [`ProviderError::NotImplemented`] — _this paragraph said it was until the
//! review of that change; it was the adapter's own module header and the
//! most-read of the stale claims._ **No REAL MTN Disbursements credential
//! exists in this project and nothing in this repository has ever called that
//! product**, so the call is WireMock-proven and rail-unproven; the method's
//! own doc comment and `docs/status.md` are the long form. _Narrowed again on
//! 2026-09-16: this said "no deployment holds a Disbursements subscription
//! key", and the e2e/demo stack holds a stub one, pointed at a
//! `wiremock/wiremock` container, so the SDKs' live refund suites can reach
//! the `202` at all._
//!
//! Credentials are never logged and never rendered; see the `token` module.

mod mapping;
mod token;
mod wire;

pub use crate::mapping::{PRODUCED_FAILURE_CODES, UNMAPPED_REASONS, UNPUBLISHED_REASONS};

use async_trait::async_trait;
use reqwest::StatusCode;
use reqwest::header::{HeaderName, HeaderValue};
use tokio::sync::RwLock;
use vpay_core::{FailureCode, Money, ProviderFlow};
use vpay_provider::{
    AccountHolder, CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderAdapter,
    ProviderConfig, ProviderError, RefExtra, RefundDestination, RefundTarget, Refunded, Submitted,
};

use crate::token::{Credentials, Product, SUBSCRIPTION_KEY_HEADER, TARGET_ENVIRONMENT_HEADER};

/// The reference *we* generate. It is MTN's transaction id, which is what
/// makes a same-reference resubmission idempotent rather than a second
/// charge.
const REFERENCE_ID_HEADER: HeaderName = HeaderName::from_static("x-reference-id");

/// The one key this rail names inside `destination[mtn_momo]`, and the only
/// place in the workspace that spells it.
///
/// It is vpay's **merchant-facing** parameter (RFC-0003 § 1), not a field of
/// MTN's API: what Disbursements calls the payee is `payee.partyId`, rendered
/// by whoever builds the `transfer` body, and the two are free to differ
/// precisely because this adapter is the only thing that sees both.
const DESTINATION_MSISDN_KEY: &str = "msisdn";

/// Per-request, and its host must match the `providerCallbackHost` registered
/// with the API user — a mismatch is one of the 500s
/// `mapping::CONFIGURATION_CODES` catches.
const CALLBACK_URL_HEADER: HeaderName = HeaderName::from_static("x-callback-url");

/// The adapter, owning the outbound HTTP client it makes rail calls with and
/// the access token those calls carry.
///
/// Neither `Copy` nor `Default`, and that is the point: `Default` would have
/// to invent a client, and the only correct client in a `FROM scratch` image
/// is the vendored-roots one a binary builds once at boot
/// (`vpay_provider::http`).
#[derive(Debug)]
pub struct Adapter {
    /// Built once per process by the binary and cloned in
    /// (`reqwest::Client` is an `Arc` internally, so a clone shares the pool).
    http: reqwest::Client,
    /// The cached Collections bearer, or `None` before the first charge-path
    /// call.
    ///
    /// An async `RwLock` because it is held across the `.await` that mints a
    /// token, and a blocking lock over an await point parks a runtime thread
    /// on a network round trip. One slot, not a map keyed by fingerprint —
    /// `docs/reference/rails.md` says why that is safe *and* why a map would
    /// be worse.
    collections_token: RwLock<Option<vpay_provider::token::CachedToken>>,
    /// The cached Disbursements bearer, kept apart from the Collections one.
    ///
    /// **Two named slots, and still not a map.** `docs/reference/rails.md`'s
    /// objection to a map is that it would be "an unbounded, never-evicted
    /// cache of bearer tokens keyed by credentials"; two fields are bounded
    /// at two by the type system, one per product this adapter can call, and
    /// a third would be a compiler-visible edit rather than a runtime insert.
    ///
    /// A single shared slot would have been *correct* — the fingerprint
    /// carries [`token::Product`], so a Collections bearer can never be
    /// handed to a `transfer` — but it would have made every refund evict the
    /// charge path's token and every subsequent charge evict the refund's, so
    /// a deployment doing both would mint on essentially every call. The
    /// fingerprint is what makes the split *safe*; this split is what makes
    /// it not cost two extra round trips per refund.
    disbursement_token: RwLock<Option<vpay_provider::token::CachedToken>>,
}

impl Adapter {
    /// Takes the process's one client rather than building its own — see the
    /// struct's doc comment.
    ///
    /// ```
    /// use vpay_core::ProviderFlow;
    /// use vpay_provider::ProviderAdapter;
    ///
    /// let client = vpay_provider::http::client().expect("the vendored-roots client builds");
    /// let adapter = vpay_adapter_mtn_momo::Adapter::new(client);
    ///
    /// assert_eq!(adapter.code(), "mtn_momo", "the code is the payment_method_types value");
    /// // A push rail: the payer is prompted on their handset, so there is
    /// // nowhere to redirect anyone and `submit` returns no URL.
    /// assert_eq!(adapter.capabilities().flow, ProviderFlow::Push);
    /// assert!(adapter.capabilities().is_coherent());
    /// ```
    #[must_use]
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            collections_token: RwLock::new(None),
            disbursement_token: RwLock::new(None),
        }
    }

    /// The cache slot this product's bearer lives in.
    ///
    /// A function rather than an index so that the two slots cannot be
    /// confused at a call site, and so that the day a third product appears
    /// the missing arm is a compiler error.
    const fn slot(&self, product: Product) -> &RwLock<Option<vpay_provider::token::CachedToken>> {
        match product {
            Product::Collections => &self.collections_token,
            Product::Disbursements => &self.disbursement_token,
        }
    }

    /// A usable bearer token for `credentials`' product, minting one only if
    /// that product's cache cannot serve the call.
    async fn bearer(
        &self,
        config: &ProviderConfig,
        credentials: &Credentials<'_>,
    ) -> Result<String, ProviderError> {
        let fingerprint = credentials.fingerprint();
        if let Some(cached) = self.slot(credentials.product()).read().await.as_ref()
            && let Some(value) = cached.usable(std::time::Instant::now(), &fingerprint)
        {
            return Ok(value.to_owned());
        }
        self.mint(config, credentials).await
    }

    /// Mints a token unconditionally and replaces whatever was cached.
    ///
    /// Also the 401 path: the rail saying "no" to a token we believed was
    /// good is the only evidence available that it expired early, and the
    /// cached copy has to go with it or every subsequent call repeats the
    /// same 401.
    ///
    /// The lock is taken to *store*, not around the round trip, so two
    /// concurrent callers can both mint. Holding the write lock across the
    /// mint would serialise every rail call behind it, and the cost of the
    /// race is one redundant token, not a wrong one. The fresh token is used
    /// for this call even if it is already inside the refresh margin: the
    /// alternative is refusing a call over a token the rail just said is
    /// good.
    ///
    /// # Errors
    ///
    /// As [`token::mint`].
    async fn mint(
        &self,
        config: &ProviderConfig,
        credentials: &Credentials<'_>,
    ) -> Result<String, ProviderError> {
        let minted = token::mint(&self.http, config, credentials).await?;
        let value = minted.value().to_owned();
        *self.slot(credentials.product()).write().await = Some(minted);
        Ok(value)
    }

    /// Sends an authenticated request, refreshing the token **once** if the
    /// rail answers 401.
    ///
    /// `build` is a closure rather than a prepared request because the retry
    /// has to be built again with the new token, and a `reqwest::Request`
    /// cannot be replayed. It is the only retry in this adapter: nothing else
    /// is resent, least of all a 500 (see the module docs). Resending after a
    /// 401 is safe on both calls that use it — `submit` carries our own
    /// `X-Reference-Id`, so a duplicate is a 409 the caller reads as success,
    /// and `query_status` is a read.
    async fn send_authorized<F>(
        &self,
        config: &ProviderConfig,
        credentials: &Credentials<'_>,
        build: F,
    ) -> Result<reqwest::Response, ProviderError>
    where
        F: Fn(&str) -> Result<reqwest::RequestBuilder, ProviderError>,
    {
        let token = self.bearer(config, credentials).await?;
        let response = build(&token)?.send().await.map_err(transport)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }

        tracing::debug!(rail = "mtn_momo", "token refused; re-minting once");
        let token = self.mint(config, credentials).await?;
        let response = build(&token)?.send().await.map_err(transport)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            // A freshly minted token was refused: the credentials are wrong,
            // not stale. Every charge on this rail is failing and no payer
            // can fix it, so this pages (`docs/flows/failures.md`).
            return Err(rail_credentials_refused(StatusCode::UNAUTHORIZED));
        }
        Ok(response)
    }

    /// The headers every authenticated MTN call carries, resolved once so the
    /// retry closure cannot fail halfway through building a request.
    fn common_headers(
        credentials: &Credentials<'_>,
    ) -> Result<(HeaderValue, HeaderValue), ProviderError> {
        let environment =
            HeaderValue::from_str(credentials.target_environment()).map_err(|_| {
                ProviderError::Config(
                    "mtn_momo: settings.target_environment is not a valid HTTP header value"
                        .to_owned(),
                )
            })?;
        Ok((credentials.subscription_header()?, environment))
    }
}

/// The rail's base URL without a trailing slash, so a path can be appended
/// without producing `//collection`, which some gateways 404.
pub(crate) fn base_url(config: &ProviderConfig) -> &str {
    config.base_url.trim_end_matches('/')
}

/// Every `reqwest` failure — DNS, connect, TLS, and the per-request deadline
/// from [`ProviderConfig::request_timeout`] — is a transport failure.
///
/// One function so no call site can decide otherwise: a rail we could not
/// finish talking to must never be reported as a payer being declined, and
/// `Category::Rail` is what tells the poll ladder to resolve it rather than
/// the merchant to start a new intent.
///
/// The `reqwest::Error` is attached as the `#[source]`, not folded into the
/// message. `reqwest`'s own `Display` for a timeout is "error sending
/// request for url (…)" and the word *timeout* is one link further down the
/// chain — flattening kept the first line and threw the diagnosis away.
pub(crate) fn transport(error: reqwest::Error) -> ProviderError {
    ProviderError::transport_from("mtn_momo: the request to the rail failed", error)
}

/// The rail's answer, bounded, as text.
///
/// [`vpay_provider::http::read_rail_body`] does the bounding and the error
/// mapping — every body this adapter reads goes through it rather than
/// `Response::text()`, which reads to end of stream and so lets the peer
/// choose how much memory a worker task allocates.
///
/// The bytes are decoded lossily rather than by the response's charset:
/// every branch downstream either parses JSON (which is UTF-8 by definition)
/// or truncates the text into a diagnostic, and a body that is not valid
/// UTF-8 must produce a readable error rather than a second, different
/// failure.
///
/// # Errors
///
/// As [`vpay_provider::http::read_rail_body`]:
/// [`ProviderError::Malformed`] naming the cap when the body exceeds it, and
/// [`ProviderError::Transport`] if the stream fails part-way. Never a
/// decline — an oversize or truncated answer says nothing about whether the
/// payment happened.
pub(crate) async fn read_body(
    response: reqwest::Response,
) -> Result<(StatusCode, String), ProviderError> {
    let (status, body) =
        vpay_provider::http::read_rail_body(response, "mtn_momo: reading the rail's response")
            .await?;
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// The rail refusing *our* partner credentials, in the one shape
/// `docs/flows/failures.md` says pages.
fn rail_credentials_refused(status: StatusCode) -> ProviderError {
    ProviderError::Rejected {
        code: FailureCode::ProviderAccountBlocked,
        message: format!("mtn_momo: the rail refused our credentials (HTTP {status})"),
    }
}

/// What a `requesttopay` response means, as a pure function of the status and
/// the body.
///
/// Pure because this is the table that has to be right: it is proven row by
/// row in this module's tests without a network, and the conformance suite
/// then proves the same rows arrive over a real socket from a real WireMock.
fn submit_outcome(status: StatusCode, body: &str) -> Result<Submitted, ProviderError> {
    // MTN returns no key material and no URL: a push rail has nowhere to
    // redirect a payer, whose handset is already ringing.
    let accepted = || {
        Ok(Submitted {
            ref_extra: RefExtra::new(),
            redirect_url: None,
        })
    };

    match status.as_u16() {
        202 => accepted(),
        // The rail already has this reference. See the module docs: this is
        // the answer that makes a same-reference retry safe.
        409 => accepted(),
        400 => {
            let error = wire::ApiError::parse(body);
            let raw = error.code.clone().unwrap_or_else(|| truncated(body));
            Err(ProviderError::Rejected {
                code: mapping::failure_code(&raw),
                message: format!(
                    "mtn_momo: {raw}{}",
                    error
                        .message
                        .map(|m| format!(" — {}", truncated(&m)))
                        .unwrap_or_default()
                ),
            })
        }
        // Not reachable through `send_authorized`, which converts a 401 that
        // survived a token refresh itself; enumerated anyway, because a total
        // function is what stops a status falling into a default that
        // flatters the rail.
        401 | 403 => Err(rail_credentials_refused(status)),
        500 => Err(mapping::internal_error(
            wire::ApiError::parse(body).code.as_deref(),
            &truncated(body),
        )),
        _ if status.is_server_error() => Err(ProviderError::transport(format!(
            "mtn_momo: requesttopay answered HTTP {status}"
        ))),
        // A 404 here is the endpoint, not the charge: the URL is wrong.
        404 => Err(ProviderError::Config(format!(
            "mtn_momo: requesttopay answered HTTP {status}; check base_url"
        ))),
        // Enumerated rather than left to the catch-all so the message says
        // what happened. `vpay_provider::http` builds every client with
        // `redirect::Policy::none()`, so a 3xx arrives here intact instead
        // of being followed with our subscription key, target environment,
        // reference and — on a 307/308 — the request body. It is
        // `Malformed`, never a decline: the charge's fate is unknown.
        _ if status.is_redirection() => Err(ProviderError::malformed(format!(
            "mtn_momo: requesttopay answered a redirect (HTTP {status}), which is not followed; \
             check base_url"
        ))),
        _ => Err(ProviderError::malformed(format!(
            "mtn_momo: requesttopay answered an unexpected HTTP {status}"
        ))),
    }
}

/// `POST {base}/disbursement/v1_0/transfer` — the refund, and the only call
/// this adapter makes that sends money out.
const TRANSFER_PATH: &str = "/v1_0/transfer";

/// What a `transfer` response means, as a pure function of the status and the
/// body.
///
/// Pure for [`submit_outcome`]'s reason, and more so: this is the money-out
/// table, it is proven row by row without a network in this module's tests,
/// and the conformance suite then proves the same rows arrive over a socket
/// from a real WireMock container.
///
/// # A 202 is *accepted*, not *settled* — and the port cannot say so
///
/// MTN's `transfer` is asynchronous exactly as `requesttopay` is: the 202
/// means the rail took the instruction, and the outcome is read back from
/// `GET /disbursement/v1_0/transfer/{referenceId}`. [`Refunded`] carries no
/// status, and [`ProviderAdapter`] has no `query_refund_status`, so the most
/// this function can honestly report is what the 202 says: **the rail
/// accepted the transfer.**
///
/// That gap is real and is recorded rather than smoothed over — see
/// [`Adapter::refund`] § "What a success does and does not mean" and
/// `docs/flows/adapter-mtn-momo.md`. A write path that marked a refund
/// `succeeded` on the strength of an `Ok` from here would be asserting
/// something no MTN response has said.
///
/// # The rows, and where they come from
///
/// The statuses mirror `requesttopay`'s because MTN's gateway is the same
/// gateway and its documented Disbursement responses are the same set. The
/// **failure vocabulary** is Collections' `ErrorReason` enum
/// ([`mapping::failure_code`]), reused here as a deliberate assumption: MTN
/// publishes no schema document for the Disbursement API at all
/// (`docs/flows/adapter-mtn-momo.md`, re-checked 2026-09-11), so there is
/// nothing to compare a Disbursements-specific table against. A reason this
/// table does not know falls to `provider_error` carrying the rail's own
/// word, which is the same safe direction the charge path takes.
fn refund_outcome(status: StatusCode, body: &str) -> Result<Refunded, ProviderError> {
    // MTN answers a transfer with 202 and an empty body: no key material, no
    // fee, nothing to carry. `RefExtra::new()` for `submit_outcome`'s reason
    // — the reference this side generated is the whole address of the
    // transfer, and inventing a second identifier to store would be key
    // material the rail never issued.
    //
    // `fee: None` is load-bearing and is NOT a placeholder for zero. MTN's
    // documented transfer response has no fee field; `None` says "the rail
    // did not report one" and `Some(zero)` would say "the rail said it was
    // free" (issue #46). An adapter that collapsed them would put an invented
    // number in a settlement statement.
    let accepted = || {
        Ok(Refunded {
            ref_extra: RefExtra::new(),
            fee: None,
        })
    };

    match status.as_u16() {
        202 => accepted(),
        // The rail already has this reference. Safe as a success **only**
        // under the invariant in `Adapter::refund` § "Which reference the
        // transfer carries": the reference is this refund's own, so "MTN has
        // already seen it" means this transfer was already instructed, which
        // is exactly what a retry after a crash needs to hear. Reporting it
        // as an error would make a caller re-refund and pay a payee twice.
        409 => accepted(),
        400 => {
            let error = wire::ApiError::parse(body);
            let raw = error.code.clone().unwrap_or_else(|| truncated(body));
            Err(ProviderError::Rejected {
                code: mapping::failure_code(&raw),
                message: format!(
                    "mtn_momo: transfer {raw}{}",
                    error
                        .message
                        .map(|m| format!(" — {}", truncated(&m)))
                        .unwrap_or_default()
                ),
            })
        }
        // Not reachable through `send_authorized`, which converts a 401 that
        // survived a token refresh itself; enumerated anyway, for
        // `submit_outcome`'s reason. On this path it means the
        // **Disbursements** credentials, which are a separate subscription
        // key and a separately scoped token from the charge path's.
        401 | 403 => Err(rail_credentials_refused(status)),
        500 => Err(mapping::internal_error(
            wire::ApiError::parse(body).code.as_deref(),
            &truncated(body),
        )),
        _ if status.is_server_error() => Err(ProviderError::transport(format!(
            "mtn_momo: transfer answered HTTP {status}"
        ))),
        // The endpoint, not the refund — and on this rail it is the likeliest
        // symptom of a deployment that has a Disbursements subscription key
        // but is pointed at a base URL with no Disbursements product behind
        // it. `Config`, so the poll ladder stops rather than retrying a URL
        // that will never exist.
        404 => Err(ProviderError::Config(format!(
            "mtn_momo: transfer answered HTTP {status}; check base_url and that this \
             deployment's subscription is for the Disbursements product"
        ))),
        // Redirects are never followed (`vpay_provider::http` builds every
        // client with `redirect::Policy::none()`), so a 3xx arrives intact
        // instead of replaying the Disbursements subscription key, bearer
        // token, reference **and the payee's number** at whatever host
        // `Location` named.
        _ if status.is_redirection() => Err(ProviderError::malformed(format!(
            "mtn_momo: transfer answered a redirect (HTTP {status}), which is not followed; \
             check base_url"
        ))),
        _ => Err(ProviderError::malformed(format!(
            "mtn_momo: transfer answered an unexpected HTTP {status}"
        ))),
    }
}

/// What a status response means, as a pure function of the status and the
/// body. Same reasoning as `submit_outcome`.
fn status_outcome(status: StatusCode, body: &str) -> Result<ChargeStatus, ProviderError> {
    match status.as_u16() {
        200 => {
            let parsed: wire::StatusResponse = serde_json::from_str(body)
                .map_err(|e| ProviderError::malformed(format!("mtn_momo: status response: {e}")))?;
            match parsed.status.to_ascii_uppercase().as_str() {
                "PENDING" => Ok(ChargeStatus::Pending),
                "SUCCESSFUL" => Ok(ChargeStatus::Succeeded {
                    provider_txn_id: parsed
                        .financial_transaction_id
                        .map(wire::Scalar::into_string),
                }),
                "FAILED" => {
                    // `raw` is never empty: an operator reading a decline
                    // needs the rail's own words even when the taxonomy has
                    // flattened them, and a rail that failed a charge without
                    // saying why is itself worth seeing.
                    let raw = parsed
                        .reason
                        .as_ref()
                        .map_or_else(|| "FAILED (no reason given)".to_owned(), wire::Reason::raw);
                    Ok(ChargeStatus::Failed {
                        code: mapping::failure_code(
                            parsed.reason.as_ref().map_or("", wire::Reason::code),
                        ),
                        raw,
                    })
                }
                other => Err(ProviderError::malformed(format!(
                    "mtn_momo: unknown status {other}"
                ))),
            }
        }
        // The whole recovery story rests on this line being `NotFound` and
        // not `Failed`.
        404 => Ok(ChargeStatus::NotFound),
        401 | 403 => Err(rail_credentials_refused(status)),
        _ if status.is_server_error() => Err(ProviderError::transport(format!(
            "mtn_momo: status query answered HTTP {status}"
        ))),
        // See `submit_outcome`: redirects are not followed, so a 3xx is an
        // answer to refuse rather than a hop to take.
        _ if status.is_redirection() => Err(ProviderError::malformed(format!(
            "mtn_momo: status query answered a redirect (HTTP {status}), which is not followed; \
             check base_url"
        ))),
        _ => Err(ProviderError::malformed(format!(
            "mtn_momo: status query answered an unexpected HTTP {status}"
        ))),
    }
}

/// The `accountHolderIdType` path segment this adapter sends.
///
/// **Lower-case, and MTN's own portal declares the enum upper-case.** The
/// APIM operation `GetBasicUserinfo` lists the parameter's values as
/// `MSISDN | Email | Alias | ID`, while every published example of the
/// endpoint — and issue #47's own citation — spells the segment `msisdn`.
/// Both cannot be right about a case-sensitive backend, and **this has never
/// been called against MTN's real sandbox**, so the constant records which
/// one vpay sends rather than pretending the question is settled
/// (`docs/flows/account-holder-lookup.md`, "unverified against the real
/// rail"). A single constant is what makes changing the answer one edit.
const ACCOUNT_HOLDER_ID_TYPE: &str = "msisdn";

/// The `basicuserinfo` URL this adapter sends, for a base and a payer
/// reference.
///
/// # Why this is a function and not two lines inside the method
///
/// The `path_segment` call is the only thing standing between a caller and
/// an arbitrary endpoint on MTN's API **under this deployment's own
/// subscription key and bearer token**: an unescaped `/` moves the request,
/// a `?` or a `#` truncates the path. It was inlined until 2026-09-06, and
/// a mutation that deleted the call left 113 tests green — the test that
/// was supposed to hold it (then named
/// `a_payer_reference_is_escaped_before_it_becomes_a_path_segment`, since
/// rewritten onto this function) exercised
/// `vpay_provider::http::path_segment` itself and never the adapter's *use*
/// of it, and every stubbed MSISDN is digits-only, where escaping is a
/// no-op. A pure function is what lets
/// [`tests::the_lookup_url_escapes_the_payer_reference_it_interpolates`]
/// assert on the string the adapter would actually put on the wire.
///
/// `ACCOUNT_HOLDER_ID_TYPE` is interpolated verbatim, not escaped: it is a
/// constant in this file, not caller data.
fn account_holder_url(base: &str, msisdn: &str) -> String {
    format!(
        "{base}/collection/v1_0/accountholder/{ACCOUNT_HOLDER_ID_TYPE}/{}/basicuserinfo",
        vpay_provider::http::path_segment(msisdn),
    )
}

/// What a `basicuserinfo` response means, as a pure function of the status
/// and the body. Same reasoning as [`submit_outcome`] and [`status_outcome`]:
/// the table is the part that has to be right, and it is proven row by row
/// without a network.
///
/// # The one row this whole method exists for
///
/// `404 -> Ok(None)`. Everything else that is not a `200` is an `Err`,
/// because `Ok(None)` is the port's word for "the rail has no record" and a
/// caller (issue #47's nominated-refund name match) treats it as a fact
/// about the *number* rather than about the lookup. A transport failure
/// reported as `Ok(None)` would tell that caller a real account is
/// unregistered.
///
/// **MTN documents no 404 for this operation** — the portal lists 200, 401
/// and 500 and nothing else, unlike `RequesttoPayTransactionStatus`, which
/// documents "404 Resource not found" explicitly. Mapping it anyway is a
/// deliberate, stated assumption: a 404 is the only status a REST resource
/// has for "no such thing", vpay must not turn one into a 502 the merchant
/// reads as an outage, and the assumption is safe in the direction that
/// matters — if MTN never sends a 404, this arm is simply dead code, and if
/// it sends one for some *other* reason, the caller's fail-closed rule
/// (`Ok(None)` refuses the nomination) still holds.
///
/// **Where it is not free, and it is not "money" but "silence":** this
/// assumption compounds with the other unverified one on this endpoint, the
/// case of [`ACCOUNT_HOLDER_ID_TYPE`]. If that segment's case is wrong,
/// MTN's gateway answers 404 to *every* lookup and this arm renders every
/// one as "the rail has no record" — a total misconfiguration that looks
/// exactly like an empty subscriber base, with no error and nothing in the
/// metrics but a `not_found` rate of 1.0. Either assumption alone fails
/// loudly; the two together fail quietly. `docs/flows/account-holder-lookup.md`
/// carries what to check on the first real sandbox call, and reversing
/// either costs one constant or this one match arm.
fn account_holder_outcome(
    status: StatusCode,
    body: &str,
) -> Result<Option<AccountHolder>, ProviderError> {
    match status.as_u16() {
        200 => {
            let parsed: wire::BasicUserInfo = serde_json::from_str(body).map_err(|e| {
                ProviderError::malformed(format!("mtn_momo: basicuserinfo response: {e}"))
            })?;
            // A 200 that names nobody is an answer this adapter cannot act
            // on, and emphatically **not** `Ok(None)`: see the function doc.
            let name = parsed.name().ok_or_else(|| {
                ProviderError::malformed(
                    "mtn_momo: basicuserinfo answered 200 with neither a given_name nor a \
                     family_name"
                        .to_owned(),
                )
            })?;
            Ok(Some(AccountHolder::new(name)))
        }
        // The row above. See the function doc for why this is mapped at all
        // and why it is safe that MTN does not document it.
        404 => Ok(None),
        401 | 403 => Err(rail_credentials_refused(status)),
        // Our request, not the rail's health: the MSISDN we interpolated is
        // not one MTN can parse. `Malformed` rather than `Config` because
        // nothing in the deployment's configuration produced it — the number
        // came from the caller, past `/v1`'s own E.164 check — and
        // emphatically not `Ok(None)`, which would report a rejected request
        // as an unregistered person.
        400 => Err(ProviderError::malformed(format!(
            "mtn_momo: basicuserinfo answered HTTP {status}: {}",
            truncated(body)
        ))),
        // The same 500 table `submit` uses: three of MTN's 500s are really
        // our misconfiguration and must not be retried forever.
        500 => Err(mapping::internal_error(
            wire::ApiError::parse(body).code.as_deref(),
            &truncated(body),
        )),
        _ if status.is_server_error() => Err(ProviderError::transport(format!(
            "mtn_momo: basicuserinfo answered HTTP {status}"
        ))),
        // Redirects are never followed (`vpay_provider::http` builds every
        // client with `redirect::Policy::none()`), so a 3xx would otherwise
        // send our subscription key and bearer token to wherever `Location`
        // pointed.
        _ if status.is_redirection() => Err(ProviderError::malformed(format!(
            "mtn_momo: basicuserinfo answered a redirect (HTTP {status}), which is not \
             followed; check base_url"
        ))),
        _ => Err(ProviderError::malformed(format!(
            "mtn_momo: basicuserinfo answered an unexpected HTTP {status}"
        ))),
    }
}

/// Bounds what a rail's body can put in a log line or an error message.
///
/// A rail is free to answer with a megabyte of HTML from a load balancer, and
/// that must not become a log line or an error string of the same size.
fn truncated(body: &str) -> String {
    const LIMIT: usize = 200;
    let trimmed = body.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_owned();
    }
    trimmed.chars().take(LIMIT).chain("…".chars()).collect()
}

#[async_trait]
impl ProviderAdapter for Adapter {
    fn code(&self) -> &'static str {
        "mtn_momo"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            flow: ProviderFlow::Push,
            supports_refunds: true,
            supports_partial_refunds: true,
            delivers_callbacks: true,
            requires_ip_allowlist: true,
            // MTN Collections exposes `GET
            // /collection/v1_0/accountholder/{idType}/{msisdn}/basicuserinfo`
            // under the subscription key and token scope this adapter
            // already holds, and `account_holder_name` below calls it — so
            // `true` here is a claim about the rail *and* about this code
            // (issue #47). The segment is spelled by
            // `ACCOUNT_HOLDER_ID_TYPE`, deliberately NOT written out here:
            // MTN's portal declares the enum upper-case and every published
            // example spells it lower-case, vpay sends lower-case, and this
            // comment said `MSISDN` until 2026-09-06 — a second, silently
            // contradicting spelling of the one thing that constant exists
            // to keep in a single place.
            supports_account_holder_lookup: true,
            // MTN returns money through **Disbursements** `transfer`, which
            // is addressed to a payee's MSISDN — there is no "send it back
            // the way it came" on this rail, because the collection and the
            // disbursement are different products with different
            // subscription keys. So the core must demand a destination
            // (RFC-0003 § 1), and it does so on this value rather than on
            // the string `"mtn_momo"` (ADR-0002).
            //
            // A claim about the rail, not about this code: `refund` below is
            // still an unbuilt `NotImplemented` token, and this declaration
            // is what the merchant-facing validation will be derived from
            // either way.
            refund_destination: RefundDestination::Required,
        }
    }

    /// `POST /collection/v1_0/requesttopay`.
    ///
    /// # Errors
    ///
    /// See `submit_outcome` for the whole table. In summary: a decline is
    /// [`ProviderError::Rejected`], our own misconfiguration is
    /// [`ProviderError::Config`], and anything that leaves the charge's fate
    /// unknown is [`ProviderError::Transport`] — never a decline, because a
    /// charge whose fate is unknown may still be alive on the rail.
    async fn submit(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        let credentials = Credentials::from_config(config, Product::Collections)?;
        let body = wire::RequestToPay::new(charge)?;
        let url = format!("{}/collection/v1_0/requesttopay", base_url(config));

        let (subscription, environment) = Adapter::common_headers(&credentials)?;
        let reference = HeaderValue::from_str(&charge.reference_id.to_string()).map_err(|_| {
            ProviderError::malformed("mtn_momo: reference_id is not a header value".to_owned())
        })?;
        let callback = HeaderValue::from_str(&config.callback_url).map_err(|_| {
            ProviderError::Config(
                "mtn_momo: callback_url is not a valid HTTP header value".to_owned(),
            )
        })?;

        let response = self
            .send_authorized(config, &credentials, |token| {
                Ok(self
                    .http
                    .post(&url)
                    .bearer_auth(token)
                    .header(SUBSCRIPTION_KEY_HEADER, subscription.clone())
                    .header(TARGET_ENVIRONMENT_HEADER, environment.clone())
                    .header(REFERENCE_ID_HEADER, reference.clone())
                    .header(CALLBACK_URL_HEADER, callback.clone())
                    .timeout(config.request_timeout)
                    .json(&body))
            })
            .await?;

        let (status, text) = read_body(response).await?;
        tracing::debug!(
            rail = "mtn_momo",
            reference_id = %charge.reference_id,
            status = status.as_u16(),
            "requesttopay answered"
        );
        submit_outcome(status, &text)
    }

    /// `GET /collection/v1_0/requesttopay/{reference_id}` — the authoritative
    /// read, and the only thing that moves money.
    ///
    /// # Errors
    ///
    /// See `status_outcome`. A rail that is down is
    /// [`ProviderError::Transport`]; a rail with no record is *not* an error
    /// at all but [`ChargeStatus::NotFound`].
    async fn query_status(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError> {
        let credentials = Credentials::from_config(config, Product::Collections)?;
        let (subscription, environment) = Adapter::common_headers(&credentials)?;
        let url = format!(
            "{}/collection/v1_0/requesttopay/{}",
            base_url(config),
            charge.reference_id
        );

        let response = self
            .send_authorized(config, &credentials, |token| {
                Ok(self
                    .http
                    .get(&url)
                    .bearer_auth(token)
                    .header(SUBSCRIPTION_KEY_HEADER, subscription.clone())
                    .header(TARGET_ENVIRONMENT_HEADER, environment.clone())
                    .timeout(config.request_timeout))
            })
            .await?;

        let (status, text) = read_body(response).await?;
        tracing::debug!(
            rail = "mtn_momo",
            reference_id = %charge.reference_id,
            status = status.as_u16(),
            "status query answered"
        );
        status_outcome(status, &text)
    }

    /// Identifiers only, out of the body MTN POSTs to `X-Callback-Url`.
    ///
    /// # This request is not authenticated in any way
    ///
    /// MTN signs nothing, sends no shared secret and no HMAC; the only thing
    /// standing between this body and the open internet is that the callback
    /// host is one MTN was told about. Anyone who can reach the URL can post
    /// anything to it. That is precisely why the port returns identifiers and
    /// not a status ("callbacks are hints" — `docs/flows/reconciler.md`): the
    /// most an attacker gains is causing us to ask MTN, over an authenticated
    /// channel, about a charge of ours. The `status` in the body is not read
    /// here and must never be.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`] if the body is not JSON, or names no
    /// reference of ours — see `wire::CallbackBody::reference` for which
    /// field is used and why.
    fn parse_callback(&self, body: &[u8]) -> Result<CallbackRef, ProviderError> {
        let parsed: wire::CallbackBody = serde_json::from_slice(body).map_err(|e| {
            ProviderError::malformed(format!("mtn_momo: callback body is not JSON: {e}"))
        })?;
        Ok(CallbackRef {
            reference_id: parsed.reference()?,
            // MTN hands us no key material: `reference_id` is everything
            // needed to query the charge, so there is nothing to repair a
            // lost `ref_extra` write with — and nothing that could be
            // smuggled in from an unauthenticated request.
            ref_extra: RefExtra::new(),
        })
    }

    /// `destination[mtn_momo][msisdn]` — the payee a Disbursements `transfer`
    /// would be addressed to.
    ///
    /// The core hands over the sub-map under this rail's code and nothing
    /// else; [`ProviderAdapter::parse_destination`] is the contract and
    /// RFC-0003 open question 4 (decided 2026-09-15) is the reason this
    /// function is here rather than in `vpay_api`.
    ///
    /// # What is refused, and what is deliberately not
    ///
    /// A missing key, a key whose value is not a JSON **string**, and a
    /// string that is empty or all whitespace are each
    /// [`ProviderError::Malformed`]. A number is refused rather than coerced:
    /// a leading `+` and a leading `0` do not survive one, so a JSON number
    /// here is a value that has already lost information, and the form
    /// encoding a merchant actually posts never produces one.
    ///
    /// A string that is **not a usable international number** is refused too,
    /// by [`RefundTarget::mobile_money`] rather than by anything here: this
    /// adapter owns the *key*, the port owns the *number*. That split is the
    /// maintainer's decision of 2026-09-15 and the reason it is not a second
    /// spelling of a rule — an adapter cannot construct an invalid
    /// [`RefundTarget`] at all, so there is nothing here to keep in step.
    ///
    /// It is a real change of behaviour from this method's first version,
    /// which applied the *confirm* path's rule (`payer_instrument`: any
    /// non-whitespace string, passed to the rail as written) on the grounds
    /// that a payee should be held to what a payer is held to. The two are
    /// not symmetric: a mistyped payer number fails the charge, a mistyped
    /// payee number sends the money to whoever owns that number. In
    /// particular a bare `600000200` — which `GET /v1/account_holders`
    /// accepts, because it knows it is in Cameroon — is refused here, and
    /// [`RefundTarget::mobile_money`] § "Why the `+` is required" is where
    /// that asymmetry is argued.
    ///
    /// The value is trimmed before it is offered to the constructor, and the
    /// canonical form the constructor returns — digits only, no `+` — is what
    /// reaches a Disbursements body, matching the `payer.partyId` this
    /// adapter already sends on the charge path.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`], and nothing else — this reaches no
    /// network. **The message never contains the value**: it names the
    /// parameter, which is what an integrator needs, and a payee's phone
    /// number in an error string would undo [`RefundTarget`]'s redacting
    /// [`Debug`] one format argument at a time.
    fn parse_destination(
        &self,
        raw: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<RefundTarget, ProviderError> {
        let msisdn = raw
            .get(DESTINATION_MSISDN_KEY)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|msisdn| !msisdn.is_empty())
            .ok_or_else(|| {
                ProviderError::malformed(format!(
                    "mtn_momo: a refund on this rail needs the payee's number, sent as a \
                     non-empty string in `destination[mtn_momo][{DESTINATION_MSISDN_KEY}]`"
                ))
            })?;
        // `InvalidMsisdn` is rendered, the input is not: every variant of it
        // is a unit variant, so this cannot echo the number even by accident.
        RefundTarget::mobile_money(msisdn).map_err(|invalid| {
            ProviderError::malformed(format!(
                "mtn_momo: `destination[mtn_momo][{DESTINATION_MSISDN_KEY}]` is not a payee this \
                 rail can be given — {invalid}"
            ))
        })
    }

    /// `POST /disbursement/v1_0/transfer` — MTN's refund, and the only call
    /// in this adapter that sends money **out**.
    ///
    /// # WireMock-proven, rail-UNPROVEN
    ///
    /// **No deployment of this system holds a Disbursements subscription key,
    /// and nothing in this repository has ever called MTN's Disbursements
    /// product — not once, not against the sandbox.** Every assertion about
    /// this method is against a `wiremock/wiremock` container answering a
    /// stub this repository wrote from MTN's own documentation. A body, a
    /// header set or a status table faithful to that documentation but not to
    /// the rail would pass all of it. `docs/status.md` and
    /// `docs/flows/adapter-mtn-momo.md` say the same thing, and none of it is
    /// the claim "MTN refunds work".
    ///
    /// The `NotImplemented("mtn_momo::refund")` token this replaced was the
    /// honest answer while there was no call at all; a written call that has
    /// never been made needs a different kind of honesty, which is this
    /// paragraph and the two status pages rather than a token.
    ///
    /// # Which reference the transfer carries, and the invariant the core owes
    ///
    /// `X-Reference-Id` and the body's `externalId` are both
    /// `charge.reference_id`, and **that is only correct if the core hands
    /// this method the refund's own `provider_reference_id`** — the value
    /// RFC-0003 § 3 step 2 mints before any rail call and migration `0017`
    /// stores on `refunds.provider_reference_id`.
    ///
    /// The port cannot express the difference: `refund` takes a
    /// [`ChargeRef`], which carries exactly one reference, and
    /// [`Refunded::ref_extra`] is documented as key material for "a reference
    /// this side generated before the call". Two references are needed and
    /// one is passed, so one of them has to be the refund's, and it is this
    /// one. What follows if it is instead the *charge's* reference is worth
    /// stating because it is silent:
    ///
    /// * a second partial refund on the same charge would reuse a reference
    ///   MTN has already seen, be answered `409 RESOURCE_ALREADY_EXIST`, and
    ///   be reported by [`refund_outcome`] as **accepted** — a refund the
    ///   merchant is told happened and for which no money moved;
    /// * the Collections charge and the Disbursements transfer would share an
    ///   id, which is at best confusing in MTN's own records.
    ///
    /// This is unsettled at the port and is recorded as such in RFC-0003
    /// rather than decided here on behalf of the Wave-3 handler that does not
    /// exist yet. `the_transfer_is_addressed_by_the_reference_the_core_supplied`
    /// pins what this method *does*, so a handler author can read one
    /// assertion instead of this paragraph.
    ///
    /// # What a success does and does not mean
    ///
    /// `Ok(Refunded)` means **the rail accepted the transfer**, not that the
    /// payee has the money. MTN's `transfer` is asynchronous exactly as
    /// `requesttopay` is, and its outcome is read back from
    /// `GET /disbursement/v1_0/transfer/{referenceId}` — a call this adapter
    /// does not make, because [`ProviderAdapter`] has no refund status read
    /// and [`Refunded`] has no status field. See [`refund_outcome`].
    ///
    /// # The destination, and what happens when the core's invariant breaks
    ///
    /// `destination` is `Some` exactly when this rail declares
    /// [`RefundDestination::Required`], which it does — the core has checked
    /// that before calling (ADR-0002). **RFC-0003 open question 6 left the
    /// broken case to "the first adapter to make a real transfer call", and
    /// this is it: a `None` here is [`ProviderError::Config`].** Not
    /// `Rejected`, which would blame a rail that was never asked; not
    /// `Malformed`, which is about an answer and there is no answer; not
    /// `Unsupported` or `NotImplemented`, which are both lies — about a rail
    /// that refunds, and about code that exists. `Config`'s own description
    /// on the port's error-surface table — "a credential, setting or URL this
    /// deployment did not supply, or supplied unusably … no retry against the
    /// rail can fix it" — is the closest true sentence available, and its
    /// classification is the behaviour that matters: it stops the poll
    /// ladder, it pages, and it never reaches a payer as a decline.
    ///
    /// The payee's number is read through [`RefundTarget::msisdn`] and is
    /// neither re-normalised nor re-validated: the constructor is fallible
    /// and canonicalising (maintainer's decision, 2026-09-15), so what comes
    /// out is already the digits-only `partyId` shape this rail takes.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] for a Disbursements credential this
    /// deployment did not supply (**which is every deployment today**), for
    /// an absent destination, and for a `base_url` with no Disbursements
    /// product behind it; [`ProviderError::Rejected`] when the rail refuses
    /// the transfer or refuses our partner credentials;
    /// [`ProviderError::Transport`] when the rail could not be reached or
    /// finished with; [`ProviderError::Malformed`] for an answer this adapter
    /// cannot act on. Never [`ProviderError::Unsupported`] — MTN refunds —
    /// and no longer [`ProviderError::NotImplemented`]. See
    /// [`refund_outcome`] for the whole table.
    async fn refund(
        &self,
        charge: &ChargeRef,
        amount: Money,
        destination: Option<&RefundTarget>,
        config: &ProviderConfig,
    ) -> Result<Refunded, ProviderError> {
        // RFC-0003 open question 6, decided above. The message names the
        // parameter and the capability that should have guaranteed it; there
        // is no number in scope to leak, and there must never be one added.
        let destination = destination.ok_or_else(|| {
            ProviderError::Config(
                "mtn_momo: a refund on this rail is a Disbursements transfer and must be \
                 addressed to a payee, but the core supplied none — this rail declares \
                 RefundDestination::Required"
                    .to_owned(),
            )
        })?;

        let credentials = Credentials::from_config(config, Product::Disbursements)?;
        let body = wire::Transfer::new(charge.reference_id, amount, destination);
        let url = format!(
            "{}/{}{TRANSFER_PATH}",
            base_url(config),
            Product::Disbursements.path_segment()
        );

        let (subscription, environment) = Adapter::common_headers(&credentials)?;
        let reference = HeaderValue::from_str(&charge.reference_id.to_string()).map_err(|_| {
            ProviderError::malformed("mtn_momo: reference_id is not a header value".to_owned())
        })?;

        // No `X-Callback-Url`. MTN documents one on `transfer` as it does on
        // `requesttopay`, and sending it would register a Disbursements
        // notification against `POST /provider/mtn_momo/callback` — a route
        // whose parser (`parse_callback`) reads a *charge* reference and
        // whose only effect is to pull a charge's poll job forward. A
        // callback naming a refund reference would find no charge and be
        // refused as `Malformed` on every delivery. The header goes in when
        // there is a refund poll job for it to pull forward, which is the
        // same gap as "a 202 is not a settlement" above.
        let response = self
            .send_authorized(config, &credentials, |token| {
                Ok(self
                    .http
                    .post(&url)
                    .bearer_auth(token)
                    .header(SUBSCRIPTION_KEY_HEADER, subscription.clone())
                    .header(TARGET_ENVIRONMENT_HEADER, environment.clone())
                    .header(REFERENCE_ID_HEADER, reference.clone())
                    .timeout(config.request_timeout)
                    .json(&body))
            })
            .await?;

        let (status, text) = read_body(response).await?;
        // `reference_id` only. The payee's number is deliberately absent:
        // `RefundTarget`'s redacting `Debug` exists precisely so a
        // `tracing::debug!(?destination)` here would be harmless, and a
        // `%destination.msisdn()` would undo it in one line.
        tracing::debug!(
            rail = "mtn_momo",
            reference_id = %charge.reference_id,
            status = status.as_u16(),
            "transfer answered"
        );
        refund_outcome(status, &text)
    }

    /// `GET /collection/v1_0/accountholder/msisdn/{msisdn}/basicuserinfo` —
    /// the registered holder's name for a number (issue #47).
    ///
    /// Under the **Collections** subscription key and token scope `submit`
    /// and `query_status` already use, which is what makes this buildable at
    /// all: unlike `refund`, no deployment needs a credential it does not
    /// hold. The response is MTN's OIDC-shaped `basicuserinfo` body and is
    /// projected to a name by `wire::BasicUserInfo`, which has no field for
    /// anything else — see that type.
    ///
    /// **Nothing here logs the number or the name.** The `debug!` below
    /// carries the HTTP status and the rail's code only; the masked MSISDN
    /// that reaches an operator's log is written by the `/v1` handler, once,
    /// and `docs/flows/account-holder-lookup.md` is the policy.
    ///
    /// # Errors
    ///
    /// See [`account_holder_outcome`] for the whole table, and the port's
    /// error-surface table for what each variant means. The row worth
    /// naming: a rail that could not be reached is
    /// [`ProviderError::Transport`] and **never** `Ok(None)`.
    async fn account_holder_name(
        &self,
        msisdn: &str,
        config: &ProviderConfig,
    ) -> Result<Option<AccountHolder>, ProviderError> {
        let credentials = Credentials::from_config(config, Product::Collections)?;
        let (subscription, environment) = Adapter::common_headers(&credentials)?;
        // Percent-encoded inside `account_holder_url`, although `/v1`'s own
        // validation admits digits only: this adapter is reachable from the
        // port by any caller, and a path segment interpolated raw is a
        // segment a `/` or a `?` could move to a different endpoint under
        // our own credentials.
        let url = account_holder_url(base_url(config), msisdn);

        let response = self
            .send_authorized(config, &credentials, |token| {
                Ok(self
                    .http
                    .get(&url)
                    .bearer_auth(token)
                    .header(SUBSCRIPTION_KEY_HEADER, subscription.clone())
                    .header(TARGET_ENVIRONMENT_HEADER, environment.clone())
                    .timeout(config.request_timeout))
            })
            .await?;

        let (status, text) = read_body(response).await?;
        tracing::debug!(
            rail = "mtn_momo",
            status = status.as_u16(),
            "basicuserinfo answered"
        );
        account_holder_outcome(status, &text)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use serde_json::{Value, json};
    use uuid::Uuid;
    use vpay_core::{Currency, Money};

    use super::*;

    /// The same vendored-roots client a binary hands in at boot. Building a
    /// real one rather than some test-only substitute keeps the constructor
    /// under test the one that ships.
    fn adapter() -> Adapter {
        Adapter::new(vpay_provider::http::client().expect("the vendored-roots client builds"))
    }

    /// A configuration pointing at a port nothing listens on, with a deadline
    /// short enough that a call which *does* reach the network fails fast.
    /// Used only by tests that assert something never leaves the process.
    fn config() -> ProviderConfig {
        ProviderConfig {
            base_url: "http://127.0.0.1:1".to_owned(),
            callback_url: "http://127.0.0.1:1/provider/mtn_momo/callback".to_owned(),
            currency: Currency::Eur,
            settings: BTreeMap::from([
                (
                    "api_user".to_owned(),
                    "11111111-2222-3333-4444-555555555555".to_owned(),
                ),
                ("target_environment".to_owned(), "sandbox".to_owned()),
            ]),
            credentials: BTreeMap::from([
                (
                    "subscription_key".to_owned(),
                    "stub-subscription-key".to_owned(),
                ),
                ("api_key".to_owned(), "stub-api-key".to_owned()),
            ]),
            connect_timeout: Duration::from_millis(100),
            request_timeout: Duration::from_millis(100),
        }
    }

    fn charge() -> ChargeRef {
        ChargeRef {
            reference_id: Uuid::from_u128(0x0202),
            amount: Money::new(5_000, Currency::Eur).expect("non-negative"),
            payer_ref: Some("237600000000".to_owned()),
            ref_extra: BTreeMap::new(),
            // A push rail has no browser, so nothing fills this on the way
            // in either. `a_return_url_is_not_carried_on_a_push_rails_body`
            // is the case that sets one anyway and proves it is dropped.
            return_url: None,
        }
    }

    #[test]
    fn capabilities_are_coherent() {
        assert!(adapter().capabilities().is_coherent());
    }

    #[test]
    fn code_matches_the_payment_method_type() {
        assert_eq!(adapter().code(), "mtn_momo");
    }

    /// A configuration that also carries the **Disbursements** credentials,
    /// which [`config`] deliberately does not.
    ///
    /// Every value differs from its Collections twin, and that is the whole
    /// point: a test that gave both products the same three strings could
    /// not tell "the refund path read the Disbursements keys" from "the
    /// refund path read whatever was there".
    fn config_with_disbursements() -> ProviderConfig {
        let mut config = config();
        config.credentials.insert(
            "disbursement_subscription_key".to_owned(),
            "stub-disbursement-subscription-key".to_owned(),
        );
        config.credentials.insert(
            "disbursement_api_key".to_owned(),
            "stub-disbursement-api-key".to_owned(),
        );
        config.settings.insert(
            "disbursement_api_user".to_owned(),
            "66666666-7777-8888-9999-000000000000".to_owned(),
        );
        config
    }

    /// The payee every refund test addresses. Not a real subscriber — it is
    /// `basicuserinfo.json`'s registered-holder number, reused so that the
    /// one number in this crate's refund tests is one a reader can look up.
    const DOCUMENTATION_PAYEE: &str = "+237600000200";

    /// The same payee in the shape the rail is handed: digits only, no `+`.
    /// Two constants because the canonicalisation between them is what
    /// `RefundTarget::mobile_money` promises and what this adapter must not
    /// re-do.
    const DOCUMENTATION_PAYEE_CANONICAL: &str = "237600000200";

    fn payee() -> RefundTarget {
        RefundTarget::mobile_money(DOCUMENTATION_PAYEE).expect("a documentation MSISDN")
    }

    /// **The structural half of the token-scope guarantee, and the half the
    /// fingerprint does not cover.**
    ///
    /// Added on review, 2026-09-15, because the claim it holds up was being
    /// made by the wrong thing. `Credentials::fingerprint`'s doc comment,
    /// `docs/reference/rails.md` and this adapter's flow page all said the
    /// product discriminator in the fingerprint was "the only thing standing
    /// between a copy-pasted sandbox configuration and a Collections-scoped
    /// bearer on the money-out path". Measured, it is not: delete
    /// `self.product.path_segment()` from the fingerprint and **the whole
    /// conformance suite stays green at 67/67**, because a cached bearer is
    /// only ever read from [`Adapter::slot`], `slot` is a match on the
    /// product, and [`Adapter::mint`] writes through the same match. The two
    /// named fields are what make cross-product serving unrepresentable; the
    /// fingerprint is defence in depth for the day someone collapses them
    /// into one slot or a map, which is exactly the design that was
    /// considered and rejected.
    ///
    /// Nothing held the fields. Collapse `slot` to `&self.collections_token`
    /// for both arms and this is the test that fails; before it, that
    /// mutation passed all 87 of this crate's tests and all 67 conformance
    /// cases, and every subsequent `transfer` would carry whatever bearer the
    /// charge path had minted.
    #[tokio::test]
    async fn a_products_bearer_is_stored_where_only_that_product_can_read_it() {
        let adapter = adapter();
        let entry = token::cache_entry(
            "a-collections-bearer".to_owned(),
            [0_u8; 32],
            std::time::Instant::now(),
            Some(3600),
        );
        *adapter.slot(Product::Collections).write().await = Some(entry);

        assert!(
            adapter.slot(Product::Disbursements).read().await.is_none(),
            "a Collections bearer landed in the slot `refund` reads: the two products share \
             a cache entry, so a transfer would carry a Collections-scoped token"
        );
        assert!(
            adapter.slot(Product::Collections).read().await.is_some(),
            "the Collections slot did not keep what was written to it, so the assertion \
             above would hold for a cache that stores nothing at all"
        );
    }

    // -- the refund path ---------------------------------------------------

    /// `refund` is no longer a `NotImplemented` token, and answering
    /// `Unsupported` would still be a lie about a rail that refunds.
    ///
    /// The configuration here has **no Disbursements credentials** — which is
    /// every deployment of this system today (`docs/status.md`) — so the
    /// honest answer is `Config` naming the first key that is missing. That
    /// is the assertion, and it is deliberately not "the call succeeds": a
    /// unit test cannot reach a rail (ADR-0006), and the wire is proven by
    /// the conformance suite against a real container.
    #[tokio::test]
    async fn a_deployment_without_a_disbursement_key_is_told_which_key_and_not_declined() {
        let charge = charge();
        let outcome = adapter()
            .refund(&charge, charge.amount, Some(&payee()), &config())
            .await;

        let Err(ProviderError::Config(message)) = outcome else {
            panic!("a missing Disbursements credential is a Config error, got {outcome:?}");
        };
        assert!(
            message.contains("disbursement_subscription_key"),
            "the message names the key an operator must set: {message}"
        );
        // The failure mode this replaces: a token that said "unbuilt" for
        // work that is now built, and an `Unsupported` that would say MTN
        // cannot refund.
        assert!(
            !message.contains("NotImplemented"),
            "the token is retired: {message}"
        );
    }

    /// Each of the three Disbursements keys is required on its own, named on
    /// its own, and never satisfied by its Collections twin.
    ///
    /// The fallback this refuses is the tempting one — `disbursement_api_key`
    /// defaulting to `api_key` — and it is refused because it would send
    /// Collections' Basic password to the Disbursements token mint and
    /// surface as `provider_account_blocked`, which pages and names nothing
    /// that is wrong. See `Credentials::from_config`.
    #[tokio::test]
    async fn every_disbursement_credential_is_required_by_name_with_no_fallback() {
        for key in [
            "disbursement_subscription_key",
            "disbursement_api_key",
            "disbursement_api_user",
        ] {
            let mut config = config_with_disbursements();
            config.credentials.remove(key);
            config.settings.remove(key);

            let charge = charge();
            let outcome = adapter()
                .refund(&charge, charge.amount, Some(&payee()), &config)
                .await;

            let Err(ProviderError::Config(message)) = outcome else {
                panic!("{key}: expected a Config error, got {outcome:?}");
            };
            assert!(message.contains(key), "{key}: {message}");
        }
    }

    /// RFC-0003 open question 6, decided by this adapter (see `refund`).
    ///
    /// The core owes `Some` on a `Required` rail. When that invariant breaks
    /// the answer is `Config` — never a decline, never `Unsupported`, never a
    /// fabricated success — and it must be reached **before** any credential
    /// is read, so a deployment that has neither is told about the thing that
    /// is actually wrong with the request rather than about its YAML.
    #[tokio::test]
    async fn a_refund_with_no_payee_is_refused_before_a_credential_is_read() {
        let charge = charge();
        // A configuration that WOULD satisfy the credential check, so the
        // only thing this can be failing on is the absent destination.
        let outcome = adapter()
            .refund(&charge, charge.amount, None, &config_with_disbursements())
            .await;

        let Err(ProviderError::Config(message)) = outcome else {
            panic!("a Required rail handed no payee answers Config, got {outcome:?}");
        };
        assert!(
            message.contains("payee") && message.contains("RefundDestination::Required"),
            "the message says what the core failed to supply: {message}"
        );
        assert!(
            !message.contains("disbursement_"),
            "the destination is checked before the credentials, so this must not be a \
             configuration complaint: {message}"
        );
    }

    /// A refusal never echoes the payee's number — the rule the whole
    /// `RefundTarget` design rests on, asserted at the one call site that
    /// holds a real one.
    ///
    /// Both spellings, because a message that hid the `+` form and printed
    /// the digits would have leaked it just the same.
    #[tokio::test]
    async fn no_refund_refusal_ever_names_the_payee() {
        let charge = charge();
        // Every configuration failure this method can reach while it still
        // has a destination in scope.
        let mut no_environment = config_with_disbursements();
        no_environment.settings.remove("target_environment");

        for config in [config(), no_environment] {
            let refused = adapter()
                .refund(&charge, charge.amount, Some(&payee()), &config)
                .await
                .expect_err("no rail is reachable from a unit test");
            for rendered in [format!("{refused}"), format!("{refused:?}")] {
                for spelling in [DOCUMENTATION_PAYEE, DOCUMENTATION_PAYEE_CANONICAL] {
                    assert!(
                        !rendered.contains(spelling),
                        "a refusal must never carry the payee's number: {rendered}"
                    );
                }
            }
        }
    }

    // -- the transfer body, without a network ------------------------------

    /// The one field that decides where the money goes.
    ///
    /// `payee`, not `payer` — the single structural difference between a
    /// collection and a disbursement — and the `partyId` inside it is the
    /// canonical digits-only form `RefundTarget` produced, not the `+` form
    /// the merchant typed. A mutation that read `charge.payer_ref` here
    /// instead would send the refund to the person who *paid*, which on a
    /// third-party refund is the wrong human.
    #[test]
    fn the_transfer_is_addressed_to_the_payee_and_never_to_the_payer() {
        let body = transfer_body(
            Uuid::from_u128(0x0202),
            Money::new(5_000, Currency::Eur).expect("non-negative"),
        );

        let payee = body.get("payee").expect("the body has a payee");
        assert_eq!(
            payee.get("partyIdType").and_then(Value::as_str),
            Some("MSISDN")
        );
        assert_eq!(
            payee.get("partyId").and_then(Value::as_str),
            Some(DOCUMENTATION_PAYEE_CANONICAL),
            "the rail takes the canonical form the constructor produced, not the merchant's \
             spelling"
        );
        assert!(
            body.get("payer").is_none(),
            "a `payer` on a transfer would be MTN's other product's body: {body}"
        );
        // The charge's payer is a different number from the payee in
        // `charge()`, so this is falsifiable.
        assert!(
            !body.to_string().contains("237600000000"),
            "the charge's payer must not appear in a transfer: {body}"
        );
    }

    /// The reference the transfer is addressed by is the one the core
    /// supplied, in both places MTN reads it — and nowhere is one invented.
    ///
    /// This is the assertion RFC-0003's open reference question points at:
    /// whether that value is the refund's `provider_reference_id` or the
    /// charge's is the core's to decide, and what this adapter does with it
    /// is here rather than only in prose.
    #[test]
    fn the_transfer_is_addressed_by_the_reference_the_core_supplied() {
        let reference = Uuid::from_u128(0x0de1);
        let body = transfer_body(
            reference,
            Money::new(5_000, Currency::Eur).expect("non-negative"),
        );
        assert_eq!(
            body.get("externalId").and_then(Value::as_str),
            Some(reference.to_string().as_str()),
            "externalId is the reference handed in, never a freshly minted one"
        );
    }

    /// The refund's **own** amount reaches the rail, in MTN's string shape
    /// and in its own currency.
    ///
    /// A partial refund is the case this exists for: `refund` takes an
    /// `amount` separately from `charge.amount` precisely because they
    /// differ, and an implementation that reached for the charge's would
    /// refund the whole thing every time.
    #[test]
    fn a_partial_refund_sends_its_own_amount_and_not_the_charges() {
        let charge = charge();
        let partial = Money::new(1_500, Currency::Eur).expect("non-negative");
        assert_ne!(
            partial, charge.amount,
            "the fixture must make this falsifiable"
        );

        let body = transfer_body(charge.reference_id, partial);
        assert_eq!(
            body.get("amount").and_then(Value::as_str),
            Some(partial.to_provider_string().as_str())
        );
        assert!(
            body.get("amount").expect("an amount").is_string(),
            "MTN takes the amount as a decimal string on both products: {body}"
        );
        assert_eq!(body.get("currency").and_then(Value::as_str), Some("EUR"));
    }

    /// The serialised `transfer` body, as a `Value`, so the assertions above
    /// are about the bytes that would go on the wire rather than about
    /// struct fields a rename would silently change.
    fn transfer_body(reference: Uuid, amount: Money) -> Value {
        serde_json::to_value(wire::Transfer::new(reference, amount, &payee()))
            .expect("the transfer body serialises")
    }

    // -- the refund outcome table, without a network -----------------------

    /// A 202 is the accepted answer, and it carries **no fee** and no key
    /// material.
    ///
    /// `fee: None` is the assertion that matters: issue #46 is about an
    /// integrator hardcoding `0`, and `None` ("the rail did not say") must
    /// never become `Some(zero)` ("the rail said it was free"). MTN's
    /// documented transfer response has no fee field at all.
    #[test]
    fn an_accepted_transfer_reports_no_fee_and_no_key_material() {
        let refunded = refund_outcome(StatusCode::ACCEPTED, "").expect("202 is accepted");
        assert!(refunded.ref_extra.is_empty());
        assert!(
            refunded.fee.is_none(),
            "an unreported fee is None, never Some(0): {:?}",
            refunded.fee
        );
    }

    /// The line a crash-safe retry depends on, on the money-out path.
    ///
    /// See `Adapter::refund` § "Which reference the transfer carries": this
    /// is correct because the reference is the refund's own, and reporting a
    /// 409 as an error would make a caller re-instruct a transfer the rail
    /// already has — paying a payee twice.
    #[test]
    fn a_duplicate_transfer_reference_is_a_success_not_an_error() {
        let refunded = refund_outcome(
            StatusCode::CONFLICT,
            r#"{"code":"RESOURCE_ALREADY_EXIST","message":"Duplicated Reference Id"}"#,
        )
        .expect("409 means the rail already has this transfer");
        assert!(refunded.fee.is_none());
    }

    /// A rail refusal is a decline with the rail's own word mapped through
    /// the shared table — and `PAYEE_NOT_FOUND` is the row a refund reaches
    /// that a charge does not.
    #[test]
    fn a_refused_transfer_is_a_decline_carrying_the_rails_own_reason() {
        let outcome = refund_outcome(
            StatusCode::BAD_REQUEST,
            r#"{"code":"PAYEE_NOT_FOUND","message":"Party not found"}"#,
        );
        let Err(ProviderError::Rejected { code, message }) = outcome else {
            panic!("a 400 from the rail is a decision, got {outcome:?}");
        };
        assert_eq!(code, FailureCode::InvalidPayee);
        assert!(message.contains("PAYEE_NOT_FOUND"), "{message}");
    }

    /// Our own Disbursements credentials being refused pages, and is never
    /// reported as a payer's or payee's problem.
    #[test]
    fn disbursement_credentials_the_rail_refuses_are_not_a_payees_problem() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            let outcome = refund_outcome(status, "");
            assert!(
                matches!(
                    outcome,
                    Err(ProviderError::Rejected {
                        code: FailureCode::ProviderAccountBlocked,
                        ..
                    })
                ),
                "{status}: {outcome:?}"
            );
        }
    }

    /// MTN's biggest wart, on the refund path too: a *logical* error arrives
    /// as HTTP 500 with a code in the body, and three of them are our own
    /// misconfiguration that no retry can fix.
    #[test]
    fn a_transfer_500_that_names_our_misconfiguration_is_never_retried() {
        let outcome = refund_outcome(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"code":"NOT_ALLOWED_TARGET_ENVIRONMENT"}"#,
        );
        assert!(
            matches!(outcome, Err(ProviderError::Config(_))),
            "a configuration 500 stops the ladder: {outcome:?}"
        );

        // A 500 with no code left is the rail, not us.
        let outcome = refund_outcome(StatusCode::INTERNAL_SERVER_ERROR, "<html>oops</html>");
        assert!(
            matches!(outcome, Err(ProviderError::Transport { .. })),
            "an opaque 500 leaves the transfer's fate unknown: {outcome:?}"
        );
    }

    /// A 3xx is refused rather than followed — which on this path would
    /// replay the Disbursements key, the bearer **and the payee's number** at
    /// whatever host `Location` named.
    #[test]
    fn a_transfer_redirect_is_refused_and_says_so() {
        let outcome = refund_outcome(StatusCode::TEMPORARY_REDIRECT, "");
        let Err(ProviderError::Malformed { context, .. }) = &outcome else {
            panic!("a redirect is Malformed, got {outcome:?}");
        };
        assert!(context.contains("not followed"), "{context}");
    }

    /// A 404 on this path is the endpoint, not the refund: the likeliest
    /// cause is a deployment pointed at a base URL with no Disbursements
    /// product behind it. `Config`, so the poll ladder stops.
    #[test]
    fn a_transfer_404_is_our_configuration_and_never_a_refund_that_failed() {
        let outcome = refund_outcome(StatusCode::NOT_FOUND, "");
        assert!(
            matches!(outcome, Err(ProviderError::Config(_))),
            "{outcome:?}"
        );
    }

    /// Every status this table does not enumerate must land somewhere that
    /// leaves the transfer's fate unknown — never on a success and never on
    /// a decline, either of which would be a claim about money.
    #[test]
    fn an_undocumented_transfer_status_is_never_a_success_and_never_a_decline() {
        for code in [200_u16, 201, 204, 302, 418, 451] {
            let status = StatusCode::from_u16(code).expect("a real status");
            let outcome = refund_outcome(status, "");
            assert!(
                outcome.is_err(),
                "HTTP {code} must not be read as an accepted transfer: {outcome:?}"
            );
            assert!(
                !matches!(outcome, Err(ProviderError::Rejected { .. })),
                "HTTP {code} says nothing about a payee: {outcome:?}"
            );
        }
    }

    #[test]
    fn a_base_url_with_a_trailing_slash_does_not_produce_a_double_slash() {
        let mut config = config();
        config.base_url = "http://example.test/".to_owned();
        assert_eq!(base_url(&config), "http://example.test");
    }

    // -- the submit outcome table, without a network -----------------------

    #[test]
    fn an_accepted_submit_returns_no_redirect_and_no_key_material() {
        let submitted = submit_outcome(StatusCode::ACCEPTED, "").expect("202 is a success");
        assert_eq!(submitted.redirect_url, None);
        assert!(submitted.ref_extra.is_empty());
    }

    /// The line a crash-safe retry depends on.
    #[test]
    fn a_duplicate_reference_is_a_success_not_an_error() {
        let submitted = submit_outcome(
            StatusCode::CONFLICT,
            r#"{"code":"RESOURCE_ALREADY_EXIST","message":"Duplicated Reference Id"}"#,
        )
        .expect("409 must be reported as Submitted");
        assert_eq!(submitted.redirect_url, None);
    }

    #[test]
    fn a_400_maps_its_code_through_the_documented_table() {
        match submit_outcome(StatusCode::BAD_REQUEST, r#"{"code":"PAYER_NOT_FOUND"}"#) {
            Err(ProviderError::Rejected { code, .. }) => {
                assert_eq!(code, FailureCode::InvalidPayer);
            }
            other => panic!("expected a decline, got {other:?}"),
        }
    }

    #[test]
    fn a_500_that_names_our_misconfiguration_is_never_retried() {
        match submit_outcome(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"code":"INVALID_CALLBACK_URL_HOST"}"#,
        ) {
            Err(error @ ProviderError::Config(_)) => {
                use vpay_core::Classify as _;
                assert_eq!(error.category(), vpay_core::Category::Configuration);
            }
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }

    #[test]
    fn a_500_with_a_body_that_is_not_json_is_a_transport_error() {
        assert!(matches!(
            submit_outcome(StatusCode::INTERNAL_SERVER_ERROR, "<html>oops</html>"),
            Err(ProviderError::Transport { .. })
        ));
        assert!(matches!(
            submit_outcome(StatusCode::SERVICE_UNAVAILABLE, ""),
            Err(ProviderError::Transport { .. })
        ));
    }

    #[test]
    fn refused_credentials_page_rather_than_look_like_a_decline() {
        use vpay_core::{Classify as _, Severity};

        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            match submit_outcome(status, "") {
                Err(error) => {
                    assert!(
                        matches!(
                            error,
                            ProviderError::Rejected {
                                code: FailureCode::ProviderAccountBlocked,
                                ..
                            }
                        ),
                        "{status}: {error:?}"
                    );
                    assert_eq!(error.severity(), Severity::Page);
                }
                Ok(other) => panic!("{status}: {other:?}"),
            }
        }
    }

    /// A rail's HTML error page must not end up in a log line or an error
    /// message in full.
    #[test]
    fn a_rails_error_body_is_bounded_before_it_reaches_a_message() {
        let huge = "x".repeat(10_000);
        let Err(ProviderError::Transport {
            context: message, ..
        }) = submit_outcome(StatusCode::INTERNAL_SERVER_ERROR, &huge)
        else {
            panic!("expected a transport error")
        };
        assert!(message.chars().count() < 300, "{}", message.len());
    }

    // -- the status outcome table, without a network -----------------------

    #[test]
    fn a_pending_charge_is_pending() {
        assert_eq!(
            status_outcome(StatusCode::OK, r#"{"status":"PENDING"}"#).expect("parses"),
            ChargeStatus::Pending
        );
    }

    #[test]
    fn a_settled_charge_carries_the_rails_transaction_id() {
        assert_eq!(
            status_outcome(
                StatusCode::OK,
                r#"{"status":"SUCCESSFUL","financialTransactionId":"1234567890"}"#
            )
            .expect("parses"),
            ChargeStatus::Succeeded {
                provider_txn_id: Some("1234567890".to_owned())
            }
        );
    }

    #[test]
    fn every_documented_decline_arrives_as_its_taxonomy_code_with_the_raw_reason() {
        for (reason, expected) in mapping::FAILURE_REASONS {
            let body = format!(r#"{{"status":"FAILED","reason":"{reason}"}}"#);
            match status_outcome(StatusCode::OK, &body).expect("a decline is an answer") {
                ChargeStatus::Failed { code, raw } => {
                    assert_eq!(code, expected, "{reason}");
                    assert_eq!(raw, reason, "the rail's own words must survive");
                }
                other => panic!("{reason}: {other:?}"),
            }
        }
    }

    /// A rail that fails a charge without saying why still has to produce a
    /// non-empty `raw` — the conformance suite asserts an operator gets
    /// *something*, and an empty string is not something.
    #[test]
    fn a_decline_with_no_reason_still_carries_words_for_an_operator() {
        match status_outcome(StatusCode::OK, r#"{"status":"FAILED"}"#).expect("parses") {
            ChargeStatus::Failed { code, raw } => {
                assert_eq!(code, FailureCode::ProviderError);
                assert!(!raw.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn no_record_of_a_reference_is_not_a_failure() {
        let status = status_outcome(StatusCode::NOT_FOUND, "").expect("404 is an answer");
        assert_eq!(status, ChargeStatus::NotFound);
        assert!(!matches!(status, ChargeStatus::Failed { .. }));
    }

    #[test]
    fn an_unavailable_rail_is_a_transport_error_never_a_decline() {
        use vpay_core::{Category, Classify as _};

        for status in [StatusCode::SERVICE_UNAVAILABLE, StatusCode::BAD_GATEWAY] {
            let error = status_outcome(status, "").expect_err("5xx is an error");
            assert!(
                matches!(error, ProviderError::Transport { .. }),
                "{error:?}"
            );
            assert_eq!(error.category(), Category::Rail);
        }
    }

    #[test]
    fn a_status_the_rail_has_never_documented_is_not_guessed_at() {
        assert!(matches!(
            status_outcome(StatusCode::OK, r#"{"status":"WHAT"}"#),
            Err(ProviderError::Malformed { .. })
        ));
    }

    // -- the token cache, without a network --------------------------------

    /// The cache short-circuit, proven by the fact that no request leaves the
    /// process: `base_url` points at a closed port, so a call that minted a
    /// token could not possibly succeed.
    #[tokio::test]
    async fn a_cached_token_is_reused_without_touching_the_rail() {
        let adapter = adapter();
        let config = config();
        let credentials = Credentials::from_config(&config, Product::Collections)
            .expect("a complete configuration");
        *adapter.collections_token.write().await = Some(token::cache_entry(
            "cached-token".to_owned(),
            credentials.fingerprint(),
            std::time::Instant::now(),
            Some(3_600),
        ));

        assert_eq!(
            adapter
                .bearer(&config, &credentials)
                .await
                .expect("the cached token serves the call"),
            "cached-token"
        );
    }

    /// The cross-tenant leak the fingerprint exists to prevent. A second
    /// configuration on the same adapter must not be served the first's
    /// token — here that shows up as the call trying to *mint* one and
    /// failing against a closed port, which is the observable difference
    /// between "reused" and "did not reuse".
    #[tokio::test]
    async fn a_second_configuration_never_reuses_the_first_configurations_token() {
        let adapter = adapter();
        let mine = config();
        let mut theirs = config();
        theirs.credentials.insert(
            "subscription_key".to_owned(),
            "another-merchants-key".to_owned(),
        );

        let credentials = Credentials::from_config(&mine, Product::Collections).expect("complete");
        *adapter.collections_token.write().await = Some(token::cache_entry(
            "cached-token".to_owned(),
            credentials.fingerprint(),
            std::time::Instant::now(),
            Some(3_600),
        ));

        let other = Credentials::from_config(&theirs, Product::Collections).expect("complete");
        let outcome = adapter.bearer(&theirs, &other).await;
        assert!(
            matches!(outcome, Err(ProviderError::Transport { .. })),
            "the second configuration must mint its own token, not reuse one: {outcome:?}"
        );
    }

    /// The `#[source]` chain, not just the sentence.
    ///
    /// `ProviderError::Transport` carries the `reqwest::Error` structurally
    /// (ADR-0011) instead of folding it into its own message, and that is
    /// only worth anything if the leaf is still reachable from a boundary
    /// that wants to log or match on it. This test fails the moment anyone
    /// goes back to `format!("{error}")`: the chain would be empty and the
    /// downcast would find nothing.
    ///
    /// The request goes to a closed port, so the failure is a real
    /// `reqwest::Error` and not a constructed one.
    #[tokio::test]
    async fn a_transport_failures_source_chain_reaches_the_reqwest_error() {
        let adapter = adapter();
        let config = config();
        let credentials =
            Credentials::from_config(&config, Product::Collections).expect("complete");

        let error = adapter
            .bearer(&config, &credentials)
            .await
            .expect_err("nothing is listening on the configured port");

        let stage = std::error::Error::source(&error)
            .expect("a Transport must carry the failure it was raised from");
        let leaf = stage
            .source()
            .expect("RailFailure::Http must carry the reqwest error itself");
        assert!(
            leaf.downcast_ref::<reqwest::Error>().is_some(),
            "the chain must reach the reqwest error, not a string that once described it: \
             {leaf}"
        );
        assert!(
            !error.to_string().contains("error sending request"),
            "the reqwest text belongs on the source, not flattened into the message: {error}"
        );
        assert!(
            vpay_core::error::source_chain(&error).contains("sending the request"),
            "the chain is what an operator sees: {}",
            vpay_core::error::source_chain(&error)
        );
    }

    /// An expired token must be re-minted rather than resent — same
    /// observation as above.
    #[tokio::test]
    async fn an_expired_token_is_not_reused() {
        let adapter = adapter();
        let config = config();
        let credentials =
            Credentials::from_config(&config, Product::Collections).expect("complete");
        *adapter.collections_token.write().await = Some(token::cache_entry(
            "cached-token".to_owned(),
            credentials.fingerprint(),
            std::time::Instant::now() - Duration::from_secs(3_600),
            Some(3_600),
        ));

        assert!(matches!(
            adapter.bearer(&config, &credentials).await,
            Err(ProviderError::Transport { .. })
        ));
    }

    /// A missing credential must be refused before anything is sent, so a
    /// misconfigured deployment fails as configuration rather than as a rail
    /// outage.
    #[tokio::test]
    async fn a_missing_credential_is_refused_before_the_first_request() {
        let mut config = config();
        config.credentials.remove("api_key");
        let charge = charge();

        assert!(matches!(
            adapter().submit(&charge, &config).await,
            Err(ProviderError::Config(_))
        ));
        assert!(matches!(
            adapter().query_status(&charge, &config).await,
            Err(ProviderError::Config(_))
        ));
    }

    #[tokio::test]
    async fn a_push_charge_without_a_payer_never_reaches_the_rail() {
        let mut charge = charge();
        charge.payer_ref = None;
        assert!(matches!(
            adapter().submit(&charge, &config()).await,
            Err(ProviderError::Config(_))
        ));
    }

    // -- account holder ----------------------------------------------------

    /// The whole [`account_holder_outcome`] table, row by row, without a
    /// network. The conformance suite then proves the same rows arrive over
    /// a real socket from a real WireMock.
    #[test]
    fn the_account_holder_table_maps_every_documented_status() {
        let found = account_holder_outcome(
            StatusCode::OK,
            r#"{"given_name":"David","family_name":"Mbarga","birthdate":"1970-01-01",
                "locale":"fr_CM","gender":"MALE","status":"ACTIVE"}"#,
        )
        .expect("a 200 with a name is an answer");
        assert_eq!(
            found.as_ref().map(vpay_provider::AccountHolder::name),
            Some("David Mbarga")
        );

        // The row the whole method exists for: 404 is "no record", not an
        // error, and not a fabricated name.
        assert_eq!(
            account_holder_outcome(StatusCode::NOT_FOUND, r#"{"code":"NOT_FOUND"}"#)
                .expect("a 404 is an answer, not a failure"),
            None
        );

        // Our own credentials, refused. Pages, per docs/flows/failures.md.
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            assert!(
                matches!(
                    account_holder_outcome(status, ""),
                    Err(ProviderError::Rejected {
                        code: FailureCode::ProviderAccountBlocked,
                        ..
                    })
                ),
                "{status}"
            );
        }

        // A 500 naming one of MTN's three configuration codes is ours to
        // fix and must not be retried forever.
        assert!(matches!(
            account_holder_outcome(
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"code":"NOT_ALLOWED_TARGET_ENVIRONMENT"}"#
            ),
            Err(ProviderError::Config(_))
        ));

        // Everything else that is not a 200 or a 404 leaves us unable to
        // answer, and **never** produces `Ok(None)`.
        for (status, body) in [
            (StatusCode::BAD_REQUEST, r#"{"code":"BAD_REQUEST"}"#),
            (StatusCode::TEMPORARY_REDIRECT, ""),
            (StatusCode::IM_A_TEAPOT, ""),
            (StatusCode::OK, "not json"),
            (StatusCode::OK, "{}"),
            (StatusCode::OK, r#"{"birthdate":"1970-01-01"}"#),
        ] {
            assert!(
                matches!(
                    account_holder_outcome(status, body),
                    Err(ProviderError::Malformed { .. })
                ),
                "{status} {body}"
            );
        }
        for status in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert!(
                matches!(
                    account_holder_outcome(status, ""),
                    Err(ProviderError::Transport { .. })
                ),
                "{status}"
            );
        }
    }

    /// The mutation this pins: an adapter that returned the rail's whole
    /// body — or any field of it other than the two names — would fail here
    /// before it ever reached the conformance suite.
    #[test]
    fn nothing_but_the_name_survives_the_projection() {
        let body = r#"{"given_name":"David","family_name":"Mbarga",
            "birthdate":"1970-01-01","locale":"fr_CM","gender":"MALE",
            "status":"ACTIVE","sub":"cust_0001"}"#;
        let holder = account_holder_outcome(StatusCode::OK, body)
            .expect("a 200 with a name is an answer")
            .expect("and it is Some");

        assert_eq!(holder.name(), "David Mbarga");
        // The type carries one field and `Debug` redacts it, so there is no
        // rendering of the value in which anything else could appear.
        let rendered = format!("{holder:?}");
        for dropped in ["1970-01-01", "fr_CM", "MALE", "ACTIVE", "cust_0001"] {
            assert!(!rendered.contains(dropped), "{dropped} in {rendered}");
        }
        assert!(
            !rendered.contains("David"),
            "AccountHolder's Debug must redact the name: {rendered}"
        );
    }

    /// `/v1` admits digits only, but the port is reachable by any caller in
    /// this process, and an unescaped `/` would move the request to another
    /// MTN endpoint carrying our subscription key and bearer token.
    ///
    /// **Asserted on the URL [`account_holder_url`] builds**, not on
    /// `path_segment` in isolation. The isolated version is what this test
    /// used to be, and a mutation on 2026-09-06 showed it proved nothing
    /// about the adapter: deleting the `path_segment` call from the method
    /// left it — and 112 other tests — green, because every stubbed MSISDN
    /// is digits-only and escaping them is a no-op.
    #[test]
    fn the_lookup_url_escapes_the_payer_reference_it_interpolates() {
        // The ordinary case, byte for byte: this is the path
        // `wiremock/mtn/mappings/basicuserinfo.json` matches on, so a change
        // to either one is a 404 in CI.
        assert_eq!(
            account_holder_url("http://rail.test", "237600000200"),
            "http://rail.test/collection/v1_0/accountholder/msisdn/237600000200/basicuserinfo",
        );

        // The case the escaping exists for: a traversal must stay inside its
        // own segment rather than becoming a different endpoint.
        let escaped = account_holder_url("http://rail.test", "../../v1_0/token");
        assert_eq!(
            escaped,
            "http://rail.test/collection/v1_0/accountholder/msisdn/\
             ..%2F..%2Fv1_0%2Ftoken/basicuserinfo",
        );
        assert!(
            !escaped.contains("/v1_0/token/basicuserinfo"),
            "the payer reference escaped its path segment: {escaped}"
        );

        // A `?` would otherwise truncate the path and turn the rest into a
        // query string, so the request would not name `basicuserinfo` at all.
        let query = account_holder_url("http://rail.test", "237?x=1");
        assert!(
            query.ends_with("/237%3Fx%3D1/basicuserinfo"),
            "a `?` must not truncate the path: {query}"
        );
    }

    /// MTN returns money by **disbursing** it to a number, so a refund here
    /// can never be "back the way it came". The core is what refuses a
    /// refund with no payee, and it does that on this value — which is why a
    /// change to it is a change to what `POST /v1/refunds` accepts, and why
    /// it is asserted rather than left to a reader of the struct literal.
    #[test]
    fn a_refund_on_this_rail_needs_a_payee() {
        assert_eq!(
            adapter().capabilities().refund_destination,
            RefundDestination::Required
        );
    }

    /// The capability is a claim about MTN *and* about this code: the rail
    /// exposes `basicuserinfo` and the adapter calls it, so `true` here must
    /// not be a rail whose method still answers `Unsupported`.
    #[tokio::test]
    async fn the_account_holder_capability_is_backed_by_an_implementation() {
        let adapter = adapter();
        assert!(adapter.capabilities().supports_account_holder_lookup);

        // `config()` points at a port nothing listens on with a 100 ms
        // deadline, so this reaches the transport and fails there — which is
        // the proof that it is not the port's `Unsupported` default.
        let outcome = adapter.account_holder_name("237600000000", &config()).await;
        assert!(
            matches!(outcome, Err(ProviderError::Transport { .. })),
            "a rail that cannot be reached is a transport failure, never Unsupported and \
             never Ok(None): {outcome:?}"
        );
    }

    // -- callbacks ---------------------------------------------------------

    #[test]
    fn a_callback_yields_identifiers_and_no_status() {
        let reference = Uuid::from_u128(0x0202);
        let body = format!(
            r#"{{"externalId":"{reference}","amount":"5000","currency":"EUR",
                "status":"SUCCESSFUL","financialTransactionId":"1234567890"}}"#
        );
        let parsed = adapter()
            .parse_callback(body.as_bytes())
            .expect("the documented body parses");
        assert_eq!(parsed.reference_id, reference);
        assert!(
            parsed.ref_extra.is_empty(),
            "nothing from an unauthenticated request is carried into ref_extra"
        );
    }

    /// The honesty test: a body that names no charge of ours is refused, not
    /// silently attributed to one.
    #[test]
    fn a_callback_that_names_no_charge_is_refused() {
        assert!(matches!(
            adapter().parse_callback(b"{}"),
            Err(ProviderError::Malformed { .. })
        ));
        assert!(matches!(
            adapter().parse_callback(b"not json at all"),
            Err(ProviderError::Malformed { .. })
        ));
    }

    /// A `Debug` that printed the cached bearer would leak it into any log
    /// line that formats the adapter.
    #[tokio::test]
    async fn debugging_the_adapter_does_not_print_the_token() {
        let adapter = adapter();
        let config = config();
        let credentials =
            Credentials::from_config(&config, Product::Collections).expect("complete");
        *adapter.collections_token.write().await = Some(token::cache_entry(
            "super-secret-token".to_owned(),
            credentials.fingerprint(),
            std::time::Instant::now(),
            Some(3_600),
        ));

        let rendered = format!("{adapter:?}");
        assert!(!rendered.contains("super-secret-token"), "{rendered}");
    }

    // -- the destination this rail parses for itself ------------------------
    //
    // RFC-0003 open question 4, decided 2026-09-15: the adapter owns the wire
    // shape, so these cases live here and not in `vpay-api`. They are
    // deliberately *not* shared with the other adapter — two rails agreeing
    // on a key today is a coincidence, and a shared helper would make the
    // next rail's different key a change to a common file.

    /// The documented shape parses to the payee, and surrounding whitespace
    /// is removed rather than carried onto the rail.
    ///
    /// The trim is the one deliberate deviation from `vpay_api`'s
    /// `payer_instrument`, which tests `trim().is_empty()` and then stores the
    /// untrimmed string — see `parse_destination`'s doc comment for why a
    /// payee is treated differently from a payer here.
    #[test]
    fn a_documented_destination_parses_to_the_payee() {
        // Every spelling a merchant may send for one payee, including the
        // whitespace a copy-paste carries, resolves to the one string the
        // rail is given — `partyId`'s twelve digits, no `+`.
        for spelling in [
            "+237600000200",
            "  +237600000200\t",
            "+237 6 00 00 02 00",
            "+237-600-000-200",
            "+237.600.000.200",
        ] {
            let parsed = adapter()
                .parse_destination(
                    json!({ "msisdn": spelling })
                        .as_object()
                        .expect("a JSON object"),
                )
                .unwrap_or_else(|error| panic!("{spelling:?} must parse: {error}"));
            assert_eq!(
                parsed.msisdn(),
                "237600000200",
                "{spelling:?} is the same payee in the shape the rail takes"
            );
        }
    }

    /// An **empty** map is the decisive case: it is the shape a merchant
    /// sends when they send `destination[mtn_momo]` with nothing under it, and
    /// the one an over-eager parser answers `Ok` to with no payee at all.
    ///
    /// Mutation this case exists for, run on 2026-09-15: making
    /// `parse_destination` answer `Ok(RefundTarget::mobile_money(""))` for an
    /// empty map fails here.
    #[test]
    fn a_destination_missing_this_rails_key_is_malformed() {
        for map in [
            json!({}),
            // A neighbouring key is not this one. Named `phone` rather than a
            // near-miss so the case reads as "we look up one key", not "we
            // guess".
            json!({ "phone": "+237600000200" }),
            json!({ "MSISDN": "+237600000200" }),
        ] {
            let refused = adapter().parse_destination(map.as_object().expect("a JSON object"));
            assert!(
                matches!(refused, Err(ProviderError::Malformed { .. })),
                "{map} must be Malformed, not a silent success: {refused:?}"
            );
        }
    }

    /// A blank value is an absent one, and must fail where a missing key
    /// does — never as an `Ok` carrying an empty payee, which is a refund
    /// addressed to nobody.
    #[test]
    fn a_blank_destination_is_malformed_and_never_a_silent_none() {
        for blank in ["", " ", "\t", "\n", "   \t  "] {
            let refused = adapter().parse_destination(
                json!({ "msisdn": blank })
                    .as_object()
                    .expect("a JSON object"),
            );
            assert!(
                matches!(refused, Err(ProviderError::Malformed { .. })),
                "{blank:?} must be Malformed: {refused:?}"
            );
        }
    }

    /// A non-string value is refused rather than coerced.
    ///
    /// A JSON number in particular: a leading `+` and a leading `0` do not
    /// survive one, so `237600000200` as a number is a value that has already
    /// lost information — and the form encoding a merchant actually posts
    /// never produces one, so accepting it would only ever admit a hand-built
    /// body whose author had already been surprised.
    #[test]
    fn a_non_string_destination_is_refused_rather_than_coerced() {
        for value in [
            json!(237_600_000_200_i64),
            json!(null),
            json!(true),
            json!(["+237600000200"]),
            json!({ "value": "+237600000200" }),
        ] {
            let map = json!({ "msisdn": value });
            let refused = adapter().parse_destination(map.as_object().expect("a JSON object"));
            assert!(
                matches!(refused, Err(ProviderError::Malformed { .. })),
                "{map} must be Malformed: {refused:?}"
            );
        }
    }

    /// The boundary a wave-3 handler has to land on, pinned from this side.
    ///
    /// `raw` is the **rail-scoped inner map** — `destination[mtn_momo]` with the
    /// rail code already stripped. Nothing in the workspace enforces that,
    /// because nothing calls `parse_destination` outside tests yet: the
    /// signature takes a `serde_json::Map` either way, so both sides of the
    /// boundary compile whichever map is handed over. What this case pins is
    /// the direction the mistake fails in. A handler that forgot to strip the
    /// code hands over the *outer* map, and this rail answers `Malformed` — a
    /// refund refused, which an integrator sees — rather than an `Ok`
    /// carrying a payee nobody nominated.
    ///
    /// It is not a substitute for the caller's own test, which wave 3 owes.
    /// It is what makes "wrong boundary" a safe failure instead of a silent
    /// one, and it fails if a future key on this rail ever collides with a
    /// rail code.
    #[test]
    fn the_outer_destination_map_is_refused_rather_than_misread() {
        let outer = json!({ "mtn_momo": { "msisdn": "+237699887766" } });
        let refused = adapter().parse_destination(outer.as_object().expect("a JSON object"));
        assert!(
            matches!(refused, Err(ProviderError::Malformed { .. })),
            "the un-stripped outer map must be refused, never read as this rail's sub-map: \
             {refused:?}"
        );
        let refused = refused.expect_err("refused just above");
        for rendered in [format!("{refused}"), format!("{refused:?}")] {
            assert!(
                !rendered.contains("699887766"),
                "a payee's number must not reach an error message: {rendered}"
            );
        }
    }

    /// A **well-formed string that is not a usable payee** is `Malformed`
    /// here, and the refusal still names no number.
    ///
    /// This is the failure mode the maintainer's decision of 2026-09-15
    /// created: before it, `mtn_momo` handed any non-whitespace string to
    /// `RefundTarget::mobile_money`, which took it. The rule now lives in
    /// `vpay-provider`, next to the type it guards, so this adapter's job is
    /// only to surface the refusal as the port's error table requires — no
    /// network was touched and no rail decided anything, so `Malformed` is
    /// the same answer for the same reason a missing key gets.
    ///
    /// `600000200` is the case to look at: `GET /v1/account_holders` accepts
    /// it and this refuses it. See `RefundTarget::mobile_money` § "Why the
    /// `+` is required".
    #[test]
    fn a_number_that_is_not_a_usable_payee_is_malformed_and_never_echoed() {
        for not_a_payee in [
            "600000200",
            "237600000200",
            "not a phone number",
            "+237699887f66",
            "+0699887766",
            "+69988",
        ] {
            let map = json!({ "msisdn": not_a_payee });
            let refused = adapter().parse_destination(map.as_object().expect("a JSON object"));
            assert!(
                matches!(refused, Err(ProviderError::Malformed { .. })),
                "{not_a_payee:?} must be Malformed, never a payee: {refused:?}"
            );
            let refused = refused.expect_err("refused just above");
            for rendered in [format!("{refused}"), format!("{refused:?}")] {
                assert!(
                    !rendered.contains(not_a_payee),
                    "a payee's number must not reach an error message: {rendered}"
                );
                assert!(
                    rendered.contains("destination[mtn_momo][msisdn]"),
                    "the refusal must still name the parameter: {rendered}"
                );
            }
        }
    }

    /// A refusal names the parameter and **never** the number in it.
    ///
    /// This is the single easiest way to leak a payee's phone number:
    /// `RefundTarget`'s `Debug` redacts, but a parse failure happens *before*
    /// there is a `RefundTarget`, and the raw value is right there in scope.
    /// `ProviderError`'s `context` is rendered into an operator's log and,
    /// through `ApiError`, into what a merchant is shown.
    ///
    /// The value used is one that *would* parse if the key were right, so the
    /// assertion cannot pass merely because the number never reached the
    /// function.
    #[test]
    fn a_refused_destination_never_echoes_the_number() {
        let map = json!({ "wrong_key": "+237699887766" });
        let refused = adapter()
            .parse_destination(map.as_object().expect("a JSON object"))
            .expect_err("a destination under the wrong key is refused");

        for rendered in [format!("{refused}"), format!("{refused:?}")] {
            assert!(
                !rendered.contains("699887766"),
                "a payee's number must not reach an error message: {rendered}"
            );
        }
        assert!(
            format!("{refused}").contains("destination[mtn_momo][msisdn]"),
            "the refusal must name the parameter an integrator should fix: {refused}"
        );
    }
}
