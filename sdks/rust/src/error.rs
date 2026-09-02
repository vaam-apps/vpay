//! The SDK's error types.
//!
//! `docs/flows/merchant-auth.md` distinguishes five failure shapes on the
//! wire — a builder-time misconfiguration, a token-endpoint rejection, a
//! resource-route error envelope, a response that is neither, and a
//! transport failure — plus webhook verification, which never touches the
//! network at all. Each gets its own variant rather than a single
//! stringly-typed error, so a caller can match on `Error::Api { status, .. }`
//! without parsing a message.

use std::time::Duration;

use thiserror::Error;

/// A misconfiguration caught at [`crate::ClientBuilder::build`] time, or by
/// [`crate::auth::mint_client_assertion`] when called directly.
///
/// Deliberately a *different* type from [`enum@Error`], not a variant of it:
/// every other `Error` variant describes something that happened on the
/// wire, and `ClientBuilder::build` cannot have put anything on the wire
/// yet — conflating the two would let a caller `match` on a network-shaped
/// error for a problem that never left the process.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// [`crate::ClientBuilder::build`] was called with no
    /// [`crate::Credentials`] — there is nothing to sign an assertion with.
    #[error("no credentials configured — call ClientBuilder::credentials(..) before build()")]
    MissingCredentials,

    /// Outside `1..=300` seconds. `docs/flows/merchant-auth.md` requires the
    /// SDK to refuse this at construction rather than silently clamping it:
    /// a clamped value would mint assertions whose `exp` differs from what
    /// the caller asked for, for a reason invisible at the call site. The
    /// bound itself mirrors `authkestra_engine::client_assertion::
    /// MAX_CLIENT_ASSERTION_LIFETIME_SECS`, which the pinned OP verifier
    /// enforces on the receiving end.
    #[error(
        "assertion lifetime must be between 1 and 300 seconds (see \
         `crate::auth::MIN_ASSERTION_LIFETIME_SECS`/`MAX_ASSERTION_LIFETIME_SECS`), got {lifetime:?}"
    )]
    InvalidAssertionLifetime {
        /// The lifetime that was rejected, for the caller's own error message.
        lifetime: Duration,
    },

    /// [`crate::Credentials::rsa_pem`] was given a PEM `jsonwebtoken` could
    /// not parse as an RSA key (PKCS#1 or PKCS#8).
    #[error("invalid RSA private key: {0}")]
    InvalidPrivateKey(String),

    /// Signing the assertion itself failed — `jsonwebtoken::encode` returned
    /// an error after the key parsed successfully. Kept distinct from
    /// [`ConfigError::InvalidPrivateKey`] because a key that decodes but
    /// then fails to sign points at a different bug than one that never
    /// parsed at all.
    #[error("failed to sign the client assertion: {0}")]
    Signing(String),

    /// The system clock reads before the Unix epoch. Not reachable on any
    /// real deployment; kept as a named, non-panicking error rather than an
    /// `unwrap` because this crate's lint policy denies both
    /// (`docs/adr/0007-lint-policy.md`).
    #[error("system clock is set before the Unix epoch")]
    SystemClockBeforeEpoch,

    /// [`reqwest::ClientBuilder::build`] failed — TLS backend construction,
    /// almost always.
    #[error("failed to construct the HTTP client: {0}")]
    Http(String),
}

/// Why [`crate::webhooks::verify`] refused a delivery.
///
/// See `docs/flows/webhooks.md`: the header must parse, the timestamp must
/// be within tolerance, and at least one `v1=` signature must match — in
/// that order, so a caller's error message names the *first* thing that was
/// actually wrong rather than a generic "invalid signature".
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WebhookError {
    /// `Vpay-Signature` was missing `t=`, carried no non-empty `v1=`, or had
    /// a `t` that is not a bare run of decimal digits (`^[0-9]+$`).
    ///
    /// The `t` rule is stricter than "parses as a number" on purpose, and is
    /// the same rule `sdks/nodejs/src/webhooks.ts` applies: a `t` this
    /// verifier had to *reinterpret* (`+1753401600`, `1753401600.0`, a hex
    /// literal) would have it compute an HMAC over bytes the sender never
    /// signed, which surfaces as a signature mismatch and sends the reader
    /// looking for the wrong bug.
    #[error("malformed Vpay-Signature header")]
    MalformedHeader,

    /// `|now - t|` exceeded the caller's tolerance. Checked *before* the
    /// HMAC comparison so a stale replay is rejected without needing the
    /// secret at all — not a security property, just a cheaper failure path.
    #[error("signature timestamp is outside the tolerance window")]
    TimestampOutOfTolerance,

    /// None of the `v1=` values in the header matched the HMAC computed
    /// over `"{t}.{raw_body}"` — where `{t}` is the header's **literal** `t`
    /// text — with the given secret.
    #[error("no v1 signature matched the computed HMAC")]
    SignatureMismatch,

    /// The signature matched, but the body underneath it is not a JSON
    /// [`crate::Event`]. This is not a security failure — the signature
    /// proved the bytes are genuine — only a decoding one.
    #[error("verified webhook body is not a valid event: {0}")]
    InvalidBody(String),
}

/// Every way a `/v1` interaction, or webhook verification, can fail.
///
/// One enum for the whole crate rather than one per module: a caller
/// integrating against `/v1` wants a single `match` at the call site, not a
/// different error type per resource.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A non-2xx response shaped like `vpay_api`'s Stripe-style error envelope
    /// (`docs/api/README.md`): `{ "error": { "type", "code", "message",
    /// "param" } }`. `code` and `param` are optional on the wire.
    #[error("vpay API error ({status}): {kind} — {message}")]
    Api {
        /// The HTTP status code.
        status: u16,
        /// The envelope's `error.type`, e.g. `invalid_request_error`.
        kind: String,
        /// The envelope's `error.code`, if present.
        code: Option<String>,
        /// The envelope's `error.message`.
        message: String,
        /// The envelope's `error.param`, if the error names one field.
        param: Option<String>,
    },

    /// A `TokenErrorResponse` from the token endpoint itself
    /// (`{ "error": "invalid_client", "error_description": "…" }`).
    /// Never retried automatically — `docs/flows/merchant-auth.md` §4 is
    /// explicit that a `401` from the token endpoint is not the same event
    /// as a `401` from a resource route.
    #[error("token endpoint error: {error}")]
    TokenEndpoint {
        /// The OAuth2 `error` code, e.g. `invalid_client`.
        error: String,
        /// The OAuth2 `error_description`, if present.
        description: Option<String>,
    },

    /// A non-2xx response that is not the Stripe-shaped envelope above — a
    /// proxy's HTML 502, for instance. Carries only a bounded prefix of the
    /// body: an unbounded one would let a misbehaving upstream hand this
    /// crate an unbounded amount of memory to hold in an error value.
    #[error("unexpected response (status {status}): {body_prefix}")]
    UnexpectedResponse {
        /// The HTTP status code.
        status: u16,
        /// The first bytes of the body, lossily decoded as UTF-8.
        body_prefix: String,
    },

    /// DNS, TLS, a timeout, or a connection refused — the request never
    /// produced an HTTP response to classify.
    #[error("transport error: {0}")]
    Transport(String),

    /// A request parameter this SDK refuses to put on the wire, caught
    /// before the request was built — today, only an amount outside
    /// `0..=2^53-1` (see [`crate::CreatePaymentIntentParams::amount`]).
    ///
    /// A variant of [`enum@Error`] rather than of [`ConfigError`] even
    /// though nothing reached the wire: [`ConfigError`] is what
    /// [`crate::ClientBuilder::build`] returns, and a caller who has already
    /// built a client is holding a `Result<_, Error>` at the call site. It
    /// exists at all — rather than letting the server reject the value —
    /// because the Node SDK refuses the same inputs
    /// (`sdks/nodejs/src/validate.ts`), and an amount one SDK sends and the
    /// other refuses is a parity defect in a *money* field.
    #[error("invalid request parameter {param}: {message}")]
    InvalidParams {
        /// The offending parameter's wire name, e.g. `amount`.
        param: String,
        /// What is wrong with it, in the caller's terms.
        message: String,
    },

    /// A [`ConfigError`] surfaced through a method that returns [`enum@Error`]
    /// (token minting during a request, for instance) rather than through
    /// [`crate::ClientBuilder::build`] directly.
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    /// A [`WebhookError`] from [`crate::webhooks::verify`]/`verify_at`.
    #[error("webhook verification failed: {0}")]
    Webhook(#[from] WebhookError),
}
