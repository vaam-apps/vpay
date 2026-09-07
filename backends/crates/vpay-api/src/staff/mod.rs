//! Staff sign-in: the routes that turn a person at a login form into an
//! `Identity` the authorization-code grant can issue for
//! ([ADR-0017](../../../../../docs/adr/0017-staff-authentication.md)).
//!
//! # These routes are NOT behind the bearer-token middleware
//!
//! They cannot be: they exist to produce the credential that middleware
//! checks. `crate::router` mounts them beside the protected `/dash/v1` reads
//! rather than inside `Router::layer`, and what authorises each of them is
//! the *session*, which is this module's own concern.
//!
//! # The session credential, and where the cookie is
//!
//! Every route below except [`login`] takes the session token in the
//! [`SESSION_HEADER`] header. The **cookie** ADR-0017 decision 2 describes —
//! httpOnly, Secure, SameSite=Lax — is set by the dashboard app on its own
//! origin, and the app sends the header server-side. vpay never sets a cookie
//! and never reads one.
//!
//! That is decision 4, and it is a deviation from the shape a reader would
//! assume: in a browser-driven flow the user agent follows the `/authorize`
//! redirect and a callback *page* completes the exchange. Here the dashboard
//! app's own server follows it. Everything the grant checks is unchanged —
//! the redirect URI is matched byte for byte, PKCE is mandatory, the code is
//! single-use — and what changes is only who follows the `302`.
//!
//! # One answer for every refusal
//!
//! No such address, wrong password, disabled account, wrong code, replayed
//! code, expired session, idle session, forged session, session at the wrong
//! stage: all of them are [`crate::ApiError::StaffSignInRefused`], one `401`
//! with one sentence. The `stage` field reaches the log and never the body.
//! A login form that answered differently for "no such account" would be an
//! account-enumeration oracle, and one that answered differently for "the
//! password was right but the code was wrong" would tell an attacker they had
//! half of a credential pair.
//!
//! The **timing** half of the same property is
//! [`crate::staff_auth::StaffCredentials::verify_absent_account`]: an address
//! with no account still costs one argon2id verification, so the two answers
//! take the same time as well as the same shape.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use vpay_db::{NewSession, SessionRow, SessionState, StaffRow, StaffSessions};

use crate::ApiError;
use crate::op::dashboard::DashboardOp;
use crate::staff_auth::{StaffCredentials, tokens, totp};

pub mod oauth;
pub mod rate_limit;

/// Where a caller presents its session token.
///
/// A header and not a cookie, for the reason in this module's header: the
/// cookie lives on the dashboard app's origin and the app is the caller here.
/// `X-Vpay-` prefixed so it cannot collide with anything standard, and named
/// in one place so the app's own constant has something to be checked
/// against.
pub const SESSION_HEADER: &str = "x-vpay-staff-session";

/// Everything the staff routes need, or `None` and they are not mounted.
///
/// One value rather than three pieces of router state, because a deployment
/// has all three or none: the credentials come from `staff_auth`, the grant
/// comes from `dashboard_client`, and a limiter with nothing to limit is
/// pointless. `crate::router` mounts these routes exactly when this is
/// `Some`, so "this deployment serves no staff login" is one `Option` rather
/// than a condition each handler re-derives.
pub struct StaffLogin {
    /// argon2's pepper and the AEAD key.
    pub credentials: Arc<StaffCredentials>,
    /// The authorization-code grant these sessions feed.
    pub dashboard_op: Arc<DashboardOp>,
    /// Per-email and per-IP sign-in limiting, this replica's.
    pub limiter: Arc<rate_limit::SignInLimiter>,
    /// What an `otpauth://` URI names this deployment as, in an
    /// authenticator app's account list.
    ///
    /// `deployment.name` from the validated configuration — the operator's
    /// own label, which is what a staff member holding a sandbox account and
    /// a production account needs to tell the two entries apart on their
    /// phone.
    ///
    /// Here rather than on `ResourceConfig` because it is a *label*, used by
    /// exactly one line of one handler, and widening the request-path
    /// projection for it would put a display string on the type every `/v1`
    /// handler reads.
    pub issuer_label: String,
}

/// Shows nothing but the fact that a login is configured. Both the
/// credentials and the OP redact themselves, and repeating them here would
/// only make a startup log longer.
impl std::fmt::Debug for StaffLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaffLogin").finish_non_exhaustive()
    }
}

/// What `POST /dash/v1/staff/login` takes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LoginRequest {
    /// The address, in any case. Lower-cased here before it reaches either
    /// the rate limiter or the lookup, because the column is the canonical
    /// form and two spellings must be neither two accounts nor two budgets.
    pub email: String,
    /// The password, as typed.
    pub password: String,
}

/// What the next step is, once a password has been accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextStep {
    /// Present a code from an authenticator app already paired with this
    /// account.
    Totp,
    /// Scan [`LoginResponse::otpauth_uri`] and present a code from it. The
    /// first sign-in, and the only one that ever answers this: ADR-0017
    /// decision 1 makes enrolment mandatory.
    TotpEnrolment,
}

/// What `POST /dash/v1/staff/login` answers.
///
/// **Never says whether the address exists.** Every failure is a `401` with
/// one sentence; this shape is only ever reached after a password was
/// accepted.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LoginResponse {
    /// The opaque session token. Returned **once**; only its SHA-256 is
    /// stored.
    pub session: String,
    /// What to do next.
    pub next: NextStep,
    /// The `otpauth://` URI to render as a QR code, on
    /// [`NextStep::TotpEnrolment`] only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otpauth_uri: Option<String>,
    /// The **sealed** TOTP secret, on [`NextStep::TotpEnrolment`] only, to be
    /// handed back to [`totp_step`] with the first code.
    ///
    /// # Why the caller holds this between two requests
    ///
    /// Enrolment cannot be committed at this step: a secret written before
    /// the person has proved they can generate a code from it locks out
    /// anybody whose scan failed, permanently, because
    /// `Staff::enrol_totp`'s guard is `totp_enrolled_at IS NULL`. It also
    /// cannot live in the session row, which has no column for it — and
    /// giving it one would mean a table of unconfirmed secrets.
    ///
    /// So it travels sealed. The value is AES-256-GCM under a deployment key
    /// the caller does not have, so the caller learns nothing from holding
    /// it, and replaying it buys nothing: `enrol_totp` is a compare-and-swap,
    /// so only the first completion wins and every later one is a no-op.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrolment: Option<String>,
}

/// What `POST /dash/v1/staff/totp` takes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TotpRequest {
    /// Six digits from the authenticator app.
    pub code: String,
    /// The sealed secret from [`LoginResponse::enrolment`], on the enrolment
    /// path only. Absent for an account that is already enrolled — and
    /// **ignored** if one is sent for such an account, because the stored
    /// secret is the only one that may authenticate it.
    #[serde(default)]
    pub enrolment: Option<String>,
}

/// What `POST /dash/v1/staff/totp` answers.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TotpResponse {
    /// Whether the one-time password the operator printed must still be
    /// replaced before this session may reach `/authorize`.
    pub password_change_required: bool,
}

/// What `POST /dash/v1/staff/password` takes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PasswordRequest {
    /// The new password. No "old password" field: the session making the
    /// change has already presented both factors, and re-checking a password
    /// this endpoint then overwrites would be a second copy of that check in
    /// the wrong layer.
    pub new_password: String,
}

/// The shortest password a staff member may choose.
///
/// Twelve. Not a composition rule ("one digit, one symbol"), which is
/// measurably worse: it shrinks the space people actually choose from and is
/// the reason `Password1!` exists. Length is the only rule, and the
/// generated one-time password this replaces is 130 bits.
const MIN_PASSWORD_CHARS: usize = 12;

/// The longest. A bound rather than a policy: argon2id hashes any length, and
/// this is what stops an unauthenticated caller making the process do
/// arbitrary work — the same reason every other body in this crate is
/// bounded.
const MAX_PASSWORD_CHARS: usize = 256;

/// What `GET /dash/v1/staff/session` answers.
///
/// The dashboard app calls this on every render: it is how the app learns who
/// is signed in, which tenant they may see, and — the part that matters —
/// **the access token to present to `/dash/v1`**. Reading it back from the
/// session row on each request rather than caching it in the app is what
/// makes deleting the row a revocation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionResponse {
    /// `stf_…`.
    pub staff_id: String,
    /// What to greet them by.
    pub display_name: String,
    /// The address they signed in with.
    pub email: String,
    /// The one tenant this session may see.
    pub merchant_id: String,
    /// Whether the one-time password still has to be replaced.
    pub password_change_required: bool,
    /// The `/dash/v1` access token, once the code exchange has minted one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
}

/// The routes this module mounts, relative to `crate::dash::DASH_NEST`.
///
/// Listed here rather than built inline for `crate::dash::DASH_ROUTES`'
/// reason: axum 0.8 cannot enumerate a built `Router`, so a test that walks
/// every unauthenticated path has to walk a table.
pub const STAFF_ROUTES: &[(&str, &[&str])] = &[
    ("/staff/login", &["POST"]),
    ("/staff/totp", &["POST"]),
    ("/staff/password", &["POST"]),
    ("/staff/session", &["GET"]),
    ("/staff/logout", &["POST"]),
    ("/oauth/authorize", &["GET"]),
    ("/oauth/token", &["POST"]),
];

/// The unauthenticated half of `/dash/v1`.
///
/// **No `.fallback`**, deliberately: `crate::dash::routes` carries the one
/// this nest has, and two routers with fallbacks cannot be merged. It is also
/// what keeps an unmatched `/dash/v1/...` path answering exactly what it
/// answered before this module existed.
pub(crate) fn routes() -> Router<crate::AppState> {
    Router::new()
        .route("/staff/login", post(login))
        .route("/staff/totp", post(totp_step))
        .route("/staff/password", post(change_password))
        .route("/staff/session", get(session))
        .route("/staff/logout", post(logout))
        .route("/oauth/authorize", get(oauth::authorize))
        .route("/oauth/token", post(oauth::token))
}

/// Step 1: the password.
///
/// Refuses **before** any credential work when over the rate limit, which is
/// the point of the limiter: an attempt over budget must not cost an argon2id
/// verification, or the limit would be the amplifier.
///
/// # Errors
///
/// [`ApiError::StaffSignInRateLimited`] over the budget;
/// [`ApiError::StaffSignInRefused`] for every credential failure, including
/// an address with no account; [`ApiError::Db`] if Postgres fails.
pub(crate) async fn login(
    State(state): State<crate::AppState>,
    peer: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    Form(request): Form<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let login = state.staff_login()?;
    let repositories = state.repositories();
    let now = OffsetDateTime::now_utc();

    // Lower-cased once, here, and used for both the budget and the lookup.
    let email = request.email.trim().to_lowercase();

    let peer_ip = peer.map(|axum::Extension(ConnectInfo(peer))| peer.ip());
    if login.limiter.check(&email, peer_ip, now) == rate_limit::Verdict::Limited {
        tracing::warn!("a staff sign-in attempt was refused by the rate limiter");
        return Err(ApiError::StaffSignInRateLimited);
    }

    let staff = repositories.find_by_email(&email).await?;

    // The two branches do the same work. `verify_absent_account` runs a real
    // argon2id verification against a dummy hash, so "no such address" costs
    // what "wrong password" costs — see `crate::staff_auth::password`.
    let Some(staff) = staff else {
        login.credentials.verify_absent_account(&request.password)?;
        return Err(refused("no such staff account"));
    };
    if !login
        .credentials
        .verify_password(&request.password, &staff.password_hash)?
    {
        return Err(refused("password"));
    }
    // Checked *after* the password, so a disabled account is not an oracle
    // for "this address exists": an attacker who does not know the password
    // gets the same answer either way.
    if !staff.is_active() {
        return Err(refused("disabled account"));
    }

    let session = tokens::mint();
    StaffSessions::create(
        repositories,
        NewSession {
            id: session.digest.clone(),
            staff_id: staff.id.clone(),
            state: SessionState::PendingTotp,
            now,
        },
    )
    .await?;

    tracing::info!(
        staff_id = %staff.id,
        enrolled = staff.is_totp_enrolled(),
        "a staff password was accepted; the session is pending its second factor"
    );

    if staff.is_totp_enrolled() {
        return Ok(Json(LoginResponse {
            session: session.token,
            next: NextStep::Totp,
            otpauth_uri: None,
            enrolment: None,
        }));
    }

    // First sign-in: mint a secret, seal it, and hand back both the URI to
    // scan and the sealed form to return with the first code. Nothing is
    // written to `staff_members` until that code verifies — see
    // `LoginResponse::enrolment`.
    let secret = tokens::totp_secret();
    let sealed = login.credentials.seal_secret(&secret)?;
    Ok(Json(LoginResponse {
        session: session.token,
        next: NextStep::TotpEnrolment,
        otpauth_uri: Some(otpauth_uri(&login.issuer_label, &staff.email, &secret)),
        enrolment: Some(sealed),
    }))
}

/// Step 2: the code, and — on a first sign-in — the enrolment it completes.
///
/// # Rate limited, and it was not until the exp28 review
///
/// [ADR-0017](../../../../docs/adr/0017-staff-authentication.md) decision 2
/// says "**sign-in** is rate limited per email and per IP … failing closed
/// with `429`", and the limiter was wired to [`login`] alone. A second factor
/// is **six digits**, [`crate::staff_auth::totp::Totp::verify`] accepts a
/// one-step skew either side (three live codes at any instant), and nothing
/// here costs an argon2id verification — so a caller holding one password and
/// one `pending_totp` session could guess the second factor at whatever rate
/// the network allowed. Measured on the real stack before this check existed:
/// thirty consecutive wrong codes, thirty `401`s, no `429` (exp28 review,
/// finding F3).
///
/// The key is `staff.email` — the row's, which migration `0035` constrains to
/// lower case, and [`login`] lower-cases the *request's* address before using
/// it as the same key. So the two legs of one sign-in share one budget, which
/// is what "per email" has to mean if it is to bound guessing at an account
/// rather than at an endpoint.
///
/// Checked **after** the session loads, because the email is the session's
/// and not the caller's to name; a caller with no usable session is already
/// refused above by [`load_session`], having spent one row read.
///
/// # Errors
///
/// [`ApiError::StaffSignInRateLimited`] over the budget;
/// [`ApiError::StaffSignInRefused`] for a session at the wrong stage, an
/// expired or idle session, a disabled account, a wrong code, and a
/// **replayed** one; [`ApiError::Db`] if Postgres fails.
pub(crate) async fn totp_step(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    peer: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    Form(request): Form<TotpRequest>,
) -> Result<Json<TotpResponse>, ApiError> {
    let login = state.staff_login()?;
    let repositories = state.repositories();
    let now = OffsetDateTime::now_utc();

    let (session, staff) = load_session(&state, &headers, now).await?;
    if session.state != SessionState::PendingTotp {
        // A session that has already presented a code must not present
        // another: the only caller that would try is one replaying the step.
        return Err(refused("session is not pending a second factor"));
    }

    // The same budget the password leg spends from, keyed on the same
    // lower-cased address — see this function's header. Without it a six-digit
    // second factor is guessable at line rate by anyone holding one password.
    let peer_ip = peer.map(|axum::Extension(ConnectInfo(peer))| peer.ip());
    if login.limiter.check(&staff.email, peer_ip, now) == rate_limit::Verdict::Limited {
        tracing::warn!("a staff second-factor attempt was refused by the rate limiter");
        return Err(ApiError::StaffSignInRateLimited);
    }

    // Which secret authenticates this account: the stored one if enrolled,
    // otherwise the sealed one the caller was handed at login. A caller that
    // sends an `enrolment` for an *enrolled* account gets the stored secret
    // used anyway — the stored one is the only one that may ever
    // authenticate, and honouring a caller-supplied secret here would be a
    // second-factor bypass with no authentication in front of it.
    let enrolling = !staff.is_totp_enrolled();
    let sealed = if enrolling {
        request
            .enrolment
            .clone()
            .ok_or_else(|| refused("enrolment secret absent on a first sign-in"))?
    } else {
        staff
            .totp_secret
            .clone()
            .ok_or_else(|| refused("enrolled account with no stored secret"))?
    };

    let secret = login.credentials.open_secret(&sealed).map_err(|error| {
        // On the enrolment path this is a caller-supplied blob, so a failure
        // to open it is an ordinary refusal rather than a page. On the
        // stored path it is this deployment's own data and the error is
        // real.
        if enrolling {
            tracing::warn!("a staff enrolment blob did not open; refusing");
            refused("enrolment secret did not open")
        } else {
            ApiError::from(error)
        }
    })?;

    let totp = totp::Totp::new(secret);
    let Some(step) = totp.verify(request.code.trim(), now.unix_timestamp()) else {
        return Err(refused("totp code"));
    };

    if enrolling {
        // The compare-and-swap that makes enrolment happen once. `false`
        // means somebody else enrolled this account between the login and
        // this request, and the honest answer is to refuse rather than to
        // sign in against a secret that is not the stored one.
        if !repositories.enrol_totp(&staff.id, &sealed, now).await? {
            return Err(refused("account was enrolled concurrently"));
        }
        tracing::info!(staff_id = %staff.id, "a staff member completed TOTP enrolment");
    }

    // THE REPLAY GUARD. `false` means this step has already been accepted,
    // which is exactly what presenting the same six digits twice looks like.
    // It runs for the enrolment path too: the code that completed enrolment
    // is spent by it.
    if !repositories.record_totp_step(&staff.id, step, now).await? {
        tracing::warn!(
            staff_id = %staff.id,
            "a staff TOTP code was presented for a step that was already accepted; refusing"
        );
        return Err(refused("totp replay"));
    }

    if !repositories.mark_authenticated(&session.id, now).await? {
        return Err(refused("session was not pending a second factor"));
    }
    repositories.record_sign_in(&staff.id, now).await?;

    tracing::info!(staff_id = %staff.id, "a staff member signed in");
    Ok(Json(TotpResponse {
        password_change_required: staff.password_change_required,
    }))
}

/// Replaces the one-time password the operator printed.
///
/// Reachable only by an **authenticated** session — both factors — so the
/// printed password cannot be replaced by whoever happens to have it without
/// also holding the second factor.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] for a password outside the bounds;
/// [`ApiError::StaffSignInRefused`] for a session that is not authenticated;
/// [`ApiError::Db`] if Postgres fails.
pub(crate) async fn change_password(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Form(request): Form<PasswordRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let login = state.staff_login()?;
    let repositories = state.repositories();
    let now = OffsetDateTime::now_utc();

    let (_, staff) = authenticated_session(&state, &headers, now).await?;

    let length = request.new_password.chars().count();
    if !(MIN_PASSWORD_CHARS..=MAX_PASSWORD_CHARS).contains(&length) {
        return Err(ApiError::InvalidParam {
            param: "new_password".to_owned(),
            message: format!(
                "A password must be between {MIN_PASSWORD_CHARS} and {MAX_PASSWORD_CHARS} \
                 characters."
            ),
        });
    }

    let hash = login.credentials.hash_password(&request.new_password)?;
    if !repositories.set_password(&staff.id, &hash, now).await? {
        return Err(refused("staff row vanished during a password change"));
    }

    tracing::info!(staff_id = %staff.id, "a staff member replaced their password");
    Ok(Json(
        serde_json::json!({ "password_change_required": false }),
    ))
}

/// Who is signed in, and the token to present to `/dash/v1`.
///
/// # Errors
///
/// [`ApiError::StaffSignInRefused`] for any session that is not
/// authenticated; [`ApiError::Db`] if Postgres fails.
pub(crate) async fn session(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, ApiError> {
    let now = OffsetDateTime::now_utc();
    let (session, staff) = authenticated_session(&state, &headers, now).await?;

    // The idle bound moves on an accepted request and nowhere else, which is
    // what makes it *idle* rather than "since login".
    state.repositories().touch(&session.id, now).await?;

    Ok(Json(SessionResponse {
        staff_id: staff.id,
        display_name: staff.display_name,
        email: staff.email,
        merchant_id: staff.merchant_id,
        password_change_required: staff.password_change_required,
        access_token: session.access_token,
    }))
}

/// Signs out: deletes the session row.
///
/// **A revocation, not a cookie clear.** The row carries the access token, so
/// deleting it makes that token unobtainable; and the cascade on
/// `oauth_authorization_codes.session_id` kills any code this session issued
/// and has not exchanged.
///
/// Idempotent, and it answers `200` for a session that was never there: a
/// caller signing out twice, or with a token they made up, has achieved
/// exactly what they asked for, and distinguishing the two would say which
/// session ids exist.
///
/// # Errors
///
/// [`ApiError::Db`] if Postgres fails.
pub(crate) async fn logout(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let Some(token) = session_token(&headers) else {
        return Ok(Json(serde_json::json!({ "signed_out": true })));
    };
    let deleted = StaffSessions::delete(state.repositories(), &tokens::digest(&token)).await?;
    if deleted {
        tracing::info!("a staff session was signed out and its access token revoked with it");
    }
    Ok(Json(serde_json::json!({ "signed_out": true })))
}

/// The session token from [`SESSION_HEADER`], if the caller sent a usable
/// one.
///
/// A non-ASCII header value is `None` rather than lossily decoded: the token
/// is base64url by construction, so anything else was not minted here.
pub(crate) fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(SESSION_HEADER)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

/// Loads the session and its staff row, at any stage.
///
/// Every refusal is one answer — see this module's header. The staff row is
/// re-read on **every** request rather than trusted from the session, which
/// is what makes disabling an account take effect on that person's next
/// request rather than at their next login.
pub(crate) async fn load_session(
    state: &crate::AppState,
    headers: &HeaderMap,
    now: OffsetDateTime,
) -> Result<(SessionRow, StaffRow), ApiError> {
    let token = session_token(headers).ok_or_else(|| refused("no session header"))?;
    let session = state
        .repositories()
        .load(&tokens::digest(&token), now)
        .await?
        .ok_or_else(|| refused("session absent, expired or idle"))?;
    let staff = state
        .repositories()
        .find(&session.staff_id)
        .await?
        .ok_or_else(|| refused("session names a staff row that is gone"))?;
    if !staff.is_active() {
        return Err(refused("disabled account"));
    }
    Ok((session, staff))
}

/// [`load_session`], plus the requirement that both factors were presented.
pub(crate) async fn authenticated_session(
    state: &crate::AppState,
    headers: &HeaderMap,
    now: OffsetDateTime,
) -> Result<(SessionRow, StaffRow), ApiError> {
    let (session, staff) = load_session(state, headers, now).await?;
    if session.state != SessionState::Authenticated {
        return Err(refused("session has not presented a second factor"));
    }
    Ok((session, staff))
}

/// The one refusal this module answers, with the stage for the log.
pub(crate) fn refused(stage: &'static str) -> ApiError {
    ApiError::StaffSignInRefused { stage }
}

/// The `otpauth://totp/...` URI an authenticator app scans (Key Uri Format).
///
/// `issuer` appears twice — once in the label prefix and once as a parameter
/// — because that is what the format specifies and what every app expects;
/// omitting the prefix makes an account show up unlabelled in some of them.
///
/// No `algorithm`, `digits` or `period` parameter, deliberately. All three
/// would be the defaults (SHA1, 6, 30), and a parameter that restates a
/// default is one more thing an app can honour incorrectly —
/// `crate::staff_auth::totp`'s header records that `algorithm=SHA256` is
/// exactly such a case.
fn otpauth_uri(issuer: &str, email: &str, secret: &[u8]) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}",
        urlencode(issuer),
        urlencode(email),
        totp::base32(secret),
        urlencode(issuer)
    )
}

/// Percent-encodes everything that is not unreserved (RFC 3986 §2.3).
///
/// Hand-written for `vpay_core::ids`' reason — it is ten lines and the
/// alternative is a crate in a payment binary's graph — and deliberately
/// conservative: it encodes `:`, `/`, `?`, `&`, `=` and `+`, every one of
/// which would otherwise change the *structure* of the URI above when it
/// appears in an email address or a deployment name.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_otpauth_uri_is_the_key_uri_format() {
        let uri = otpauth_uri("vpay sandbox", "ada@example.test", b"12345678901234567890");

        assert!(uri.starts_with("otpauth://totp/"), "{uri}");
        assert!(
            uri.contains("secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"),
            "the RFC 4648 base32 of the RFC 6238 test secret: {uri}"
        );
        assert!(uri.contains("issuer=vpay%20sandbox"), "{uri}");
        assert!(
            uri.contains("vpay%20sandbox:ada%40example.test"),
            "the label carries the issuer prefix too: {uri}"
        );
        assert!(
            !uri.contains("algorithm=") && !uri.contains("digits=") && !uri.contains("period="),
            "a parameter that restates a default is one more thing an app can honour \
             incorrectly: {uri}"
        );
    }

    /// Every character that would change the URI's structure is encoded. The
    /// decisive case is an address containing `?` or `&`, which would
    /// otherwise inject a parameter an app would read.
    #[test]
    fn the_encoder_cannot_be_used_to_inject_a_parameter() {
        let uri = otpauth_uri("vpay", "a&secret=evil?x=1@example.test", b"secret");
        assert_eq!(
            uri.matches('?').count(),
            1,
            "exactly one query separator: {uri}"
        );
        assert!(!uri.contains("&secret=evil"), "{uri}");
        assert!(uri.contains("%26secret%3Devil%3Fx%3D1%40"), "{uri}");
    }

    /// A password is bounded at both ends, and the bounds are the ones the
    /// handler enforces.
    #[test]
    fn the_password_bounds_are_length_only() {
        assert_eq!(MIN_PASSWORD_CHARS, 12);
        assert_eq!(MAX_PASSWORD_CHARS, 256);
        // Not `MIN < MAX` — clippy calls that a constant assertion, and it
        // is right: the interesting claim is that the *handler's* comparison
        // admits a password at each end of the range and refuses one outside
        // it, which is a claim about the range operator rather than about
        // two literals.
        let admits = |length: usize| (MIN_PASSWORD_CHARS..=MAX_PASSWORD_CHARS).contains(&length);
        assert!(admits(MIN_PASSWORD_CHARS), "the shortest legal password");
        assert!(admits(MAX_PASSWORD_CHARS), "the longest legal password");
        assert!(!admits(MIN_PASSWORD_CHARS - 1));
        assert!(!admits(MAX_PASSWORD_CHARS + 1));
        assert!(!admits(0), "an empty password is not a password");
    }

    /// The header is read case-insensitively (axum lower-cases header names)
    /// and a blank or non-ASCII value is no token at all.
    #[test]
    fn a_blank_or_unusable_session_header_is_no_token() {
        use axum::http::HeaderValue;

        let mut headers = HeaderMap::new();
        assert_eq!(session_token(&headers), None);

        headers.insert(SESSION_HEADER, HeaderValue::from_static("   "));
        assert_eq!(
            session_token(&headers),
            None,
            "a blank value is not a token"
        );

        headers.insert(SESSION_HEADER, HeaderValue::from_static(" abc "));
        assert_eq!(
            session_token(&headers),
            Some("abc".to_owned()),
            "trimmed, because a proxy may pad"
        );
    }

    /// Every route this module mounts is listed in `STAFF_ROUTES`, which is
    /// what the boundary test walks.
    #[test]
    fn the_route_table_names_seven_unauthenticated_paths() {
        assert_eq!(STAFF_ROUTES.len(), 7);
        for (path, methods) in STAFF_ROUTES {
            assert!(path.starts_with('/'), "{path}");
            assert!(!methods.is_empty(), "{path}");
        }
    }
}
