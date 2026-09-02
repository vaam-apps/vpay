//! Proves `authkestra_op::sqlx_store::SqlxOpStore<Postgres>` genuinely works
//! against `backends/migrations/0006_create-authkestra-op-tables.sql` plus
//! `0013_add-authkestra-op-0-7-columns.sql` — the transcribed copies of that
//! store's own hardcoded DDL at 0.3.4 and the additive delta at 0.7.1.
//!
//! This is the strongest available check for migration 0006's whole risk:
//! that DDL is not a design we control, it is a byte-for-byte transcription
//! of string literals baked into a pinned dependency, so a mismatched column
//! name or type would compile cleanly and only fail at runtime. Running our
//! migrations against a real Postgres and then exercising the store's own
//! `find_client`/`store_code`/`consume_code` methods end to end is the only
//! way to actually observe that the transcription is correct, rather than
//! merely re-reading the SQL and asserting it looks right (CLAUDE.md: "Reading
//! SQL and asserting it is correct fails this task").
//!
//! Repeats the small pool-and-migrate helper from `tests/postgres_smoke.rs`
//! rather than sharing it — each `tests/*.rs` file compiles to its own test
//! binary, so there is no `pub` item to import without introducing a
//! `tests/common/mod.rs` shared module; the duplication is a handful of lines
//! and keeps this file independently readable. The container start it wraps
//! *is* shared: `vpay_testkit::containers::start_postgres_with_retry`.

use std::collections::HashMap;

use anyhow::Context;
use authkestra_engine::auth::state::Identity;
use authkestra_op::client::TokenEndpointAuthMethod;
use authkestra_op::refresh::{RefreshToken, RefreshTokenStore};
use authkestra_op::sqlx_store::SqlxOpStore;
use authkestra_op::{AuthorizationCode, AuthorizationCodeStore, ClientStore, OpStore};
use chrono::{Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// Same as `tests/postgres_smoke.rs`'s: the container itself comes from
/// `vpay_testkit::containers::start_postgres_with_retry` (why the tag is
/// pinned, and which start errors are retried, are documented there); what
/// stays per-file is the pool and the migration run.
async fn migrated_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool)> {
    let container = vpay_testkit::containers::start_postgres_with_retry()
        .await
        .context("postgres:16-alpine container starts (it is cached locally on this machine)")?;

    let host = container.get_host().await.context("container host")?;
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .context("container port")?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPool::connect(&url)
        .await
        .context("connects to the freshly started container")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("every migration under backends/migrations applies cleanly")?;

    Ok((container, pool))
}

/// Inserts a client row directly (no insert method exists on `ClientStore` —
/// only `find_client`; registration is expected to happen out of band, e.g.
/// via vpay configuration per docs/flows/dashboard-auth.md), matching every
/// column `SqlxOpStore::find_client`'s `SELECT` names.
///
/// `token_endpoint_auth_method` and `jwks` are the two columns migration 0013
/// adds for authkestra-op 0.7.1 (authkestra#287). They are populated here —
/// even though the dashboard client itself is a public PKCE client that would
/// register `"none"`/no keys in real life — precisely so the round trip
/// below proves the store *reads* them back through its own JSONB decoding,
/// rather than silently returning `None` for a column it never selected.
async fn insert_dashboard_client(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO authkestra.oauth_clients \
            (client_id, client_secret_hash, require_pkce, redirect_uris, grant_types, scopes, \
             allowed_audiences, token_endpoint_auth_method, jwks) \
         VALUES \
            ('vpay_dashboard', NULL, true, \
             '[\"https://dash.vpay.test/callback\"]'::jsonb, \
             '[\"authorization_code\"]'::jsonb, \
             '[\"openid\", \"profile\"]'::jsonb, \
             '[]'::jsonb, \
             '\"private_key_jwt\"'::jsonb, \
             '{\"keys\": [{\"kty\": \"RSA\", \"kid\": \"k1\", \"n\": \"AQAB\", \"e\": \"AQAB\"}]}'::jsonb)",
    )
    .execute(pool)
    .await
    .context("inserting the dashboard's own client registration")?;
    Ok(())
}

fn test_identity() -> Identity {
    Identity {
        provider_id: "vpay-dashboard".to_string(),
        external_id: "staff_1".to_string(),
        email: Some("staff@vpay.test".to_string()),
        username: Some("staff".to_string()),
        attributes: HashMap::new(),
    }
}

/// End-to-end proof that migration 0006's transcribed DDL is genuinely
/// compatible with `SqlxOpStore<Postgres>`:
///
/// 1. `find_client` reads back a row inserted with exactly the columns
///    `oauth_clients` declares, including decoding the JSONB
///    `redirect_uris`/`grant_types`/`scopes`/`allowed_audiences` columns into
///    their typed Rust shapes.
/// 2. `store_code` writes an `AuthorizationCode` (including its `identity`
///    JSONB column) and `consume_code` reads it back atomically.
/// 3. A second `consume_code` of the same code must return `None` — proving
///    the crate's own single-use enforcement
///    (`UPDATE ... SET used = TRUE WHERE code = $1 AND used = FALSE RETURNING
///    *`) actually fires against our schema, not just that it parses.
#[tokio::test]
async fn sqlx_op_store_round_trips_a_client_and_enforces_single_use_codes() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    insert_dashboard_client(&pool).await?;

    let store = SqlxOpStore::<sqlx::Postgres>::new(pool.clone());

    // --- 1. find_client reads back what we inserted, through the store's
    // own SELECT and JSONB decoding, not a query we wrote ourselves. ---
    let client = ClientStore::find_client(&store, "vpay_dashboard")
        .await
        .map_err(|e| anyhow::anyhow!("find_client: {e}"))?
        .context("the client row we just inserted must be found")?;

    assert_eq!(client.client_id, "vpay_dashboard");
    assert!(
        client.client_secret_hash.is_none(),
        "public client, no secret"
    );
    // `require_pkce` is deprecated as of authkestra-op 0.7.0 (authkestra#273):
    // PKCE is mandatory for every client on the authorization code grant,
    // unconditionally, and the field is no longer read by any handler. The
    // column still exists (the store still SELECTs it), but asserting on it
    // would only prove a value nothing consults — so nothing here does.
    assert_eq!(
        client.redirect_uris,
        vec!["https://dash.vpay.test/callback".to_string()],
        "redirect_uris JSONB must decode back to exactly what was stored — redirect_uris are matched exactly, never by prefix"
    );
    assert_eq!(
        client.scopes,
        vec!["openid".to_string(), "profile".to_string()]
    );
    // Migration 0013's two `oauth_clients` columns, decoded by the 0.7.1
    // store's own `try_get::<Option<Json<_>>>` — at 0.3.4 `find_client`
    // hardcoded both to `None` (the premise of ADR-0010's context section).
    assert_eq!(
        client.token_endpoint_auth_method,
        Some(TokenEndpointAuthMethod::PrivateKeyJwt),
        "token_endpoint_auth_method JSONB must decode through the store's own type"
    );
    assert_eq!(
        client
            .jwks
            .as_ref()
            .and_then(|jwks| jwks.pointer("/keys/0/kid"))
            .and_then(|kid| kid.as_str()),
        Some("k1"),
        "jwks JSONB must round-trip as the raw JSON the OP re-validates on every use"
    );

    // --- 2 & 3. store_code / consume_code single-use. ---
    let code_value = "authz_code_test_123".to_string();
    // `AuthorizationCode` became `#[non_exhaustive]` in authkestra-op 0.6.0
    // (authkestra#259), so struct-literal construction from outside the
    // crate no longer compiles; `new` (authkestra#268) is the seam its own
    // store implementations use. `used: false` is a required argument by
    // design — see the constructor's doc comment for why it never defaults.
    let mut code = AuthorizationCode::new(
        code_value.clone(),
        "vpay_dashboard".to_string(),
        "https://dash.vpay.test/callback".to_string(),
        "openid profile".to_string(),
        test_identity(),
        Utc::now() + ChronoDuration::seconds(60),
        false,
    );
    code.code_challenge = Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_string());
    code.code_challenge_method = Some("S256".to_string());

    AuthorizationCodeStore::store_code(&store, code)
        .await
        .map_err(|e| anyhow::anyhow!("store_code: {e}"))?;

    let consumed = AuthorizationCodeStore::consume_code(&store, &code_value)
        .await
        .map_err(|e| anyhow::anyhow!("first consume_code: {e}"))?
        .context("the first consume of a freshly stored code must succeed")?;
    assert_eq!(consumed.code, code_value);
    assert_eq!(consumed.client_id, "vpay_dashboard");
    assert_eq!(
        consumed.identity.external_id, "staff_1",
        "the identity JSONB column must round-trip through consume_code's RETURNING *"
    );

    let second_consume = AuthorizationCodeStore::consume_code(&store, &code_value)
        .await
        .map_err(|e| anyhow::anyhow!("second consume_code: {e}"))?;
    assert!(
        second_consume.is_none(),
        "a second consume of an already-used code must return None — this is the whole \
         single-use guarantee AuthorizationCodeStore::consume_code documents, and the reason \
         its Postgres implementation is an atomic `UPDATE ... WHERE used = FALSE RETURNING *` \
         rather than a separate SELECT then UPDATE"
    );

    Ok(())
}

/// Migration 0013's `oauth_refresh_tokens.jkt` column, proven through the
/// store's own `store_token` INSERT and `get_token` SELECT — both of which
/// name `jkt` unconditionally at 0.7.1, so this test fails against 0006's
/// table alone with a "column does not exist" error rather than passing
/// vacuously. vpay's dashboard flow issues no refresh token at all
/// (docs/flows/dashboard-auth.md) and offers no DPoP-bound grant; this is
/// schema compatibility with the pinned crate, not a feature claim.
#[tokio::test]
async fn sqlx_op_store_round_trips_a_refresh_token_with_its_jkt_column() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    insert_dashboard_client(&pool).await?;
    let store = SqlxOpStore::<sqlx::Postgres>::new(pool.clone());

    let token = RefreshToken::new(
        "rt_test_0013".to_string(),
        "vpay_dashboard".to_string(),
        test_identity(),
        "openid".to_string(),
        Utc::now() + ChronoDuration::seconds(60),
        Some("jkt-thumbprint-0013".to_string()),
    );
    RefreshTokenStore::store_token(&store, token)
        .await
        .map_err(|e| anyhow::anyhow!("store_token: {e}"))?;

    let fetched = RefreshTokenStore::get_token(&store, "rt_test_0013")
        .await
        .map_err(|e| anyhow::anyhow!("get_token: {e}"))?
        .context("the refresh token just stored must be readable back")?;
    assert_eq!(fetched.client_id, "vpay_dashboard");
    assert_eq!(
        fetched.jkt,
        Some("jkt-thumbprint-0013".to_string()),
        "jkt must round-trip through the store's own INSERT/SELECT of the 0013 column"
    );

    Ok(())
}

/// Migration 0013's `authkestra.oauth_dpop_jti` table, proven through the
/// store's own `check_and_record_dpop_jti` override — a single
/// `INSERT ... ON CONFLICT (jti) DO UPDATE ... WHERE expires_at <= now()`
/// against that exact table. First claim of a fresh `jti` must succeed and a
/// second, unexpired presentation must be refused: the replay guard actually
/// firing against this schema, not just DDL that parses.
#[tokio::test]
async fn sqlx_op_store_records_a_dpop_jti_once_against_migration_0013s_table() -> anyhow::Result<()>
{
    let (_container, pool) = migrated_postgres().await?;
    let store = SqlxOpStore::<sqlx::Postgres>::new(pool.clone());
    let expires_at = Utc::now() + ChronoDuration::seconds(120);

    let first = OpStore::check_and_record_dpop_jti(&store, "dpop_jti_0013", expires_at)
        .await
        .map_err(|e| anyhow::anyhow!("first check_and_record_dpop_jti: {e}"))?;
    assert!(first, "a never-seen jti must be accepted");

    let replay = OpStore::check_and_record_dpop_jti(&store, "dpop_jti_0013", expires_at)
        .await
        .map_err(|e| anyhow::anyhow!("second check_and_record_dpop_jti: {e}"))?;
    assert!(
        !replay,
        "an unexpired jti presented again must be refused — the ON CONFLICT ... WHERE \
         expires_at <= now() clause is what makes the table a replay guard"
    );

    Ok(())
}
