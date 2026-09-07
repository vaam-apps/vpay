//! The authorization-code grant, served for the dashboard client and for
//! nothing else
//! ([ADR-0017](../../../../../docs/adr/0017-staff-authentication.md)
//! decision 3).
//!
//! Two routes. [`authorize`] turns an authenticated staff session into a
//! code; [`token`] turns a code and its PKCE verifier into an access token
//! carrying the merchant claim `/dash/v1` requires.
//!
//! # Which half is upstream's and which is vpay's
//!
//! [`authorize`] delegates to `authkestra_op::handlers::authorize::handle_authorize`,
//! which does the client lookup, the **exact** redirect-URI match, the
//! per-scope registration check, the `response_type` check, the unconditional
//! PKCE requirement (OAuth 2.1 §4.1) and the error-redirect encoding. What
//! this module adds in front of it is the only thing authkestra cannot do:
//! **produce the `Identity`**. That is the whole of exp23's blocker 1.
//!
//! [`token`] is vpay's own, and `crate::op::dashboard`'s header says why:
//! `default_handle_authorization_code`'s last step is the mint, and the mint
//! has to stamp `vpay_config::DASHBOARD_MERCHANT_CLAIM`. Every check it
//! performs is performed here, in the same order, each with its own test.

use std::collections::HashMap;

use authkestra_op::handlers::authorize::{AuthorizeOutcome, AuthorizeRequest, handle_authorize};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Form, Json};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use time::OffsetDateTime;

use crate::ApiError;
use crate::op::dashboard;
use crate::staff::{authenticated_session, refused};
use crate::staff_auth::tokens;

/// The one `grant_type` this endpoint serves.
const GRANT_TYPE: &str = "authorization_code";

/// The one PKCE method. `plain` is refused, which is OAuth 2.1 §7.5.2's
/// requirement and what `handle_authorize` already enforces at the other end.
const PKCE_METHOD: &str = "S256";

/// What `GET /dash/v1/oauth/authorize` takes, as a query string.
///
/// Deserialised into a `HashMap` first and reassembled, rather than derived
/// straight onto `authkestra_op`'s `AuthorizeRequest`: that type is
/// `#[non_exhaustive]`, so this crate cannot build one with a struct literal
/// and cannot rely on its `Deserialize` staying total across a version bump.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AuthorizeQuery {
    /// Must be the registered dashboard client. Anything else is
    /// `UnknownClient` from the store.
    pub client_id: String,
    /// Matched byte for byte against the registration.
    pub redirect_uri: String,
    /// Must be `code`.
    pub response_type: String,
    /// Space-delimited. Checked per scope against the registration.
    #[serde(default)]
    pub scope: String,
    /// Echoed back on the redirect, as RFC 6749 §4.1.1 requires.
    #[serde(default)]
    pub state: Option<String>,
    /// The PKCE challenge. Required — `handle_authorize` refuses a request
    /// without one, unconditionally.
    #[serde(default)]
    pub code_challenge: Option<String>,
    /// Must be `S256`.
    #[serde(default)]
    pub code_challenge_method: Option<String>,
    /// OIDC's replay nonce.
    #[serde(default)]
    pub nonce: Option<String>,
}

/// What `POST /dash/v1/oauth/token` takes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TokenForm {
    /// Must be `authorization_code`.
    pub grant_type: String,
    /// The code from the redirect.
    #[serde(default)]
    pub code: String,
    /// The **same** redirect URI the authorization request named.
    #[serde(default)]
    pub redirect_uri: String,
    /// The PKCE verifier. This is what authenticates a public client.
    #[serde(default)]
    pub code_verifier: String,
    /// The client the code was issued to.
    #[serde(default)]
    pub client_id: String,
}

/// What `POST /dash/v1/oauth/token` answers, on success.
///
/// vpay's own shape rather than `authkestra_op::handlers::token::TokenResponse`,
/// because this endpoint issues neither a refresh token nor an id token and a
/// response type carrying `Option`s for both would advertise two things that
/// are never there. Refresh tokens are not issued on this surface at all
/// (`docs/flows/dashboard-auth.md`, "Token lifetimes"), and the dashboard has
/// no use for an id token: it reads who is signed in from
/// `GET /dash/v1/staff/session`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TokenResponse {
    /// The `/dash/v1` bearer token.
    pub access_token: String,
    /// `Bearer`, always.
    pub token_type: String,
    /// Seconds, from `crate::op::ACCESS_TOKEN_TTL_SECS`.
    pub expires_in: u64,
    /// What was granted, space-delimited.
    pub scope: String,
}

/// Turns an authenticated staff session into an authorization code.
///
/// # What this adds in front of `handle_authorize`
///
/// Four refusals, and none of them is something authkestra could make,
/// because all four are about the *person*:
///
/// 1. the session exists, is inside both bounds, and has presented **both**
///    factors ([`crate::staff::authenticated_session`]);
/// 2. the staff row is `active` — re-read on this request, not trusted from
///    the session;
/// 3. the one-time password has been replaced. A session that still carries
///    `password_change_required` may change its password and nothing else,
///    so a printed credential cannot reach `/dash/v1` at all;
/// 4. the staff member's `merchant_id` is the tenant the dashboard client is
///    bound to. A staff row for another merchant is refused here rather than
///    minted a token `require_dashboard_token` would then refuse — the same
///    answer, one round trip earlier, and with the log line at the point
///    where the mismatch is visible.
///
/// # Errors
///
/// [`ApiError::StaffSignInRefused`] for any of the four;
/// [`ApiError::NotFound`] if this deployment serves no staff login;
/// [`ApiError::Db`] if Postgres fails.
pub(crate) async fn authorize(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    Query(query): Query<AuthorizeQuery>,
) -> Result<Response, ApiError> {
    let login = state.staff_login()?;
    let now = OffsetDateTime::now_utc();

    let (session, staff) = authenticated_session(&state, &headers, now).await?;
    if staff.password_change_required {
        return Err(refused("the one-time password has not been replaced"));
    }
    let binding = login.dashboard_op.binding();
    if staff.merchant_id != binding.merchant_id {
        tracing::warn!(
            staff_id = %staff.id,
            "a staff member signed in against a deployment whose dashboard is bound to another \
             merchant; refusing to mint a code for a tenant they may not read"
        );
        return Err(refused(
            "staff merchant does not match the dashboard binding",
        ));
    }

    // The identity carries the session and the tenant as attributes, because
    // that is the only field `handle_authorize` passes through to the code
    // store. They are stripped before the token is minted — see
    // `crate::op::dashboard::ATTRIBUTE_SESSION`.
    let identity = dashboard::identity_for(&staff.id, &session.id, &staff.merchant_id);

    let request = authorize_request(query);
    let outcome = handle_authorize(
        request,
        identity,
        login.dashboard_op.config(),
        login.dashboard_op.store(),
    )
    .await;

    match outcome {
        AuthorizeOutcome::Redirect(url) => {
            tracing::info!(staff_id = %staff.id, "issued a dashboard authorization code");
            // `302 Found`, built by hand rather than with `Redirect::to`,
            // which emits `303 See Other`. RFC 6749 §4.1.2 specifies 302 for
            // the authorization response, and a client that matched on it
            // exactly — several do — would not follow a 303. The two are
            // interchangeable for a GET; the spec is not a matter of taste
            // here, so the spec wins.
            Ok((
                axum::http::StatusCode::FOUND,
                [(axum::http::header::LOCATION, url)],
            )
                .into_response())
        }
        AuthorizeOutcome::DirectError(error) => {
            // An unknown client or a redirect URI that does not match the
            // registration. **Never redirected to**: RFC 6749 §4.1.2.1 is
            // explicit that a server MUST NOT automatically redirect to an
            // unvalidated URI, because that turns the endpoint into an open
            // redirector. `handle_authorize` returns this variant precisely
            // so the caller cannot get that wrong, and the answer here is a
            // refusal rendered in vpay's own envelope.
            tracing::warn!(
                %error,
                staff_id = %staff.id,
                "a dashboard authorization request named an unknown client or an unregistered \
                 redirect_uri; refusing without redirecting"
            );
            Err(ApiError::InvalidParam {
                param: "redirect_uri".to_owned(),
                message: "The client_id and redirect_uri must match this deployment's \
                          registered dashboard client."
                    .to_owned(),
            })
        }
    }
}

/// `AuthorizeQuery`, in `authkestra_op`'s shape.
///
/// Through `serde_json` because `AuthorizeRequest` is `#[non_exhaustive]`:
/// this crate cannot write a struct literal for it, and `Deserialize` is the
/// constructor upstream actually supports. The map is built field by field
/// from a type this crate owns, so a field upstream adds is a field this
/// function leaves at its own default rather than one silently taken from a
/// caller's query string.
fn authorize_request(query: AuthorizeQuery) -> AuthorizeRequest {
    let mut fields: HashMap<&str, serde_json::Value> = HashMap::new();
    fields.insert("client_id", query.client_id.into());
    fields.insert("redirect_uri", query.redirect_uri.into());
    fields.insert("response_type", query.response_type.into());
    fields.insert("scope", query.scope.into());
    fields.insert("state", query.state.into());
    fields.insert("code_challenge", query.code_challenge.into());
    fields.insert("code_challenge_method", query.code_challenge_method.into());
    fields.insert("nonce", query.nonce.into());

    // Infallible in practice — every field above is a `String` or a
    // `null` — and this crate denies `expect`, so a failure falls through to
    // a request `handle_authorize` will refuse for missing PKCE rather than
    // panicking on the login path.
    serde_json::from_value(serde_json::json!(fields)).unwrap_or_else(|error| {
        tracing::error!(%error, "rebuilding an authorize request for authkestra-op");
        serde_json::from_value(serde_json::json!({
            "client_id": "",
            "redirect_uri": "",
            "response_type": "",
        }))
        .unwrap_or_else(|_| unreachable_request())
    })
}

/// The last resort for [`authorize_request`], and it is genuinely
/// unreachable: `AuthorizeRequest`'s three required fields are all `String`,
/// so the three-key object above always deserialises. It exists because this
/// crate denies `unwrap`, `expect` and `panic`, and a `Default` impl
/// upstream does not provide.
fn unreachable_request() -> AuthorizeRequest {
    // A second attempt at the same object. If this ever failed the function
    // would loop, which it cannot: `serde_json::from_value` is pure.
    match serde_json::from_value(serde_json::json!({
        "client_id": "",
        "redirect_uri": "",
        "response_type": "",
    })) {
        Ok(request) => request,
        Err(_) => unreachable_request(),
    }
}

/// Exchanges a code for a `/dash/v1` access token.
///
/// # The checks, in order, and what each refuses
///
/// 1. **`grant_type`** — anything but `authorization_code` is refused. This
///    endpoint serves one grant; `/v1/oauth/token` serves the other.
/// 2. **`client_id`** — must be the registered dashboard client, checked
///    before the code is spent so a wrong client cannot burn a code.
/// 3. **the code**, spent by a compare-and-swap
///    (`vpay_db::AuthorizationCodes::consume_code`). A second presentation of
///    one code is refused whatever else is right, which is RFC 6749 §4.1.2's
///    single-use requirement and `AuthorizationCodeStore`'s own "single most
///    important correctness property".
/// 4. **the code's own client**, against the one presented — a code issued to
///    one client may not be redeemed by another.
/// 5. **`redirect_uri`**, byte for byte against the one the authorization
///    request named. RFC 6749 §4.1.3, and it is what stops a code being
///    redeemed by a party that intercepted it at a different URI.
/// 6. **PKCE**: `S256(verifier)` must equal the stored challenge, compared in
///    constant time. This is what authenticates a public client, and it is
///    the check the whole grant rests on.
///
/// Only then is a token minted, and only then is it written to the session
/// row — so a failed exchange leaves the session with no token, and the code
/// spent.
///
/// # Errors
///
/// [`ApiError::InvalidParam`] naming the offending parameter for 1 and 2;
/// [`ApiError::StaffSignInRefused`] for 3 to 6, which are one answer for
/// [`crate::staff`]'s reason; [`ApiError::Db`] if Postgres fails.
pub(crate) async fn token(
    State(state): State<crate::AppState>,
    Form(form): Form<TokenForm>,
) -> Result<Json<TokenResponse>, ApiError> {
    let login = state.staff_login()?;
    let repositories = state.repositories();
    let now = OffsetDateTime::now_utc();
    let registration = login.dashboard_op.registration();

    if form.grant_type != GRANT_TYPE {
        return Err(ApiError::InvalidParam {
            param: "grant_type".to_owned(),
            message: format!("This endpoint serves the {GRANT_TYPE} grant and no other."),
        });
    }
    if form.client_id != registration.client_id {
        return Err(ApiError::InvalidParam {
            param: "client_id".to_owned(),
            message: "The client_id must be this deployment's registered dashboard client."
                .to_owned(),
        });
    }

    // Spent here, before any other check that depends on its content — a
    // code presented with a wrong verifier is still spent, which is what
    // stops an attacker probing a captured code against many verifiers.
    let Some(code) = repositories
        .consume_code(&tokens::digest(&form.code), now)
        .await?
    else {
        return Err(refused(
            "authorization code absent, expired or already spent",
        ));
    };

    if code.client_id != form.client_id {
        return Err(refused("code was issued to another client"));
    }
    if code.redirect_uri != form.redirect_uri {
        return Err(refused(
            "redirect_uri does not match the authorization request",
        ));
    }
    if code.code_challenge_method != PKCE_METHOD {
        // Unreachable through `/authorize` — `handle_authorize` refuses any
        // other method and migration 0035's `method_is_s256` refuses the
        // row — so this is the fail-closed answer for a code that reached
        // storage some other way, exactly as
        // `default_handle_authorization_code` treats the same case.
        return Err(refused("stored code carries an unsupported PKCE method"));
    }
    if !verifier_matches(&form.code_verifier, &code.code_challenge) {
        return Err(refused("pkce verifier"));
    }

    // The identity that reaches the token: the staff id and nothing else.
    // NOT the one stored with the code — see
    // `crate::op::dashboard::ATTRIBUTE_SESSION` for what a leftover attribute
    // would publish.
    let identity = dashboard::token_identity_for(&code.staff_id);
    let mut extra = HashMap::new();
    extra.insert(
        vpay_config::DASHBOARD_MERCHANT_CLAIM.to_owned(),
        serde_json::Value::String(code.merchant_id.clone()),
    );

    let access_token = login
        .dashboard_op
        .tokens()
        .issue_user_token_with_extra(
            identity,
            crate::op::ACCESS_TOKEN_TTL_SECS,
            Some(code.scope.clone()),
            // THE AUDIENCE, and ADR-0017 decision 3 in one argument: the
            // dashboard client's own id, which is what
            // `resource_auth::JwtValidator` is built to expect.
            Some(registration.client_id.clone()),
            extra,
        )
        .map_err(|error| {
            tracing::error!(%error, "minting a dashboard access token");
            ApiError::Internal("the dashboard access token could not be signed".to_owned())
        })?;

    // Written to the session row, which is what makes signing out a
    // revocation: the dashboard's server reads the token back from here on
    // every render, and a deleted row has none to read.
    if !repositories
        .record_access_token(&code.session_id, &access_token, now)
        .await?
    {
        // The session was signed out between `/authorize` and here. The code
        // is spent and the token is not stored, so nothing can present it:
        // the cascade would have deleted the code too, and losing that race
        // must not hand out a token the sign-out was meant to prevent.
        tracing::warn!(
            staff_id = %code.staff_id,
            "a dashboard code was exchanged for a session that no longer exists; discarding \
             the minted token"
        );
        return Err(refused("session was signed out during the exchange"));
    }

    tracing::info!(staff_id = %code.staff_id, "issued a dashboard access token");
    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".to_owned(),
        expires_in: crate::op::ACCESS_TOKEN_TTL_SECS,
        scope: code.scope,
    }))
}

/// `BASE64URL(SHA256(verifier)) == challenge`, in constant time.
///
/// RFC 7636 §4.6. Constant time because the challenge is a value an attacker
/// holding a captured code may probe against guessed verifiers, and a `==` on
/// two base64url strings exits at the first differing byte.
///
/// A `false` for a verifier outside RFC 7636 §4.1's length bounds before the
/// hash is computed: a verifier of 43 to 128 characters is what the RFC
/// permits, and anything else was not produced by a conforming client.
#[must_use]
fn verifier_matches(verifier: &str, challenge: &str) -> bool {
    if !(43..=128).contains(&verifier.len()) {
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(hasher.finalize());
    bool::from(computed.as_bytes().ct_eq(challenge.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 Appendix B's own vector: this exact verifier hashes to this
    /// exact challenge. Without it, "the PKCE check works" would be a claim
    /// about two functions in this file agreeing with each other.
    #[test]
    fn the_pkce_check_matches_rfc_7636_appendix_b() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        assert!(verifier_matches(verifier, challenge));
        assert!(
            !verifier_matches(verifier, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cN"),
            "one character off is a different challenge"
        );
    }

    /// A verifier outside RFC 7636 §4.1's bounds is refused before it is
    /// hashed — including the empty one, which is what a client that omitted
    /// the parameter sends.
    #[test]
    fn a_verifier_outside_the_rfc_bounds_is_refused() {
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        for verifier in ["", "short", &"a".repeat(42), &"a".repeat(129)] {
            assert!(
                !verifier_matches(verifier, challenge),
                "accepted {} characters",
                verifier.len()
            );
        }
    }

    /// The empty challenge — what a row would carry if the `NOT NULL` were
    /// ever dropped — is not satisfied by any verifier. The decisive
    /// mutation is `verifier_matches` returning `true` when `challenge` is
    /// empty, which is the shape "PKCE is optional" would take.
    #[test]
    fn no_verifier_satisfies_an_empty_challenge() {
        for verifier in [
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            &"a".repeat(43),
            &"z".repeat(128),
        ] {
            assert!(!verifier_matches(verifier, ""), "{verifier}");
        }
    }

    /// The query is rebuilt into upstream's `#[non_exhaustive]` type without
    /// losing a field — the PKCE pair especially, which `handle_authorize`
    /// refuses a request without.
    #[test]
    fn the_authorize_query_survives_the_rebuild() {
        let request = authorize_request(AuthorizeQuery {
            client_id: "vpay-dashboard".to_owned(),
            redirect_uri: "https://dash.example.test/cb".to_owned(),
            response_type: "code".to_owned(),
            scope: "dashboard:read".to_owned(),
            state: Some("xyz".to_owned()),
            code_challenge: Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_owned()),
            code_challenge_method: Some("S256".to_owned()),
            nonce: Some("n-0S6".to_owned()),
        });

        assert_eq!(request.client_id, "vpay-dashboard");
        assert_eq!(request.redirect_uri, "https://dash.example.test/cb");
        assert_eq!(request.response_type, "code");
        assert_eq!(request.scope, "dashboard:read");
        assert_eq!(request.state.as_deref(), Some("xyz"));
        assert_eq!(
            request.code_challenge.as_deref(),
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"),
            "a lost code_challenge would make handle_authorize refuse every login"
        );
        assert_eq!(request.code_challenge_method.as_deref(), Some("S256"));
        assert_eq!(request.nonce.as_deref(), Some("n-0S6"));
    }
}
