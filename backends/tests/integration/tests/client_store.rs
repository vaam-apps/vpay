//! The two halves of merchant client resolution that only a real database can
//! prove: `vpay_api::op::clients::YamlClientStore`'s kill-switch interception,
//! and `vpay_db::delete_expired_client_assertion_jtis`'s sweep.
//!
//! `YamlClientStore`'s own unit tests (`vpay-api/src/op/clients.rs`) cover the
//! `MerchantClient` → `ClientRegistration` conversion and prove an unknown
//! `client_id` is answered without touching Postgres. What they cannot cover
//! is the part that matters operationally: that flipping a row in
//! `disabled_clients` actually makes `find_client` stop returning a client
//! that YAML still declares, and that flipping it back restores access
//! (ADR-0010: "an operator flips a client to disabled and it takes effect
//! immediately, no deploy required"). That is a claim about two crates and a
//! table agreeing, so it is asserted against the real table.
//!
//! The pool-and-migrate helper is `tests/support/mod.rs`'s — Step 2
//! introduced the shared module the older comment here said was not worth
//! introducing, because `RouterDeps` and boot step 4 made the shared surface
//! more than a handful of lines. The container start underneath it is
//! `vpay_testkit::containers::start_postgres_with_retry`.

use anyhow::Context;
use authkestra_op::client::ClientStore;
use authkestra_op::client_assertion::ClientAssertionStore;
use chrono::{Duration as ChronoDuration, Utc};
use vpay_api::op::clients::YamlClientStore;
use vpay_config::MERCHANT_AUDIENCE;
use vpay_config::oauth::{GrantType, MerchantClient};
use vpay_db::SqlClientAssertionStore;

mod support;

use support::migrated_postgres;

const CLIENT_ID: &str = "acme-cameroon";

/// The tenant `CLIENT_ID` acts for. Deliberately not equal to the
/// `client_id` — see `MerchantClient::merchant_id`.
const MERCHANT_ID: &str = "acme-cameroon-tenant";

/// A merchant registration shaped exactly as `config/application.yml`'s is,
/// including the `vpay:v1` audience `Config::validate_all` now requires — a
/// fixture that could not load from YAML would prove nothing about the real
/// path. The JWK modulus is a placeholder: nothing in this file verifies a
/// signature (`vpay-api`'s own tests do that against real key material), only
/// whether the client resolves at all.
fn configured_merchant() -> MerchantClient {
    MerchantClient {
        client_id: CLIENT_ID.to_owned(),
        merchant_id: MERCHANT_ID.to_owned(),
        jwks: Some(serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "kid": "acme-cameroon-2026-08",
                "alg": "RS256",
                "n": "placeholder-rsa-modulus",
                "e": "AQAB",
            }]
        })),
        grant_types: vec![GrantType::ClientCredentials],
        scopes: vec!["payments:write".to_owned()],
        allowed_audiences: vec![MERCHANT_AUDIENCE.to_owned()],
        client_secret: None,
    }
}

/// The kill switch's whole point, end to end: a configured client resolves,
/// an unconfigured one does not, disabling one takes effect on the next
/// lookup with no restart, and re-enabling restores it.
///
/// All four assertions live in one test on purpose — they are one state
/// machine observed at four points, and splitting them would start four
/// Postgres containers to prove less (`.config/nextest.toml` serializes
/// container starts, so each one is wall-clock time this suite pays for).
#[tokio::test]
async fn find_client_reflects_the_disabled_clients_kill_switch() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    let store = YamlClientStore::new(&[configured_merchant()], pool.clone());

    let found = store
        .find_client(CLIENT_ID)
        .await
        .map_err(|e| anyhow::anyhow!("find_client failed: {e}"))?
        .context("a configured, non-disabled client must resolve")?;
    assert_eq!(found.client_id, CLIENT_ID);

    let unknown = store
        .find_client("not-in-yaml")
        .await
        .map_err(|e| anyhow::anyhow!("find_client failed: {e}"))?;
    assert!(
        unknown.is_none(),
        "a client_id absent from YAML must never resolve, disabled or not"
    );

    vpay_db::disable_client(&pool, CLIENT_ID, Some("key compromised, ticket INC-123"))
        .await
        .context("disabling the client")?;

    let after_disable = store
        .find_client(CLIENT_ID)
        .await
        .map_err(|e| anyhow::anyhow!("find_client failed: {e}"))?;
    assert!(
        after_disable.is_none(),
        "a disabled client must stop resolving immediately, with no restart and no config change"
    );

    vpay_db::enable_client(&pool, CLIENT_ID)
        .await
        .context("re-enabling the client")?;

    let after_enable = store
        .find_client(CLIENT_ID)
        .await
        .map_err(|e| anyhow::anyhow!("find_client failed: {e}"))?;
    assert!(
        after_enable.is_some(),
        "re-enabling must restore access; the table only ever subtracts (ADR-0010)"
    );

    Ok(())
}

/// The sweep removes what has expired and keeps what has not — asserted by
/// reading the rows back, not by trusting the returned count alone, so a
/// `DELETE` with a wrong or missing `WHERE` would fail here rather than
/// report a plausible number.
#[tokio::test]
async fn expired_client_assertion_jtis_are_swept_and_live_ones_are_kept() -> anyhow::Result<()> {
    let (_container, pool) = migrated_postgres().await?;
    let store = SqlClientAssertionStore::new(pool.clone());

    // Recorded through the real store rather than a hand-written INSERT, so
    // the sweep is proven against rows in exactly the shape the production
    // write path produces.
    let expired_is_fresh = store
        .record_jti("expired-jti", Utc::now() - ChronoDuration::hours(1))
        .await
        .map_err(|e| anyhow::anyhow!("record_jti failed: {e}"))?;
    let live_is_fresh = store
        .record_jti("live-jti", Utc::now() + ChronoDuration::minutes(5))
        .await
        .map_err(|e| anyhow::anyhow!("record_jti failed: {e}"))?;
    assert!(expired_is_fresh && live_is_fresh, "both jtis are first use");

    let deleted = vpay_db::delete_expired_client_assertion_jtis(&pool)
        .await
        .context("sweeping expired jtis")?;
    assert_eq!(deleted, 1, "exactly the expired row is deleted");

    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT jti FROM oauth_client_assertion_jtis ORDER BY jti")
            .fetch_all(&pool)
            .await
            .context("reading the surviving rows back")?;
    assert_eq!(remaining, vec!["live-jti".to_owned()]);

    // Idempotent: a second sweep with nothing expired deletes nothing. A
    // boot-time stopgap runs on every restart, so "no rows" must be an
    // ordinary outcome, not an error.
    let deleted_again = vpay_db::delete_expired_client_assertion_jtis(&pool)
        .await
        .context("second sweep")?;
    assert_eq!(deleted_again, 0);

    Ok(())
}
