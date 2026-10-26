//! The SDK's error types.
//!
//! `docs/flows/merchant-auth.md` distinguishes the failure shapes on the
//! wire — a builder-time misconfiguration, a resource-route error envelope, a
//! response that is unsigned or does not verify, a response that is neither,
//! and a transport failure — plus webhook verification, which never touches the
//! network at all. Each gets its own variant rather than a single
//! stringly-typed error, so a caller can match on `Error::Api { status, .. }`
//! without parsing a message.

use thiserror::Error;

/// A misconfiguration caught at [`crate::ClientBuilder::build`] time, or while
/// constructing [`crate::Credentials`] / [`crate::ServerKeys`].
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
    /// [`crate::Credentials`] — there is nothing to sign requests with.
    #[error("no credentials configured — call ClientBuilder::credentials(..) before build()")]
    MissingCredentials,

    /// [`crate::ClientBuilder::build`] was called with no
    /// [`crate::ServerKeys`]. There is no default and no "trust the first key
    /// you see": a client that did not know whose answers to believe would
    /// believe anyone's.
    #[error("no server keys pinned — call ClientBuilder::server_keys(..) before build()")]
    MissingServerKeys,

    /// A seed that is not 32 bytes of unpadded base64url. Never quoted.
    #[error("an Ed25519 seed is 32 bytes as unpadded base64url")]
    InvalidSeed,

    /// A public key that is not 32 bytes of unpadded base64url, or not a point
    /// on the curve. Never quoted.
    #[error("an Ed25519 public key is 32 bytes as unpadded base64url, and a point on the curve")]
    InvalidPublicKey,

    /// The signed-transport envelope could not be assembled (an empty
    /// audience, for instance).
    #[error("failed to assemble the signed transport: {0}")]
    Envelope(String),

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

    /// The response was not a signed message: a bare `401` from vpay's
    /// envelope layer (an unknown or disabled key, a bad signature, a clock
    /// more than five minutes off, a replayed `cti` — **deliberately the same
    /// answer for every cause**, cratestack ADR 0006 §10), or something in
    /// front of vpay (a proxy's HTML 502). Nothing in its body was read.
    ///
    /// A `401` here is almost always one of two things the merchant can fix:
    /// the key is not the one registered under `cose_keys`, or this host's
    /// clock is wrong.
    #[error(
        "the response was not signed (status {status}); for a 401 check the registered key and this host's clock"
    )]
    Unsigned {
        /// The HTTP status code.
        status: u16,
    },

    /// A response that claimed to be signed did not verify under any pinned
    /// server key. Treat it as hostile: either the pinned `{current, next}` is
    /// stale (a rotation outran this client's configuration) or something other
    /// than vpay answered.
    #[error("the response's signature did not verify under any pinned server key")]
    Unverified,

    /// The request could not be sealed: the signer failed (a KMS callback
    /// erroring, for instance) or the call is one the transport cannot carry.
    /// Nothing was sent.
    #[error("failed to seal the request: {0}")]
    Seal(String),

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

    /// The answer's body exceeded [`crate::MAX_RESPONSE_BYTES`]. Reading stopped
    /// at the bound; nothing of the body was kept or opened. The honest server
    /// never sends one this large (a page of 100 objects is a few hundred
    /// kilobytes), so this is a misbehaving peer or proxy, and an unbounded
    /// read would let it make this process hold whatever it liked.
    #[error("the response body exceeded {limit} bytes and was not read further")]
    ResponseTooLarge {
        /// The bound, in bytes.
        limit: usize,
    },

    /// A request parameter this SDK refuses to put on the wire, caught
    /// before the request was built: an amount outside `0..=2^53-1` (see
    /// [`crate::CreatePaymentIntentParams::amount`]), a URL a combination of
    /// fields gives no meaning to, and — since 2026-10-21 — a path `id` that
    /// is empty or only dots, on every method that takes one. Such an id
    /// would have addressed a different endpoint once the URL was parsed;
    /// that refusal's `message` is one fixed sentence that never quotes the
    /// id.
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
    /// rather than through [`crate::ClientBuilder::build`] directly.
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),

    /// A [`WebhookError`] from [`crate::webhooks::verify`]/`verify_at`.
    #[error("webhook verification failed: {0}")]
    Webhook(#[from] WebhookError),
}
