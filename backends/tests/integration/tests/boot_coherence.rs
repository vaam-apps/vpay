//! Which layer refuses an incoherent rail, and in which order — boot step 4,
//! against a real Postgres (issue #61).
//!
//! # Why a container, for a guard that touches no database
//!
//! `vpay_api::boot::boot_seeds` is pure CPU, and *that* it refuses is a unit
//! test in `vpay-api`
//! (`a_provider_with_incoherent_capabilities_is_a_config_error`). What a unit
//! test cannot say is which layer answers **first**, and that is the whole of
//! issue #61: the same deployment used to reach `ConfigReconcile::reconcile`,
//! break migration 0002's `partial_refunds_imply_refunds` CHECK, and exit `1`
//! — "page someone" — about a database that was working perfectly, for a
//! mistake whose only fix is a redeploy. The two answers differ only at
//! runtime, in the order `vpay-server`'s `main` calls the two shipping
//! functions, and one of them needs a database.
//!
//! So this file calls those two functions in that order and asserts both
//! answers in one test. Either half alone is satisfiable by a mistake: an
//! assertion that boot refuses says nothing about what the CHECK would have
//! done, and an assertion about the CHECK says nothing about what runs first.
//! Delete the `is_coherent` call from `boot_seeds` and the first half fails
//! carrying the second half's error — which is exactly the regression, spelled
//! by the failure message.
//!
//! # The incoherent rail below is not a stub rail
//!
//! AGENTS.md rule 1 and `docs/adr/0006-no-mocks-in-main-processes.md` forbid a
//! test double reachable from a shipping process; a stub *rail* is a WireMock
//! host in configuration, and this file configures none. `IncoherentRail`
//! answers no call — every method that would reach a rail returns
//! `ProviderError::Unsupported`, never a plausible success — and exists only
//! to **declare** the one capability set no shipping adapter may declare. Same
//! role, same reasoning as `TestRail` in `vpay-api`'s own boot tests; it lives
//! in a test binary, so nothing links it.
//!
//! There is no subprocess version of this test, and that is not an omission.
//! `vpay-server` links `mtn_momo` and `orange_money`, whose capability tables
//! are asserted coherent by `no_adapter_advertises_partial_without_full_refunds`
//! and by the conformance suite's `every_adapter_declares_coherent_capabilities`
//! — so no YAML this repository can write makes the shipping binary reach this
//! refusal. What the guard catches is a **linking** mistake, and the only way
//! to reach one is to link one.

use std::collections::BTreeMap;

use anyhow::Context as _;
use vpay_config::{Config, ConfigError, CurrencyEntry, Deployment, HostEntry, ProviderHost};
use vpay_core::error::{Category, Classify as _};
use vpay_core::{Money, ProviderFlow};
use vpay_provider::{
    CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderAdapter, ProviderConfig,
    ProviderError, Refunded, Submitted,
};

mod support;

use support::migrated_postgres;

/// The rail code this file configures. Deliberately not one a binary links:
/// spelling `mtn_momo` here would put a rail an operator recognises on an
/// impossible capability set, in a test whose whole subject is that no
/// shipping rail may have one.
const RAIL: &str = "incoherent_rail";

/// A rail whose declared capabilities contradict themselves — partial refunds
/// without refunds, the pair `Capabilities::is_coherent` and migration 0002's
/// `partial_refunds_imply_refunds` CHECK both refuse.
#[derive(Debug)]
struct IncoherentRail;

#[async_trait::async_trait]
impl ProviderAdapter for IncoherentRail {
    fn code(&self) -> &'static str {
        RAIL
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            flow: ProviderFlow::Push,
            supports_refunds: false,
            supports_partial_refunds: true,
            delivers_callbacks: false,
            requires_ip_allowlist: false,
            supports_account_holder_lookup: false,
        }
    }

    async fn submit(
        &self,
        _charge: &ChargeRef,
        _config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    async fn query_status(
        &self,
        _charge: &ChargeRef,
        _config: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    async fn refund(
        &self,
        _charge: &ChargeRef,
        _amount: Money,
        _config: &ProviderConfig,
    ) -> Result<Refunded, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}

/// One rail on XAF, `livemode: false` — the shape `config/application.yml`
/// has, minus everything boot step 4's join does not read. The host is a
/// closed port on purpose: nothing here may reach a network, and a stub
/// marker in the label would be a second subject.
fn config_with(code: &str) -> Config {
    Config {
        deployment: Deployment {
            name: "exp42-boot-coherence".to_owned(),
            livemode: false,
            public_base_url: "http://localhost:8080".to_owned(),
        },
        providers: vec![ProviderHost {
            code: code.to_owned(),
            enabled: true,
            host: HostEntry {
                url: "http://127.0.0.1:1".to_owned(),
                label: "unreachable-on-purpose".to_owned(),
            },
            settings: BTreeMap::new(),
            callback_url: None,
            currency: "XAF".to_owned(),
            credentials: BTreeMap::new(),
        }],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: Vec::new(),
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
        staff_auth: vpay_config::StaffAuth::default(),
    }
}

/// Boot refuses the rail as a **configuration** failure and writes nothing;
/// the CHECK, which is what used to answer, is still there and still answers
/// `1`.
#[tokio::test]
async fn boot_refuses_an_incoherent_rail_as_78_before_the_check_can_answer_it_as_1()
-> anyhow::Result<()> {
    let (_container, repositories, pool) = migrated_postgres().await?;

    // Steps 6 and 8 of the boot sequence, the shipping functions, in `main`'s
    // order: key the linked adapters, then join the YAML against them. Both
    // run before a pool is opened in production, which is why the container
    // above is here for the second half of this test rather than the first.
    let adapters = vpay_api::boot::adapters_by_code(vec![Box::new(IncoherentRail)]);
    let refusal = vpay_api::boot::boot_seeds(&config_with(RAIL), &adapters).expect_err(
        "boot step 4 must refuse a rail whose declared capabilities contradict themselves. An \
         Ok here means the `is_coherent` call in `boot_seeds` is gone and this deployment is \
         back to exiting 1 from the database CHECK — issue #61's regression, whose number is \
         measured at the bottom of this test",
    );

    assert!(
        matches!(refusal, ConfigError::IncoherentCapabilities { .. }),
        "expected ConfigError::IncoherentCapabilities, got {refusal:?}"
    );
    assert_eq!(refusal.category(), Category::Configuration);
    assert_eq!(
        refusal.category().exit_code(),
        78,
        "78 is EX_CONFIG — the number a bad flow label in the same file already gets, and the \
         one an operator reads as 'fix the deploy'"
    );
    let message = refusal.to_string();
    assert!(
        message.contains(RAIL),
        "the message must name the rail an operator has to go and look at: {message}"
    );
    assert!(
        message.contains("partial_refunds_imply_refunds"),
        "the message must name the rule, spelled the way the migration and the database's own \
         error spell it, so one grep finds every layer: {message}"
    );

    let providers_written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(&pool)
        .await
        .context("counting providers must succeed")?;
    assert_eq!(
        providers_written, 0,
        "boot refused before `reconcile_reference_tables` ran, so no row may exist. A 1 here \
         means the guard moved below the reconcile, where it no longer prevents anything"
    );

    // What the next call would have met. The seed is spelled out rather than
    // taken from `boot_seeds`, because `boot_seeds` now refuses to produce it
    // — that refusal is the subject above, and this is the value it withheld.
    let seed = vpay_db::ProviderSeed {
        code: RAIL.to_owned(),
        display_name: "Incoherent Rail".to_owned(),
        flow: "push".to_owned(),
        supports_refunds: false,
        supports_partial_refunds: true,
        delivers_callbacks: false,
        requires_ip_allowlist: false,
        enabled: true,
    };
    let currencies = [vpay_db::CurrencySeed {
        code: "XAF".to_owned(),
        exponent: 0,
    }];
    let from_the_database = vpay_api::boot::reconcile_reference_tables(
        repositories.as_ref(),
        &currencies,
        std::slice::from_ref(&seed),
    )
    .await
    .expect_err("the CHECK is still the last line and must still refuse the row");

    let vpay_db::DbError::Persistence(vpay_db::PersistenceError::Check { constraint, .. }) =
        &from_the_database
    else {
        panic!("expected PersistenceError::Check, got {from_the_database:?}");
    };
    assert_eq!(
        constraint, "partial_refunds_imply_refunds",
        "the refusal must come from the coherence CHECK specifically"
    );
    assert_eq!(
        from_the_database.category().exit_code(),
        1,
        "the CHECK's answer is exit 1, and that it is *not* 78 is the reason boot answers first"
    );

    Ok(())
}
