//! `/dash/v1`, end to end: the real `vpay_api::router` on a real socket, over
//! a real Postgres, with a dashboard bound to exactly one of the two merchants
//! configured, read by signed messages under paired console keys (VP-6a,
//! [ADR-0036](../../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).
//!
//! # What changed, stated before anything else
//!
//! Until VP-6a every request here carried a bearer token the test minted with
//! the server's own signing key, and this suite proved a *resource server*:
//! which rows a validly-minted token may read. There is no token now. A request
//! is a COSE_Sign1 message under a P-256 console key
//! ([`support::sealed`]), opened by the real `EnvelopeLayer` against a real
//! Postgres replay cache and a real `console_keys` table, and answered by a
//! message sealed under the server's key. The keys are paired through the
//! shipping repository (`pair_console_key`), the way `staff pair-key` and
//! `POST /svc/v1/console_keys` pair them — `console_pairing.rs` is the suite
//! for that door.
//!
//! # What it claims
//!
//! Tenancy (unchanged from the bearer era, because it is the boundary and not
//! the credential):
//!
//! 1. a key paired to a plain member lists **A's** intents and not B's;
//! 2. another merchant's intent is, byte for byte, the 404 an id that never
//!    existed gets;
//! 3. the detail carries the charge, refunds and timeline, and **no
//!    `client_secret`**; the `status` filter narrows and refuses an unknown one;
//! 4. a cursor naming another merchant's intent positions nothing;
//! 5. an admin's `?merchant_id=` is honoured and a non-admin's is not even
//!    parsed.
//!
//! The envelope (new, and each one is the layer's own coarse `401` — status,
//! `UNAUTHORIZED` and an unsigned body, so loosening `Required` to `Optional`
//! fails them):
//!
//! 6. no signature at all, on every route in `DASH_ROUTES`;
//! 7. a key nobody paired, a **merchant's** key, a **service's** key;
//! 8. a console key on `/v1`;
//! 9. an **expired** key (paired 31 days ago) and the control paired 29 days
//!    ago that still reads;
//! 10. a **revoked** key, on the very next request;
//! 11. a wrong audience, and a replay;
//! 12. a bodiless anonymous `OPTIONS` is the router's `405`, never a `500`;
//! 13. a write method is refused by the layer, not by the route table.
//!
//! And the member re-read on every request: a disabled member and a member of
//! another merchant are refused after the signature verified (`403`, sealed).
//!
//! # Why raw `reqwest` and no SDK
//!
//! There is no dashboard SDK and there must not be one: `docs/sdks/parity.md`
//! covers merchant surfaces, and a `/dash/v1` method in a merchant SDK would
//! be a merchant credential reaching for a staff surface.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::Value;
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use time::{Duration, OffsetDateTime};
use vpay_config::{Config, CurrencyEntry, Dashboard, Deployment, HostEntry, ProviderHost};
use vpay_db::{NewPaymentIntent, Repositories, UnitOfWork as _};

mod support;

use support::sealed::{self, SealedHttp};
use support::{
    assert_coarse_401, create_staff, ensure_crypto_provider_installed, generate_key,
    merchant_client, migrated_postgres, pair_console_key, serve,
};

/// The tenant the dashboard is bound to, and the credential that acts for it
/// on `/v1`. Never the same string, for `payment_intents.rs`' reason: a query
/// filtered by `client_id` instead of `merchant_id` would otherwise pass.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";

/// The tenant the dashboard is **not** bound to. Its rows exist so that
/// "lists A's intents" is a claim with something to fail against — a suite
/// with one merchant proves nothing about a tenancy filter.
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

/// The member the ordinary console key is paired to: a plain member of
/// `MERCHANT_A`.
const STAFF_ID: &str = "stf_0000000000000000000dash";

/// The administrator ADR-0018's cases read as — a second, distinct row, so a
/// case for one flag can never pass because it forgot to check it. An admin is
/// made by `staff add --admin` and keyed by `staff pair-key`; this suite does
/// the second half through the repository.
const ADMIN_STAFF_ID: &str = "stf_00000000000000000admin";

/// The labels the two members' console keys are derived from
/// ([`sealed::console_scalar`]).
const READER: &str = "reader";
const ADMIN: &str = "admin";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "XAF";
const AMOUNT: i64 = 5000;

/// The recognisable prefix every seeded intent's `client_secret_suffix`
/// carries, so the detail test can grep a rendered body for it.
const SECRET_MARKER: &str = "dashsuffixmarker";

// ------------------------------------------------------------------ harness

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    #[allow(dead_code)]
    server: tokio::task::JoinHandle<()>,
    repositories: Arc<dyn Repositories>,
    pool: PgPool,
    base_url: String,
}

impl Harness {
    /// The label of the plain member's console key.
    fn reader_label(&self) -> String {
        READER.to_owned()
    }

    /// The label of the administrator's console key. Nothing on a request tells
    /// the two apart: ADR-0018 reads the admin grant off the re-read
    /// `staff_members` row, which is the point.
    fn admin_label(&self) -> String {
        ADMIN.to_owned()
    }

    /// `GET path`, signed by the console key `label`, or unsigned when `None`.
    async fn get(&self, path: &str, label: Option<&str>) -> anyhow::Result<(u16, String)> {
        let response = self.send(reqwest::Method::GET, path, label).await?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((status, body))
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        label: Option<&str>,
    ) -> anyhow::Result<support::sealed::SealedResponse> {
        let url = format!("{}{path}", self.base_url);
        let request = SealedHttp::new().request(method.as_str(), url);
        let request = match label {
            Some(label) => request.signed_by_console(label),
            None => return self.unsigned(method, path).await,
        };
        request.send().await.context("sending a /dash/v1 request")
    }

    /// The request no signature accompanies, asking for JSON.
    async fn unsigned(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> anyhow::Result<support::sealed::SealedResponse> {
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
            .request(method, format!("{}{path}", self.base_url))
            .header("accept", "application/json")
            .send()
            .await
            .context("sending an unsigned request")?;
        Ok(sealed::SealedResponse::from_plain(response).await?)
    }

    /// A `/v1` read signed by `client_id`, as a merchant's backend makes it:
    /// the merchant half of the contrast these tests draw.
    async fn v1_get(&self, client_id: &str, path: &str) -> anyhow::Result<(u16, String)> {
        let response = SealedHttp::new()
            .get(format!("{}{path}", self.base_url))
            .signed_by(client_id)
            .send()
            .await
            .context("sending a signed /v1 request")?;
        let status = response.status().as_u16();
        let body = response.text().await.context("reading the body")?;
        Ok((status, body))
    }

    /// `GET` signed by the reader's key, decoded — the shape almost every test
    /// below wants.
    async fn dash_json(&self, path: &str) -> anyhow::Result<(u16, Value)> {
        self.dash_json_as(path, &self.reader_label()).await
    }

    /// [`Self::dash_json`] with the key as a parameter, for the ADR-0018 cases
    /// that must read as the **admin** rather than the ordinary member.
    async fn dash_json_as(&self, path: &str, label: &str) -> anyhow::Result<(u16, Value)> {
        let (status, body) = self.get(path, Some(label)).await?;
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    async fn pair(
        &self,
        staff_id: &str,
        label: &str,
        at: OffsetDateTime,
    ) -> anyhow::Result<String> {
        pair_console_key(self.repositories.as_ref(), staff_id, label, at).await
    }
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value, dashboard: bool) -> Config {
    Config {
        deployment: Deployment {
            name: "dashboard-read-surface".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: vec![ProviderHost {
            code: PUSH_RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
                // Unreachable on purpose: this suite reads rows, it never
                // confirms anything, so no rail is ever called.
                url: "http://127.0.0.1:1".to_owned(),
                label: "unreachable-by-design".to_owned(),
            },
            settings: BTreeMap::from([
                ("target_environment".to_owned(), "sandbox".to_owned()),
                (
                    "api_user".to_owned(),
                    "11111111-2222-3333-4444-555555555555".to_owned(),
                ),
            ]),
            callback_url: None,
            currency: CURRENCY.to_owned(),
            credentials: BTreeMap::from([
                (
                    "subscription_key".to_owned(),
                    "stub-subscription-key".to_owned(),
                ),
                ("api_key".to_owned(), "stub-api-key".to_owned()),
            ]),
            display_name: None,
            callbacks: None,
        }],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            merchant_client(CLIENT_A, MERCHANT_A, jwks_a),
            merchant_client(CLIENT_B, MERCHANT_B, jwks_b),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        // The whole point of the `dashboard` flag: one test boots the *same*
        // configuration with this set to `None` and asserts no management nest
        // is mounted. Two configurations that differed in more than this field
        // would make that test's `404` attributable to something else.
        dashboard: dashboard.then(|| Dashboard {
            merchant_id: MERCHANT_A.to_owned(),
        }),
        service_clients: Vec::new(),
        routing: Default::default(),
    }
}

async fn harness_with(dashboard: bool) -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (container, repositories, pool) = migrated_postgres().await?;

    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b, dashboard)
    })
    .await?;

    seed_staff(repositories.as_ref()).await?;
    seed_admin_staff(repositories.as_ref()).await?;

    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
    })
}

async fn harness() -> anyhow::Result<Harness> {
    harness_with(true).await
}

/// The plain member and its live console key (paired a moment ago).
async fn seed_staff(repositories: &dyn Repositories) -> anyhow::Result<()> {
    create_staff(
        repositories,
        STAFF_ID,
        MERCHANT_A,
        "dash-reader@example.test",
        false,
    )
    .await?;
    pair_console_key(repositories, STAFF_ID, READER, OffsetDateTime::now_utc()).await?;
    Ok(())
}

/// [`seed_staff`]'s ADR-0018 counterpart: a second member of the same merchant
/// with `is_admin` set, and its own key. What `is_admin` buys is entirely about
/// what happens *after* the signature verifies: this row may have
/// `?merchant_id=` honoured rather than ignored.
async fn seed_admin_staff(repositories: &dyn Repositories) -> anyhow::Result<()> {
    create_staff(
        repositories,
        ADMIN_STAFF_ID,
        MERCHANT_A,
        "dash-admin@example.test",
        true,
    )
    .await?;
    pair_console_key(
        repositories,
        ADMIN_STAFF_ID,
        ADMIN,
        OffsetDateTime::now_utc(),
    )
    .await?;
    Ok(())
}

/// Writes one payment intent directly through the repository.
///
/// Not through `POST /v1/payment_intents`: this suite's subject is the read
/// surface, and creating through the merchant API would make every test
/// depend on the merchant SDK's credential flow as well. The rows are the
/// same rows — the same `NewPaymentIntent` the create handler builds.
async fn seed_intent(
    repositories: &dyn Repositories,
    merchant_id: &str,
    id: &str,
) -> anyhow::Result<()> {
    repositories
        .insert(&NewPaymentIntent {
            id: id.to_owned(),
            merchant_id: merchant_id.to_owned(),
            livemode: false,
            amount: AMOUNT,
            currency_code: CURRENCY.to_owned(),
            status: vpay_core::IntentStatus::INITIAL.as_wire_str().to_owned(),
            payment_method_types: serde_json::json!([PUSH_RAIL]),
            metadata: serde_json::json!({}),
            description: None,
            customer_id: None,
            last_payment_error_code: None,
            last_payment_error_message: None,
            // A recognisable literal, unlike `support::confirmed_intent`'s
            // generated suffix, and deliberately: test 8 greps the rendered
            // body for `SECRET_MARKER`, so a fixture whose secret nothing
            // could recognise would make "no client_secret was leaked" a
            // claim that passes whether the renderer leaks or not. Padded to
            // migration `0026`'s 32-character floor, which exists because a
            // short suffix is a guessable credential.
            client_secret_suffix: format!("{SECRET_MARKER}{id:x<32}"),
            created_at: time::OffsetDateTime::now_utc(),
        })
        .await
        .with_context(|| format!("seeding {id} for {merchant_id}"))?;
    Ok(())
}

/// One field of a JSON body, by RFC 6901 pointer, panicking with the pointer
/// when it is absent.
///
/// Not `body["data"]`: `clippy::indexing_slicing` is denied in these suites,
/// and the lint is right here for a reason beyond panics — `serde_json`'s
/// `Index` impl answers `Value::Null` for a key that is not there, so an
/// assertion against a *mistyped* path compares `Null` with `Null` and
/// passes. A missing field must fail the test that names it.
fn at<'a>(body: &'a Value, pointer: &str) -> &'a Value {
    body.pointer(pointer)
        .unwrap_or_else(|| panic!("{pointer} is absent from {body}"))
}

// ------------------------------------------------------------------ test 1

/// A plain member's console key sees its own merchant's intents and none of the other's.
///
/// The decisive mutation: make `dash_scope` insert a
/// `MerchantScope` built from anything other than
/// `Dashboard::merchant_id` — the key's own thumbprint, say — and
/// this fails, because `vpay-dashboard` is not a `merchant_id` any row
/// carries and the list comes back empty.
#[tokio::test]
async fn the_dashboard_lists_only_the_merchant_it_is_bound_to() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_dash_a_one").await?;
    seed_intent(repositories, MERCHANT_A, "pi_dash_a_two").await?;
    seed_intent(repositories, MERCHANT_B, "pi_dash_b_one").await?;

    let (status, body) = harness.dash_json("/dash/v1/payment_intents").await?;
    assert_eq!(status, 200, "{body}");

    let ids: Vec<&str> = at(&body, "/data")
        .as_array()
        .expect("the list envelope carries an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();

    assert_eq!(ids.len(), 2, "{body}");
    assert!(ids.contains(&"pi_dash_a_one"), "{body}");
    assert!(ids.contains(&"pi_dash_a_two"), "{body}");
    assert!(
        !ids.contains(&"pi_dash_b_one"),
        "the dashboard is bound to {MERCHANT_A} and must not see {MERCHANT_B}'s rows: {body}"
    );
    assert_eq!(at(&body, "/url"), "/dash/v1/payment_intents", "{body}");

    // Over the serialised body, for the reason
    // `the_detail_read_carries_the_charge_and_never_the_client_secret` gives:
    // `PaymentIntentObject` has no such field, but rendering
    // `PaymentIntentWithSecret` here instead is one word that still compiles,
    // and the *list* had no assertion against it until 2026-09-06. Both
    // seeded intents carry `SECRET_MARKER` in their suffix, so there is
    // something to leak.
    let rendered = body.to_string();
    assert!(
        !rendered.contains("client_secret") && !rendered.contains(SECRET_MARKER),
        "the staff payments list must not render the payer credential: {rendered}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 2

/// Another merchant's id and an id that never existed are the **same**
/// answer, byte for byte.
///
/// Byte-for-byte and not merely "both 404": a body that named the resource
/// differently, or echoed a different id, would let anyone holding a
/// console key enumerate which payment intents exist on the other tenant.
/// That is exactly the property `/v1`'s own test 8 pins, applied to the
/// surface that has a *fixed* tenant rather than a resolved one.
#[tokio::test]
async fn another_merchants_intent_is_indistinguishable_from_one_that_never_existed()
-> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_B, "pi_dash_b_only").await?;

    let label = harness.reader_label();
    let (foreign_status, foreign_body) = harness
        .get("/dash/v1/payment_intents/pi_dash_b_only", Some(&label))
        .await?;
    let (absent_status, absent_body) = harness
        .get("/dash/v1/payment_intents/pi_dash_b_only", Some(&label))
        .await?;

    assert_eq!(foreign_status, 404, "{foreign_body}");
    assert_eq!(absent_status, 404, "{absent_body}");

    // The same id in both requests, so the bodies are comparable: what is
    // being asserted is that the answer does not depend on whether the row
    // exists. A second request with a *different* id would differ in the
    // echoed id alone and prove nothing.
    let (never_status, never_body) = harness
        .get(
            "/dash/v1/payment_intents/pi_never_written_at_all",
            Some(&label),
        )
        .await?;
    assert_eq!(never_status, 404, "{never_body}");
    assert_eq!(
        foreign_body.replace("pi_dash_b_only", "X"),
        never_body.replace("pi_never_written_at_all", "X"),
        "the two 404s differ in more than the id they echo"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 8

/// The detail read assembles the charge, the refunds and the timeline — and
/// never renders a `client_secret`, which the merchant surface's own
/// `retrieve` does.
///
/// The two halves are asserted together on purpose: a detail endpoint that
/// returned the *merchant* rendering would pass a "does it have a charge"
/// test and would be handing a staff surface the credential that confirms
/// the payment from any browser.
#[tokio::test]
async fn the_detail_read_carries_the_charge_and_never_the_client_secret() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();
    seed_intent(repositories, MERCHANT_A, "pi_dash_detail").await?;

    // Through the same `TxRepositories::insert_for_intent` the confirm path
    // uses, in a transaction, because that is the only way `charges` is ever
    // written.
    let charge_id = "ch_dash_detail".to_owned();
    repositories
        .transaction(|tx| {
            let charge_id = charge_id.clone();
            Box::pin(async move {
                tx.insert_for_intent(&vpay_db::NewCharge {
                    id: charge_id,
                    payment_intent_id: "pi_dash_detail".to_owned(),
                    provider_code: PUSH_RAIL.to_owned(),
                    provider_reference_id: uuid::Uuid::new_v4(),
                    provider_ref_extra: None,
                    redirect_url: None,
                    return_url: None,
                    state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
                    amount: AMOUNT,
                    currency_code: CURRENCY.to_owned(),
                    payer_ref: None,
                    // `None`, because nothing in this system writes it — see
                    // `vpay_api::dash::payment_intents::ChargeSummary`.
                    // Written out rather than left implicit so the day it
                    // *is* written, this fixture is where a reader looks.
                    payer_ref_masked: None,
                    method_type: None,
                })
                .await
                .context("seeding a charge")?;
                Ok::<_, anyhow::Error>(vpay_db::TxOutcome::Commit(()))
            })
        })
        .await?;

    let (status, body) = harness
        .dash_json("/dash/v1/payment_intents/pi_dash_detail")
        .await?;
    assert_eq!(status, 200, "{body}");

    assert_eq!(at(&body, "/object"), "dashboard.payment_detail", "{body}");
    assert_eq!(at(&body, "/payment_intent/id"), "pi_dash_detail", "{body}");
    assert_eq!(at(&body, "/charge/id"), "ch_dash_detail", "{body}");
    assert_eq!(at(&body, "/charge/provider_code"), PUSH_RAIL, "{body}");
    assert_eq!(
        at(&body, "/charge/payer_ref_masked"),
        &Value::Null,
        "{body}"
    );
    assert_eq!(at(&body, "/refunds"), &serde_json::json!([]), "{body}");
    assert_eq!(at(&body, "/events"), &serde_json::json!([]), "{body}");

    let rendered = body.to_string();
    assert!(
        !rendered.contains("client_secret"),
        "the staff surface must not render the payer credential: {rendered}"
    );
    assert!(
        !rendered.contains(SECRET_MARKER),
        "nor the suffix it is built from: {rendered}"
    );

    // The *merchant* surface does render it, from the same row — so the
    // assertion above is about this handler's choice and not about the row
    // having no secret to leak.
    let (v1_status, v1_body) = harness
        .v1_get(CLIENT_A, "/v1/payment_intents/pi_dash_detail")
        .await?;
    assert_eq!(v1_status, 200, "{v1_body}");
    assert!(
        v1_body.contains(SECRET_MARKER),
        "the merchant surface is expected to carry it: {v1_body}"
    );
    Ok(())
}

// ------------------------------------------------------------------ test 9

/// The `status` filter narrows the page, and an unknown status is a `400`
/// naming `status`. Since 2026-09-23 it also pins that `customer` — `/v1`'s
/// filter, not this list's — is a `400` naming `customer` rather than an
/// ignored key that answers the whole list.
///
/// The second half is the one worth having: a filter that passed an unknown
/// value through to the `WHERE` clause would answer an empty list, and "no
/// payments are `succeded`" is a sentence an operator reads as an answer
/// about their payments rather than about their typo.
#[tokio::test]
async fn the_status_filter_narrows_the_page_and_refuses_an_unknown_status() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_dash_open_one").await?;
    seed_intent(repositories, MERCHANT_A, "pi_dash_open_two").await?;
    seed_intent(repositories, MERCHANT_A, "pi_dash_canceled").await?;
    repositories
        .transition(
            MERCHANT_A,
            "pi_dash_canceled",
            vpay_core::IntentStatus::INITIAL.as_wire_str(),
            vpay_core::IntentStatus::Canceled.as_wire_str(),
        )
        .await
        .context("cancelling one intent so the filter has two groups")?
        .expect("the transition fires");

    let (status, body) = harness
        .dash_json("/dash/v1/payment_intents?status=canceled")
        .await?;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = at(&body, "/data")
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, vec!["pi_dash_canceled"], "{body}");

    let (unknown_status, unknown_body) = harness
        .dash_json("/dash/v1/payment_intents?status=succeded")
        .await?;
    assert_eq!(unknown_status, 400, "{unknown_body}");
    assert_eq!(
        at(&unknown_body, "/error/param"),
        "status",
        "{unknown_body}"
    );

    // `customer` is `/v1`'s filter (RFC-0004 § 5) and is refused here rather
    // than ignored: ignored, this answered all three intents — every
    // customer's — as though it were the filtered list.
    let (customer_status, customer_body) = harness
        .dash_json("/dash/v1/payment_intents?customer=cus_00000000000000000000000x")
        .await?;
    assert_eq!(customer_status, 400, "{customer_body}");
    assert_eq!(
        at(&customer_body, "/error/param"),
        "customer",
        "{customer_body}"
    );
    Ok(())
}

// ----------------------------------------------------------------- test 11

/// The detail read renders the events and the refunds an intent actually
/// has — and renders **only this tenant's** events.
///
/// # Why this test exists
///
/// Every other assertion in this file about `refunds` and `events` is
/// `== []`, over fixtures that have neither. Mutation testing on 2026-09-06
/// measured the consequence: making **both** `Events::list_for_objects` and
/// `Refunds::list_for_intent` `return Ok(Vec::new())` unconditionally left
/// all ten tests green. Two repository methods whose only caller is the
/// detail route could answer "nothing happened to this payment" forever, and
/// `AGENTS.md` rule 2 names that shape by hand: unwritten code "never returns
/// a plausible-looking success, an empty list, or a zero".
///
/// # The second half, and why it needs a row nothing writes
///
/// `events.object_id` is a plain `TEXT NOT NULL` with **no foreign key**
/// (migration `0018`) that points into three different tables depending on
/// `type`. So `list_for_objects`' `merchant_id = $1` predicate is the *only*
/// thing keeping another tenant's event out of this tenant's timeline, and
/// the only way to exercise it is a row no code path produces: an event owned
/// by `MERCHANT_B` whose `object_id` is `MERCHANT_A`'s intent. It is written
/// through the shipping `TxRepositories::insert_in_tx` — the row is real, its
/// `merchant_id` is simply the other one.
///
/// `Refunds::list_for_intent`'s own `p.merchant_id = $1` cannot be mutated
/// into a failure and is deliberately not asserted: `refunds.payment_intent_id`
/// is a real foreign key onto `payment_intents(id)`, so a refund reachable
/// from A's intent *is* A's. That predicate is defence in depth, and saying so
/// is more honest than a test that would pass with it deleted.
///
/// The decisive mutations: `Ok(Vec::new())` from either repository method,
/// or dropping `merchant_id = $1` from `Events::list_for_objects`.
#[tokio::test]
async fn the_detail_read_renders_the_timeline_and_the_refunds_it_has() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    seed_intent(repositories, MERCHANT_A, "pi_dash_timeline").await?;
    seed_intent(repositories, MERCHANT_B, "pi_dash_timeline_b").await?;

    let charge_id = "ch_dash_timeline".to_owned();
    repositories
        .transaction(|tx| {
            let charge_id = charge_id.clone();
            Box::pin(async move {
                tx.insert_for_intent(&vpay_db::NewCharge {
                    id: charge_id.clone(),
                    payment_intent_id: "pi_dash_timeline".to_owned(),
                    provider_code: PUSH_RAIL.to_owned(),
                    provider_reference_id: uuid::Uuid::new_v4(),
                    provider_ref_extra: None,
                    redirect_url: None,
                    return_url: None,
                    state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
                    amount: AMOUNT,
                    currency_code: CURRENCY.to_owned(),
                    payer_ref: None,
                    payer_ref_masked: None,
                    method_type: None,
                })
                .await
                .context("seeding a charge")?;

                // One event about the intent and one about the *charge* —
                // the two-object case `Events::list_for_objects` takes a
                // slice for. A timeline that asked only about the intent
                // would render the first and silently drop the second.
                for (id, event_type, object_id) in [
                    (
                        "evt_dash_timeline_created",
                        "payment_intent.created",
                        "pi_dash_timeline",
                    ),
                    (
                        "evt_dash_timeline_refunded",
                        "charge.refunded",
                        charge_id.as_str(),
                    ),
                ] {
                    tx.insert_in_tx(&vpay_db::NewEvent {
                        id: id.to_owned(),
                        merchant_id: MERCHANT_A.to_owned(),
                        livemode: false,
                        event_type: event_type.to_owned(),
                        object_id: object_id.to_owned(),
                        data: serde_json::json!({ "object": { "id": object_id } }),
                    })
                    .await
                    .context("seeding an event")?;
                }

                // The row no code path produces: MERCHANT_B's event pointing
                // at MERCHANT_A's intent. See this test's doc comment.
                tx.insert_in_tx(&vpay_db::NewEvent {
                    id: "evt_dash_timeline_foreign".to_owned(),
                    merchant_id: MERCHANT_B.to_owned(),
                    livemode: false,
                    event_type: "payment_intent.canceled".to_owned(),
                    object_id: "pi_dash_timeline".to_owned(),
                    data: serde_json::json!({ "object": { "id": "pi_dash_timeline" } }),
                })
                .await
                .context("seeding the foreign event")?;

                Ok::<_, anyhow::Error>(vpay_db::TxOutcome::Commit(()))
            })
        })
        .await?;

    // `refunds` has no writer anywhere in this workspace — `vpay_db::refunds`
    // is a read-only module and says why — so the row goes in the way
    // `tests/refunds.rs` puts one in.
    for (id, intent) in [
        ("re_dash_timeline_one", "pi_dash_timeline"),
        ("re_dash_timeline_two", "pi_dash_timeline"),
        // The other tenant's refund, on the other tenant's intent: it must
        // not appear, and it is what makes "two refunds" a count rather than
        // "every refund in the table".
        ("re_dash_timeline_b", "pi_dash_timeline_b"),
    ] {
        sqlx::query(
            "INSERT INTO refunds (id, payment_intent_id, amount, currency_code, status, metadata) \
             VALUES ($1, $2, $3, 'XAF', 'pending', '{}'::jsonb)",
        )
        .bind(id)
        .bind(intent)
        .bind(AMOUNT)
        .execute(&harness.pool)
        .await
        .with_context(|| format!("seeding refund {id}"))?;
    }

    let (status, body) = harness
        .dash_json("/dash/v1/payment_intents/pi_dash_timeline")
        .await?;
    assert_eq!(status, 200, "{body}");

    let event_ids: Vec<&str> = at(&body, "/events")
        .as_array()
        .expect("the timeline is an array")
        .iter()
        .filter_map(|event| event.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        event_ids,
        vec!["evt_dash_timeline_created", "evt_dash_timeline_refunded"],
        "the timeline must carry both objects' events, oldest first, and \
         nothing belonging to {MERCHANT_B}: {body}"
    );
    assert_eq!(
        at(&body, "/events/1/type"),
        "charge.refunded",
        "the charge's event must be rendered with its own type: {body}"
    );
    assert_eq!(
        at(&body, "/events/1/object_id"),
        "ch_dash_timeline",
        "{body}"
    );

    let refund_ids: Vec<&str> = at(&body, "/refunds")
        .as_array()
        .expect("the refunds are an array")
        .iter()
        .filter_map(|refund| refund.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        refund_ids,
        vec!["re_dash_timeline_one", "re_dash_timeline_two"],
        "{body}"
    );

    Ok(())
}

// ----------------------------------------------------------------- test 12

/// A cursor is resolved **inside the bound tenant**: one naming another
/// merchant's intent positions nothing, and answers an empty page rather
/// than a slice of this merchant's list.
///
/// # Why this is worth a test on this surface
///
/// `another_merchants_intent_is_indistinguishable_from_one_that_never_existed`
/// pins that property for `retrieve`. It holds for the *list* only because
/// both cursor subqueries in `PaymentIntents::list_page_filtered` carry
/// `AND merchant_id = $1`, so a foreign id resolves to `NULL`, `seq < NULL`
/// is `NULL`, and no row matches. Drop that predicate and the subquery
/// resolves to the other tenant's `seq` instead: the page comes back
/// populated with *this* tenant's rows, which turns `starting_after` into an
/// existence oracle for ids the caller may not read — the same thing
/// `retrieve` refuses to be, reached by a different door.
///
/// Measured on 2026-09-06: with that predicate removed from both subqueries,
/// all 35 tests in `dashboard_read_surface` and `payment_intents` passed. The
/// predicate is **pre-existing and unchanged** — byte for byte what `3694e34`
/// shipped — and `/v1` had no test for it either; it is pinned here because
/// `/dash/v1` is the second surface now standing on that statement.
///
/// The ids are generated by `vpay_core::ids::payment_intent_id` rather than
/// spelled out like every other fixture in this file, because a cursor —
/// unlike a path parameter — is checked for well-formedness before it reaches
/// SQL (`v1::paging::validated_cursor`), and a readable id would be a `400`
/// that never exercised the tenancy predicate at all.
///
/// The decisive mutation: delete `AND merchant_id = $1` from either cursor
/// subquery in `list_page_filtered`.
#[tokio::test]
async fn a_cursor_naming_another_merchants_intent_answers_an_empty_page() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();

    // B's row first, so its `seq` is the *lowest*: a `starting_after` that
    // resolved to it would answer every one of A's rows, which is the
    // loudest form the leak can take and the one this asserts against.
    let b_intent = vpay_core::ids::payment_intent_id();
    let a_older = vpay_core::ids::payment_intent_id();
    let a_newer = vpay_core::ids::payment_intent_id();
    seed_intent(repositories, MERCHANT_B, &b_intent).await?;
    seed_intent(repositories, MERCHANT_A, &a_older).await?;
    seed_intent(repositories, MERCHANT_A, &a_newer).await?;

    // The control: A's own cursor pages A's own list, so the empty pages
    // below are about the tenant and not about cursors being broken.
    let (own_status, own_body) = harness
        .dash_json(&format!(
            "/dash/v1/payment_intents?starting_after={a_newer}"
        ))
        .await?;
    assert_eq!(own_status, 200, "{own_body}");
    let own_ids: Vec<&str> = at(&own_body, "/data")
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(own_ids, vec![a_older.as_str()], "{own_body}");

    // A well-formed id nothing ever wrote, so "the other tenant's row" and
    // "no such row" are held to the same answer here too.
    let never_written = vpay_core::ids::payment_intent_id();

    for cursor in [&b_intent, &never_written] {
        for param in ["starting_after", "ending_before"] {
            let (status, body) = harness
                .dash_json(&format!("/dash/v1/payment_intents?{param}={cursor}"))
                .await?;
            assert_eq!(status, 200, "{param}={cursor}: {body}");
            assert_eq!(
                at(&body, "/data"),
                &serde_json::json!([]),
                "{param}={cursor} must position nothing and answer an empty page"
            );
        }
    }

    // And the two answers are the same answer, so the list cannot be used to
    // learn whether an id exists on another tenant.
    let (foreign, foreign_body) = harness
        .get(
            &format!("/dash/v1/payment_intents?starting_after={b_intent}"),
            Some(&harness.reader_label()),
        )
        .await?;
    let (absent, absent_body) = harness
        .get(
            &format!("/dash/v1/payment_intents?starting_after={never_written}"),
            Some(&harness.reader_label()),
        )
        .await?;
    assert_eq!((foreign, absent), (200, 200));
    assert_eq!(
        foreign_body, absent_body,
        "another tenant's cursor must answer exactly what an unwritten one does"
    );
    Ok(())
}

// ------------------------------------------------------------------ ADR-0018: the admin role

/// **An admin's `?merchant_id=` is honoured**, and the read it returns is
/// `MERCHANT_B`'s rows, not `MERCHANT_A`'s (the dashboard's own binding, and
/// the admin's home merchant).
///
/// The decisive mutation: make `dash_scope` ignore
/// `staff.is_admin` (always resolve `DashboardTenancy::Bound`), and this
/// fails with an empty page — the admin key still reads its own merchant
/// only, exactly like `the_dashboard_lists_only_the_merchant_it_is_bound_to`.
#[tokio::test]
async fn an_admin_reads_a_merchant_other_than_its_own() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();
    seed_intent(repositories, MERCHANT_A, "pi_dash_admin_a").await?;
    seed_intent(repositories, MERCHANT_B, "pi_dash_admin_b").await?;

    let label = harness.admin_label();

    // No override: an admin who names nothing reads its own merchant, same
    // as anyone else.
    let (bound_status, bound_body) = harness
        .dash_json_as("/dash/v1/payment_intents", &label)
        .await?;
    assert_eq!(bound_status, 200, "{bound_body}");
    let bound_ids: Vec<&str> = at(&bound_body, "/data")
        .as_array()
        .expect("the list envelope carries an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(bound_ids, vec!["pi_dash_admin_a"], "{bound_body}");

    // `?merchant_id=` names the OTHER tenant this deployment serves.
    let (cross_status, cross_body) = harness
        .dash_json_as(
            &format!("/dash/v1/payment_intents?merchant_id={MERCHANT_B}"),
            &label,
        )
        .await?;
    assert_eq!(cross_status, 200, "{cross_body}");
    let cross_ids: Vec<&str> = at(&cross_body, "/data")
        .as_array()
        .expect("the list envelope carries an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        cross_ids,
        vec!["pi_dash_admin_b"],
        "an admin naming {MERCHANT_B} must read {MERCHANT_B}'s rows, not the key's own \
         merchant: {cross_body}"
    );

    // And the detail read for the id honours the same override: MERCHANT_A's
    // id is a 404 once the request is scoped to MERCHANT_B, exactly as it
    // would be for a non-admin bound to MERCHANT_B.
    let (foreign_status, foreign_body) = harness
        .get(
            &format!("/dash/v1/payment_intents/pi_dash_admin_a?merchant_id={MERCHANT_B}"),
            Some(&label),
        )
        .await?;
    assert_eq!(
        foreign_status, 404,
        "scoped to {MERCHANT_B}, {MERCHANT_A}'s id must answer the same 404 a missing one \
         does: {foreign_body}"
    );
    Ok(())
}

/// **The uniform-404 property is unweakened for a non-admin**, even when a
/// non-admin sends the exact query parameter an admin's request honours.
///
/// This is the mutation that matters most: if `?merchant_id=` were read
/// before `staff.is_admin` were checked, a non-admin could use it to move
/// the tenant it reads exactly as an admin does, which is precisely the
/// oracle ADR-0018 says an admin role must not open for anyone who is not
/// an admin.
#[tokio::test]
async fn a_non_admin_cannot_move_the_tenant_with_the_query_parameter() -> anyhow::Result<()> {
    let harness = harness().await?;
    let repositories = harness.repositories.as_ref();
    seed_intent(repositories, MERCHANT_A, "pi_dash_nonadmin_a").await?;
    seed_intent(repositories, MERCHANT_B, "pi_dash_nonadmin_b").await?;

    let label = harness.reader_label(); // STAFF_ID, is_admin = false

    let (list_status, list_body) = harness
        .dash_json_as(
            &format!("/dash/v1/payment_intents?merchant_id={MERCHANT_B}"),
            &label,
        )
        .await?;
    assert_eq!(list_status, 200, "{list_body}");
    let ids: Vec<&str> = at(&list_body, "/data")
        .as_array()
        .expect("the list envelope carries an array")
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        vec!["pi_dash_nonadmin_a"],
        "a non-admin's ?merchant_id= must be silently ignored, not honoured: {list_body}"
    );
    assert!(
        !ids.contains(&"pi_dash_nonadmin_b"),
        "a non-admin must never see another tenant's rows, override or not: {list_body}"
    );

    // The uniform-404 half: naming a foreign id, with the override present,
    // must still answer the same 404 a missing id would — not the row, and
    // not a *different* refusal that would tell the two cases apart.
    let (foreign_status, foreign_body) = harness
        .get(
            &format!("/dash/v1/payment_intents/pi_dash_nonadmin_b?merchant_id={MERCHANT_B}"),
            Some(&label),
        )
        .await?;
    let (absent_status, absent_body) = harness
        .get(
            &format!("/dash/v1/payment_intents/pi_never_written_at_all?merchant_id={MERCHANT_B}"),
            Some(&label),
        )
        .await?;
    assert_eq!(foreign_status, 404, "{foreign_body}");
    assert_eq!(absent_status, 404, "{absent_body}");
    assert_eq!(
        foreign_body.replace("pi_dash_nonadmin_b", "X"),
        absent_body.replace("pi_never_written_at_all", "X"),
        "a non-admin's override parameter must not turn this surface into an existence oracle \
         for another tenant's ids"
    );
    Ok(())
}

/// **The flag defaults to non-admin.** [`seed_staff`] writes [`STAFF_ID`]'s
/// row through the same `vpay_db::NewStaff` literal every other case in
/// this file uses, with `is_admin: false` named explicitly — this test is
/// what makes that a proven property of the boundary rather than an
/// assertion resting on a fixture nobody re-checks. A row that named no
/// value at all is not reachable from Rust (the field is not `Option` and
/// carries no `#[derive(Default)]`); what this test guards is the *other*
/// way that guarantee could be lost — a future default on the column, or a
/// boundary that reads `Ok(None)` as "admin" instead of refusing.
#[tokio::test]
async fn the_admin_flag_defaults_to_non_admin() -> anyhow::Result<()> {
    let harness = harness().await?;
    let staff = vpay_db::Staff::find(harness.repositories.as_ref(), STAFF_ID)
        .await?
        .expect("seed_staff wrote this row");
    assert!(
        !staff.is_admin,
        "seed_staff names is_admin explicitly and it must be false by default"
    );

    seed_intent(
        harness.repositories.as_ref(),
        MERCHANT_B,
        "pi_dash_default_b",
    )
    .await?;
    let label = harness.reader_label();
    let (status, body) = harness
        .dash_json_as(
            &format!("/dash/v1/payment_intents?merchant_id={MERCHANT_B}"),
            &label,
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        at(&body, "/data").as_array().map(Vec::len),
        Some(0),
        "a non-admin row, even one that names an override, reads only its own merchant: {body}"
    );
    Ok(())
}

/// An admin naming a merchant this deployment does not serve gets a `400`
/// naming the parameter, not a silently empty page — the same "an unknown
/// value is not an empty answer" rule `status` already follows on this
/// surface.
#[tokio::test]
async fn an_admin_naming_an_unknown_merchant_is_a_400() -> anyhow::Result<()> {
    let harness = harness().await?;
    let label = harness.admin_label();

    let (status, body) = harness
        .get(
            "/dash/v1/payment_intents?merchant_id=no-such-tenant-anywhere",
            Some(&label),
        )
        .await?;
    assert_eq!(status, 400, "{body}");
    assert!(
        body.contains("merchant_id"),
        "the 400 must name the parameter an operator got wrong: {body}"
    );
    Ok(())
}

// ------------------------------------------------------- the envelope: refusals

/// A refusal's whole answer, for byte comparison: status, whether it was sealed,
/// and the body.
async fn refusal_of(
    answer: &support::sealed::SealedResponse,
) -> anyhow::Result<(u16, bool, Vec<u8>)> {
    Ok((
        answer.status().as_u16(),
        answer.is_sealed(),
        answer.bytes().await?.to_vec(),
    ))
}

/// Every route in `DASH_ROUTES` refuses a request that carries no signature,
/// with the layer's own coarse `401`.
///
/// Walks the constant rather than listing paths, for `V1_ROUTES`' reason: a
/// route added to the table without the layer in front of it is what this
/// catches, and a hand-written list would simply not mention it.
///
/// The decisive mutation: build the dash layer with `EnvelopeMode::Optional`.
/// An unsigned request then reaches `dash_scope`, which answers a `500` ("no
/// `VerifiedSigner`") — a status, a body and a reason this assertion names.
#[tokio::test]
async fn no_dash_route_is_reachable_without_a_signature() -> anyhow::Result<()> {
    let harness = harness().await?;

    assert!(
        !vpay_api::DASH_ROUTES.is_empty(),
        "an empty table proves nothing"
    );
    for route in vpay_api::DASH_ROUTES {
        // `{id}` is a path parameter; any well-formed value does, because the
        // refusal happens before the router matches.
        let path = format!(
            "{}{}",
            vpay_api::DASH_NEST,
            route.path.replace("{id}", "pi_anything")
        );
        for method in route.methods {
            let answer = harness
                .unsigned(method.parse().expect("a method"), &path)
                .await?;
            assert_coarse_401(
                answer.status().as_u16(),
                answer.is_sealed(),
                &answer.bytes().await?,
                &format!("{method} {path}"),
            );
        }
    }
    Ok(())
}

/// Every way to be refused for the key alone is the same unsigned bytes: a key
/// nobody paired, a **merchant's** key, a **service's** key, a wrong audience.
///
/// A refusal that varied with the cause would say which part of a guess was
/// right. And the three keys that are not console keys are the heart of the
/// credential boundary: `/dash/v1` resolves a signature against `console_keys`
/// and nothing else, so a merchant's registered Ed25519 key — valid on `/v1` the
/// same minute — is not a key here.
///
/// The decisive mutation: resolve `/dash/v1` signatures against the merchant
/// registry as well. The merchant case then reads `200`.
#[tokio::test]
async fn a_key_that_is_not_a_paired_console_key_is_the_same_coarse_refusal() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;
    let url = format!("{}/dash/v1/payment_intents", harness.base_url);

    let unpaired = SealedHttp::new()
        .get(&url)
        .signed_by_console("never-paired")
        .send()
        .await?;
    let merchant = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_A)
        .send()
        .await?;
    let service = SealedHttp::new()
        .get(&url)
        .signed_by_service("some-service")
        .send()
        .await?;
    let wrong_audience = SealedHttp::new()
        .get(&url)
        .signed_by_console(READER)
        .addressed_to("https://some-other-vpay.example")
        .send()
        .await?;
    let unsigned = harness
        .unsigned(reqwest::Method::GET, "/dash/v1/payment_intents")
        .await?;

    let reference = refusal_of(&unpaired).await?;
    assert_coarse_401(reference.0, reference.1, &reference.2, "an unpaired key");
    for (what, answer) in [
        ("a merchant's key", &merchant),
        ("a service's key", &service),
        ("the wrong audience", &wrong_audience),
    ] {
        assert_eq!(refusal_of(answer).await?, reference, "{what}");
        assert!(
            !String::from_utf8_lossy(&answer.bytes().await?).contains("pi_dash_a_only"),
            "{what}: a refused request carries no rows"
        );
    }
    // An unsigned request asked for JSON, so its refusal is the JSON spelling
    // of the same document; it is still the layer's.
    assert_coarse_401(
        unsigned.status().as_u16(),
        unsigned.is_sealed(),
        &unsigned.bytes().await?,
        "no signature",
    );

    // The merchant key is a real credential: the same minute it reads `/v1`.
    let (v1_status, v1_body) = harness.v1_get(CLIENT_A, "/v1/payment_intents").await?;
    assert_eq!(v1_status, 200, "{v1_body}");
    Ok(())
}

/// A console key is not a merchant credential either: signed correctly and
/// paired, it is the coarse `401` on `/v1`.
///
/// The other half of the boundary above, and the reason the two surfaces have
/// two registries and two resolvers rather than one table with a column.
#[tokio::test]
async fn a_console_key_is_refused_on_v1() -> anyhow::Result<()> {
    let harness = harness().await?;
    let url = format!("{}/v1/payment_intents", harness.base_url);

    let answer = SealedHttp::new()
        .get(&url)
        .signed_by(CLIENT_A)
        .signed_with_console_scalar(sealed::console_scalar(READER))
        .send()
        .await?;
    assert_coarse_401(
        answer.status().as_u16(),
        answer.is_sealed(),
        &answer.bytes().await?,
        "a paired console key on /v1",
    );
    Ok(())
}

/// A console key is good for 30 days from the instant it was paired, and not
/// one instant longer. A key paired 29 days ago still reads; one paired 31 days
/// ago is the coarse `401`. The expiry is enforced on every request, with no
/// job to run.
///
/// The decisive mutation: drop `expires_at > now` from
/// `live_console_keys_for_kid` and the 31-day-old key reads.
#[tokio::test]
async fn a_key_expires_thirty_days_after_it_was_paired() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;
    let now = OffsetDateTime::now_utc();

    harness
        .pair(STAFF_ID, "twenty-nine-days", now - Duration::days(29))
        .await?;
    harness
        .pair(STAFF_ID, "thirty-one-days", now - Duration::days(31))
        .await?;

    let (fresh_status, fresh_body) = harness
        .dash_json_as("/dash/v1/payment_intents", "twenty-nine-days")
        .await?;
    assert_eq!(fresh_status, 200, "{fresh_body}");

    let url = format!("{}/dash/v1/payment_intents", harness.base_url);
    let stale = SealedHttp::new()
        .get(&url)
        .signed_by_console("thirty-one-days")
        .send()
        .await?;
    assert_coarse_401(
        stale.status().as_u16(),
        stale.is_sealed(),
        &stale.bytes().await?,
        "a key paired 31 days ago",
    );
    assert!(
        !String::from_utf8_lossy(&stale.bytes().await?).contains("pi_dash_a_only"),
        "an expired key reads nothing"
    );
    Ok(())
}

/// Revocation takes effect on the **next request**: the same key reads, is
/// revoked, and is the coarse `401` the very next time — no cache, no TTL.
///
/// The decisive mutation: cache `live_console_keys_for_kid` in the resolver for
/// any time at all and the second read here is a `200`.
#[tokio::test]
async fn a_revoked_key_is_refused_on_its_very_next_request() -> anyhow::Result<()> {
    let harness = harness().await?;
    let thumbprint = harness
        .pair(STAFF_ID, "to-be-revoked", OffsetDateTime::now_utc())
        .await?;

    let (before, body) = harness
        .dash_json_as("/dash/v1/payment_intents", "to-be-revoked")
        .await?;
    assert_eq!(before, 200, "{body}");

    assert!(
        harness
            .repositories
            .revoke_console_key(&thumbprint, OffsetDateTime::now_utc())
            .await?,
        "this call revoked it"
    );

    let url = format!("{}/dash/v1/payment_intents", harness.base_url);
    let after = SealedHttp::new()
        .get(&url)
        .signed_by_console("to-be-revoked")
        .send()
        .await?;
    assert_coarse_401(
        after.status().as_u16(),
        after.is_sealed(),
        &after.bytes().await?,
        "a revoked key",
    );

    // The member's other key is untouched: revocation is per key.
    let (other, other_body) = harness.dash_json("/dash/v1/payment_intents").await?;
    assert_eq!(other, 200, "{other_body}");
    Ok(())
}

/// A request is accepted once. The identical bytes sent again are the coarse
/// `401`, and the first answer is an ordinary sealed `200`.
#[tokio::test]
async fn a_replayed_request_is_refused_the_second_time() -> anyhow::Result<()> {
    let harness = harness().await?;
    let url = format!("{}/dash/v1/payment_intents", harness.base_url);

    let answers = SealedHttp::new()
        .get(&url)
        .signed_by_console(READER)
        .send_replayed(2)
        .await?;
    let [first, second] = answers.as_slice() else {
        panic!("two answers were asked for");
    };
    assert_eq!(first.status().as_u16(), 200);
    assert!(first.is_sealed(), "an accepted answer is sealed");
    assert_coarse_401(
        second.status().as_u16(),
        second.is_sealed(),
        &second.bytes().await?,
        "the replay",
    );
    Ok(())
}

/// The member behind a verified key is re-read on every request: disabling them
/// takes effect on their next request, and a key whose member belongs to
/// **another merchant** than the dashboard's reads nothing. One answer for both
/// — a sealed `403`, since the signature did verify.
#[tokio::test]
async fn a_disabled_member_and_a_foreign_member_are_refused_after_the_signature_verified()
-> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(harness.repositories.as_ref(), MERCHANT_A, "pi_dash_a_only").await?;

    // The other merchant's member, with a live key of its own.
    create_staff(
        harness.repositories.as_ref(),
        "stf_000000000000000000other",
        MERCHANT_B,
        "other@example.test",
        false,
    )
    .await?;
    harness
        .pair(
            "stf_000000000000000000other",
            "elsewhere",
            OffsetDateTime::now_utc(),
        )
        .await?;
    let (foreign_status, foreign_body) = harness
        .get("/dash/v1/payment_intents", Some("elsewhere"))
        .await?;
    assert_eq!(foreign_status, 403, "{foreign_body}");
    assert!(!foreign_body.contains("pi_dash_a_only"), "{foreign_body}");

    // The control reads; then the member is disabled; then it does not.
    let (control, control_body) = harness.dash_json("/dash/v1/payment_intents").await?;
    assert_eq!(control, 200, "{control_body}");
    sqlx::query("UPDATE staff_members SET status = 'disabled' WHERE id = $1")
        .bind(STAFF_ID)
        .execute(&harness.pool)
        .await
        .context("disabling the member")?;
    let (disabled_status, disabled_body) = harness
        .get("/dash/v1/payment_intents", Some(READER))
        .await?;
    assert_eq!(disabled_status, 403, "{disabled_body}");
    assert_eq!(
        foreign_body, disabled_body,
        "a disabled member and a foreign one are one answer"
    );
    Ok(())
}

/// A bodiless anonymous `OPTIONS` is the router's `405`, never a `500`.
///
/// cratestack's layer lets a bodiless `OPTIONS` through unsigned (a CORS
/// preflight carries no body to sign), and `route_layer` wraps a method router's
/// own `405` fallback, so the request reaches the scope step with no
/// `VerifiedSigner`. That step must pass it on rather than raise the "layer is
/// not mounted" `500`: an anonymous caller must not be able to page an operator
/// by asking.
///
/// The decisive mutation: delete the `Method::OPTIONS` arm in `dash_scope`.
#[tokio::test]
async fn an_anonymous_options_is_the_routers_405_not_a_500() -> anyhow::Result<()> {
    let harness = harness().await?;

    for path in [
        "/dash/v1/payment_intents",
        "/dash/v1/payment_intents/pi_anything",
        "/dash/v1/$procs/searchPaymentIntents",
    ] {
        let answer = harness.unsigned(reqwest::Method::OPTIONS, path).await?;
        assert_eq!(
            answer.status().as_u16(),
            405,
            "OPTIONS {path} must be the router's 405, not a 500: {}",
            String::from_utf8_lossy(&answer.bytes().await?)
        );
        assert!(!answer.is_sealed(), "{path}");
    }
    Ok(())
}

/// A write method is refused by the layer — before the router matches and
/// before any signature is looked at — not by the route table happening to
/// mount no `post(..)`.
///
/// Two spellings, both the layer's own: a plain write is its `405` with `Allow:
/// GET`, and a *signed* one (a COSE body, bound to the read's contract because
/// no contract lists a write) is its `415`. Neither reaches `dash_scope`, so an
/// administrator's key is no different — which the second loop proves directly.
///
/// The decisive mutation: list `POST` in `DASH_ROUTES`' methods for
/// `/payment_intents`; the plain write then answers `404` or reaches a handler.
#[tokio::test]
async fn a_write_method_is_refused_by_the_layer_not_by_the_route_table() -> anyhow::Result<()> {
    let harness = harness().await?;
    seed_intent(
        harness.repositories.as_ref(),
        MERCHANT_A,
        "pi_dash_readonly",
    )
    .await?;
    let read_digest = vpay_wire::dash_contract_digest("GET", "/payment_intents");

    for label in [READER, ADMIN] {
        for method in [
            reqwest::Method::POST,
            reqwest::Method::PUT,
            reqwest::Method::PATCH,
            reqwest::Method::DELETE,
        ] {
            for path in [
                "/dash/v1/payment_intents",
                "/dash/v1/payment_intents/pi_dash_readonly",
                "/dash/v1/payment_intents?merchant_id=beta-douala-tenant",
            ] {
                // A plain write.
                let plain = harness.unsigned(method.clone(), path).await?;
                assert_eq!(
                    plain.status().as_u16(),
                    405,
                    "{method} {path}: {}",
                    String::from_utf8_lossy(&plain.bytes().await?)
                );
                assert!(
                    plain
                        .headers()
                        .get("allow")
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|allow| allow.contains("GET")),
                    "{method} {path}: the layer's 405 names what is allowed"
                );

                // A signed write: sealed under a real, live key, bound to the
                // read's contract.
                let template = if path.contains("pi_dash_readonly") {
                    "/payment_intents/{id}"
                } else {
                    "/payment_intents"
                };
                let signed = SealedHttp::new()
                    .request(method.as_str(), format!("{}{path}", harness.base_url))
                    .signed_by_console(label)
                    .bound_to(template, vpay_wire::dash_contract_digest("GET", template))
                    .send()
                    .await?;
                assert_eq!(
                    signed.status().as_u16(),
                    415,
                    "{label}: {method} {path}: {}",
                    String::from_utf8_lossy(&signed.bytes().await?)
                );
                assert!(!signed.is_sealed(), "{label}: {method} {path}");
                assert!(
                    !String::from_utf8_lossy(&signed.bytes().await?).contains("pi_dash_readonly")
                        || path.contains("pi_dash_readonly"),
                    "a refused request must not carry rows"
                );
            }
        }
    }
    let _ = read_digest;

    // The same keys still read, so the refusals are about the method.
    for label in [READER, ADMIN] {
        let (status, body) = harness.get("/dash/v1/payment_intents", Some(label)).await?;
        assert_eq!(status, 200, "{label}: {body}");
    }
    Ok(())
}

// ----------------------------------------------------------------- unmounted

/// A deployment that registers no `dashboard:` mounts no `/dash/v1` nest, so
/// every path under it is the honest `404`.
///
/// `404` and not `401`: a `401` tells a caller that a credential would help,
/// and this deployment could never accept one. It is also the only way an
/// operator can tell "I forgot to configure the dashboard" apart from "my key is
/// wrong".
///
/// Two independent guards produce the same `404` — `vpay_api::router` does not
/// mount the nest without a `ManagementEnvelope`, and `dash_scope` answers
/// `UnknownRoute` when the binding is absent — and removing one leaves the
/// other. That redundancy is deliberate: a router assembled by some other
/// binary is not obliged to consult the first.
#[tokio::test]
async fn a_deployment_with_no_dashboard_mounts_no_dash_nest() -> anyhow::Result<()> {
    let harness = harness_with(false).await?;

    // With no signature at all — a mounted nest would answer 401 here.
    let anonymous = harness
        .unsigned(reqwest::Method::GET, "/dash/v1/payment_intents")
        .await?;
    assert_eq!(
        anonymous.status().as_u16(),
        404,
        "{}",
        String::from_utf8_lossy(&anonymous.bytes().await?)
    );

    // And with a signature that would be perfectly valid on a dashboarded
    // deployment (the key is paired, as on any other harness here), so the 404
    // is about the nest and not about the key.
    let signed = SealedHttp::new()
        .get(format!("{}/dash/v1/payment_intents", harness.base_url))
        .signed_by_console(READER)
        .send()
        .await?;
    assert_eq!(
        signed.status().as_u16(),
        404,
        "{}",
        String::from_utf8_lossy(&signed.bytes().await?)
    );
    Ok(())
}
