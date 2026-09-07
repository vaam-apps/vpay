//! The `oauth_authorization_codes` repository
//! (`backends/migrations/0035_create-staff-auth.sql`) — "this 60-second
//! string may be exchanged once, by this client, for a token naming that
//! person" ([ADR-0017](../../../../docs/adr/0017-staff-authentication.md)
//! decision 3).
//!
//! # Why this is not `authkestra.oauth_codes`
//!
//! Migration `0006` transcribes `authkestra-op`'s own DDL for `SqlxOpStore`,
//! and `SqlxOpStore` is behind the `sqlx-postgres` feature, which pins
//! `sqlx ^0.8`. This workspace moved to `=0.9.0` so CrateStack and `vpay-db`
//! could share a transaction, so reviving it would take the whole workspace
//! back a major (`docs/status.md`, "The three OP stores that pinned sqlx
//! 0.8"). This table is vpay's own — and it carries two columns authkestra's
//! shape has nowhere to put: `session_id` and `merchant_id`.
//!
//! # The single-use guarantee
//!
//! `AuthorizationCodeStore::consume_code`'s own doc calls atomic consumption
//! "the single most important correctness property in this crate" and names
//! the failure: "a check-then-mark implemented as two separate storage calls
//! is a TOCTOU race that permits code replay". [`AuthorizationCodes::consume_code`]
//! is a compare-and-swap — one `UPDATE … WHERE consumed_at IS NULL` — and the
//! read that precedes it reads only columns nothing ever updates, so the swap
//! is what decides the race and the read cannot change the answer.

use std::fmt;

use async_trait::async_trait;
use time::OffsetDateTime;

use crate::error::DbError;
use crate::persistence::{classify_cratestack, system_context};
use crate::schema::cratestack_schema::{self, oauth_authorization_code as code_col};
use crate::staff::{from_chrono, to_chrono};

/// The `.cstack` model these calls name.
const MODEL: &str = "OauthAuthorizationCode";

/// One `oauth_authorization_codes` row, as stored.
///
/// Everything except `consumed_at`, which the caller never needs: the answer
/// to "was it already spent?" is [`AuthorizationCodes::consume_code`]'s `Ok(None)`,
/// and a field carrying it would invite a caller to check it in Rust instead.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationCodeRow {
    /// The client the code was issued to. Checked again at the exchange.
    pub client_id: String,
    /// Whose code it is. Becomes the token's `sub`.
    pub staff_id: String,
    /// The session that authorised it. The cascade on this column is what
    /// makes signing out kill a code in flight.
    pub session_id: String,
    /// The tenant to stamp into the minted token's merchant claim. A copy
    /// taken at issue, so a staff row edited in between cannot move a token.
    pub merchant_id: String,
    /// Space-delimited, as granted.
    pub scope: String,
    /// The PKCE challenge. Not `Option`: the exchange refuses a stored code
    /// without one, so a nullable column would be a shape whose only legal
    /// value the exchange rejects.
    pub code_challenge: String,
    /// `S256`, and the CHECK admits nothing else.
    pub code_challenge_method: String,
    /// Matched byte for byte at the exchange.
    pub redirect_uri: String,
    /// OIDC's replay nonce, echoed into an id token when one is issued.
    pub nonce: Option<String>,
    /// When the code stops being exchangeable.
    pub expires_at: OffsetDateTime,
}

/// Redacts the challenge, which is a hash of the verifier the caller is about
/// to present, and the nonce. Neither is a bearer credential on its own —
/// the *code* is, and the code is never in this struct — but both are inputs
/// to a check, and a log line is not where a check's inputs belong.
impl fmt::Debug for AuthorizationCodeRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorizationCodeRow")
            .field("client_id", &self.client_id)
            .field("staff_id", &self.staff_id)
            .field("session_id", &self.session_id)
            .field("merchant_id", &self.merchant_id)
            .field("scope", &self.scope)
            .field("code_challenge", &"[redacted]")
            .field("code_challenge_method", &self.code_challenge_method)
            .field("redirect_uri", &self.redirect_uri)
            .field("nonce", &self.nonce.as_ref().map(|_| "[redacted]"))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Everything `/dash/v1/oauth/authorize` supplies for one code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAuthorizationCode {
    /// The SHA-256 of the code, hex. Never the code.
    pub code_hash: String,
    /// The row's content — see [`AuthorizationCodeRow`].
    pub row: AuthorizationCodeRow,
    /// When it was issued.
    pub now: OffsetDateTime,
}

/// Reads and writes of `oauth_authorization_codes`. Every method through
/// CrateStack.
#[async_trait]
pub trait AuthorizationCodes {
    /// Stores a newly issued code.
    ///
    /// Named for `authkestra_op::code::AuthorizationCodeStore`'s own method
    /// rather than `store`, because the whole point of this trait is to be
    /// the thing `vpay_api::op::dashboard`'s store delegates to — and because
    /// `Idempotency::store` already exists on the same `Repositories` object,
    /// where a second bare `store` is an `E0034` ambiguity at every call site.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::ForeignKey`] if
    /// the session was signed out between the authorization decision and this
    /// write (which is correct: that code must not exist),
    /// [`crate::PersistenceError::Check`] if the method is not `S256`,
    /// [`crate::PersistenceError::Backend`] otherwise.
    async fn store_code(&self, new: NewAuthorizationCode) -> Result<(), DbError>;

    /// **Atomically** spends a code, returning what it stood for.
    ///
    /// `Ok(None)` covers "no such code", "already spent" and "expired",
    /// without distinguishing them — which is what
    /// `AuthorizationCodeStore::consume_code`'s own contract requires, "to
    /// avoid leaking timing/existence information to a potential attacker".
    ///
    /// # Why the read before the swap is not a TOCTOU
    ///
    /// The order is read, then compare-and-swap. The **swap** is what decides
    /// the race — exactly one caller gets `ok == 1`, because
    /// `consumed_at IS NULL` is evaluated by Postgres under the row lock the
    /// `UPDATE` takes — and the loser is told `None` whatever it read. The
    /// read exists only to carry the row's content back, and every column it
    /// reads is written once at issue and never updated, so it cannot be
    /// stale by the time the swap succeeds.
    ///
    /// Expiry is checked in Rust, after the swap, and that ordering is
    /// deliberate: an expired code is **still spent**, so presenting one
    /// twice cannot be told from presenting it once.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn consume_code(
        &self,
        code_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Option<AuthorizationCodeRow>, DbError>;
}

#[async_trait]
impl AuthorizationCodes for crate::repository::PgRepositories {
    async fn store_code(&self, new: NewAuthorizationCode) -> Result<(), DbError> {
        let row = new.row;
        self.cs
            .oauth_authorization_code()
            .create(cratestack_schema::CreateOauthAuthorizationCodeInput {
                code_hash: new.code_hash,
                client_id: row.client_id,
                staff_id: row.staff_id,
                session_id: row.session_id,
                merchant_id: row.merchant_id,
                scope: row.scope,
                code_challenge: row.code_challenge,
                code_challenge_method: row.code_challenge_method,
                redirect_uri: row.redirect_uri,
                nonce: row.nonce,
                created_at: to_chrono(new.now),
                expires_at: to_chrono(row.expires_at),
                consumed_at: None,
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;

        Ok(())
    }

    async fn consume_code(
        &self,
        code_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Option<AuthorizationCodeRow>, DbError> {
        let Some(model) = self
            .cs
            .oauth_authorization_code()
            .find_unique(code_hash.to_owned())
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?
        else {
            return Ok(None);
        };

        // THE SWAP. `consumed_at IS NULL` is evaluated by Postgres under the
        // row lock this UPDATE takes, so exactly one concurrent caller can
        // see `ok == 1`. Everything above is content; this is the decision.
        let summary = self
            .cs
            .oauth_authorization_code()
            .update_many()
            .where_(code_col::code_hash().eq(code_hash.to_owned()))
            .where_(code_col::consumed_at().is_null())
            .set(cratestack_schema::UpdateOauthAuthorizationCodeInput {
                consumed_at: Some(Some(to_chrono(now))),
                ..cratestack_schema::UpdateOauthAuthorizationCodeInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        if summary.ok != 1 {
            return Ok(None);
        }

        let row = row_from_model(model);
        // AFTER the swap, deliberately: an expired code is still spent, so a
        // caller cannot tell "expired" from "already used" by presenting one
        // twice.
        if now >= row.expires_at {
            return Ok(None);
        }

        Ok(Some(row))
    }
}

/// The generated model row, in vpay's own types. Infallible: this table has
/// no closed vocabulary to parse — `code_challenge_method` is closed by a
/// CHECK and by the exchange, and neither needs a Rust enum to be safe.
fn row_from_model(
    model: cratestack_schema::models::OauthAuthorizationCode,
) -> AuthorizationCodeRow {
    AuthorizationCodeRow {
        client_id: model.client_id,
        staff_id: model.staff_id,
        session_id: model.session_id,
        merchant_id: model.merchant_id,
        scope: model.scope,
        code_challenge: model.code_challenge,
        code_challenge_method: model.code_challenge_method,
        redirect_uri: model.redirect_uri,
        nonce: model.nonce,
        expires_at: from_chrono(model.expires_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// See [`crate::staff`]'s equivalent. The `update` slot is again the
    /// dangerous one, and here it is dangerous in the *safe* direction in a
    /// way worth naming: a missing arm makes the swap match zero rows, so
    /// `consume_code` answers `None` for every code and no dashboard login can
    /// complete. Loud, and fail-closed — but the fix somebody reaches for
    /// when every login breaks is what this assertion exists to aim.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::OAUTH_AUTHORIZATION_CODE_MODEL as descriptor;

        assert!(
            !descriptor.read_allow_policies.is_empty(),
            "model OauthAuthorizationCode lost @@allow(\"read\", …): every code would read as \
             absent"
        );
        assert!(
            !descriptor.create_allow_policies.is_empty(),
            "model OauthAuthorizationCode lost @@allow(\"create\", …): /authorize would fail \
             loudly"
        );
        assert!(
            !descriptor.update_allow_policies.is_empty(),
            "model OauthAuthorizationCode lost @@allow(\"update\", …): the single-use \
             compare-and-swap would match zero rows and consume_code() would answer None for every \
             code, with no error anywhere"
        );
        assert!(
            !descriptor.delete_allow_policies.is_empty(),
            "model OauthAuthorizationCode lost @@allow(\"delete\", …)"
        );
    }

    /// Neither the challenge nor the nonce reaches a `{:?}`.
    #[test]
    fn debug_redacts_the_pkce_challenge_and_the_nonce() {
        let row = AuthorizationCodeRow {
            client_id: "vpay-dashboard".to_owned(),
            staff_id: "stf_1".to_owned(),
            session_id: "c".repeat(64),
            merchant_id: "acct_1".to_owned(),
            scope: "dashboard:read".to_owned(),
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_owned(),
            code_challenge_method: "S256".to_owned(),
            redirect_uri: "https://dash.example.test/callback".to_owned(),
            nonce: Some("n-0S6_WzA2Mj".to_owned()),
            expires_at: OffsetDateTime::UNIX_EPOCH,
        };

        let rendered = format!("{row:?}");
        assert!(!rendered.contains("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert!(!rendered.contains("n-0S6_WzA2Mj"));
        assert!(rendered.contains("stf_1"), "the ids are what a log needs");
    }
}
