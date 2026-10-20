//! Staff single sign-on: the three routes that let a person authenticate to
//! `/dash/v1` through the deployment's OpenID Connect provider instead of a
//! password and a TOTP code
//! ([ADR-0030](../../../../../../docs/adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
//!
//! # Broker, not pass-through
//!
//! vpay stays the OpenID Provider for `/dash/v1`. **Only the human
//! authentication step federates**; the invariant "no `/dash/v1` request runs
//! without a token vpay itself issued" holds unchanged, because a successful
//! callback ends exactly where a successful TOTP step ends — an
//! `authenticated` `staff_sessions` row — and the dashboard then runs the
//! existing authorization-code leg against it. The provider's tokens are
//! consumed in this module and go no further: the id_token is verified and its
//! claims read, the access token is never looked at, and neither is stored,
//! logged or sent to a browser.
//!
//! ```text
//! browser      dashboard app            vpay                    provider
//!   |  POST /api/auth/sso/start         |                           |
//!   |------------->|  POST /staff/sso/start                         |
//!   |              |-------------->|  state, nonce, PKCE verifier   |
//!   |              |               |  -> staff_sso_attempts row     |
//!   |              |  { session, authorization_url }                |
//!   |  302 ------------------------------------------------------->|
//!   |  (login forms, second factor)                                 |
//!   |<------------------------------------------ 302 ?code&state --|
//!   |  GET /api/auth/sso/callback       |                           |
//!   |------------->|  POST /staff/sso/callback { code, state }      |
//!   |              |-------------->|  POST token endpoint -------->|
//!   |              |               |<-------------- id_token ------|
//!   |              |               |  verify, group, 2nd factor,   |
//!   |              |               |  link or provision            |
//!   |              |  { outcome: signed_in, session }              |
//! ```
//!
//! # What the callback decides, in this order, and why this order
//!
//! 1. the flow exists, is unexpired and is **taken exactly once**, and the
//!    `state` is its own (constant time);
//! 2. the code redeems, and the id_token verifies (signature, `iss`, `aud`,
//!    `azp`, `exp`, `iat`) and carries this flow's nonce;
//! 3. the person is in `required_group`;
//! 4. if a link for `(issuer, sub)` exists, its member is **active**;
//! 5. the token proves a second factor — and *only here*, after every check
//!    above has passed, a miss starts the one `prompt=login` retry, so a bad
//!    nonce or a missing group is refused immediately and never earns the
//!    caller a second pass through the provider;
//! 6. the link is used, or — the first time — the member is provisioned,
//!    atomically with the link;
//! 7. a **fresh** session token is minted, already `authenticated`.
//!
//! # The linking rule, and what a token may never touch
//!
//! A member is found by `(issuer, sub)` and by nothing else
//! (ADR-0019 decision 5). The `email` claim is display data: it is written once,
//! when a member is provisioned, and matched on never — an address that already
//! belongs to a member not linked to this identity is a refusal, not a link.
//! No claim reads or writes `is_admin` or `merchant_id`
//! (ADR-0019 § R2): a provisioned member is a non-admin bound to
//! `dashboard_client.merchant_id`, and roles stay vpay's.

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::net::SocketAddr;
use subtle::ConstantTimeEq as _;
use time::OffsetDateTime;
use vpay_db::{
    CredentialKind, NewCredential, NewSession, NewSsoAttempt, NewStaff, Repositories, SessionState,
    StaffRow, StaffSessions, StaffSsoAttempts, TxOutcome, UnitOfWork as _,
};

use super::{StaffLogin, caller_address, rate_limit, session_token};
use crate::ApiError;
use crate::staff_auth::tokens;

mod error;
mod provider;
mod verify;

pub use provider::SsoProvider;

pub(crate) use error::SsoError;

/// The longest `code` or `state` the callback will read.
///
/// An authorization code and a `state` this deployment minted are a few dozen
/// characters; the bound is what stops an unauthenticated caller handing the
/// hashing and the token request an arbitrary body.
const MAX_CALLBACK_PARAM_CHARS: usize = 2048;

/// The longest display name written to `staff_members.display_name`
/// (`staff_members_display_name_length`).
const MAX_DISPLAY_NAME_CHARS: usize = 256;

/// The longest address `staff_members.email` takes.
const MAX_EMAIL_CHARS: usize = 512;

/// What `GET /dash/v1/staff/sso` answers.
///
/// A route rather than a build-time constant in the dashboard because whether
/// SSO is configured is the deployment's property, not the app's: one
/// dashboard image runs against sandbox and production
/// ([ADR-0003](../../../../../../docs/adr/0003-yaml-configuration.md)).
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SsoStatusResponse {
    /// Whether `staff_auth.sso` is configured.
    pub enabled: bool,
}

/// What `POST /dash/v1/staff/sso/start` answers.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SsoStartResponse {
    /// The pending-flow token. **Not a session**: `GET /staff/session`
    /// refuses it. It identifies this browser's flow to the callback and
    /// nothing else, and the dashboard keeps it in the same httpOnly cookie it
    /// keeps sessions in.
    pub session: String,
    /// Where to send the browser.
    pub authorization_url: String,
}

/// What `POST /dash/v1/staff/sso/callback` takes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SsoCallbackRequest {
    /// The authorization code, as the provider redirected with it.
    pub code: String,
    /// The `state`, as the provider redirected with it.
    pub state: String,
}

/// What `POST /dash/v1/staff/sso/callback` answers.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SsoCallbackResponse {
    /// Signed in. `session` is a **new** token for an `authenticated`
    /// session, not the pending-flow token the request carried: the token
    /// that identified an unauthenticated browser's flow never becomes the
    /// credential of an authenticated one.
    SignedIn {
        /// The session token, returned once.
        session: String,
    },
    /// The token's only defect was a missing second factor, and this flow was
    /// not already the retry: send the browser back to the provider with
    /// `prompt=login`, under a new pending-flow token.
    Reauthenticate {
        /// The new pending-flow token.
        session: String,
        /// Where to send the browser.
        authorization_url: String,
    },
}

impl StaffLogin {
    /// The SSO provider, or the honest `404` of an unconfigured route.
    pub(crate) fn sso(&self) -> Result<&SsoProvider, ApiError> {
        self.sso.as_deref().ok_or(ApiError::NotFound {
            resource: "route",
            id: "/dash/v1/staff/sso".to_owned(),
        })
    }
}

/// Whether SSO is configured here. Unauthenticated, cheap and database-free.
///
/// # Errors
///
/// [`ApiError::NotFound`] if this deployment mounts no staff login at all.
pub(crate) async fn status(
    State(state): State<crate::AppState>,
) -> Result<Json<SsoStatusResponse>, ApiError> {
    let login = state.staff_login()?;
    Ok(Json(SsoStatusResponse {
        enabled: login.sso.is_some(),
    }))
}

/// Begins a sign-in: mints the flow and answers where to send the browser.
///
/// Behind the sign-in rate limiter, per address: the one unauthenticated route
/// here that writes a row, and the one that starts every outbound request to
/// the provider. The callback needs no budget of its own — it cannot be
/// reached without a pending-flow token this route minted (256 bits, taken
/// exactly once), so its volume is bounded by this route's.
///
/// # Errors
///
/// [`ApiError::NotFound`] when SSO is not configured;
/// [`ApiError::StaffSignInRateLimited`] over the budget;
/// [`ApiError::StaffSignInRefused`] if the provider cannot be discovered;
/// [`ApiError::Db`] if Postgres fails.
pub(crate) async fn start(
    State(state): State<crate::AppState>,
    peer: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
) -> Result<Json<SsoStartResponse>, ApiError> {
    let login = state.staff_login()?;
    let sso = login.sso()?;
    let repositories = state.repositories();
    let now = OffsetDateTime::now_utc();

    let address = caller_address(login, &headers, peer);
    if login
        .limiter
        .check_sso_start(repositories, address, now)
        .await?
        == rate_limit::Verdict::Limited
    {
        tracing::warn!("an SSO start was refused by the rate limiter");
        return Err(ApiError::StaffSignInRateLimited);
    }

    // Opportunistic, because this route is unauthenticated and writes a row:
    // see `StaffSsoAttempts::sweep_expired_sso_attempts`.
    repositories.sweep_expired_sso_attempts(now).await?;

    let (session, authorization_url) = begin_flow(login, sso, repositories, now, false).await?;
    Ok(Json(SsoStartResponse {
        session,
        authorization_url,
    }))
}

/// Completes a sign-in: see this module's header for the order.
///
/// # Errors
///
/// [`ApiError::NotFound`] when SSO is not configured;
/// [`ApiError::StaffSignInRefused`] for every refusal — one `401`, the reason
/// in the log; [`ApiError::Db`] if Postgres fails.
pub(crate) async fn callback(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    axum::Form(request): axum::Form<SsoCallbackRequest>,
) -> Result<Json<SsoCallbackResponse>, ApiError> {
    let login = state.staff_login()?;
    let sso = login.sso()?;
    let repositories = state.repositories();
    let now = OffsetDateTime::now_utc();

    if request.code.is_empty()
        || request.state.is_empty()
        || request.code.chars().count() > MAX_CALLBACK_PARAM_CHARS
        || request.state.chars().count() > MAX_CALLBACK_PARAM_CHARS
    {
        return Err(SsoError::FlowAbsent.into());
    }

    // 1. TAKE THE FLOW, once. Everything after this line burns it: a wrong
    // `state`, a refused code and a bad token all leave the flow gone, so the
    // pending-flow token is a one-shot credential and there is nothing to
    // brute-force across attempts.
    let token = session_token(&headers).ok_or(SsoError::FlowAbsent)?;
    let attempt = StaffSsoAttempts::consume(repositories, &tokens::digest(&token), now)
        .await?
        .ok_or(SsoError::FlowAbsent)?;

    let presented_state = tokens::digest(&request.state);
    if !bool::from(
        presented_state
            .as_bytes()
            .ct_eq(attempt.state_hash.as_bytes()),
    ) {
        return Err(SsoError::StateMismatch.into());
    }

    // 2. REDEEM, AND VERIFY.
    let verifier = login
        .credentials
        .open_secret(&attempt.code_verifier)
        .map_err(ApiError::from)?;
    let verifier = String::from_utf8(verifier)
        .map_err(|_| ApiError::Internal("a stored SSO code verifier is not UTF-8".to_owned()))?;
    let id_token = sso.exchange_code(&request.code, &verifier).await?;
    let claims = sso.verify_id_token(&id_token).await?;
    if !claims.nonce_matches(&attempt.nonce_hash) {
        return Err(SsoError::Nonce.into());
    }

    // 3. THE GROUP GATE.
    if !claims.in_group(&sso.config().required_group) {
        return Err(SsoError::NotInGroup.into());
    }

    // 4. AN EXISTING LINK, and whether its member may sign in at all. Before
    // the second-factor decision on purpose: a disabled member is refused now
    // rather than sent round the provider's login first.
    let issuer = sso.issuer();
    let linked = repositories.find_federated(issuer, &claims.subject).await?;
    let existing = match &linked {
        Some(credential) => {
            let staff_id = credential
                .staff_member_id
                .as_deref()
                .ok_or(SsoError::LinkOrphaned)?;
            let staff = repositories
                .find(staff_id)
                .await?
                .ok_or(SsoError::LinkOrphaned)?;
            if !staff.is_active() {
                return Err(SsoError::Disabled.into());
            }
            Some(staff)
        }
        None => None,
    };

    // 5. THE SECOND FACTOR — and the one retry.
    let config = sso.config();
    if !claims.proves_second_factor(&config.accepted_acr, &config.accepted_amr) {
        let (acr, amr) = claims.authentication_methods();
        tracing::info!(
            acr = acr.unwrap_or("-"),
            ?amr,
            is_retry = attempt.is_retry,
            "an SSO id_token proves no second factor under the configured accepted_acr / accepted_amr"
        );
        if attempt.is_retry {
            return Err(SsoError::NoSecondFactor.into());
        }
        tracing::info!(
            "the id_token was otherwise valid; re-running the provider's login once with \
             prompt=login"
        );
        let (session, authorization_url) = begin_flow(login, sso, repositories, now, true).await?;
        return Ok(Json(SsoCallbackResponse::Reauthenticate {
            session,
            authorization_url,
        }));
    }

    // 6. LINK, OR PROVISION.
    let staff = match existing {
        Some(staff) => staff,
        None => provision(sso, repositories, &claims, now).await?,
    };

    // 7. A FRESH SESSION, already authenticated.
    let session = tokens::mint();
    StaffSessions::create(
        repositories,
        NewSession {
            id: session.digest.clone(),
            staff_id: staff.id.clone(),
            state: SessionState::Authenticated,
            now,
        },
    )
    .await?;
    repositories.record_sign_in(&staff.id, now).await?;

    tracing::info!(staff_id = %staff.id, "a staff member signed in through SSO");
    Ok(Json(SsoCallbackResponse::SignedIn {
        session: session.token,
    }))
}

/// Mints one flow — a pending-flow token, a `state`, a nonce and a PKCE
/// verifier — records its secrets, and answers the token and the URL.
///
/// The URL is built **before** the row is written so that a provider that
/// cannot be discovered leaves nothing behind.
async fn begin_flow(
    login: &StaffLogin,
    sso: &SsoProvider,
    repositories: &dyn Repositories,
    now: OffsetDateTime,
    is_retry: bool,
) -> Result<(String, String), ApiError> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    // Four independent 256-bit values. `tokens::mint` is the one CSPRNG path
    // in this crate; its base64url output is also a legal PKCE verifier
    // (RFC 7636 §4.1: 43–128 unreserved characters, and this is 43).
    let pending = tokens::mint();
    let state = tokens::mint().token;
    let nonce = tokens::mint().token;
    let verifier = tokens::mint().token;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let authorization_url = sso
        .authorization_url(&state, &nonce, &challenge, is_retry)
        .await?;

    StaffSsoAttempts::create(
        repositories,
        NewSsoAttempt {
            id: pending.digest.clone(),
            state_hash: tokens::digest(&state),
            nonce_hash: tokens::digest(&nonce),
            code_verifier: login.credentials.seal_secret(verifier.as_bytes())?,
            is_retry,
            now,
        },
    )
    .await?;

    Ok((pending.token, authorization_url))
}

/// Provisions the member a first sign-in names, atomically with its link.
///
/// # What a unique violation means, and which of two it is
///
/// `staff_members_email_key` firing means the address belongs to a member
/// this identity is **not** linked to: refused, and logged, never linked
/// (decision 5). `credentials_federated_identity_key` firing means another
/// request provisioned this very identity a moment ago and won the race; the
/// link now exists, so this request signs in as that member. They are told
/// apart by asking again — the loser of the first race finds a link, the
/// loser of the second does not.
async fn provision(
    sso: &SsoProvider,
    repositories: &dyn Repositories,
    claims: &verify::IdentityClaims,
    now: OffsetDateTime,
) -> Result<StaffRow, ApiError> {
    let email = claims
        .email
        .as_deref()
        .and_then(normalise_email)
        .ok_or(SsoError::NoEmail)?;
    let display_name = display_name(claims, &email);

    let staff_id = vpay_core::ids::staff_id();
    let new_staff = NewStaff {
        id: staff_id.clone(),
        // From CONFIGURATION, never a claim (ADR-0019 § R2).
        merchant_id: sso.merchant_id().to_owned(),
        email,
        display_name,
        // Never a claim (ADR-0019 § R2): roles stay vpay's.
        is_admin: false,
        now,
    };
    let credential = NewCredential {
        id: vpay_core::ids::credential_id(),
        staff_member_id: Some(staff_id.clone()),
        kind: CredentialKind::Oidc,
        material: None,
        issuer: Some(sso.issuer().to_owned()),
        subject: Some(claims.subject.clone()),
        must_change: false,
        expires_at: None,
        now,
    };

    let outcome = repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.provision_federated_staff_in_tx(&new_staff, credential)
                    .await?;
                Ok::<_, ApiError>(TxOutcome::Commit(()))
            })
        })
        .await;

    match outcome {
        Ok(_) => {
            tracing::info!(
                staff_id = %staff_id,
                "a staff member was provisioned by their first SSO sign-in"
            );
        }
        Err(ApiError::Db(vpay_db::DbError::Persistence(vpay_db::PersistenceError::Unique {
            ..
        }))) => {
            // Which unique index fired? Ask whether the identity is linked now.
            if repositories
                .find_federated(sso.issuer(), &claims.subject)
                .await?
                .is_none()
            {
                return Err(SsoError::EmailTaken.into());
            }
        }
        Err(error) => return Err(error),
    }

    // Re-read through the link rather than trusting either branch above:
    // whichever request won, this is the member the identity names.
    let credential = repositories
        .find_federated(sso.issuer(), &claims.subject)
        .await?
        .ok_or(SsoError::LinkOrphaned)?;
    let staff_id = credential.staff_member_id.ok_or(SsoError::LinkOrphaned)?;
    let staff = repositories
        .find(&staff_id)
        .await?
        .ok_or(SsoError::LinkOrphaned)?;
    if !staff.is_active() {
        return Err(SsoError::Disabled.into());
    }
    Ok(staff)
}

/// An address fit for `staff_members.email`, or `None`.
///
/// Lower-cased and trimmed (the column's CHECK refuses anything else), within
/// the column's bounds, with an `@` that is neither first nor last and no
/// whitespace or control character. **Deliberately not an RFC 5322 parser**:
/// the address is display data the provider vouches for, and what this must
/// not do is write something the unique index or a later render chokes on.
fn normalise_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    let length = email.chars().count();
    if !(3..=MAX_EMAIL_CHARS).contains(&length) {
        return None;
    }
    if email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let at = email.find('@')?;
    (at > 0 && at < email.len().saturating_sub(1)).then_some(email)
}

/// What the dashboard greets a provisioned member by: the provider's `name`
/// (or `preferred_username`), else the local part of the address, bounded to
/// the column.
fn display_name(claims: &verify::IdentityClaims, email: &str) -> String {
    let chosen = claims
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or_else(|| email.split('@').next().filter(|local| !local.is_empty()))
        .unwrap_or(email);
    chosen.chars().take(MAX_DISPLAY_NAME_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_normalised_or_refused() {
        assert_eq!(
            normalise_email("  Ada@Example.TEST "),
            Some("ada@example.test".to_owned())
        );
        for bad in ["", "a", "ab", "@x", "x@", "no-at-sign", "a b@c.d", "a@b\nc"] {
            assert_eq!(normalise_email(bad), None, "{bad:?}");
        }
        assert_eq!(
            normalise_email(&format!("{}@x.test", "a".repeat(600))),
            None
        );
    }
}
