//! Why an SSO step refused, written for the log and never for the wire.
//!
//! # One answer on the wire, a precise one in the log
//!
//! Every variant becomes [`crate::ApiError::StaffSignInRefused`] — the one
//! `401` and the one sentence the rest of `crate::staff` answers — so a person
//! at a login form cannot tell "you are not in the staff group" from "your
//! token was for another client" from "the identity provider is down". That
//! is the same property the password and TOTP legs keep and for the same
//! reason: a caller who could tell the refusals apart is holding an oracle
//! over which step of an attack worked. The operator gets the difference from
//! [`SsoError::stage`], which reaches the `Display` of the `ApiError` and so
//! the log.
//!
//! **No variant carries a token, a code, a secret, a claim value or a body.**
//! The reasons are `&'static str` chosen by the check that refused, never by
//! anything the identity provider or the caller sent.

use vpay_core::{Category, Classify};

/// A refusal of one SSO step.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SsoError {
    /// Discovery, the token endpoint or the key set could not be reached or
    /// answered garbage. Not the person's fault; still one `401` on the wire,
    /// because the alternative is a `503` that says "SSO is configured and
    /// down" to an unauthenticated caller.
    #[error("the identity provider could not be reached or answered unusably: {0}")]
    Idp(&'static str),

    /// The discovery document broke a rule this deployment holds it to.
    #[error("the identity provider's discovery document is refused: {0}")]
    Discovery(&'static str),

    /// The callback named a flow that is absent, already taken, or expired.
    #[error("no such SSO flow in flight")]
    FlowAbsent,

    /// The `state` the browser came back with is not the one this flow sent.
    #[error("the SSO state does not match")]
    StateMismatch,

    /// The identity provider refused the authorization code.
    #[error("the identity provider's token endpoint refused the code")]
    CodeRefused,

    /// The token response held no `id_token`.
    #[error("the token response carried no id_token")]
    NoIdToken,

    /// The id_token's header is unacceptable: an algorithm outside the
    /// allow-list (this includes `none` and every HMAC), or no `kid`.
    #[error("the id_token header is refused: {0}")]
    Header(&'static str),

    /// The issuer publishes no signing key under the token's `kid`.
    #[error("the issuer publishes no signing key for the id_token's kid")]
    UnknownKey,

    /// The signature or a registered claim failed: signature, `iss`, `aud`,
    /// `azp`, `exp`, `iat`, or a missing required claim.
    #[error("the id_token is refused: {0}")]
    Token(&'static str),

    /// The id_token's `nonce` is not this flow's.
    #[error("the id_token's nonce does not match")]
    Nonce,

    /// The person is not in `staff_auth.sso.required_group`.
    #[error("the person is not in the required group")]
    NotInGroup,

    /// The token does not prove a second factor, and the flow was already the
    /// one `prompt=login` retry.
    #[error("the id_token does not prove a second factor, after the one retry")]
    NoSecondFactor,

    /// The linked staff member is disabled.
    #[error("the linked staff member is disabled")]
    Disabled,

    /// The link names a staff member that no longer exists.
    #[error("the federated link names a staff member that is gone")]
    LinkOrphaned,

    /// A first sign-in could not be provisioned: the token carries no usable
    /// email address.
    #[error("the id_token carries no usable email address to provision a member with")]
    NoEmail,

    /// A first sign-in's address already belongs to a staff member who is not
    /// linked to this identity. **Not linked by email**
    /// (ADR-0019 decision 5): refused.
    #[error("the email address belongs to a staff member not linked to this identity")]
    EmailTaken,
}

impl SsoError {
    /// The `stage` the `ApiError` carries into the log.
    ///
    /// A distinct literal per variant, prefixed `sso ` so a log search for the
    /// federated path is one grep.
    pub(crate) fn stage(&self) -> &'static str {
        match self {
            Self::Idp(_) => "sso identity provider unavailable",
            Self::Discovery(_) => "sso discovery document refused",
            Self::FlowAbsent => "sso flow absent, expired or already used",
            Self::StateMismatch => "sso state mismatch",
            Self::CodeRefused => "sso authorization code refused by the provider",
            Self::NoIdToken => "sso token response without an id_token",
            Self::Header(_) => "sso id_token header refused",
            Self::UnknownKey => "sso id_token signed by an unpublished key",
            Self::Token(_) => "sso id_token refused",
            Self::Nonce => "sso nonce mismatch",
            Self::NotInGroup => "sso person not in the required group",
            Self::NoSecondFactor => "sso id_token proves no second factor",
            Self::Disabled => "sso linked staff member disabled",
            Self::LinkOrphaned => "sso link names a missing staff member",
            Self::NoEmail => "sso first sign-in without a usable email",
            Self::EmailTaken => "sso email belongs to an unlinked staff member",
        }
    }
}

impl Classify for SsoError {
    /// `Authentication` for everything the person or their token did,
    /// `Rail`-like unavailability for the provider being down is deliberately
    /// **not** distinguished: the wire answer is uniform, and the category
    /// only decides log severity.
    fn category(&self) -> Category {
        match self {
            Self::Idp(_) | Self::Discovery(_) => Category::Configuration,
            _ => Category::Authentication,
        }
    }
}

impl From<SsoError> for crate::ApiError {
    /// Logs the precise reason, then answers the module's one refusal.
    ///
    /// `warn`, and the `Display` of the error and nothing else: see this
    /// module's header for what the `Display` can and cannot contain.
    fn from(error: SsoError) -> Self {
        tracing::warn!(reason = %error, "an SSO sign-in step was refused");
        crate::staff::refused(error.stage())
    }
}
