//! The three `OpStore` slots `/v1` is obliged to fill and no `/v1` grant can
//! reach, filled with stores that **refuse** instead of storing.
//!
//! `authkestra_op::OpStore` is a supertrait of `AuthorizationCodeStore`,
//! `RefreshTokenStore` and `DeviceCodeStore`, so a value implementing it has
//! to supply all three whether or not any grant reaches them. `/v1` serves
//! `client_credentials` and nothing else ([`crate::op::OP_GRANT_TYPES`],
//! `vpay_config::ConfigError::DisallowedMerchantGrant`), so none of the twelve
//! methods below is reachable — see "Why nothing can call these" for the
//! argument, which is structural rather than a matter of configuration.
//!
//! # Why refusing, and not the real Postgres store
//!
//! Until 2026-09-05 all three slots held
//! `authkestra_op::sqlx_store::SqlxOpStore<sqlx::Postgres>` over
//! `vpay_db::Repositories::op_store_pool`. That type lives behind
//! `authkestra-op`'s optional `sqlx-postgres` feature, which pins `sqlx ^0.8`
//! — and it was the **only** thing in the workspace still on that major, so
//! the whole of vpay was held at sqlx 0.8 to keep three slots warm that
//! nothing calls. `authkestra-op` 0.8.x does not help: it deletes
//! `src/sqlx_store.rs` and moves it to `authkestra-store-sqlx`, which is
//! itself still `sqlx ^0.8`.
//!
//! The alternative to refusing was for vpay to *implement* the three stores
//! over its own pool: roughly four hundred lines of ported SQL, three of
//! whose methods (`consume_code`, `consume_token`, `consume_device_code`)
//! must be atomic compare-and-swap or they are an authorization-code replay
//! vulnerability — written for grants this deployment refuses at the door.
//! Storage nothing reads is not safer than no storage; it is the same amount
//! of unexercised SQL with a larger surface. So these fail closed instead,
//! which is exactly the pattern `authkestra-op` applies to its own optional
//! seams (`NoClientAssertionStore`, `NoDpopReplayStore`: refuse rather than
//! silently degrade).
//!
//! **This is not a stub, a mock or a double** (AGENTS.md rule 1). A double
//! stands in for a real implementation and pretends to succeed; every method
//! here returns `Err` and can never be mistaken for one. The day `/v1` does
//! mount one of these grants, the first request fails loudly with a message
//! naming the grant, instead of quietly reading an empty table.
//!
//! # Why nothing can call these
//!
//! `authkestra_op::handlers::token::handle_token` dispatches on
//! `req.grant_type`, and each of the three grant handlers checks
//! `client.allows_grant_type(..)` *before* it touches a store
//! (`default_handle_authorization_code`'s first statement,
//! `default_handle_refresh_token`'s first statement, `handle_device_code`'s
//! first statement — all at `authkestra-op-0.7.1`). A merchant registration
//! can only ever declare `client_credentials`, so all three return
//! `unauthorized_client` before any store is consulted.
//!
//! The one grant `/v1` does serve cannot reach a store at all:
//! `handle_client_credentials` does not take an `op_store` argument
//! (`authkestra-op-0.7.1/src/handlers/token.rs`, the `"client_credentials"`
//! dispatch arm), so no change to a registration or a config file can route
//! it here. `backends/tests/integration/tests/merchant_token_flow.rs` case
//! (i) proves the first half against a real server on a real socket: all
//! three grants come back `unauthorized_client`/400, never a 500.
//!
//! # What was given up
//!
//! `backends/tests/integration/tests/authkestra_op_smoke.rs` — which drove
//! `SqlxOpStore`'s own hand-built SQL against migrations 0006 and 0013 to
//! prove that transcription was faithful — was deleted in the same commit.
//! It could not compile without the feature, and what it proved (that a
//! store vpay no longer constructs matches a schema vpay no longer reads) no
//! longer describes this system. `postgres_smoke.rs` still asserts the four
//! `authkestra.*` tables exist and that `oauth_codes.client_id`'s foreign key
//! fires, so the migrations themselves stay covered. Dropping those tables is
//! a separate decision and is **not** taken here (see `docs/status.md`).
//!
//! Migrations `0006` and `0013` still carry header comments citing that
//! deleted file, and they were **deliberately left wrong**: `sqlx::migrate!`
//! checksums each migration's whole file content, comments included, so
//! editing one that has already been applied turns the next boot into a
//! version-mismatch failure. The correction lives in `docs/status.md`.

use async_trait::async_trait;
use authkestra_op::code::{AuthorizationCode, AuthorizationCodeStore};
use authkestra_op::device::{DeviceCodeSession, DeviceCodeStore};
use authkestra_op::error::OpError;
use authkestra_op::refresh::{RefreshToken, RefreshTokenStore};
use vpay_core::{Category, Classify};

/// One of the three OAuth grants `/v1` declines to serve.
///
/// An enum rather than three bare `&'static str` constants so that
/// [`UnservedGrantError`] cannot be constructed naming a grant that is not one
/// of these three, and so a fourth grant becomes a compile error at every
/// match rather than a fourth string somebody has to notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnservedGrant {
    /// RFC 6749 §4.1. `/v1` has no authorization endpoint and no end user.
    AuthorizationCode,
    /// RFC 6749 §6. `client_credentials` mints no refresh token
    /// (`handle_client_credentials` hardcodes `refresh_token: None`), so
    /// there is never one to present.
    RefreshToken,
    /// RFC 8628. `/v1` has no device-authorization endpoint.
    DeviceCode,
}

impl UnservedGrant {
    /// The `grant_type` string a client would put in the form body.
    ///
    /// Written out rather than derived from the variant name because these
    /// are wire values: RFC 8628's is a URN, and a `Debug`-derived spelling
    /// would silently stop matching it.
    #[must_use]
    pub const fn grant_type(self) -> &'static str {
        match self {
            Self::AuthorizationCode => "authorization_code",
            Self::RefreshToken => "refresh_token",
            Self::DeviceCode => "urn:ietf:params:oauth:grant-type:device_code",
        }
    }

    /// Every unserved grant, so a test can walk them without repeating the
    /// list — the same reason [`vpay_core::Category::ALL`] exists.
    pub const ALL: [Self; 3] = [
        Self::AuthorizationCode,
        Self::RefreshToken,
        Self::DeviceCode,
    ];
}

/// What a refusing store answers: a named grant, a named method, and the
/// fact that `/v1` does not serve it.
///
/// The `Display` is written for an **operator** reading a log line (ADR-0011:
/// `Display` is for operators), because that is the only audience it can ever
/// have — nothing in `authkestra_op` renders an `OpError`'s cause to a
/// caller, and [`Self::public_message`] is the category's generic sentence.
/// Reaching this error at all means a grant this deployment refuses at the
/// door somehow got past the door, so the log line has to say which one and
/// where.
///
/// ```
/// use vpay_api::op::refusing_stores::{UnservedGrant, UnservedGrantError};
/// use vpay_core::{Category, Classify};
///
/// let error = UnservedGrantError::new(UnservedGrant::DeviceCode, "consume_device_code");
///
/// // The operator's half names the grant, the method and the reason.
/// assert!(error.to_string().contains("urn:ietf:params:oauth:grant-type:device_code"));
/// assert!(error.to_string().contains("consume_device_code"));
///
/// // The caller's half says nothing about either.
/// assert_eq!(error.category(), Category::NotImplemented);
/// assert_eq!(error.public_message(), Category::NotImplemented.generic_message());
/// ```
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error(
    "`{method}` was called on the store for the `{}` grant, which vpay does not serve: \
     /v1 offers client_credentials only (ADR-0010), so this store holds nothing and \
     refuses everything",
    .grant.grant_type()
)]
pub struct UnservedGrantError {
    /// Which of the three grants the caller wanted storage for.
    grant: UnservedGrant,
    /// The trait method that was called, so the log names the call site
    /// rather than only the grant — `store_code` reaching this means
    /// something issued a code, which is a different defect from
    /// `consume_code` reaching it.
    method: &'static str,
}

impl UnservedGrantError {
    /// Names the grant and the trait method that reached a refusing store.
    ///
    /// `pub` so that a test — or a future caller that wires one of these
    /// stores somewhere else — can construct the same error rather than
    /// matching on its rendered text.
    #[must_use]
    pub const fn new(grant: UnservedGrant, method: &'static str) -> Self {
        Self { grant, method }
    }

    /// The grant this refusal is about.
    #[must_use]
    pub const fn grant(&self) -> UnservedGrant {
        self.grant
    }
}

impl Classify for UnservedGrantError {
    /// [`Category::NotImplemented`], and deliberately not
    /// [`Category::Internal`].
    ///
    /// Both would answer a caller sensibly, but they say different things to
    /// the operator reading `severity`/`retry`: `Internal` claims vpay broke,
    /// while what actually happened is that a capability vpay has never
    /// offered was asked for. `NotImplemented` is the same answer
    /// `ProviderError::NotImplemented` gives for a rail operation vpay has
    /// not built, and it is the category AGENTS.md rule 2 expects a declared
    /// gap to carry — the three grants are listed in `docs/status.md` for
    /// exactly that reason.
    fn category(&self) -> Category {
        Category::NotImplemented
    }
}

/// The `OpError` handed back to `authkestra_op`, after logging the vpay-side
/// error that explains it.
///
/// `OpError::GrantTypeNotPermitted` rather than `OpError::Storage`: nothing
/// is wrong with any storage, and a log reader who sees "storage error"
/// starts checking Postgres. It makes no difference to the caller — every one
/// of the three grant handlers maps *any* `Err` from a store to
/// `server_error` without inspecting it — so the variant is chosen for the
/// operator, and the operator's real information is the `tracing::error!`
/// below, which carries [`UnservedGrantError`]'s whole `Display`.
///
/// `error!` and not `warn!`: [`Category::NotImplemented`]'s default severity
/// is what the classification says, and this line is only ever emitted on a
/// path the module doc argues is unreachable. If it appears in a log, an
/// invariant of the OP assembly has been broken and somebody should be woken
/// up.
fn refuse(grant: UnservedGrant, method: &'static str) -> OpError {
    let error = UnservedGrantError::new(grant, method);
    tracing::error!(grant_type = grant.grant_type(), method, "{error}");
    OpError::GrantTypeNotPermitted
}

/// The `AuthorizationCodeStore` slot: refuses to issue or spend a code.
///
/// ```
/// use authkestra_op::code::AuthorizationCodeStore;
/// use vpay_api::op::refusing_stores::RefusingAuthorizationCodeStore;
///
/// let runtime = tokio::runtime::Runtime::new().expect("a Tokio runtime starts");
/// let answer = runtime.block_on(RefusingAuthorizationCodeStore.consume_code("anything"));
///
/// // Not `Ok(None)`: "no such code" is a thing a real store says, and a
/// // caller treats it as a bad request. This store has no codes to not-find.
/// assert!(answer.is_err());
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct RefusingAuthorizationCodeStore;

#[async_trait]
impl AuthorizationCodeStore for RefusingAuthorizationCodeStore {
    async fn store_code(&self, _code: AuthorizationCode) -> Result<(), OpError> {
        Err(refuse(UnservedGrant::AuthorizationCode, "store_code"))
    }

    async fn consume_code(&self, _code: &str) -> Result<Option<AuthorizationCode>, OpError> {
        Err(refuse(UnservedGrant::AuthorizationCode, "consume_code"))
    }
}

/// The `RefreshTokenStore` slot: refuses to issue, read, spend or revoke a
/// refresh token.
///
/// `revoke_token` refuses too, rather than answering `Ok(())` on the grounds
/// that a token that was never stored is already revoked. That reading is
/// true and still the wrong answer: `Ok(())` from a revocation is a *promise*
/// that a credential can no longer be used, and this store is in no position
/// to make one.
///
/// ```
/// use authkestra_op::refresh::RefreshTokenStore;
/// use vpay_api::op::refusing_stores::RefusingRefreshTokenStore;
///
/// let runtime = tokio::runtime::Runtime::new().expect("a Tokio runtime starts");
/// assert!(runtime.block_on(RefusingRefreshTokenStore.revoke_token("anything")).is_err());
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct RefusingRefreshTokenStore;

#[async_trait]
impl RefreshTokenStore for RefusingRefreshTokenStore {
    async fn store_token(&self, _token: RefreshToken) -> Result<(), OpError> {
        Err(refuse(UnservedGrant::RefreshToken, "store_token"))
    }

    async fn get_token(&self, _token: &str) -> Result<Option<RefreshToken>, OpError> {
        Err(refuse(UnservedGrant::RefreshToken, "get_token"))
    }

    async fn revoke_token(&self, _token: &str) -> Result<(), OpError> {
        Err(refuse(UnservedGrant::RefreshToken, "revoke_token"))
    }

    async fn consume_token(&self, _token: &str) -> Result<Option<RefreshToken>, OpError> {
        Err(refuse(UnservedGrant::RefreshToken, "consume_token"))
    }
}

/// The `DeviceCodeStore` slot: refuses every step of RFC 8628.
///
/// ```
/// use authkestra_op::device::DeviceCodeStore;
/// use vpay_api::op::refusing_stores::RefusingDeviceCodeStore;
///
/// let runtime = tokio::runtime::Runtime::new().expect("a Tokio runtime starts");
/// let answer = runtime.block_on(RefusingDeviceCodeStore.get_by_user_code("WDJB-MJHT"));
/// assert!(answer.is_err());
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct RefusingDeviceCodeStore;

#[async_trait]
impl DeviceCodeStore for RefusingDeviceCodeStore {
    async fn store_device_code(&self, _session: DeviceCodeSession) -> Result<(), OpError> {
        Err(refuse(UnservedGrant::DeviceCode, "store_device_code"))
    }

    async fn get_device_code(
        &self,
        _device_code: &str,
    ) -> Result<Option<DeviceCodeSession>, OpError> {
        Err(refuse(UnservedGrant::DeviceCode, "get_device_code"))
    }

    async fn get_by_user_code(
        &self,
        _user_code: &str,
    ) -> Result<Option<DeviceCodeSession>, OpError> {
        Err(refuse(UnservedGrant::DeviceCode, "get_by_user_code"))
    }

    async fn update_device_code(&self, _session: DeviceCodeSession) -> Result<(), OpError> {
        Err(refuse(UnservedGrant::DeviceCode, "update_device_code"))
    }

    async fn delete_device_code(&self, _device_code: &str) -> Result<(), OpError> {
        Err(refuse(UnservedGrant::DeviceCode, "delete_device_code"))
    }

    async fn consume_device_code(
        &self,
        _device_code: &str,
    ) -> Result<Option<DeviceCodeSession>, OpError> {
        Err(refuse(UnservedGrant::DeviceCode, "consume_device_code"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of the twelve methods refuses, and none of them answers
    /// `Ok(None)`.
    ///
    /// The distinction is the whole point of the type. `Ok(None)` is what a
    /// *working* store says about a code it does not hold, and every caller
    /// in `authkestra_op` turns it into `invalid_grant` — a 400 telling the
    /// client its code was wrong. A store that cannot hold anything must not
    /// be able to produce that answer, or "vpay does not serve this grant"
    /// becomes indistinguishable from "you sent a bad code".
    ///
    /// Written out method by method rather than through a loop: the twelve
    /// signatures differ, and a helper that hid them would also hide a
    /// thirteenth being added without a refusal.
    #[tokio::test]
    async fn every_method_of_every_slot_refuses() {
        assert!(
            RefusingAuthorizationCodeStore
                .consume_code("c")
                .await
                .is_err()
        );
        assert!(
            RefusingAuthorizationCodeStore
                .store_code(sample_code())
                .await
                .is_err()
        );

        assert!(RefusingRefreshTokenStore.get_token("t").await.is_err());
        assert!(RefusingRefreshTokenStore.consume_token("t").await.is_err());
        assert!(RefusingRefreshTokenStore.revoke_token("t").await.is_err());
        assert!(
            RefusingRefreshTokenStore
                .store_token(sample_refresh_token())
                .await
                .is_err()
        );

        assert!(RefusingDeviceCodeStore.get_device_code("d").await.is_err());
        assert!(RefusingDeviceCodeStore.get_by_user_code("u").await.is_err());
        assert!(
            RefusingDeviceCodeStore
                .delete_device_code("d")
                .await
                .is_err()
        );
        assert!(
            RefusingDeviceCodeStore
                .consume_device_code("d")
                .await
                .is_err()
        );
        assert!(
            RefusingDeviceCodeStore
                .store_device_code(sample_device_session())
                .await
                .is_err()
        );
        assert!(
            RefusingDeviceCodeStore
                .update_device_code(sample_device_session())
                .await
                .is_err()
        );
    }

    /// The message an operator gets names the grant by its wire spelling and
    /// the method by its trait name.
    ///
    /// Asserted against the literal `grant_type` strings rather than against
    /// `UnservedGrant::grant_type`, so a typo in that function fails here
    /// instead of agreeing with itself. The device-code URN in particular is
    /// the one a `Debug`-derived name would silently get wrong.
    #[test]
    fn the_refusal_names_the_grant_on_the_wire_and_the_method_that_asked() {
        let error = UnservedGrantError::new(UnservedGrant::AuthorizationCode, "consume_code");
        let text = error.to_string();
        assert!(text.contains("authorization_code"), "{text}");
        assert!(text.contains("consume_code"), "{text}");
        assert!(text.contains("does not serve"), "{text}");

        assert!(
            UnservedGrantError::new(UnservedGrant::DeviceCode, "get_device_code")
                .to_string()
                .contains("urn:ietf:params:oauth:grant-type:device_code")
        );
        assert!(
            UnservedGrantError::new(UnservedGrant::RefreshToken, "get_token")
                .to_string()
                .contains("refresh_token")
        );
    }

    /// Nothing from the operator's message reaches a caller.
    ///
    /// `public_message` is the only thing `vpay_api` ever renders into a
    /// response body, and ADR-0011 forbids it carrying anything from the
    /// `Display`/`source` side. Here that is not merely tidiness: the
    /// `Display` names an internal trait method.
    #[test]
    fn the_public_message_leaks_neither_the_grant_nor_the_method() {
        for grant in UnservedGrant::ALL {
            let error = UnservedGrantError::new(grant, "consume_code");
            assert_eq!(error.category(), Category::NotImplemented);
            assert_eq!(
                error.public_message(),
                Category::NotImplemented.generic_message()
            );
            assert!(!error.public_message().contains("consume_code"));
            assert!(!error.public_message().contains(grant.grant_type()));
        }
    }

    /// The three wire spellings, pinned as literals.
    ///
    /// These are the strings `handle_token` matches on
    /// (`authkestra-op-0.7.1/src/handlers/token.rs`'s
    /// `match req.grant_type.as_str()`), and the ones
    /// `merchant_token_flow.rs` case (i) posts. A drift here would make that
    /// integration test exercise a grant nothing dispatches — it would still
    /// pass, because an unknown grant is also refused, and it would stop
    /// proving anything.
    #[test]
    fn the_grant_type_strings_are_the_ones_authkestra_dispatches_on() {
        assert_eq!(
            UnservedGrant::ALL.map(UnservedGrant::grant_type),
            [
                "authorization_code",
                "refresh_token",
                "urn:ietf:params:oauth:grant-type:device_code",
            ]
        );
    }

    fn sample_code() -> AuthorizationCode {
        AuthorizationCode::new(
            "code".to_owned(),
            "acme".to_owned(),
            "https://merchant.example/cb".to_owned(),
            "payments:write".to_owned(),
            identity(),
            chrono::Utc::now(),
            false,
        )
    }

    fn sample_refresh_token() -> RefreshToken {
        RefreshToken::new(
            "token".to_owned(),
            "acme".to_owned(),
            identity(),
            "payments:write".to_owned(),
            chrono::Utc::now(),
            // `jkt`: the RFC 9449 DPoP key thumbprint a sender-constrained
            // token would carry. `None` — `/v1` mounts no DPoP, and this
            // token is only ever handed to a method that refuses it.
            None,
        )
    }

    fn sample_device_session() -> DeviceCodeSession {
        DeviceCodeSession::new(
            "device".to_owned(),
            "WDJB-MJHT".to_owned(),
            "acme".to_owned(),
            "payments:write".to_owned(),
            chrono::Utc::now(),
            authkestra_op::device::DeviceCodeStatus::Pending,
        )
    }

    fn identity() -> authkestra_engine::auth::state::Identity {
        authkestra_engine::auth::state::Identity {
            provider_id: "vpay".to_owned(),
            external_id: "nobody".to_owned(),
            email: None,
            username: None,
            attributes: std::collections::HashMap::new(),
        }
    }
}
