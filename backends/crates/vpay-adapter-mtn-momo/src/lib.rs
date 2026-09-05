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
//! 404 is not a failure — and why [`Adapter::refund`] is
//! [`ProviderError::NotImplemented`] rather than
//! [`ProviderError::Unsupported`].
//!
//! Credentials are never logged and never rendered; see the `token` module.

mod mapping;
mod token;
mod wire;

use async_trait::async_trait;
use reqwest::StatusCode;
use reqwest::header::{HeaderName, HeaderValue};
use tokio::sync::RwLock;
use vpay_core::{FailureCode, Money, ProviderFlow};
use vpay_provider::{
    AccountHolder, CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderAdapter,
    ProviderConfig, ProviderError, RefExtra, Submitted,
};

use crate::token::{Credentials, SUBSCRIPTION_KEY_HEADER, TARGET_ENVIRONMENT_HEADER};

/// The reference *we* generate. It is MTN's transaction id, which is what
/// makes a same-reference resubmission idempotent rather than a second
/// charge.
const REFERENCE_ID_HEADER: HeaderName = HeaderName::from_static("x-reference-id");

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
    /// The cached bearer token, or `None` before the first call.
    ///
    /// An async `RwLock` because it is held across the `.await` that mints a
    /// token, and a blocking lock over an await point parks a runtime thread
    /// on a network round trip. One slot, not a map keyed by fingerprint —
    /// `docs/reference/rails.md` says why that is safe *and* why a map would
    /// be worse.
    token: RwLock<Option<vpay_provider::token::CachedToken>>,
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
            token: RwLock::new(None),
        }
    }

    /// A usable bearer token, minting one only if the cache cannot serve the
    /// call.
    async fn bearer(
        &self,
        config: &ProviderConfig,
        credentials: &Credentials<'_>,
    ) -> Result<String, ProviderError> {
        let fingerprint = credentials.fingerprint();
        if let Some(cached) = self.token.read().await.as_ref()
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
        *self.token.write().await = Some(minted);
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
/// a mutation that deleted the call left 113 tests green —
/// `a_payer_reference_is_escaped_before_it_becomes_a_path_segment` exercised
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
            // /collection/v1_0/accountholder/MSISDN/{msisdn}/basicuserinfo`
            // under the subscription key and token scope this adapter
            // already holds, and `account_holder_name` below calls it — so
            // `true` here is a claim about the rail *and* about this code
            // (issue #47).
            supports_account_holder_lookup: true,
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
        let credentials = Credentials::from_config(config)?;
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
        let credentials = Credentials::from_config(config)?;
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

    /// Not built, and honestly so.
    ///
    /// MTN refunds are the **Disbursements** product: a different
    /// subscription key, a separately-scoped token, and a `transfer` call
    /// this adapter does not make. No deployment of this system holds those
    /// credentials, so there is nothing to implement against — and
    /// [`Capabilities::supports_refunds`] stays `true` because the rail *does*
    /// support refunds; it is we who have not built them. Answering
    /// [`ProviderError::Unsupported`] instead would be a lie about the rail.
    ///
    /// # Errors
    ///
    /// Always [`ProviderError::NotImplemented`] — listed in `docs/status.md`,
    /// which `cargo xtask verify-status` enforces.
    async fn refund(
        &self,
        _charge: &ChargeRef,
        _amount: Money,
        _config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        Err(ProviderError::NotImplemented("mtn_momo::refund"))
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
        let credentials = Credentials::from_config(config)?;
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

    /// The one operation that is genuinely unbuilt must say so — not answer
    /// `Unsupported`, which would claim the rail cannot refund, and not
    /// fabricate a success.
    #[tokio::test]
    async fn refund_is_not_implemented_and_does_not_pretend() {
        let charge = charge();
        let outcome = adapter().refund(&charge, charge.amount, &config()).await;
        assert!(matches!(
            outcome,
            Err(ProviderError::NotImplemented("mtn_momo::refund"))
        ));
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
        let credentials = Credentials::from_config(&config).expect("a complete configuration");
        *adapter.token.write().await = Some(token::cache_entry(
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

        let credentials = Credentials::from_config(&mine).expect("complete");
        *adapter.token.write().await = Some(token::cache_entry(
            "cached-token".to_owned(),
            credentials.fingerprint(),
            std::time::Instant::now(),
            Some(3_600),
        ));

        let other = Credentials::from_config(&theirs).expect("complete");
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
        let credentials = Credentials::from_config(&config).expect("complete");

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
        let credentials = Credentials::from_config(&config).expect("complete");
        *adapter.token.write().await = Some(token::cache_entry(
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
        let credentials = Credentials::from_config(&config).expect("complete");
        *adapter.token.write().await = Some(token::cache_entry(
            "super-secret-token".to_owned(),
            credentials.fingerprint(),
            std::time::Instant::now(),
            Some(3_600),
        ));

        let rendered = format!("{adapter:?}");
        assert!(!rendered.contains("super-secret-token"), "{rendered}");
    }
}
