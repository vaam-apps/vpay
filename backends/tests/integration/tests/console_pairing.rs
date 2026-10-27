//! `POST /svc/v1/console_keys` and `DELETE /svc/v1/console_keys/{thumbprint}`,
//! end to end: the real `vpay_api::router` on a real socket, over a real
//! Postgres (VP-6a,
//! [ADR-0036](../../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).
//!
//! `/svc/v1` is the door vaam walks through after its admin phone has vouched
//! for a person: a *service* signs a request with an Ed25519 key registered in
//! `service_clients`, names the person, and hands over the console key the
//! person's browser generated, with a proof that the browser holds its private
//! half. These cases are the acceptance of that door, each a thing the owner's
//! rulings forbid or require:
//!
//! 1. a paired key reads `/dash/v1` as a **plain member of the dashboard's
//!    merchant**, lives exactly 30 days, and pairing it twice is idempotent;
//! 2. a **merchant's** key, a console key and an unsigned request cannot reach
//!    `/svc` at all — the layer's coarse `401`;
//! 3. a service without `console_pairing` is refused (`403`) and writes nothing,
//!    and a *disabled* service is the coarse `401`;
//! 4. pairing never creates or upgrades an administrator: a body naming a role
//!    is a `400`, an already-linked administrator is refused, a person promoted
//!    after their first pairing is refused on the next, and an administrator who
//!    exists under the address a vaam id would be provisioned at is not hijacked;
//! 5. a bad proof, a stale one and a compressed point write nothing;
//! 6. a key cannot be moved to another person, and a revoked key is never
//!    reinstated;
//! 7. revocation is idempotent and takes effect on the key's next request.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;

use anyhow::Context as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use time::OffsetDateTime;
use vpay_config::{
    Config, CurrencyEntry, Dashboard, Deployment, HostEntry, ProviderHost, ServiceCapability,
};
use vpay_db::{Repositories, UnitOfWork as _};

mod support;

use support::sealed::{self, SealedHttp};
use support::{
    assert_coarse_401, create_staff, ensure_crypto_provider_installed, generate_key,
    merchant_client, migrated_postgres, serve, service_client,
};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const CLIENT_B: &str = "beta-douala";
const MERCHANT_B: &str = "beta-douala-tenant";

/// The service that holds `console_pairing` — vaam's.
const VAAM: &str = "vaam";
/// A registered service that holds nothing.
const BYSTANDER: &str = "bystander";

const PUSH_RAIL: &str = "mtn_momo";
const CURRENCY: &str = "XAF";

const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    #[allow(dead_code)]
    server: tokio::task::JoinHandle<()>,
    repositories: std::sync::Arc<dyn Repositories>,
    pool: PgPool,
    base_url: String,
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// `POST /svc/v1/console_keys` signed by `service`, with `body`.
    async fn pair_as(&self, service: &str, body: &Value) -> anyhow::Result<(u16, Value)> {
        let answer = SealedHttp::new()
            .post(self.url("/svc/v1/console_keys"))
            .signed_by_service(service)
            .json(body)
            .send()
            .await
            .context("sending a pairing request")?;
        let status = answer.status().as_u16();
        let text = answer.text().await?;
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }

    async fn pair(&self, body: &Value) -> anyhow::Result<(u16, Value)> {
        self.pair_as(VAAM, body).await
    }

    /// `DELETE /svc/v1/console_keys/{thumbprint}` signed by vaam.
    async fn revoke(&self, thumbprint: &str) -> anyhow::Result<(u16, Value)> {
        let answer = SealedHttp::new()
            .delete(self.url(&format!("/svc/v1/console_keys/{thumbprint}")))
            .signed_by_service(VAAM)
            .send()
            .await
            .context("sending a revocation")?;
        let status = answer.status().as_u16();
        let text = answer.text().await?;
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }

    /// `GET /dash/v1/payment_intents` signed by console key `label`.
    async fn read_as(&self, label: &str) -> anyhow::Result<support::sealed::SealedResponse> {
        SealedHttp::new()
            .get(self.url("/dash/v1/payment_intents"))
            .signed_by_console(label)
            .send()
            .await
            .context("reading as a console key")
    }

    async fn count(&self, table: &str) -> anyhow::Result<i64> {
        // The table name is one of this file's literals, never input.
        let sql = match table {
            "staff_members" => "SELECT count(*) FROM staff_members",
            "console_keys" => "SELECT count(*) FROM console_keys",
            "credentials" => "SELECT count(*) FROM credentials",
            other => panic!("count() does not know {other}"),
        };
        Ok(sqlx::query_scalar(sql).fetch_one(&self.pool).await?)
    }
}

/// A well-formed pairing body for console key `label` and vaam user `user`.
fn body_for(label: &str, user: &str) -> Value {
    let issued_at = OffsetDateTime::now_utc().unix_timestamp();
    let public_key = sealed::console_public_b64(label);
    json!({
        "vaam_user_id": user,
        "display_name": format!("Person {user}"),
        "label": label,
        "public_key": public_key,
        "possession_proof": sealed::possession_proof(label, &public_key, issued_at),
        "issued_at": issued_at,
    })
}

fn config_with(base_url: &str, jwks_a: Value, jwks_b: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "console-pairing".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces: None,
        },
        providers: vec![ProviderHost {
            code: PUSH_RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
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
        dashboard: Some(Dashboard {
            merchant_id: MERCHANT_A.to_owned(),
        }),
        service_clients: vec![
            service_client(VAAM, &[ServiceCapability::ConsolePairing]),
            // Registered, signing, holding nothing: the capability check is
            // the only thing standing between it and the pairing handler.
            service_client(BYSTANDER, &[]),
        ],
        routing: Default::default(),
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();
    let (container, repositories, pool) = migrated_postgres().await?;
    let (server_pem, _server_jwks) = generate_key();
    let (_pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();
    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, jwks_a, jwks_b)
    })
    .await?;
    Ok(Harness {
        _container: container,
        server: served.server,
        repositories,
        pool,
        base_url: served.base_url,
    })
}

/// `body[key] = value`, without the indexing the workspace lints refuse.
fn set(body: &mut Value, key: &str, value: Value) {
    body.as_object_mut()
        .expect("a JSON object")
        .insert(key.to_owned(), value);
}

fn at<'a>(body: &'a Value, pointer: &str) -> &'a Value {
    body.pointer(pointer)
        .unwrap_or_else(|| panic!("{pointer} is absent from {body}"))
}

// ------------------------------------------------------------------ pairing

/// A pairing provisions a plain member of the dashboard's merchant, stores a
/// key that lives exactly 30 days, and the person reads `/dash/v1` with it —
/// their own merchant's rows and no other's. Pairing the same key again is a
/// `200` and writes nothing.
///
/// The decisive mutations: provision with `is_admin: true` (the member row says
/// so and the read honours `?merchant_id=`); take the tenant from anywhere but
/// `dashboard.merchant_id`; make the lifetime anything but 720 hours.
#[tokio::test]
async fn vaam_pairs_a_key_and_the_person_reads_as_a_plain_member_for_thirty_days()
-> anyhow::Result<()> {
    let harness = harness().await?;
    support::sealed::console_thumbprint("alice-browser");
    for (id, merchant) in [("pi_svc_a", MERCHANT_A), ("pi_svc_b", MERCHANT_B)] {
        harness
            .repositories
            .insert(&vpay_db::NewPaymentIntent {
                id: id.to_owned(),
                merchant_id: merchant.to_owned(),
                livemode: false,
                amount: 5000,
                currency_code: CURRENCY.to_owned(),
                status: vpay_core::IntentStatus::INITIAL.as_wire_str().to_owned(),
                payment_method_types: json!([PUSH_RAIL]),
                metadata: json!({}),
                description: None,
                customer_id: None,
                last_payment_error_code: None,
                last_payment_error_message: None,
                client_secret_suffix: format!("svcsuffixmarker{id:x<32}"),
                created_at: OffsetDateTime::now_utc(),
            })
            .await?;
    }

    let body = body_for("alice-browser", "alice-at-vaam");
    let (status, key) = harness.pair(&body).await?;
    assert_eq!(status, 201, "{key}");
    assert_eq!(
        at(&key, "/thumbprint"),
        &json!(sealed::console_thumbprint("alice-browser")),
        "{key}"
    );
    assert_eq!(at(&key, "/alg"), "esp256", "{key}");
    assert_eq!(
        at(&key, "/expires_at").as_i64().unwrap() - at(&key, "/paired_at").as_i64().unwrap(),
        THIRTY_DAYS_SECS,
        "a key lives exactly 30 days from the pairing: {key}"
    );
    assert_eq!(at(&key, "/revoked_at"), &Value::Null, "{key}");
    assert!(
        !key.to_string()
            .contains(&sealed::console_public_b64("alice-browser")),
        "the public point is not echoed: {key}"
    );

    // The member: plain, and of the dashboard's merchant.
    let staff_id = at(&key, "/staff_member_id").as_str().unwrap().to_owned();
    let member = vpay_db::Staff::find(harness.repositories.as_ref(), &staff_id)
        .await?
        .expect("pairing provisioned a member");
    assert!(!member.is_admin, "a pairing never creates an administrator");
    assert_eq!(member.merchant_id, MERCHANT_A);
    assert_eq!(harness.count("staff_members").await?, 1);

    // The person reads their own tenant — and `?merchant_id=` is not honoured.
    let read = harness.read_as("alice-browser").await?;
    assert_eq!(read.status().as_u16(), 200, "{}", read.text().await?);
    let listed: Value = read.json().await?;
    let ids: Vec<&str> = at(&listed, "/data")
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, ["pi_svc_a"], "{listed}");
    let crossing = SealedHttp::new()
        .get(harness.url(&format!(
            "/dash/v1/payment_intents?merchant_id={MERCHANT_B}"
        )))
        .signed_by_console("alice-browser")
        .send()
        .await?;
    let crossing: Value = crossing.json().await?;
    let ids: Vec<&str> = at(&crossing, "/data")
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|intent| intent.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        ["pi_svc_a"],
        "a paired member is never an administrator: {crossing}"
    );

    // Idempotent: the same key for the same person is a 200 and one row.
    let (again, repeat) = harness
        .pair(&body_for("alice-browser", "alice-at-vaam"))
        .await?;
    assert_eq!(again, 200, "{repeat}");
    assert_eq!(at(&repeat, "/thumbprint"), at(&key, "/thumbprint"));
    assert_eq!(harness.count("console_keys").await?, 1);
    assert_eq!(harness.count("staff_members").await?, 1);
    Ok(())
}

/// A second key for the same person is a second row on the same member.
#[tokio::test]
async fn a_second_key_for_the_same_person_joins_the_same_member() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (_, first) = harness.pair(&body_for("laptop", "bob-at-vaam")).await?;
    let (status, second) = harness.pair(&body_for("phone", "bob-at-vaam")).await?;
    assert_eq!(status, 201, "{second}");
    assert_eq!(
        at(&first, "/staff_member_id"),
        at(&second, "/staff_member_id")
    );
    assert_eq!(harness.count("staff_members").await?, 1);
    assert_eq!(harness.count("console_keys").await?, 2);
    Ok(())
}

// --------------------------------------------------- who may call /svc/v1

/// A merchant's key, a paired console key and an unsigned request cannot reach
/// `/svc/v1`: each is the layer's coarse `401`, on both routes, and nothing is
/// written.
///
/// The merchant key is a real credential, valid on `/v1` the same minute: `/svc`
/// resolves signatures against `service_clients` and nothing else.
///
/// The decisive mutation: resolve `/svc/v1` signatures against the merchant
/// registry, or against both registries. The merchant case then pairs a key.
#[tokio::test]
async fn a_merchant_key_cannot_reach_svc() -> anyhow::Result<()> {
    let harness = harness().await?;
    let body = body_for("mallory-browser", "mallory");

    let merchant = SealedHttp::new()
        .post(harness.url("/svc/v1/console_keys"))
        .signed_by(CLIENT_A)
        .json(&body)
        .send()
        .await?;
    assert_coarse_401(
        merchant.status().as_u16(),
        merchant.is_sealed(),
        &merchant.bytes().await?,
        "a merchant's key pairing",
    );

    // A console key already in the table.
    support::pair_console_key(
        harness.repositories.as_ref(),
        &{
            create_staff(
                harness.repositories.as_ref(),
                "stf_000000000000000000plain",
                MERCHANT_A,
                "plain@example.test",
                false,
            )
            .await?;
            "stf_000000000000000000plain".to_owned()
        },
        "existing-console",
        OffsetDateTime::now_utc(),
    )
    .await?;
    let console = SealedHttp::new()
        .post(harness.url("/svc/v1/console_keys"))
        .signed_by_console("existing-console")
        .json(&body)
        .send()
        .await?;
    assert_coarse_401(
        console.status().as_u16(),
        console.is_sealed(),
        &console.bytes().await?,
        "a console key pairing",
    );

    let unsigned = reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
        .post(harness.url("/svc/v1/console_keys"))
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .body(body.to_string())
        .send()
        .await?;
    let unsigned_status = unsigned.status().as_u16();
    assert_coarse_401(
        unsigned_status,
        false,
        &unsigned.bytes().await?,
        "an unsigned pairing",
    );

    // Revocation is the same door.
    let merchant_revoke = SealedHttp::new()
        .delete(harness.url(&format!(
            "/svc/v1/console_keys/{}",
            sealed::console_thumbprint("existing-console")
        )))
        .signed_by(CLIENT_A)
        .send()
        .await?;
    assert_coarse_401(
        merchant_revoke.status().as_u16(),
        merchant_revoke.is_sealed(),
        &merchant_revoke.bytes().await?,
        "a merchant's key revoking",
    );
    let (status, still) = harness
        .read_as("existing-console")
        .await
        .map(|answer| (answer.status().as_u16(), answer))?;
    assert_eq!(status, 200, "{}", still.text().await?);

    assert_eq!(
        harness.count("console_keys").await?,
        1,
        "nothing was paired"
    );
    Ok(())
}

/// Anonymous `OPTIONS` on `/svc/v1` is the router's `405`, not a `500`.
#[tokio::test]
async fn an_anonymous_options_on_svc_is_not_a_500() -> anyhow::Result<()> {
    let harness = harness().await?;
    for path in ["/svc/v1/console_keys", "/svc/v1/console_keys/abc"] {
        let response = reqwest::Client::builder()
            .build()
            .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
            .request(reqwest::Method::OPTIONS, harness.url(path))
            .send()
            .await?;
        assert_eq!(response.status().as_u16(), 405, "OPTIONS {path}");
    }
    Ok(())
}

/// A registered service that does not hold `console_pairing` is refused — a
/// sealed `403` — on both routes, and **nothing is written**: no member, no
/// link, no key.
///
/// The decisive mutation: skip the `service.holds(required)` arm in
/// `service_scope`. The bystander then pairs a key.
#[tokio::test]
async fn a_service_without_console_pairing_cannot_pair_or_revoke() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (status, body) = harness
        .pair_as(BYSTANDER, &body_for("eve-browser", "eve"))
        .await?;
    assert_eq!(status, 403, "{body}");
    assert_eq!(harness.count("staff_members").await?, 0);
    assert_eq!(harness.count("credentials").await?, 0);
    assert_eq!(harness.count("console_keys").await?, 0);

    // The same call from the service that does hold it pairs.
    let (status, key) = harness.pair(&body_for("eve-browser", "eve")).await?;
    assert_eq!(status, 201, "{key}");
    let thumbprint = at(&key, "/thumbprint").as_str().unwrap().to_owned();

    let revoke = SealedHttp::new()
        .delete(harness.url(&format!("/svc/v1/console_keys/{thumbprint}")))
        .signed_by_service(BYSTANDER)
        .send()
        .await?;
    assert_eq!(revoke.status().as_u16(), 403, "{}", revoke.text().await?);
    let read = harness.read_as("eve-browser").await?;
    assert_eq!(read.status().as_u16(), 200, "the key was not revoked");
    Ok(())
}

/// A disabled service (`disabled_clients`, the merchants' kill switch) is the
/// coarse `401` on its very next request, like a disabled merchant.
#[tokio::test]
async fn a_disabled_service_is_refused_on_its_next_request() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (status, body) = harness.pair(&body_for("one", "first-person")).await?;
    assert_eq!(status, 201, "{body}");

    harness
        .repositories
        .disable_client(VAAM, Some("console_pairing test"))
        .await?;
    let answer = SealedHttp::new()
        .post(harness.url("/svc/v1/console_keys"))
        .signed_by_service(VAAM)
        .json(&body_for("two", "second-person"))
        .send()
        .await?;
    assert_coarse_401(
        answer.status().as_u16(),
        answer.is_sealed(),
        &answer.bytes().await?,
        "a disabled service",
    );
    assert_eq!(harness.count("console_keys").await?, 1);
    Ok(())
}

// ------------------------------------------------- never an administrator

/// Pairing cannot create or upgrade an administrator.
///
/// Four doors, all shut:
///
/// * the body has no role field and `deny_unknown_fields` makes one a `400`;
/// * an administrator already linked to a vaam id is **refused** — a service
///   holding `console_pairing` can name any linked person, so this route must not
///   be a way to a key that speaks as an administrator;
/// * a member provisioned plain and promoted afterwards is refused on the next
///   pairing (the flag is read as it is now);
/// * an administrator who exists under the address a vaam id would be provisioned
///   at (`<id>@vaam.invalid`) is not adopted: the pairing is a `409`.
///
/// Nothing is written by any of them. The decisive mutation: provision with
/// `is_admin: true`, or drop `require_pairable`'s `is_admin` arm.
#[tokio::test]
async fn pairing_never_creates_or_upgrades_an_administrator() -> anyhow::Result<()> {
    let harness = harness().await?;

    // (1) A body that tries to name a role.
    for extra in ["is_admin", "role", "merchant_id", "staff_member_id"] {
        let mut body = body_for("grabby", "grabby-person");
        set(&mut body, extra, json!(true));
        let (status, answer) = harness.pair(&body).await?;
        assert_eq!(status, 400, "{extra}: {answer}");
    }
    assert_eq!(harness.count("staff_members").await?, 0);
    assert_eq!(harness.count("console_keys").await?, 0);

    // (2) An administrator linked to a vaam id.
    // An administrator linked to a vaam id, written through the same
    // transaction the first pairing of a person uses.
    let now = OffsetDateTime::now_utc();
    harness
        .repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.provision_federated_staff_in_tx(
                    &vpay_db::NewStaff {
                        id: "stf_0000000000000000000root".to_owned(),
                        merchant_id: MERCHANT_A.to_owned(),
                        email: "root@example.test".to_owned(),
                        display_name: "Root".to_owned(),
                        is_admin: true,
                        now,
                    },
                    vpay_db::NewCredential {
                        id: vpay_core::ids::credential_id(),
                        staff_member_id: Some("stf_0000000000000000000root".to_owned()),
                        kind: vpay_db::CredentialKind::Oidc,
                        material: None,
                        issuer: Some("vaam".to_owned()),
                        subject: Some("root-at-vaam".to_owned()),
                        must_change: false,
                        expires_at: None,
                        now,
                    },
                )
                .await?;
                Ok::<_, vpay_db::DbError>(vpay_db::TxOutcome::Commit(()))
            })
        })
        .await
        .context("linking the administrator to a vaam id")?;
    let (status, answer) = harness
        .pair(&body_for("root-browser", "root-at-vaam"))
        .await?;
    assert_eq!(status, 403, "{answer}");
    assert_eq!(harness.count("console_keys").await?, 0);

    // (3) Promoted after the first pairing.
    let (status, first) = harness
        .pair(&body_for("carol-laptop", "carol-at-vaam"))
        .await?;
    assert_eq!(status, 201, "{first}");
    sqlx::query("UPDATE staff_members SET is_admin = true WHERE id = $1")
        .bind(at(&first, "/staff_member_id").as_str().unwrap())
        .execute(&harness.pool)
        .await?;
    let (status, answer) = harness
        .pair(&body_for("carol-phone", "carol-at-vaam"))
        .await?;
    assert_eq!(status, 403, "{answer}");
    assert_eq!(harness.count("console_keys").await?, 1);

    // (4) An administrator under the address a vaam id would be provisioned at.
    create_staff(
        harness.repositories.as_ref(),
        "stf_000000000000000000squat",
        MERCHANT_A,
        "dave-at-vaam@vaam.invalid",
        true,
    )
    .await?;
    let staff_before = harness.count("staff_members").await?;
    let (status, answer) = harness
        .pair(&body_for("dave-browser", "dave-at-vaam"))
        .await?;
    assert_eq!(status, 409, "{answer}");
    // Not the key's fault, and the answer does not say it is.
    assert!(
        at(&answer, "/error/message")
            .as_str()
            .is_some_and(|message| message.contains("cannot be provisioned")
                && !message.contains("public key")),
        "{answer}"
    );
    assert_eq!(harness.count("staff_members").await?, staff_before);
    assert_eq!(harness.count("console_keys").await?, 1);
    Ok(())
}

/// Two vaam user ids that differ only in letter case map to one synthetic
/// address (the column is lower-case and unique). The second person is refused
/// with an answer about the *identity* ("cannot be provisioned"), not the key's
/// ("already registered"), and the refusal leaves no member, link or key behind
/// even though the key itself was fresh.
///
/// The decisive mutation: provision outside the key's transaction, and the
/// orphan member shows in the counts; map the collision to `KeyTaken`, and the
/// message assertion fails.
#[tokio::test]
async fn an_identity_collision_is_not_a_taken_key_and_writes_nothing() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (status, first) = harness.pair(&body_for("erin-browser", "Erin")).await?;
    assert_eq!(status, 201, "{first}");
    let (members, links, keys) = (
        harness.count("staff_members").await?,
        harness.count("credentials").await?,
        harness.count("console_keys").await?,
    );

    let (status, answer) = harness.pair(&body_for("erin-phone", "erin")).await?;
    assert_eq!(status, 409, "{answer}");
    let message = at(&answer, "/error/message").as_str().unwrap();
    assert!(
        message.contains("cannot be provisioned") && !message.contains("public key"),
        "{answer}"
    );
    assert_eq!(harness.count("staff_members").await?, members);
    assert_eq!(harness.count("credentials").await?, links);
    assert_eq!(harness.count("console_keys").await?, keys);
    Ok(())
}

// --------------------------------------------------- the proof of possession

/// A refused pairing writes nothing — no member, no link, no key — whether the
/// proof is wrong, stale, for a different key, or the point is a compressed
/// encoding of a valid key.
///
/// The order inside `pair_through_vaam` is what makes "nothing" true: the shape
/// of every field, then a stale request, then a bad signature, and only then
/// the database.
#[tokio::test]
async fn a_bad_proof_a_stale_one_and_a_compressed_point_write_nothing() -> anyhow::Result<()> {
    let harness = harness().await?;
    let now = OffsetDateTime::now_utc().unix_timestamp();

    // A proof made by a different key.
    let mut wrong_key = body_for("alice-browser", "frank");
    let wrong_proof = sealed::possession_proof(
        "somebody-else",
        at(&wrong_key, "/public_key").as_str().unwrap(),
        now,
    );
    set(&mut wrong_key, "possession_proof", json!(wrong_proof));
    let (status, answer) = harness.pair(&wrong_key).await?;
    assert_eq!(status, 400, "{answer}");
    assert_eq!(at(&answer, "/error/param"), "possession_proof", "{answer}");

    // A proof over a different `issued_at` than the one sent.
    let mut moved = body_for("alice-browser", "frank");
    set(&mut moved, "issued_at", json!(now + 1));
    let (status, answer) = harness.pair(&moved).await?;
    assert_eq!(status, 400, "{answer}");

    // A proof signed an hour ago: stale, however good the signature.
    let public_key = sealed::console_public_b64("alice-browser");
    let stale_at = now - 3600;
    let stale = json!({
        "vaam_user_id": "frank",
        "public_key": public_key,
        "possession_proof": sealed::possession_proof("alice-browser", &public_key, stale_at),
        "issued_at": stale_at,
    });
    let (status, answer) = harness.pair(&stale).await?;
    assert_eq!(status, 400, "{answer}");
    assert_eq!(at(&answer, "/error/param"), "issued_at", "{answer}");

    // The compressed encoding of a valid key (33 bytes, 0x02/0x03): refused, not
    // normalised.
    let point = sealed::console_public("alice-browser");
    let mut compressed = vec![if point[64].is_multiple_of(2) {
        0x02
    } else {
        0x03
    }];
    compressed.extend_from_slice(&point[1..33]);
    let compressed_b64 = base64_url(&compressed);
    let compressed_body = json!({
        "vaam_user_id": "frank",
        "public_key": compressed_b64,
        "possession_proof": sealed::possession_proof("alice-browser", &compressed_b64, now),
        "issued_at": now,
    });
    let (status, answer) = harness.pair(&compressed_body).await?;
    assert_eq!(status, 400, "{answer}");
    assert_eq!(at(&answer, "/error/param"), "public_key", "{answer}");

    // A vaam id that cannot become an address.
    let mut odd = body_for("alice-browser", "frank");
    set(&mut odd, "vaam_user_id", json!("frank@example.test"));
    let (status, _) = harness.pair(&odd).await?;
    assert_eq!(status, 400);

    assert_eq!(harness.count("staff_members").await?, 0, "no member");
    assert_eq!(harness.count("credentials").await?, 0, "no link");
    assert_eq!(harness.count("console_keys").await?, 0, "no key");
    Ok(())
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// ------------------------------------------------------ keys are not moved

/// A key is never moved to another person and never reinstated.
///
/// The thumbprint is unique for ever: pairing a key already held by another
/// member is a `409`; and a revoked key stays taken, so a stolen browser
/// profile's key cannot be re-registered after the person revoked it.
#[tokio::test]
async fn a_key_is_not_moved_to_another_person_and_a_revoked_one_is_not_reinstated()
-> anyhow::Result<()> {
    let harness = harness().await?;
    let (status, first) = harness.pair(&body_for("shared-browser", "gina")).await?;
    assert_eq!(status, 201, "{first}");

    let (members, links) = (
        harness.count("staff_members").await?,
        harness.count("credentials").await?,
    );
    let (status, answer) = harness.pair(&body_for("shared-browser", "hank")).await?;
    assert_eq!(status, 409, "{answer}");
    assert!(
        at(&answer, "/error/message")
            .as_str()
            .is_some_and(|message| message.contains("already registered")),
        "{answer}"
    );
    // A refused first pairing writes nothing: hank was to be provisioned
    // together with the key, so the refusal leaves no orphan member or link.
    assert_eq!(
        harness.count("staff_members").await?,
        members,
        "no orphan member"
    );
    assert_eq!(harness.count("credentials").await?, links, "no orphan link");

    let thumbprint = at(&first, "/thumbprint").as_str().unwrap().to_owned();
    let (status, revoked) = harness.revoke(&thumbprint).await?;
    assert_eq!(status, 200, "{revoked}");
    let (status, answer) = harness.pair(&body_for("shared-browser", "gina")).await?;
    assert_eq!(
        status, 409,
        "a revoked key is never reinstated, not even for its own person: {answer}"
    );
    assert_eq!(harness.count("console_keys").await?, 1);
    Ok(())
}

// --------------------------------------------------------------- revocation

/// Revocation takes effect on the key's next request, is idempotent (the first
/// `revoked_at` stays), and answers the honest errors for a malformed or unknown
/// thumbprint.
///
/// The decisive mutation: skip the `revoked_at IS NULL` guard in the update, and
/// the second revoke moves `revoked_at`.
#[tokio::test]
async fn a_revocation_takes_effect_on_the_next_request_and_is_idempotent() -> anyhow::Result<()> {
    let harness = harness().await?;
    let (_, key) = harness.pair(&body_for("ivy-browser", "ivy")).await?;
    let thumbprint = at(&key, "/thumbprint").as_str().unwrap().to_owned();

    let before = harness.read_as("ivy-browser").await?;
    assert_eq!(before.status().as_u16(), 200);

    let (status, revoked) = harness.revoke(&thumbprint).await?;
    assert_eq!(status, 200, "{revoked}");
    let first_revoked_at = at(&revoked, "/revoked_at").as_i64().expect("revoked");

    let after = harness.read_as("ivy-browser").await?;
    assert_coarse_401(
        after.status().as_u16(),
        after.is_sealed(),
        &after.bytes().await?,
        "the revoked key's next request",
    );

    // Idempotent, and the first instant is kept.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (status, again) = harness.revoke(&thumbprint).await?;
    assert_eq!(status, 200, "{again}");
    assert_eq!(at(&again, "/revoked_at").as_i64(), Some(first_revoked_at));

    // Malformed and unknown.
    let (status, answer) = harness.revoke("not-hex").await?;
    assert_eq!(status, 400, "{answer}");
    let (status, answer) = harness.revoke(&"0".repeat(64)).await?;
    assert_eq!(status, 404, "{answer}");
    Ok(())
}
