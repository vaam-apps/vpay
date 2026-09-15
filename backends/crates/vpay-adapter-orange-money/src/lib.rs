//! Orange Money Cameroun adapter — the Web Payment (redirect) rail.
//!
//! Implements `submit`, `query_status` and `parse_callback` against the three
//! calls transcribed in `docs/flows/adapter-orange-money.md`. `refund` is
//! overridden with a declared `NotImplemented("orange_money::refund")` token:
//! per the maintainer's decision of 2026-09-15 (RFC-0003 § 5) an Orange refund
//! *is* an outbound transfer to a payee, so the rail can do it and
//! [`Capabilities::supports_refunds`] is `true` — what is missing is vpay's
//! call, which is an admission about *us* and not a capability answer the core
//! may branch on. Inheriting the port's [`ProviderError::Unsupported`] would
//! now be a claim about Orange rather than about this repository.
//!
//! **No Orange transfer wire call is written here, and that is deliberate.**
//! No Orange transfer API is documented in this repository — not even
//! reconstructed — so an endpoint path and a request body would be invented in
//! the money path. The token stays until item 5 of
//! `docs/flows/adapter-orange-money.md`'s "To confirm with Orange Cameroun"
//! list has an answer.
//!
//! **That flow doc is reconstructed from Orange Developer's public overview
//! and community SDKs, not from a vendor specification**, and the error-body
//! shapes in particular are inferred. `docs/reference/rails.md` says what is
//! proven and by what, why nothing in the `wire` module derives `Debug`, and
//! why a missing `pay_token` is a configuration error rather than
//! [`ChargeStatus::NotFound`].

mod mapping;
mod token;
mod wire;

pub use crate::mapping::PRODUCED_FAILURE_CODES;

use std::time::Instant;

use async_trait::async_trait;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use tokio::sync::RwLock;
use uuid::Uuid;
use vpay_core::{FailureCode, Money, ProviderFlow};
use vpay_provider::{
    CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderAdapter, ProviderConfig,
    ProviderError, RefExtra, RefundDestination, RefundTarget, Refunded, Submitted,
};

use crate::token::{cache_entry, fingerprint, token_url};
use crate::wire::{CallbackBody, TokenResponse, TransactionStatusRequest, WebPaymentRequest};

/// The client-credentials grant, as a fixed form body.
///
/// Written out rather than serialised with `RequestBuilder::form` so this
/// crate does not need reqwest's `form` feature — which would add
/// `serde_urlencoded` to every binary in the workspace to encode eleven
/// constant bytes.
const CLIENT_CREDENTIALS_GRANT: &str = "grant_type=client_credentials";
const FORM_URLENCODED: &str = "application/x-www-form-urlencoded";

/// The hosted page's language when a deployment does not configure one.
///
/// French, matching the flow doc's example and Orange Cameroun's default. It
/// is the only defaulted value in the request body, and it is defaulted rather
/// than required because it selects the wording on Orange's page and nothing
/// else: refusing to take a payment over a missing display language would be a
/// worse failure than showing the wrong one.
const DEFAULT_LANG: &str = "fr";

/// How much of a rail error body is carried into an error message.
///
/// Enough for an operator to recognise Orange's own wording, short enough that
/// a rail answering with an HTML error page does not put a screenful into
/// every log line.
const MAX_RAIL_REASON_CHARS: usize = 256;

/// The adapter, owning the outbound HTTP client and its cached rail token.
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
    /// `RwLock` rather than `Mutex` because the overwhelmingly common
    /// operation is a hit — every payment call reads it and only an expiry or
    /// a 401 writes. Two concurrent misses may both mint; that costs one
    /// redundant token call and is strictly better than serialising every
    /// rail call behind a single lock holder.
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
    /// let adapter = vpay_adapter_orange_money::Adapter::new(client);
    ///
    /// assert_eq!(adapter.code(), "orange_money");
    /// // A redirect rail: the payer authenticates with Orange, so `submit`
    /// // returns a hosted-page URL and a `pay_token` the core must commit
    /// // before that URL reaches anyone.
    /// assert_eq!(adapter.capabilities().flow, ProviderFlow::Redirect);
    /// // `true` since 2026-09-15, and it is a claim about the *rail*: the
    /// // maintainer decided that an Orange refund *is* an outbound transfer
    /// // to a payee, which Orange plainly can make. What vpay has not built
    /// // is the call — `refund` answers its own
    /// // `NotImplemented("orange_money::refund")` token, declared in
    /// // `docs/status.md`, rather than the port's `Unsupported`.
    /// assert!(adapter.capabilities().supports_refunds);
    /// // `false`, and decided rather than inherited: see `capabilities()`.
    /// assert!(!adapter.capabilities().supports_partial_refunds);
    /// ```
    #[must_use]
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            token: RwLock::new(None),
        }
    }

    /// A usable bearer, from the cache when one is there.
    async fn access_token(&self, config: &ProviderConfig) -> Result<String, ProviderError> {
        // Both halves, so rotating only the secret evicts the bearer minted
        // from the old one on the very next call — see `token::fingerprint`.
        let fingerprint = fingerprint(
            credential(config, "client_id")?,
            credential(config, "client_secret")?,
        );
        {
            let cached = self.token.read().await;
            let now = Instant::now();
            if let Some(value) = cached
                .as_ref()
                .and_then(|token| token.usable(now, &fingerprint))
            {
                return Ok(value.to_owned());
            }
        }
        self.mint_token(config).await
    }

    /// Mints unconditionally, replacing whatever is cached.
    ///
    /// Called on a miss and again on a 401 from a payment call: a token the
    /// rail revoked early is indistinguishable from a wrong credential until
    /// we have tried a fresh one, and exactly one retry is what separates
    /// "the token aged out" (recoverable, invisible) from "this partner
    /// account is blocked" (pages an operator).
    async fn mint_token(&self, config: &ProviderConfig) -> Result<String, ProviderError> {
        let client_id = credential(config, "client_id")?;
        let client_secret = credential(config, "client_secret")?;
        let url = token_url(&config.base_url)?;

        // Recorded before the send, not after, and for the reason MTN's
        // adapter records it before its own: a token's life starts when the
        // rail mints it, so counting from the moment the response arrived
        // would credit the token with however long the round trip took —
        // and on a slow rail that is exactly the margin `EXPIRY_MARGIN`
        // exists to keep.
        let minted_at = Instant::now();

        let response = self
            .http
            .post(url)
            .basic_auth(client_id, Some(client_secret))
            .header(CONTENT_TYPE, FORM_URLENCODED)
            .body(CLIENT_CREDENTIALS_GRANT)
            // Per-request rather than per-client: one `reqwest::Client` is
            // shared by every rail in the process, so the deadline has to
            // come from this rail's `ProviderConfig`. Without it a token
            // call — which every payment call blocks on — is bounded only
            // by whatever the process-wide client happens to carry, and on
            // a client built with none, by nothing at all.
            .timeout(config.request_timeout)
            .send()
            .await
            .map_err(|error| transport("requesting an access token", error))?;

        let (status, body) = bounded(response, "reading the token response").await?;

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            // Not a payer's problem and not retryable by anyone but us: every
            // charge on this rail is failing (docs/flows/failures.md).
            return Err(ProviderError::Rejected {
                code: FailureCode::ProviderAccountBlocked,
                message: format!(
                    "orange_money: the rail refused the configured client credentials (HTTP {})",
                    status.as_u16()
                ),
            });
        }
        if status.is_server_error() {
            return Err(ProviderError::transport(format!(
                "orange_money: the token endpoint answered HTTP {}",
                status.as_u16()
            )));
        }
        if status.is_redirection() {
            // `vpay_provider::http` builds every client with
            // `redirect::Policy::none()`, so a 3xx arrives here instead of
            // being followed — with the Basic credentials replayed at
            // whatever host the `Location` named. Refusing it is the point.
            return Err(ProviderError::malformed(format!(
                "orange_money: the token endpoint answered a redirect (HTTP {}), which is not \
                 followed; check providers[].host.url",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            // A 4xx that is not an auth refusal means we are asking wrongly —
            // a wrong base URL, or a rail that has moved the endpoint.
            return Err(ProviderError::Config(format!(
                "orange_money: the token endpoint answered HTTP {}; check providers[].host.url",
                status.as_u16()
            )));
        }

        let parsed: TokenResponse = serde_json::from_slice(&body).map_err(|error| {
            ProviderError::malformed(format!(
                "orange_money: the token response is not the documented JSON: {error}"
            ))
        })?;
        if parsed.access_token.trim().is_empty() {
            return Err(ProviderError::malformed(
                "orange_money: the token response carries an empty access_token".to_owned(),
            ));
        }

        // A missing `expires_in` means "use it for this call, do not cache
        // it". The alternative — inventing a lifetime — is a guess that
        // expresses itself as intermittent 401s under load, and the honest
        // cost of not guessing is one extra token call per payment call on a
        // rail that has never been observed to omit the field.
        if let Some(seconds) = parsed.expires_in {
            let mut cache = self.token.write().await;
            *cache = Some(cache_entry(
                parsed.access_token.clone(),
                minted_at,
                std::time::Duration::from_secs(seconds),
                fingerprint(client_id, client_secret),
            ));
        } else {
            tracing::warn!(
                provider = "orange_money",
                "token response carried no expires_in; not caching the bearer"
            );
        }

        Ok(parsed.access_token)
    }

    /// Sends `body` to `url` with a bearer, minting a fresh one and retrying
    /// exactly once if the rail answers 401.
    ///
    /// Returns the final status and body; interpreting them is each caller's
    /// job, because a 404 means "no such charge" to `query_status` and "the
    /// base URL is wrong" to `submit`.
    async fn post_authenticated<B: serde::Serialize>(
        &self,
        config: &ProviderConfig,
        url: &str,
        body: &B,
        what: &str,
    ) -> Result<(StatusCode, Vec<u8>), ProviderError> {
        let token = self.access_token(config).await?;
        let (status, response_body) = self.post_once(config, url, &token, body, what).await?;
        if status != StatusCode::UNAUTHORIZED {
            return Ok((status, response_body));
        }

        tracing::info!(
            provider = "orange_money",
            operation = what,
            "rail answered 401; minting a fresh token and retrying once"
        );
        let token = self.mint_token(config).await?;
        self.post_once(config, url, &token, body, what).await
    }

    /// One authenticated POST, bounded in time and in memory.
    ///
    /// Takes the whole `ProviderConfig` because the deadline is the *rail's*,
    /// not the process's — see `docs/reference/rails.md`. This call carried
    /// none at all until the Step 3 security review, so a black-holed Orange
    /// host held a worker task for as long as the shared client allowed.
    async fn post_once<B: serde::Serialize>(
        &self,
        config: &ProviderConfig,
        url: &str,
        token: &str,
        body: &B,
        what: &str,
    ) -> Result<(StatusCode, Vec<u8>), ProviderError> {
        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(body)
            .timeout(config.request_timeout)
            .send()
            .await
            .map_err(|error| transport(what, error))?;
        bounded(response, what).await
    }
}

/// The rail's answer, bounded, as bytes.
///
/// A thin naming of [`vpay_provider::http::read_rail_body`] so that every
/// call site says `orange_money: {what}` the same way. Every body this
/// adapter reads goes through it rather than through `Response::bytes()`,
/// which reads to end of stream and so lets the peer decide how much memory
/// a worker task allocates.
///
/// # Errors
///
/// As [`vpay_provider::http::read_rail_body`]:
/// [`ProviderError::Malformed`] naming the cap when the body exceeds it, and
/// [`ProviderError::Transport`] if the stream fails part-way. Never a
/// decline — an oversize or truncated answer says nothing about whether the
/// payment happened.
async fn bounded(
    response: reqwest::Response,
    what: &str,
) -> Result<(StatusCode, Vec<u8>), ProviderError> {
    vpay_provider::http::read_rail_body(response, &format!("orange_money: {what}")).await
}

#[async_trait]
impl ProviderAdapter for Adapter {
    fn code(&self) -> &'static str {
        "orange_money"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            flow: ProviderFlow::Redirect,
            // `true` since 2026-09-15, and a claim about the *rail*, not
            // about vpay: an Orange refund is an outbound transfer to a payee
            // and Orange makes transfers. It was `false` until the maintainer
            // decided what an Orange refund *is* (RFC-0003 § 5); answering
            // the port's `Unsupported` now would tell the core that Orange
            // has no refund product, which is a lie about Orange rather than
            // an admission about us. The admission is `refund`'s
            // `NotImplemented("orange_money::refund")` token below, which
            // `verify-status` counts and `docs/status.md` declares.
            supports_refunds: true,
            // `false`, and *decided* rather than left over from the flip.
            // `partial_refunds_imply_refunds` (migration `0002`) and
            // `Capabilities::is_coherent` both permit `true` now, and
            // permitted is not decided.
            //
            // The flag above rests on one thing being known — that an Orange
            // refund is a transfer — and nothing else about Orange transfers
            // is known here: no endpoint, no body, no amount semantics, no
            // minimum, no per-transaction limit (item 7 of the flow doc's "to
            // confirm" list is open for payments, and there is no transfer
            // equivalent of it at all). "Any amount up to the charge" is a
            // property of a transfer product, and this repository has never
            // seen Orange's.
            //
            // Which means **neither value is a true statement about Orange**,
            // and that is the part worth being explicit about: the flag above
            // is defended as a claim about the rail, and this one cannot be,
            // because nobody here knows the answer. It is the same modelling
            // gap `Capabilities::is_coherent` records for `refund_destination`
            // — a two-valued field with no spelling for "unknown" — and a
            // different criterion has to break the tie. Saying so is the
            // difference between a decision and a declaration dressed as one.
            //
            // The tie-break is the merchant-visible direction. If the real
            // transfer product turns out to reverse whole payments only, or to
            // floor at some amount, then a `true` merchants had already
            // integrated against has to be *withdrawn*, which is a breaking
            // change made on a guess. `false` → `true` is additive and costs
            // nobody anything. Flipping it is part of writing the transfer
            // call against a real specification, not part of this declaration.
            //
            // **No core code reads this today** (2026-09-15), on either rail:
            // `POST /v1/refunds` is unrouted, and `is_coherent` plus the
            // `providers` seed write are the only readers in the workspace. An
            // earlier version of this comment said "the core refuses a
            // part-refund on this rail and a merchant is told no" — there is
            // no such refusal, and describing one is how this repository would
            // start sounding more finished than it is.
            supports_partial_refunds: false,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
            // `false`, and it means "Orange's Web Payment product documents
            // no account-holder lookup", not "we have not written one".
            // That is why this adapter overrides nothing and inherits the
            // port's `Unsupported` — which is no longer the shape `refund`
            // has here, and is the opposite of `mtn_momo`, whose rail *does*
            // expose one. Orange has a KYC/customer product elsewhere; its route is
            // unconfirmed from this repository and belongs on
            // `docs/flows/adapter-orange-money.md`'s "to confirm" list, not
            // in a `true` nobody can honour (issue #47).
            supports_account_holder_lookup: false,
            // `Required`, and it is the same sentence `supports_refunds`
            // above is now true by: an Orange refund *is* an outbound
            // transfer to a payee. There is no "back the way it came" on a
            // redirect rail where `payer_ref` is `None` and vpay never learns
            // who paid, so a refund here is addressed or it is nowhere.
            //
            // This value was declared, and declared `Required`, while
            // `supports_refunds` was still `false` — deliberately, so that
            // the flip would be one line about refunds and not also a fresh
            // guess about destinations. It was. `Capabilities::is_coherent`
            // records why the pair was never a coherence violation.
            refund_destination: RefundDestination::Required,
        }
    }

    /// `POST {base_url}/v1/webpayment`.
    ///
    /// `order_id` is our own `reference_id` rendered as a string, which is
    /// what makes a same-reference retry safe: the rail keys the hosted page
    /// on it and answers a repeat with the same `pay_token`, so a duplicate
    /// submission comes back as [`Submitted`], never as an error.
    ///
    /// Where the payer ends up is [`ChargeRef::return_url`]'s answer and
    /// never this adapter's: `return_url` and `cancel_url` both carry it
    /// verbatim (Step 9, D2). Until 2026-09-04 they were read from
    /// *deployment* settings, falling back to the notification endpoint —
    /// one answer per deployment to a question that is per charge, so a
    /// merchant's `return_url` was stored on `charges`, echoed back to them
    /// on `next_action`, and never sent to the rail that would act on it.
    /// The two settings keys are gone with it; nothing shipped set them
    /// (`config/application.yml` never had either).
    ///
    /// Both fields get the same value. Orange's page distinguishes them —
    /// "paid" against "the payer gave up" — and vpay cannot yet tell those
    /// apart on the way back: the outcome comes from the authenticated
    /// status query, not from which link the payer clicked, and a charge the
    /// payer abandoned is `Pending` until it expires. Sending two different
    /// URLs would be a claim this system does not check.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] for a missing credential, an unusable base
    /// URL, or a charge with no `return_url` — a redirect rail must be told
    /// where the payer goes next, and this adapter will not invent it;
    /// [`ProviderError::Rejected`] with
    /// [`FailureCode::ProviderAccountBlocked`] when the rail refuses our
    /// credentials even after a fresh token; [`ProviderError::Transport`] for
    /// a 5xx, a timeout or a connection failure; [`ProviderError::Malformed`]
    /// if a 2xx body is not the documented one.
    async fn submit(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        let merchant_key = credential(config, "merchant_key")?;
        let callback_url = config.callback_url.as_str();
        let return_url = return_url(charge)?;

        let body = WebPaymentRequest {
            merchant_key,
            // The amount's *own* currency, not the deployment's default: the
            // pair (amount, currency) has to stay the one `Money` guarantees.
            // If a route ever hands this rail a charge in another currency,
            // the rail refusing it is the correct outcome — relabelling the
            // amount would not be.
            currency: charge.amount.currency().code(),
            order_id: charge.reference_id.to_string(),
            // A JSON number, per the flow doc. See `Money::to_provider_minor`.
            amount: charge.amount.to_provider_minor(),
            return_url,
            cancel_url: return_url,
            notif_url: callback_url,
            lang: setting(config, "lang").unwrap_or(DEFAULT_LANG),
        };

        let url = endpoint(&config.base_url, "v1/webpayment");
        let (status, response_body) = self
            .post_authenticated(config, &url, &body, "submitting a web payment")
            .await?;

        if status.is_success() {
            return mapping::submitted(&response_body);
        }
        Err(submit_error(status, &response_body))
    }

    /// `POST {base_url}/v1/transactionstatus`.
    ///
    /// A missing `pay_token` is a [`ProviderError::Config`] and deliberately
    /// not [`ChargeStatus::NotFound`] — `docs/reference/rails.md` says why
    /// the difference decides whether a payer's money is looked for.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] if `ref_extra` carries no `pay_token` or a
    /// credential is missing; [`ProviderError::Rejected`] with
    /// [`FailureCode::ProviderAccountBlocked`] when the rail refuses our
    /// credentials; [`ProviderError::Transport`] for a 5xx, a timeout or a
    /// connection failure; [`ProviderError::Malformed`] for a body that is not
    /// the documented one, including an unrecognised status string.
    async fn query_status(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError> {
        let pay_token = charge
            .ref_extra
            .get("pay_token")
            .map(String::as_str)
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| {
                ProviderError::Config(format!(
                    "orange_money: charge {} has no pay_token in ref_extra; the rail cannot be \
                     asked about an order_id alone",
                    charge.reference_id
                ))
            })?;

        let body = TransactionStatusRequest {
            order_id: charge.reference_id.to_string(),
            amount: charge.amount.to_provider_minor(),
            pay_token,
        };

        let url = endpoint(&config.base_url, "v1/transactionstatus");
        let (status, response_body) = self
            .post_authenticated(config, &url, &body, "querying a transaction status")
            .await?;

        // Before `is_success`, and before any decline mapping: "I have no
        // record" is the canonical answer, never a failure.
        if status == StatusCode::NOT_FOUND {
            return Ok(ChargeStatus::NotFound);
        }
        if status.is_success() {
            return mapping::charge_status(&response_body);
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ProviderError::Rejected {
                code: FailureCode::ProviderAccountBlocked,
                message: format!(
                    "orange_money: the rail refused the configured credentials on a status query \
                     (HTTP {})",
                    status.as_u16()
                ),
            });
        }
        if status.is_server_error() {
            return Err(ProviderError::transport(format!(
                "orange_money: the rail answered HTTP {} to a status query: {}",
                status.as_u16(),
                rail_reason(&response_body)
            )));
        }
        if status.is_redirection() {
            // Redirects are not followed (`vpay_provider::http`), so a 3xx
            // is an answer to refuse rather than a hop to take — and taking
            // it would have replayed the bearer and the `pay_token` at
            // whatever host the `Location` named.
            return Err(ProviderError::malformed(format!(
                "orange_money: the rail answered a redirect (HTTP {}) to a status query, which \
                 is not followed; check providers[].host.url",
                status.as_u16()
            )));
        }
        // Any other 4xx on a *read* is our request being wrong, not the
        // charge being declined — a decline arrives as a 200 with a status
        // string. Classifying it as a decline would fail a live charge on the
        // strength of a malformed request.
        Err(ProviderError::Config(format!(
            "orange_money: the rail answered HTTP {} to a status query: {}",
            status.as_u16(),
            rail_reason(&response_body)
        )))
    }

    /// Identifiers only, and fails closed.
    ///
    /// The body's `status` is read by nothing here — `CallbackBody` has no
    /// field for it. A `notif_token` is *required*, and
    /// `docs/reference/rails.md` says why refusing to parse beats handing a
    /// caller a hint it cannot check. `pay_token` is carried through when
    /// present so a callback can repair a charge whose `ref_extra` write was
    /// lost; whether Orange actually sends it is item 1 of the flow doc's
    /// "To confirm" list.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`] if the body is not JSON, carries no
    /// `order_id`, carries an `order_id` that is not a reference we could have
    /// generated, or carries no `notif_token`.
    fn parse_callback(&self, body: &[u8]) -> Result<CallbackRef, ProviderError> {
        let parsed: CallbackBody = serde_json::from_slice(body).map_err(|error| {
            ProviderError::malformed(format!(
                "orange_money: the notification body is not JSON: {error}"
            ))
        })?;

        let order_id = parsed.order_id.ok_or_else(|| {
            ProviderError::malformed(
                "orange_money: the notification carries no order_id".to_owned(),
            )
        })?;
        let reference_id = Uuid::parse_str(order_id.trim()).map_err(|error| {
            ProviderError::malformed(format!(
                "orange_money: the notification's order_id is not a reference we generated: {error}"
            ))
        })?;
        let notif_token = parsed
            .notif_token
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| {
                ProviderError::malformed(
                    "orange_money: the notification carries no notif_token; refusing to parse an \
                     unverifiable callback"
                        .to_owned(),
                )
            })?;

        let mut ref_extra = RefExtra::new();
        ref_extra.insert("notif_token".to_owned(), notif_token);
        if let Some(pay_token) = parsed.pay_token.filter(|token| !token.trim().is_empty()) {
            ref_extra.insert("pay_token".to_owned(), pay_token);
        }

        Ok(CallbackRef {
            reference_id,
            ref_extra,
        })
    }

    /// `destination[orange_money][msisdn]` — the payee a refund on this rail
    /// would be transferred to.
    ///
    /// # Why this is built while `refund` is a token
    ///
    /// Nothing calls this today: `refund` is
    /// `NotImplemented("orange_money::refund")` and `POST /v1/refunds` is
    /// unrouted. It is built anyway, because the two answer different
    /// questions and only one of them needs Orange's transfer
    /// specification. *Who* a refund is addressed to is vpay's own
    /// merchant-facing parameter (RFC-0003 § 1) and is fully known — it is
    /// the sub-map the merchant sends, and this method reads it. *How* an
    /// Orange transfer body would carry that payee is not known here at all,
    /// and stays unwritten, which is why `refund` is a token.
    ///
    /// This heading used to read "why this exists on a rail whose `refund`
    /// does not", and rested on `supports_refunds: false` — before
    /// 2026-09-15, when the maintainer decided an Orange refund *is* a
    /// transfer back and the flag became `true`. The method did not move; the
    /// reason it is here got simpler.
    ///
    /// Leaving this to the port's default would still be wrong for the
    /// original reason: [`ProviderError::Unsupported`] answered to a question
    /// about the payee claims Orange returns money to the instrument that
    /// paid, which is false on a redirect rail where `payer_ref` is `None`
    /// and vpay never learns who paid.
    ///
    /// # What is refused
    ///
    /// A missing key, a non-string value, and a string that is empty or all
    /// whitespace, each [`ProviderError::Malformed`]. Empty is absent, for
    /// the reason [`credential`] gives: a blank value is a lost one, not a
    /// choice, and must fail where a missing one does rather than travel as
    /// `""`.
    ///
    /// A string that is not a usable international number is refused as well,
    /// but by [`vpay_provider::RefundTarget::mobile_money`] and not by
    /// anything here: this adapter owns the *key*, the port owns the
    /// *number*. That is the maintainer's decision of 2026-09-15, and it is
    /// why there is no second spelling of the rule to keep in step — an
    /// adapter cannot construct an invalid `RefundTarget` at all.
    ///
    /// It replaced the confirm path's rule, which this method applied first:
    /// any non-whitespace string, handed to the rail as written. A mistyped
    /// payer number fails a charge; a mistyped payee number sends money to
    /// whoever owns it. A bare `600000200` is therefore refused here while
    /// `GET /v1/account_holders` accepts it, and
    /// [`vpay_provider::RefundTarget::mobile_money`] § "Why the `+` is
    /// required" argues that asymmetry.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`], and nothing else — this reaches no
    /// network. The message names the parameter and **never** its value: a
    /// payee's number in an error string would undo `RefundTarget`'s
    /// redacting `Debug`.
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
                    "orange_money: a refund on this rail needs the payee's number, sent as a \
                     non-empty string in `destination[orange_money][{DESTINATION_MSISDN_KEY}]`"
                ))
            })?;
        // `InvalidMsisdn` is rendered, the input is not: every variant of it
        // is a unit variant, so this cannot echo the number even by accident.
        RefundTarget::mobile_money(msisdn).map_err(|invalid| {
            ProviderError::malformed(format!(
                "orange_money: `destination[orange_money][{DESTINATION_MSISDN_KEY}]` is not a payee this \
                 rail can be given — {invalid}"
            ))
        })
    }

    /// Not built, and honestly so.
    ///
    /// Overrides the port's default so this rail answers
    /// [`ProviderError::NotImplemented`] rather than
    /// [`ProviderError::Unsupported`]. `Unsupported` is a claim about
    /// *Orange* — "this rail has no refund API" — and since the maintainer's
    /// decision of 2026-09-15 (RFC-0003 § 5) it is not true: an Orange refund
    /// is an outbound transfer to a payee, which Orange makes. The token is
    /// an admission about *vpay*, `cargo xtask verify-status` is what counts
    /// it, and `docs/status.md` is where it is declared.
    ///
    /// # Why there is no wire call here
    ///
    /// **No Orange transfer API is documented in this repository — not even
    /// reconstructed.** The three calls this adapter does make came from
    /// Orange Developer's public overview plus several community SDKs that
    /// agree with each other; for transfers no such source exists here, so an
    /// endpoint path and a request body would be *invented*, in the money
    /// path, on a rail nobody has ever called. That is the failure mode
    /// `CLAUDE.md` names first. What unblocks it is an answer to item 5 of
    /// `docs/flows/adapter-orange-money.md`'s "To confirm with Orange
    /// Cameroun" list, not more reading.
    ///
    /// `destination` is `Some` here because this rail declares
    /// [`RefundDestination::Required`], and [`Self::parse_destination`] is
    /// what produced it. It is ignored rather than read: there is no call to
    /// put it in, and reading it would be the first half of a pretence.
    ///
    /// # Errors
    ///
    /// Always [`ProviderError::NotImplemented`].
    async fn refund(
        &self,
        _charge: &ChargeRef,
        _amount: Money,
        _destination: Option<&RefundTarget>,
        _config: &ProviderConfig,
    ) -> Result<Refunded, ProviderError> {
        Err(ProviderError::NotImplemented("orange_money::refund"))
    }
}

/// The one key this rail names inside `destination[orange_money]`, and the
/// only place in this crate that spells it.
///
/// vpay's **merchant-facing** parameter (RFC-0003 § 1), not a field of
/// Orange's API — there is no documented Orange transfer body in this
/// repository to have taken a name from.
const DESTINATION_MSISDN_KEY: &str = "msisdn";

/// `{base}/{path}`, tolerating a configured trailing slash.
fn endpoint(base: &str, path: &str) -> String {
    format!("{}/{path}", base.trim_end_matches('/'))
}

/// A required credential, or the configuration error that names it.
///
/// Empty is treated as absent: an unset `${ORANGE_CLIENT_ID}` that expanded to
/// nothing must fail at the same place a missing key does, not as a 401 an
/// operator reads as "the rail blocked us".
fn credential<'a>(config: &'a ProviderConfig, key: &str) -> Result<&'a str, ProviderError> {
    config
        .credentials
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ProviderError::Config(format!(
                "orange_money: credentials.{key} is missing or empty"
            ))
        })
}

/// Where this charge's payer is sent when the hosted page is done with them.
///
/// Required, and the twin of `mtn_momo`'s "payer_ref required on a push rail":
/// a redirect rail hands the payer to a page vpay does not control, and a
/// charge that cannot say where they come back to is one whose payer is
/// stranded on the rail's own thank-you screen. Refusing before the call is
/// the only place that can still be fixed without a payer having moved money.
///
/// Empty is absent, for the same reason it is in [`credential`]: a blank
/// column is a lost value, not a choice, and it must fail where a missing one
/// does rather than reach the rail as `""`.
///
/// # Errors
///
/// [`ProviderError::Config`], naming the reference so an operator can find the
/// charge. The URL itself is never quoted — it is the merchant's, and this
/// error line goes to a log.
fn return_url(charge: &ChargeRef) -> Result<&str, ProviderError> {
    charge
        .return_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            ProviderError::Config(format!(
                "orange_money: charge {} has no return_url; a redirect rail must be told where \
                 the payer goes when the hosted page is done with them",
                charge.reference_id
            ))
        })
}

/// An optional non-empty setting.
fn setting<'a>(config: &'a ProviderConfig, key: &str) -> Option<&'a str> {
    config
        .settings
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
}

/// How a non-2xx `webpayment` response is classified.
///
/// A 5xx or a 401 is about us or the rail; a 400 is about the request. None of
/// them is a payer's problem, which is why nothing here maps to a payer-facing
/// [`FailureCode`]: Orange documents no error-code vocabulary for this call
/// (the flow doc is a reconstruction), so inventing a mapping table would
/// produce confident, wrong `insufficient_funds` on a body nobody has seen.
/// [`FailureCode::ProviderError`] is the documented answer for "unmapped,
/// carries the raw reason" and is meant to be alerted on.
fn submit_error(status: StatusCode, body: &[u8]) -> ProviderError {
    let reason = rail_reason(body);
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return ProviderError::Rejected {
            code: FailureCode::ProviderAccountBlocked,
            message: format!(
                "orange_money: the rail refused the configured credentials on a submit (HTTP {})",
                status.as_u16()
            ),
        };
    }
    if status.is_server_error() {
        return ProviderError::transport(format!(
            "orange_money: the rail answered HTTP {} to a submit: {reason}",
            status.as_u16()
        ));
    }
    if status == StatusCode::NOT_FOUND {
        // The payment API is not where the configuration says it is. A charge
        // must not be declined for that.
        return ProviderError::Config(format!(
            "orange_money: no webpayment endpoint under the configured base_url (HTTP 404): \
             {reason}"
        ));
    }
    if status.is_redirection() {
        // Before the catch-all, because the catch-all is a *decline* and a
        // 3xx is not one: nobody refused the payment, the rail pointed
        // somewhere else. `vpay_provider::http` does not follow it — a
        // 307/308 would have replayed this request body, `merchant_key` and
        // all, at whatever host the `Location` named — so it arrives here,
        // and it leaves the charge's fate unknown, which is `Malformed`.
        return ProviderError::malformed(format!(
            "orange_money: the rail answered a redirect (HTTP {}) to a submit, which is not \
             followed; check providers[].host.url",
            status.as_u16()
        ));
    }
    ProviderError::Rejected {
        code: FailureCode::ProviderError,
        message: format!(
            "orange_money: the rail refused the payment (HTTP {}): {reason}",
            status.as_u16()
        ),
    }
}

/// The rail's own words, bounded, for an operator.
///
/// Only ever called on a non-2xx body: a success body carries the `pay_token`
/// and must not reach a message. Lossy UTF-8 because a rail behind a proxy can
/// answer with anything, and a decode failure must not replace the diagnostic.
fn rail_reason(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "<empty body>".to_owned();
    }
    let short: String = trimmed.chars().take(MAX_RAIL_REASON_CHARS).collect();
    if short.chars().count() < trimmed.chars().count() {
        format!("{short}…")
    } else {
        short
    }
}

/// A `reqwest` failure as [`ProviderError::Transport`], carrying the error
/// itself as the `#[source]`.
///
/// The chain matters: reqwest's own `Display` for a timeout is "error sending
/// request for url (…)", and the word "timeout" only appears on the source.
/// An operator diagnosing a rail from a log line needs the leaf. This used to
/// walk `Error::source()` by hand and concatenate the result into a `String`,
/// because the variant could not hold a source; it can now, so the walk is
/// the *logger's* (`vpay_core::error::source_chain`) and the structure
/// survives all the way to it.
fn transport(what: &str, error: reqwest::Error) -> ProviderError {
    ProviderError::transport_from(format!("orange_money: {what}"), error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use serde_json::json;
    use vpay_core::{Currency, Money};

    use super::*;

    /// The same vendored-roots client a binary hands in at boot. Building a
    /// real one rather than some test-only substitute keeps the constructor
    /// under test the one that ships.
    fn adapter() -> Adapter {
        Adapter::new(vpay_provider::http::client().expect("the vendored-roots client builds"))
    }

    fn config() -> ProviderConfig {
        ProviderConfig {
            base_url: "https://api.orange.com/orange-money-webpay/dev".to_owned(),
            callback_url: "https://vpay.example/provider/orange_money/callback".to_owned(),
            currency: Currency::Xaf,
            settings: BTreeMap::from([
                ("env".to_owned(), "dev".to_owned()),
                ("lang".to_owned(), "en".to_owned()),
            ]),
            credentials: BTreeMap::from([
                ("merchant_key".to_owned(), "mk".to_owned()),
                ("client_id".to_owned(), "ci".to_owned()),
                ("client_secret".to_owned(), "cs".to_owned()),
            ]),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(20),
        }
    }

    fn reference() -> Uuid {
        Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0202)
    }

    /// Where the core says this charge's payer goes next. A merchant's own
    /// URL here; for a session-driven charge it would be vpay's return page,
    /// and this adapter cannot tell the two apart — which is the point of
    /// `ChargeRef::return_url` being a string the core decides.
    const RETURN_URL: &str = "https://shop.example/order/1234/return";

    /// A charge as the confirm path hands it over: the reference the mappings
    /// key on, and a `return_url` because a redirect rail's charge always has
    /// one.
    fn charge(return_url: Option<&str>, amount: Money) -> ChargeRef {
        ChargeRef {
            reference_id: reference(),
            amount,
            // A redirect rail authenticates the payer itself and may never
            // learn who they are.
            payer_ref: None,
            ref_extra: BTreeMap::new(),
            return_url: return_url.map(ToOwned::to_owned),
        }
    }

    /// Rebuilds `submit`'s body the way `submit` does, so the assertions below
    /// are about the bytes that go on the wire.
    ///
    /// # Panics
    ///
    /// If the charge carries no usable `return_url` — the cases about that
    /// call [`super::return_url`] directly.
    fn request_body(config: &ProviderConfig, charge: &ChargeRef) -> serde_json::Value {
        let callback_url = config.callback_url.as_str();
        let return_url = super::return_url(charge).expect("the charge carries a return_url");
        let body = WebPaymentRequest {
            merchant_key: credential(config, "merchant_key").expect("configured"),
            currency: charge.amount.currency().code(),
            order_id: charge.reference_id.to_string(),
            amount: charge.amount.to_provider_minor(),
            return_url,
            cancel_url: return_url,
            notif_url: callback_url,
            lang: setting(config, "lang").unwrap_or(DEFAULT_LANG),
        };
        serde_json::to_value(&body).expect("the request body serialises")
    }

    /// One field of a serialised body. `get` rather than `Value`'s `Index`,
    /// which panics on a missing key — `clippy.toml` exempts tests from
    /// `expect`, not from `indexing_slicing`, and a named `expect` says which
    /// field went missing where an index would only say "null".
    fn field<'a>(body: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
        body.get(key)
            .unwrap_or_else(|| panic!("the request body has a {key} field"))
    }

    #[test]
    fn capabilities_are_coherent() {
        assert!(adapter().capabilities().is_coherent());
    }

    /// An Orange refund is addressed or it is nowhere, and the core must
    /// learn that from this value rather than from the string
    /// `"orange_money"` (ADR-0002).
    ///
    /// This case carried a second assertion, `!supports_refunds`, written as
    /// a **tripwire** whose failure message was the checklist of prose that
    /// had to move with the flag. RFC-0003 § 5 flipped the flag on
    /// 2026-09-15, the tripwire fired as designed, and its checklist was
    /// worked: `Adapter::new`'s doctest at the top of this file, the
    /// `supports_refunds` row and the paragraph under it in
    /// `docs/flows/provider-port.md`, and `docs/status.md`'s
    /// `NotImplemented` list, which gained `orange_money::refund`. The line
    /// is gone because what it claimed stopped being true; the checklist is
    /// restated here because a tripwire deleted without a trace is
    /// indistinguishable from one deleted to go green.
    ///
    /// What guards the pair now is
    /// [`refund_is_a_token_about_vpay_not_an_answer_about_orange`] below.
    #[test]
    fn a_refund_on_this_rail_would_need_a_payee() {
        let capabilities = adapter().capabilities();
        assert_eq!(capabilities.refund_destination, RefundDestination::Required);
    }

    /// The flip's other half, and the one that can rot silently.
    ///
    /// `supports_refunds: true` with an un-overridden `refund` answers the
    /// port's `Unsupported` — a rail declaring it refunds and then denying
    /// the operation exists, which is the likeliest way to leave this change
    /// half-done. It is invisible to the compiler and to `verify-status`
    /// alike, because the token that should be counted would simply not be
    /// there.
    ///
    /// The exact token string is asserted, not merely the variant:
    /// `docs/status.md` declares it verbatim and `cargo xtask verify-status`
    /// fails in both directions on a mismatch, so a typo here breaks a build
    /// somewhere else and this is where it is legible.
    #[tokio::test]
    async fn refund_is_a_token_about_vpay_not_an_answer_about_orange() {
        let charge = charge(
            None,
            Money::new(5_000, Currency::Xaf).expect("non-negative"),
        );
        let outcome = adapter()
            .refund(
                &charge,
                charge.amount,
                Some(&RefundTarget::mobile_money("+237600000200").expect("a documentation MSISDN")),
                &config(),
            )
            .await;
        assert!(
            matches!(
                outcome,
                Err(ProviderError::NotImplemented("orange_money::refund"))
            ),
            "a rail declaring supports_refunds owes a token, not Unsupported: {outcome:?}"
        );
        assert!(
            adapter().capabilities().supports_refunds,
            "the token above is only the honest answer while the rail is declared able to \
             refund; were this flag ever to go back to false, `refund` would have to go back \
             to the port's Unsupported in the same commit"
        );
    }

    #[test]
    fn code_matches_the_payment_method_type() {
        assert_eq!(adapter().code(), "orange_money");
    }

    /// The one place MTN and Orange disagree about the same `Money`: Orange's
    /// `amount` is a JSON *number*. Sending `"5000"` here is a 400 from the
    /// rail; sending `50` on a two-decimal currency is a 100× error nothing
    /// downstream can detect.
    #[test]
    fn the_amount_is_a_json_number_in_minor_units() {
        let body = request_body(
            &config(),
            &charge(
                Some(RETURN_URL),
                Money::new(5_000, Currency::Xaf).expect("non-negative"),
            ),
        );
        assert_eq!(*field(&body, "amount"), json!(5_000));
        assert!(
            field(&body, "amount").is_number(),
            "not a string: {}",
            field(&body, "amount")
        );
    }

    #[test]
    fn the_body_is_the_documented_shape() {
        let body = request_body(
            &config(),
            &charge(
                Some(RETURN_URL),
                Money::new(5_000, Currency::Xaf).expect("non-negative"),
            ),
        );
        assert_eq!(
            body,
            json!({
                "merchant_key": "mk",
                "currency": "XAF",
                "order_id": "00000000-0000-0000-0000-000000000202",
                "amount": 5_000,
                "return_url": RETURN_URL,
                "cancel_url": RETURN_URL,
                "notif_url": "https://vpay.example/provider/orange_money/callback",
                "lang": "en",
            })
        );
    }

    /// The charge decides where the payer goes, and both link fields carry
    /// it — Step 9's D2, and the assertion that would fail if this adapter
    /// went back to answering a per-charge question out of deployment
    /// settings.
    ///
    /// The settings are set here *and must be ignored*: they are the keys
    /// this adapter read until 2026-09-04, so a revert that restored the
    /// `setting(config, "return_url")` lookup would make the body carry
    /// `https://deployment.example/…` and fail on the exact value rather
    /// than on a shape.
    #[test]
    fn both_link_fields_carry_the_charges_return_url_and_no_setting_overrides_it() {
        let mut config = config();
        config.settings.insert(
            "return_url".to_owned(),
            "https://deployment.example/ok".to_owned(),
        );
        config.settings.insert(
            "cancel_url".to_owned(),
            "https://deployment.example/cancelled".to_owned(),
        );

        let body = request_body(
            &config,
            &charge(
                Some(RETURN_URL),
                Money::new(5_000, Currency::Xaf).expect("non-negative"),
            ),
        );
        assert_eq!(*field(&body, "return_url"), json!(RETURN_URL));
        assert_eq!(*field(&body, "cancel_url"), json!(RETURN_URL));
        // The notification URL is never a merchant's to choose: it is where
        // the rail talks to *us*.
        assert_eq!(
            *field(&body, "notif_url"),
            json!("https://vpay.example/provider/orange_money/callback")
        );
    }

    /// A redirect charge with nowhere to send the payer is refused before the
    /// rail is called, and the refusal names the reference rather than
    /// quoting the URL.
    ///
    /// Absent and blank are the same answer: a blank column is a value that
    /// was lost, and letting `""` reach the rail would strand the payer on
    /// Orange's own page with vpay believing it had told them where to go.
    #[test]
    fn a_redirect_charge_with_no_return_url_is_a_configuration_error() {
        let amount = Money::new(5_000, Currency::Xaf).expect("non-negative");
        for absent in [None, Some(""), Some("   ")] {
            let charge = charge(absent, amount);
            let outcome = super::return_url(&charge);
            let ProviderError::Config(message) = outcome.expect_err("must be refused") else {
                panic!("a missing return_url must be a Config error, not a decline");
            };
            assert!(
                message.contains("00000000-0000-0000-0000-000000000202"),
                "the refusal must name the charge: {message}"
            );
        }
    }

    #[test]
    fn a_missing_lang_falls_back_rather_than_refusing_a_payment() {
        let mut config = config();
        config.settings.remove("lang");
        let body = request_body(
            &config,
            &charge(
                Some(RETURN_URL),
                Money::new(5_000, Currency::Xaf).expect("non-negative"),
            ),
        );
        assert_eq!(*field(&body, "lang"), json!(DEFAULT_LANG));
    }

    /// A EUR charge must not be relabelled XAF because the deployment's
    /// default currency says so.
    #[test]
    fn the_currency_is_the_amounts_own() {
        let body = request_body(
            &config(),
            &charge(
                Some(RETURN_URL),
                Money::new(5_000, Currency::Eur).expect("non-negative"),
            ),
        );
        assert_eq!(*field(&body, "currency"), json!("EUR"));
        assert_eq!(*field(&body, "amount"), json!(5_000), "still minor units");
    }

    #[test]
    fn a_missing_credential_is_a_configuration_error_that_names_the_key() {
        let mut config = config();
        config.credentials.remove("merchant_key");
        let error = credential(&config, "merchant_key").expect_err("it is required");
        assert!(matches!(error, ProviderError::Config(_)), "{error:?}");
        assert!(error.to_string().contains("merchant_key"), "{error}");
    }

    #[test]
    fn an_empty_credential_is_treated_as_missing() {
        let mut config = config();
        config
            .credentials
            .insert("client_id".to_owned(), "   ".to_owned());
        assert!(credential(&config, "client_id").is_err());
    }

    #[test]
    fn endpoints_tolerate_a_configured_trailing_slash() {
        assert_eq!(
            endpoint("https://h/orange-money-webpay/dev/", "v1/webpayment"),
            "https://h/orange-money-webpay/dev/v1/webpayment"
        );
        assert_eq!(
            endpoint("https://h/orange-money-webpay/dev", "v1/transactionstatus"),
            "https://h/orange-money-webpay/dev/v1/transactionstatus"
        );
    }

    // ---------------------------------------------------------------
    // parse_callback: identifiers only, and fail closed.
    // ---------------------------------------------------------------

    fn documented_callback(reference: Uuid) -> Vec<u8> {
        format!(
            r#"{{"order_id":"{reference}","status":"SUCCESS","txnid":"stub-txn",
                "notif_token":"nt","pay_token":"pt"}}"#
        )
        .into_bytes()
    }

    #[test]
    fn a_documented_callback_yields_the_reference_and_both_tokens() {
        let parsed = adapter()
            .parse_callback(&documented_callback(reference()))
            .expect("the documented body parses");

        assert_eq!(parsed.reference_id, reference());
        assert_eq!(
            parsed.ref_extra.get("notif_token").map(String::as_str),
            Some("nt")
        );
        assert_eq!(
            parsed.ref_extra.get("pay_token").map(String::as_str),
            Some("pt")
        );
        assert_eq!(parsed.ref_extra.len(), 2);
    }

    /// Item 1 of the flow doc's "To confirm" list: whether the notification
    /// carries a `pay_token` at all. Until it is confirmed, its absence must
    /// not break the callback.
    #[test]
    fn a_callback_without_a_pay_token_still_parses() {
        let body = format!(
            r#"{{"order_id":"{}","status":"SUCCESS","notif_token":"nt"}}"#,
            reference()
        );
        let parsed = adapter()
            .parse_callback(body.as_bytes())
            .expect("pay_token is optional");
        assert!(!parsed.ref_extra.contains_key("pay_token"));
        assert!(parsed.ref_extra.contains_key("notif_token"));
    }

    /// The fail-closed case. Without a `notif_token` there is nothing to
    /// compare against, so the callback is indistinguishable from a forged
    /// POST by anyone who can guess an order id.
    #[test]
    fn a_callback_without_a_notif_token_is_refused() {
        let body = format!(r#"{{"order_id":"{}","status":"SUCCESS"}}"#, reference());
        let error = adapter()
            .parse_callback(body.as_bytes())
            .expect_err("an unverifiable callback must not parse");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_callback_with_an_empty_notif_token_is_refused() {
        let body = format!(r#"{{"order_id":"{}","notif_token":"  "}}"#, reference());
        assert!(adapter().parse_callback(body.as_bytes()).is_err());
    }

    #[test]
    fn a_callback_whose_order_id_is_not_our_reference_is_refused() {
        let error = adapter()
            .parse_callback(br#"{"order_id":"ORDER-42","notif_token":"nt"}"#)
            .expect_err("an order_id we did not generate identifies nothing");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn an_empty_callback_body_is_refused() {
        let error = adapter()
            .parse_callback(b"{}")
            .expect_err("no identifiers, no CallbackRef");
        assert!(
            matches!(error, ProviderError::Malformed { .. }),
            "{error:?}"
        );
        assert!(adapter().parse_callback(b"not json").is_err());
    }

    // ---------------------------------------------------------------
    // Error classification.
    // ---------------------------------------------------------------

    #[test]
    fn credentials_the_rail_refuses_page_an_operator_and_are_not_a_decline() {
        use vpay_core::{Classify as _, Severity};

        let error = submit_error(StatusCode::UNAUTHORIZED, b"");
        match &error {
            ProviderError::Rejected { code, .. } => {
                assert_eq!(*code, FailureCode::ProviderAccountBlocked);
                assert_eq!(error.severity(), Severity::Page);
            }
            other => panic!("expected a blocked-account rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_rail_5xx_on_submit_is_transport_never_a_decline() {
        use vpay_core::{Category, Classify as _};

        let error = submit_error(StatusCode::SERVICE_UNAVAILABLE, b"upstream down");
        assert!(
            matches!(error, ProviderError::Transport { .. }),
            "{error:?}"
        );
        assert_eq!(error.category(), Category::Rail);
    }

    #[test]
    fn a_404_on_submit_is_a_misconfigured_base_url_not_a_declined_charge() {
        let error = submit_error(StatusCode::NOT_FOUND, b"");
        assert!(matches!(error, ProviderError::Config(_)), "{error:?}");
    }

    #[test]
    fn an_unmapped_submit_refusal_carries_the_rails_own_words() {
        let error = submit_error(StatusCode::BAD_REQUEST, br#"{"message":"bad order_id"}"#);
        match &error {
            ProviderError::Rejected { code, message } => {
                assert_eq!(*code, FailureCode::ProviderError);
                assert!(message.contains("bad order_id"), "{message}");
            }
            other => panic!("expected an unmapped refusal, got {other:?}"),
        }
        // The taxonomy's meaning reaches the merchant; the rail's words do not.
        use vpay_core::Classify as _;
        assert!(!error.public_message().contains("bad order_id"));
    }

    #[test]
    fn a_long_rail_body_is_bounded_before_it_reaches_a_log_line() {
        let body = "x".repeat(10_000);
        let reason = rail_reason(body.as_bytes());
        assert!(
            reason.chars().count() <= MAX_RAIL_REASON_CHARS + 1,
            "{}",
            reason.len()
        );
        assert!(reason.ends_with('…'));
    }

    #[test]
    fn an_empty_rail_body_still_says_something() {
        assert_eq!(rail_reason(b"   "), "<empty body>");
    }

    /// `ProviderConfig`'s derived `Debug` prints credentials in full, so the
    /// adapter's own `Debug` — which a `tracing` field could reach — must not
    /// carry any.
    #[test]
    fn the_adapters_debug_carries_no_credentials() {
        let rendered = format!("{:?}", adapter());
        assert!(rendered.contains("Adapter"), "{rendered}");
        assert!(!rendered.contains("client_secret"), "{rendered}");
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
    /// sends when they send `destination[orange_money]` with nothing under it, and
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
    /// `raw` is the **rail-scoped inner map** — `destination[orange_money]` with the
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
        let outer = json!({ "orange_money": { "msisdn": "+237699887766" } });
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
    /// created: before it, `orange_money` handed any non-whitespace string to
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
                    rendered.contains("destination[orange_money][msisdn]"),
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
            format!("{refused}").contains("destination[orange_money][msisdn]"),
            "the refusal must name the parameter an integrator should fix: {refused}"
        );
    }
}
