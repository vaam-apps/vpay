//! Proves `authkestra_op::sqlx_store::SqlxOpStore<Postgres>` genuinely works
//! against `backends/migrations/0006_create-authkestra-op-tables.sql` — the
//! transcribed copy of that store's own hardcoded DDL.
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
//! Duplicates the small container-bootstrap helper from
//! `tests/postgres_smoke.rs` rather than sharing it — each `tests/*.rs` file
//! compiles to its own test binary, so there is no `pub` item to import
//! without introducing a `tests/common/mod.rs` shared module; the duplication
//! is a handful of lines and keeps this file independently readable.

use std::collections::HashMap;

use anyhow::Context;
use authkestra_engine::auth::state::Identity;
use authkestra_op::sqlx_store::SqlxOpStore;
use authkestra_op::{AuthorizationCode, AuthorizationCodeStore, ClientStore};
use chrono::{Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// Same rationale and pin as `tests/postgres_smoke.rs`: `testcontainers-modules`
/// defaults to `postgres:11-alpine`, which is not cached here and this
/// machine cannot reach Docker Hub to pull it. `16-alpine` is cached and
/// matches `compose.yml`.
async fn migrated_postgres() -> anyhow::Result<(ContainerAsync<PostgresImage>, PgPool)> {
    let container = PostgresImage::default()
        .with_tag("16-alpine")
        .start()
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
async fn insert_dashboard_client(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO authkestra.oauth_clients \
            (client_id, client_secret_hash, require_pkce, redirect_uris, grant_types, scopes, allowed_audiences) \
         VALUES \
            ('vpay_dashboard', NULL, true, \
             '[\"https://dash.vpay.test/callback\"]'::jsonb, \
             '[\"authorization_code\"]'::jsonb, \
             '[\"openid\", \"profile\"]'::jsonb, \
             '[]'::jsonb)",
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
    assert!(
        client.require_pkce,
        "PKCE is mandatory for the dashboard client (docs/flows/dashboard-auth.md)"
    );
    assert_eq!(
        client.redirect_uris,
        vec!["https://dash.vpay.test/callback".to_string()],
        "redirect_uris JSONB must decode back to exactly what was stored — redirect_uris are matched exactly, never by prefix"
    );
    assert_eq!(
        client.scopes,
        vec!["openid".to_string(), "profile".to_string()]
    );

    // --- 2 & 3. store_code / consume_code single-use. ---
    let code_value = "authz_code_test_123".to_string();
    let code = AuthorizationCode {
        code: code_value.clone(),
        client_id: "vpay_dashboard".to_string(),
        redirect_uri: "https://dash.vpay.test/callback".to_string(),
        scope: "openid profile".to_string(),
        code_challenge: Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_string()),
        code_challenge_method: Some("S256".to_string()),
        nonce: None,
        identity: test_identity(),
        expires_at: Utc::now() + ChronoDuration::seconds(60),
        used: false,
    };

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
