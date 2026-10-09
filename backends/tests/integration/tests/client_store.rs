//! The two halves of merchant client resolution that only a real database can
//! prove: `vpay_api::op::clients::YamlClientStore`'s kill-switch interception,
//! and `vpay_db::ClientAssertions::delete_expired_client_assertion_jtis`'s
//! sweep, including how long after its `exp` a spent `jti` is kept
//! ([ADR-0028](../../../../docs/adr/0028-a-spent-jti-outlives-the-validators-leeway.md)).
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
use std::sync::Arc;
use vpay_api::op::clients::YamlClientStore;
use vpay_config::MERCHANT_AUDIENCE;
use vpay_config::oauth::{GrantType, MerchantClient};

mod support;

use support::{db_now, migrated_postgres};

/// The horizon the worker's hourly sweep passes — the production constant,
/// not a copy of its value, so a change to it moves these cases with it.
const RETENTION: std::time::Duration = vpay_worker::CLIENT_ASSERTION_JTI_RETENTION;

/// `instant`, which `db_now` read off Postgres, as the `chrono` type
/// `ClientAssertionStore::record_jti` takes.
///
/// Fixtures that decide whether a row is past a horizon the **database**
/// judges (`expires_at < now() - retention`) are positioned relative to the
/// database's `now()`, not this process's: a testcontainer's clock is not
/// this host's (`support::db_now`).
fn as_chrono(instant: time::OffsetDateTime) -> anyhow::Result<chrono::DateTime<Utc>> {
    chrono::DateTime::from_timestamp(instant.unix_timestamp(), instant.nanosecond())
        .context("a database instant is within chrono's range")
}

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
        // Nothing the OP does renders a name to anyone.
        display_name: None,
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
        // The client store answers authentication questions; a delivery
        // destination is not one, and nothing in the conversion to
        // `ClientRegistration` may ever read this.
        webhooks: Vec::new(),
        // And neither is a publishable key: it names a tenant for a payer's
        // browser, never a credential this store resolves.
        publishable_keys: Vec::new(),
        checkout_origins: Vec::new(),
        // Nor is a forwarding URL: where a payer lands after paying an
        // invoice is not a credential this store resolves.
        invoices: vpay_config::InvoiceDefaults::default(),
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
    let (_container, repositories, _pool) = migrated_postgres().await?;
    let store = YamlClientStore::new(&[configured_merchant()], Arc::clone(&repositories));

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

    repositories
        .disable_client(CLIENT_ID, Some("key compromised, ticket INC-123"))
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

    repositories
        .enable_client(CLIENT_ID)
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
    let (_container, repositories, pool) = migrated_postgres().await?;
    let store = vpay_db::client_assertion_store(repositories.op_store_pool());
    let now = as_chrono(db_now(&pool).await?)?;

    // Recorded through the real store rather than a hand-written INSERT, so
    // the sweep is proven against rows in exactly the shape the production
    // write path produces.
    let expired_is_fresh = store
        .record_jti("expired-jti", now - ChronoDuration::hours(1))
        .await
        .map_err(|e| anyhow::anyhow!("record_jti failed: {e}"))?;
    let live_is_fresh = store
        .record_jti("live-jti", now + ChronoDuration::minutes(5))
        .await
        .map_err(|e| anyhow::anyhow!("record_jti failed: {e}"))?;
    assert!(expired_is_fresh && live_is_fresh, "both jtis are first use");

    let deleted = repositories
        .delete_expired_client_assertion_jtis(RETENTION)
        .await
        .context("sweeping expired jtis")?;
    assert_eq!(deleted, 1, "exactly the expired row is deleted");

    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT jti FROM oauth_client_assertion_jtis ORDER BY jti")
            .fetch_all(&pool)
            .await
            .context("reading the surviving rows back")?;
    assert_eq!(remaining, vec!["live-jti".to_owned()]);

    // Idempotent: a second sweep with nothing expired deletes nothing. The
    // sweep runs hourly for the life of the deployment and finds nothing most
    // hours, so "no rows" must be an ordinary outcome, not an error.
    let deleted_again = repositories
        .delete_expired_client_assertion_jtis(RETENTION)
        .await
        .context("second sweep")?;
    assert_eq!(deleted_again, 0);

    Ok(())
}

/// A spent `jti` survives the sweep while the validator would still accept
/// its assertion, and goes once it is older than the horizon — asserted on
/// the **database's** clock, by reading the rows back.
///
/// This is the cut the whole of ADR-0028 is about. `expires_at` is the
/// client's own `exp`, and `authkestra-op` 0.7.1 accepts an assertion for a
/// further 60 seconds past it (`jsonwebtoken`'s default leeway), plus the
/// second its whole-second `now` truncates away. A sweep that deleted at
/// `expires_at < now()` removed a `jti` that could still be presented, and
/// the same assertion then minted a second token. Three rows on both sides of
/// that line:
///
/// - `exp` 30 s ago: inside the leeway, so the validator still accepts it.
///   Deleting this row is the replay. It must survive.
/// - `exp` 61 s ago: the last second the validator can still accept. Survives.
/// - `exp` one second beyond the production horizon: no validator with the
///   pinned leeway accepts it (`merchant_token_flow.rs` proves that half), so
///   the row is dead weight and must go.
///
/// The horizon is the worker's production constant, so this is a statement
/// about what a deployment does, not about a number written here. Decisive:
/// pass `Duration::ZERO` (the pre-ADR-0028 behaviour) and the first two rows
/// are deleted.
#[tokio::test]
async fn a_spent_jti_survives_the_sweep_while_its_assertion_could_still_verify()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let store = vpay_db::client_assertion_store(repositories.op_store_pool());
    let now = as_chrono(db_now(&pool).await?)?;
    let horizon = ChronoDuration::from_std(RETENTION).context("the horizon fits chrono")?;

    for (jti, exp) in [
        ("exp-30s-ago", now - ChronoDuration::seconds(30)),
        ("exp-61s-ago", now - ChronoDuration::seconds(61)),
        (
            "exp-beyond-horizon",
            now - horizon - ChronoDuration::seconds(1),
        ),
    ] {
        let fresh = store
            .record_jti(jti, exp)
            .await
            .map_err(|e| anyhow::anyhow!("record_jti failed: {e}"))?;
        assert!(fresh, "{jti} is a first use");
    }

    let deleted = repositories
        .delete_expired_client_assertion_jtis(RETENTION)
        .await
        .context("sweeping with the production horizon")?;
    assert_eq!(deleted, 1, "exactly the row older than the horizon goes");

    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT jti FROM oauth_client_assertion_jtis ORDER BY jti")
            .fetch_all(&pool)
            .await
            .context("reading the surviving rows back")?;
    assert_eq!(
        remaining,
        vec!["exp-30s-ago".to_owned(), "exp-61s-ago".to_owned()],
        "a row whose assertion the validator still accepts must survive the sweep"
    );

    // And the surviving rows still do their job: the jti is spent.
    let replay = store
        .record_jti("exp-30s-ago", now + ChronoDuration::minutes(1))
        .await
        .map_err(|e| anyhow::anyhow!("record_jti failed: {e}"))?;
    assert!(!replay, "a jti that survived the sweep is still spent");

    Ok(())
}

/// The horizon is a duration the database subtracts from its own `now()`, so
/// the cut falls where the caller says: a row 20 s inside a 120 s horizon
/// survives and one 20 s outside it goes.
///
/// A short horizon is used on purpose. It shows the parameter is what decides
/// (and not a constant inside the statement), and that the comparison is on
/// the database's clock: the rows are positioned from `db_now`, never from
/// this process's.
#[tokio::test]
async fn the_sweep_horizon_is_the_callers_duration_on_the_databases_clock() -> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;
    let store = vpay_db::client_assertion_store(repositories.op_store_pool());
    let now = as_chrono(db_now(&pool).await?)?;
    let horizon = std::time::Duration::from_secs(120);

    for (jti, age) in [("inside", 100), ("outside", 140)] {
        store
            .record_jti(jti, now - ChronoDuration::seconds(age))
            .await
            .map_err(|e| anyhow::anyhow!("record_jti failed: {e}"))?;
    }

    // A sub-microsecond duration is accepted rather than refused, as
    // `vpay_db::jobs` does for a delay.
    let deleted = repositories
        .delete_expired_client_assertion_jtis(horizon + std::time::Duration::from_nanos(1))
        .await
        .context("sweeping with a 120 s horizon")?;
    assert_eq!(deleted, 1);

    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT jti FROM oauth_client_assertion_jtis ORDER BY jti")
            .fetch_all(&pool)
            .await
            .context("reading the surviving rows back")?;
    assert_eq!(remaining, vec!["inside".to_owned()]);

    Ok(())
}
