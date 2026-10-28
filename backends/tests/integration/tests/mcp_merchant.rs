//! ADR-0033 step 3: the merchant MCP endpoint, end to end.
//!
//! A real `vpay-server` router, over a real Postgres, with tokens minted by
//! the server's own signer (and some by the real OP's `/v1/oauth/token`),
//! called with the JSON-RPC requests an MCP client sends on the wire
//! (revision `2026-07-28`, Streamable HTTP). Each test names the threat-model
//! row it proves (`docs/security/threat-models/mcp.md`).
//!
//! Every `/v1` call here is sealed through [`support::sealed::SealedHttp`]
//! (VP-3) as the merchant whose MCP token the same case uses. Since ADR-0037,
//! `/v1` also takes a `vpay:v1` bearer token where `auth.methods` permits
//! `machine_tokens`; an MCP token is still refused there.
//!
//! The rails here are unreachable by design, as in `payment_intents.rs`:
//! this suite is about the surface. A confirm that a rail settles is
//! `confirm_rails.rs`'s subject, over `/v1`, and the confirm path an MCP
//! call runs is the same function (`confirm_with`).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod support;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Context as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, MERCHANT_AUDIENCE, ProviderHost};
use vpay_db::Repositories;

use support::sealed::SealedHttp;
use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client, merchant_client_with_scopes,
    migrated_postgres, serve,
};

const MERCHANT_A: &str = "acct_mcp_a";
const MERCHANT_B: &str = "acct_mcp_b";
const MERCHANT_R: &str = "acct_mcp_reader";
const CLIENT_A: &str = "mcp-client-a";
const CLIENT_B: &str = "mcp-client-b";
/// Registered for `payments:read` only.
const CLIENT_R: &str = "mcp-client-reader";
/// Registered, and not opted in to MCP: its `allowed_audiences` is
/// `vpay:v1` alone.
const CLIENT_V1: &str = "mcp-client-v1-only";
const MERCHANT_V1: &str = "acct_mcp_v1_only";

const PROTOCOL: &str = "2026-07-28";

struct Harness {
    _container: ContainerAsync<PostgresImage>,
    server: tokio::task::JoinHandle<()>,
    pool: PgPool,
    /// The same repositories the server holds, so a test can flip the kill
    /// switch (`disable_client`) the way an operator does.
    repositories: Arc<dyn Repositories>,
    base_url: String,
    pem_a: String,
    pem_v1: String,
    signing_key: LoadedSigningKey,
}

impl Harness {
    fn resource(&self) -> String {
        vpay_api::mcp::merchant_resource(&self.base_url)
    }

    /// A token for `client_id`, for `audience`, carrying `scope`, minted with
    /// the server's own signer: what the OP would mint.
    fn token(&self, client_id: &str, audience: &str, scope: &str) -> String {
        self.signing_key
            .token_manager()
            .issue_client_token_with_extra(
                client_id,
                900,
                Some(scope.to_owned()),
                Some(audience.to_owned()),
                HashMap::new(),
            )
            .expect("the server's own signer mints a merchant token")
    }

    fn mcp_token(&self, client_id: &str, scope: &str) -> String {
        self.token(client_id, &self.resource(), scope)
    }

    /// One JSON-RPC request to `/mcp/v1`, as a conforming client sends it.
    /// Returns the HTTP status and the body (the JSON-RPC envelope, or the
    /// transport's own error document).
    async fn rpc(
        &self,
        token: Option<&str>,
        origin: Option<&str>,
        method: &str,
        name: Option<&str>,
        params: Value,
    ) -> anyhow::Result<(u16, Value)> {
        let mut params = params;
        if let Some(object) = params.as_object_mut() {
            object.insert(
                "_meta".to_owned(),
                json!({
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": { "name": "vpay-tests", "version": "0" },
                }),
            );
        }
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let mut request = raw_client()
            .post(format!(
                "{}{}",
                self.base_url,
                vpay_api::mcp::MERCHANT_MCP_PATH
            ))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", PROTOCOL)
            .header("mcp-method", method)
            .json(&body);
        if let Some(name) = name {
            request = request.header("mcp-name", name);
        }
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = request.send().await.context("POST /mcp/v1")?;
        let status = response.status().as_u16();
        let text = response.text().await.context("reading the MCP response")?;
        let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok((status, value))
    }

    /// `tools/call`; the JSON-RPC `result` (panics on a protocol error or a
    /// non-200, neither of which is what the call-level tests are about).
    async fn call(&self, token: &str, tool: &str, arguments: Value) -> anyhow::Result<Value> {
        let (status, body) = self
            .rpc(
                Some(token),
                None,
                "tools/call",
                Some(tool),
                json!({ "name": tool, "arguments": arguments }),
            )
            .await?;
        anyhow::ensure!(status == 200, "tools/call {tool}: HTTP {status}: {body}");
        body.get("result")
            .cloned()
            .with_context(|| format!("tools/call {tool} was a protocol error: {body}"))
    }

    async fn intents_of(&self, merchant_id: &str) -> anyhow::Result<i64> {
        sqlx::query_scalar("SELECT count(*) FROM payment_intents WHERE merchant_id = $1")
            .bind(merchant_id)
            .fetch_one(&self.pool)
            .await
            .context("counting payment intents")
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

/// The REST envelope an `isError` result carries as its text.
fn envelope(result: &Value) -> Value {
    assert_eq!(
        at(result, "/isError"),
        json!(true),
        "expected an isError result: {result}"
    );
    let text = text_at(result, "/content/0/text");
    serde_json::from_str(&text).expect("the text is the REST error envelope")
}

fn ok_object(result: &Value) -> Value {
    assert_eq!(
        at(result, "/isError"),
        json!(false),
        "expected a successful result: {result}"
    );
    at(result, "/structuredContent")
}

/// `value`'s member at the JSON pointer `pointer`, or `null`.
///
/// Not `value["a"]["b"]`: `clippy::indexing_slicing` is denied in these
/// suites, and a pointer says the whole path at once.
fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

/// The string at `pointer`, or empty.
fn text_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A client assertion (RFC 7523) as a merchant's client presents it at the
/// token endpoint: `iss = sub = client_id`, the endpoint as `aud`, a fresh
/// `jti`, valid for a minute.
///
/// Signed here with `jsonwebtoken`, as `merchant_token_flow.rs` signs its own,
/// because the merchant SDK no longer mints one: it seals `/v1` requests
/// instead (VP-5).
fn client_assertion(client_id: &str, pem: &str, token_endpoint: &str) -> String {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let claims = json!({
        "iss": client_id,
        "sub": client_id,
        "aud": token_endpoint,
        "jti": uuid::Uuid::new_v4().to_string(),
        "iat": now,
        "exp": now + 60,
    });
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).expect("the PEM is an RSA key"),
    )
    .expect("signing a client assertion")
}

fn raw_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("a plain-HTTP reqwest client builds once a CryptoProvider is installed")
}

fn rail(code: &str) -> ProviderHost {
    ProviderHost {
        code: code.to_owned(),
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
        currency: "XAF".to_owned(),
        credentials: BTreeMap::from([
            (
                "subscription_key".to_owned(),
                "stub-subscription-key".to_owned(),
            ),
            ("api_key".to_owned(), "stub-api-key".to_owned()),
        ]),
        display_name: None,
        callbacks: None,
    }
}

fn config_with(base_url: &str, jwks: [Value; 4], surfaces: Option<Vec<String>>) -> Config {
    let resource = vpay_api::mcp::merchant_resource(base_url);
    let [jwks_a, jwks_b, jwks_r, jwks_v1] = jwks;
    let opted_in = |mut client: vpay_config::MerchantClient| {
        client.allowed_audiences.push(resource.clone());
        client
    };
    Config {
        deployment: Deployment {
            name: "mcp-merchant".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
            surfaces,
        },
        providers: vec![rail("mtn_momo")],
        currencies: vec![CurrencyEntry {
            code: "XAF".to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![
            opted_in(merchant_client(CLIENT_A, MERCHANT_A, jwks_a)),
            opted_in(merchant_client(CLIENT_B, MERCHANT_B, jwks_b)),
            opted_in(merchant_client_with_scopes(
                CLIENT_R,
                MERCHANT_R,
                jwks_r,
                &[vpay_api::SCOPE_PAYMENTS_READ],
            )),
            merchant_client(CLIENT_V1, MERCHANT_V1, jwks_v1),
        ],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard: None,
        service_clients: Vec::new(),
        routing: Default::default(),
        auth: Default::default(),
    }
}

async fn harness_with(surfaces: Option<Vec<String>>) -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();
    let (container, repositories, pool): (_, Arc<dyn Repositories>, _) =
        migrated_postgres().await?;
    let (server_pem, _) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let (_, jwks_b) = generate_key();
    let (_, jwks_r) = generate_key();
    let (pem_v1, jwks_v1) = generate_key();
    let served = serve(&repositories, &server_pem, |base_url| {
        config_with(base_url, [jwks_a, jwks_b, jwks_r, jwks_v1], surfaces)
    })
    .await?;
    Ok(Harness {
        _container: container,
        server: served.server,
        pool,
        repositories,
        base_url: served.base_url,
        pem_a,
        pem_v1,
        signing_key: served.signing_key,
    })
}

/// An MCP pod that also serves `/v1`, so a test can compare the two.
async fn harness() -> anyhow::Result<Harness> {
    harness_with(Some(vec!["business".to_owned(), "mcp".to_owned()])).await
}

fn create_args(key: &str, amount: i64) -> Value {
    json!({ "args": {
        "idempotency_key": key,
        "amount": amount,
        "currency": "XAF",
        "payment_method_types": ["mtn_momo"],
        "description": "Order #42",
        "metadata": [ { "key": "order_id", "value": "1234" } ],
    } })
}

/// The tool table is exactly the surface ADR-0033 step 3 ships: the two
/// existing searches, and the seven payment-intent and refund tools.
#[tokio::test]
async fn the_endpoint_lists_exactly_the_step_3_tools() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let (status, body) = harness
        .rpc(Some(&token), None, "tools/list", None, json!({}))
        .await?;
    assert_eq!(status, 200, "{body}");
    let tools = at(&body, "/result/tools");
    let mut names: Vec<&str> = tools
        .as_array()
        .expect("a tools array")
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "cancel_payment_intent",
            "cancel_refund",
            "confirm_payment_intent",
            "create_payment_intent",
            "create_refund",
            "retrieve_payment_intent",
            "retrieve_refund",
            "search_payment_intents",
            "search_refunds",
        ],
        "{body}"
    );

    // Threat model § 3: a refund tool tells an agent what `docs/status.md`
    // says about refunds, so its success is not read as money returned.
    let description = |name: &str| {
        at(&body, "/result/tools")
            .as_array()
            .and_then(|tools| {
                tools
                    .iter()
                    .find(|tool| tool.get("name") == Some(&json!(name)))
            })
            .and_then(|tool| tool.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let refund = description("create_refund");
    assert!(
        refund.contains("NotImplemented") && refund.contains("pending"),
        "create_refund's description must say what is not built: {refund}"
    );
    assert!(description("create_payment_intent").contains("idempotency_key"));
    harness.shutdown().await;
    Ok(())
}

/// A tool's output is the `/v1` object, reshaped in the three places
/// `schemas/vpay.cstack` names: create over MCP, read the same intent over
/// `/v1`, and compare.
#[tokio::test]
async fn an_intent_created_over_mcp_is_the_one_v1_serves() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);

    let created = ok_object(
        &harness
            .call(
                &token,
                "create_payment_intent",
                create_args("k-create-1", 1500),
            )
            .await?,
    );
    let id = at(&created, "/id").as_str().expect("an id").to_owned();
    assert_eq!(at(&created, "/amount"), json!(1500), "{created}");
    assert_eq!(at(&created, "/status"), json!("requires_payment_method"));
    assert_eq!(
        at(&created, "/metadata"),
        json!([ { "key": "order_id", "value": "1234" } ])
    );

    let retrieved = ok_object(
        &harness
            .call(&token, "retrieve_payment_intent", json!({ "id": id }))
            .await?,
    );

    let v1 = SealedHttp::new()
        .get(format!("{}/v1/payment_intents/{id}", harness.base_url))
        .signed_by(CLIENT_A)
        .send()
        .await?
        .json::<Value>()
        .await?;
    let mut expected = vpay_api::mcp::reshape_payment_intent(v1.clone());
    // The merchant surface renders `client_secret` on a retrieve (`/v1`'s
    // own choice); the MCP type declares no such field, so it is not part
    // of what an agent is handed.
    if let Some(object) = expected.as_object_mut() {
        object.remove("client_secret");
    }
    assert_eq!(retrieved, expected, "the /v1 object was {v1}");
    // Threat model § 4: `/v1`'s retrieve carries the payer credential, and
    // the MCP output type does not, so it never reaches an agent.
    assert!(v1.get("client_secret").is_some(), "{v1}");
    assert!(retrieved.get("client_secret").is_none(), "{retrieved}");
    harness.shutdown().await;
    Ok(())
}

/// Threat model § 3, "Merchant A reads or acts on merchant B's objects":
/// the tenant comes from the token, and B is told exactly what B would be
/// told for an id that does not exist.
#[tokio::test]
async fn merchant_b_cannot_read_or_cancel_merchant_as_intent() -> anyhow::Result<()> {
    let harness = harness().await?;
    let a = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let b = harness.mcp_token(CLIENT_B, vpay_api::SCOPE_PAYMENTS_WRITE);
    let created = ok_object(
        &harness
            .call(&a, "create_payment_intent", create_args("k-tenancy", 100))
            .await?,
    );
    let id = text_at(&created, "/id");

    let foreign = envelope(
        &harness
            .call(&b, "retrieve_payment_intent", json!({ "id": id }))
            .await?,
    );
    let missing = envelope(
        &harness
            .call(
                &b,
                "retrieve_payment_intent",
                json!({ "id": "pi_000000000000000000000000" }),
            )
            .await?,
    );
    assert_eq!(at(&foreign, "/code"), json!("NOT_FOUND"), "{foreign}");
    assert_eq!(at(&foreign, "/code"), at(&missing, "/code"));

    let cancel = envelope(
        &harness
            .call(
                &b,
                "cancel_payment_intent",
                json!({ "args": { "idempotency_key": "k-b-cancel", "id": id } }),
            )
            .await?,
    );
    assert_eq!(at(&cancel, "/code"), json!("NOT_FOUND"), "{cancel}");

    // And A's intent is untouched.
    let still = ok_object(
        &harness
            .call(&a, "retrieve_payment_intent", json!({ "id": id }))
            .await?,
    );
    assert_eq!(at(&still, "/status"), json!("requires_payment_method"));

    // B's search sees none of A's rows.
    let page = ok_object(
        &harness
            .call(
                &b,
                "search_payment_intents",
                json!({ "page": {}, "filter": {} }),
            )
            .await?,
    );
    assert_eq!(at(&page, "/items"), json!([]), "{page}");
    harness.shutdown().await;
    Ok(())
}

/// Threat model § 3, the first row: a `payments:read` token cannot call a
/// write, and the write never ran (no intent exists for that merchant).
#[tokio::test]
async fn a_read_scoped_token_cannot_call_a_mutation() -> anyhow::Result<()> {
    let harness = harness().await?;
    let reader = harness.mcp_token(CLIENT_R, vpay_api::SCOPE_PAYMENTS_READ);
    let refused = envelope(
        &harness
            .call(
                &reader,
                "create_payment_intent",
                create_args("k-reader", 100),
            )
            .await?,
    );
    assert_eq!(at(&refused, "/code"), json!("FORBIDDEN"), "{refused}");
    assert_eq!(harness.intents_of(MERCHANT_R).await?, 0);

    // The same token reads.
    let page = ok_object(
        &harness
            .call(
                &reader,
                "search_payment_intents",
                json!({ "page": {}, "filter": {} }),
            )
            .await?,
    );
    assert_eq!(at(&page, "/items"), json!([]));
    harness.shutdown().await;
    Ok(())
}

/// ADR-0037: `disable_client` reaches an MCP token on its next request, as it
/// reaches a signer and a `/v1` token. A token minted before the switch flipped
/// is otherwise good until it expires (900 s here). The switch is per client:
/// another merchant's token is untouched.
#[tokio::test]
async fn a_token_minted_before_disable_client_is_refused_after_it() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token_a = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let token_b = harness.mcp_token(CLIENT_B, vpay_api::SCOPE_PAYMENTS_WRITE);
    let search = json!({ "page": {}, "filter": {} });

    // The control: both tokens work while the clients are enabled.
    for token in [&token_a, &token_b] {
        harness
            .call(token, "search_payment_intents", search.clone())
            .await?;
    }

    harness
        .repositories
        .disable_client(CLIENT_A, Some("mcp_merchant test"))
        .await
        .context("flipping the kill switch")?;

    let (status, body) = harness
        .rpc(
            Some(&token_a),
            None,
            "tools/call",
            Some("search_payment_intents"),
            json!({ "name": "search_payment_intents", "arguments": search }),
        )
        .await?;
    assert_eq!(
        status, 401,
        "a token minted before the switch is refused the moment it flips: {body}"
    );
    harness
        .call(&token_b, "search_payment_intents", search)
        .await
        .context("another client's token is untouched")?;
    harness.shutdown().await;
    Ok(())
}

/// ADR-0037: a token's scopes are held to the registration's YAML. This token
/// claims `payments:write`; its registration (`CLIENT_R`) grants `payments:read`
/// alone, so it reads and cannot write, whatever the token says.
#[tokio::test]
async fn a_token_claiming_more_than_its_registration_grants_cannot_write() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.mcp_token(
        CLIENT_R,
        &format!(
            "{} {}",
            vpay_api::SCOPE_PAYMENTS_READ,
            vpay_api::SCOPE_PAYMENTS_WRITE
        ),
    );
    let refused = envelope(
        &harness
            .call(
                &token,
                "create_payment_intent",
                create_args("k-widened", 100),
            )
            .await?,
    );
    assert_eq!(at(&refused, "/code"), json!("FORBIDDEN"), "{refused}");
    assert_eq!(harness.intents_of(MERCHANT_R).await?, 0);
    harness
        .call(
            &token,
            "search_payment_intents",
            json!({ "page": {}, "filter": {} }),
        )
        .await?;
    harness.shutdown().await;
    Ok(())
}

/// Threat model § 1: each surface accepts only its own audience, in both
/// directions, and a request without a token is the 401 challenge naming
/// the metadata document.
#[tokio::test]
async fn each_surface_accepts_only_its_own_audience() -> anyhow::Result<()> {
    let harness = harness().await?;

    let v1_token = harness.token(CLIENT_A, MERCHANT_AUDIENCE, vpay_api::SCOPE_PAYMENTS_WRITE);
    let (status, body) = harness
        .rpc(Some(&v1_token), None, "tools/list", None, json!({}))
        .await?;
    assert_eq!(status, 401, "a vpay:v1 token on /mcp/v1: {body}");

    // `/v1` takes a bearer token only for `vpay:v1` (ADR-0037, and only where
    // `machine_tokens` is permitted): an MCP token there is a 401, whichever
    // refusal this deployment's `auth.methods` gives it.
    let mcp_token = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let v1 = raw_client()
        .get(format!("{}/v1/payment_intents", harness.base_url))
        .bearer_auth(&mcp_token)
        .send()
        .await?;
    assert_eq!(v1.status().as_u16(), 401, "an MCP token on /v1");

    let response = raw_client()
        .post(format!("{}/mcp/v1", harness.base_url))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", PROTOCOL)
        .header("mcp-method", "tools/list")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 401);
    let challenge = response
        .headers()
        .get("www-authenticate")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        challenge.contains("resource_metadata="),
        "the challenge names where to authenticate: {challenge}"
    );

    // The metadata document is at the host root and names this resource.
    let metadata = raw_client()
        .get(format!(
            "{}/.well-known/oauth-protected-resource/mcp/v1",
            harness.base_url
        ))
        .send()
        .await?
        .json::<Value>()
        .await?;
    assert_eq!(
        at(&metadata, "/resource"),
        json!(harness.resource()),
        "{metadata}"
    );
    harness.shutdown().await;
    Ok(())
}

/// Threat model § 2, the first row, through the **real** OP: a client that
/// lists the MCP resource in `allowed_audiences` is minted a token for it,
/// and that token is accepted on `/mcp/v1`; a client that does not list it
/// is minted none. The two requests differ in the client alone, so the
/// refusal is the audience gate's and not a malformed request's.
#[tokio::test]
async fn the_op_mints_an_mcp_token_only_for_a_client_that_opted_in() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token_endpoint = format!("{}/v1/oauth/token", harness.base_url);
    let request = |client_id: &'static str, pem: &str| {
        let assertion = client_assertion(client_id, pem, &token_endpoint);
        raw_client().post(&token_endpoint).form(&[
            ("grant_type", "client_credentials".to_owned()),
            ("client_id", client_id.to_owned()),
            (
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".to_owned(),
            ),
            ("client_assertion", assertion),
            ("audience", harness.resource()),
        ])
    };

    let granted = request(CLIENT_A, &harness.pem_a).send().await?;
    assert_eq!(granted.status().as_u16(), 200, "an opted-in client");
    let granted: Value = granted.json().await?;
    let token = text_at(&granted, "/access_token");
    anyhow::ensure!(!token.is_empty(), "the OP answered an access token");
    let (status, body) = harness
        .rpc(Some(&token), None, "tools/list", None, json!({}))
        .await?;
    assert_eq!(status, 200, "the OP's own MCP token on /mcp/v1: {body}");

    let refused = request(CLIENT_V1, &harness.pem_v1).send().await?;
    let status = refused.status().as_u16();
    let body = refused.text().await?;
    assert!(
        (400..500).contains(&status),
        "a client not opted in to MCP must not be minted an MCP token: {status} {body}"
    );
    harness.shutdown().await;
    Ok(())
}

/// RFC 8707, which the MCP authorization specification uses: a client that
/// sends `resource` rather than `audience` is minted a token for that
/// resource, through the same `allowed_audiences` gate. Before
/// `op::token::apply_resource_indicator` the OP dropped the parameter and
/// minted a default-audience token, which `/mcp/v1` then refused `401`
/// (measured 2026-10-26).
#[tokio::test]
async fn an_rfc_8707_resource_is_the_audience_it_names() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token_endpoint = format!("{}/v1/oauth/token", harness.base_url);
    let request = |client_id: &'static str, pem: &str| {
        let assertion = client_assertion(client_id, pem, &token_endpoint);
        raw_client().post(&token_endpoint).form(&[
            ("grant_type", "client_credentials".to_owned()),
            ("client_id", client_id.to_owned()),
            (
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".to_owned(),
            ),
            ("client_assertion", assertion),
            ("resource", harness.resource()),
        ])
    };

    let granted = request(CLIENT_A, &harness.pem_a).send().await?;
    assert_eq!(granted.status().as_u16(), 200);
    let granted: Value = granted.json().await?;
    let token = text_at(&granted, "/access_token");
    anyhow::ensure!(!token.is_empty(), "an access token");
    let (status, body) = harness
        .rpc(Some(&token), None, "tools/list", None, json!({}))
        .await?;
    assert_eq!(
        status, 200,
        "a `resource`-requested token on /mcp/v1: {body}"
    );

    // And the gate still applies to it.
    let refused = request(CLIENT_V1, &harness.pem_v1).send().await?;
    assert_eq!(refused.status().as_u16(), 400);
    let refused: Value = refused.json().await?;
    assert_eq!(at(&refused, "/error"), json!("invalid_target"), "{refused}");
    harness.shutdown().await;
    Ok(())
}

/// Threat model § 1: a browser page on another origin is refused.
#[tokio::test]
async fn a_foreign_origin_is_refused() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let (status, body) = harness
        .rpc(
            Some(&token),
            Some("https://evil.example"),
            "tools/list",
            None,
            json!({}),
        )
        .await?;
    assert_eq!(status, 403, "{body}");
    harness.shutdown().await;
    Ok(())
}

/// Threat model § 3, "A retried call charges twice": a replay answers the
/// first result and writes no second row; the same key for a different call
/// is refused; an empty key is refused before anything is written.
#[tokio::test]
async fn idempotency_is_vpays_own_and_mandatory() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);

    let first = ok_object(
        &harness
            .call(
                &token,
                "create_payment_intent",
                create_args("k-replay", 700),
            )
            .await?,
    );
    let again = ok_object(
        &harness
            .call(
                &token,
                "create_payment_intent",
                create_args("k-replay", 700),
            )
            .await?,
    );
    assert_eq!(
        at(&first, "/id"),
        at(&again, "/id"),
        "a replay is the first answer"
    );
    assert_eq!(harness.intents_of(MERCHANT_A).await?, 1);

    let mismatch = envelope(
        &harness
            .call(
                &token,
                "create_payment_intent",
                create_args("k-replay", 701),
            )
            .await?,
    );
    let message = text_at(&mismatch, "/message");
    assert!(
        message.starts_with("idempotency_key_in_use"),
        "the same key for another call is /v1's mismatch refusal: {mismatch}"
    );

    let empty = envelope(
        &harness
            .call(&token, "create_payment_intent", create_args("", 700))
            .await?,
    );
    assert_eq!(at(&empty, "/code"), json!("BAD_REQUEST"), "{empty}");
    assert_eq!(harness.intents_of(MERCHANT_A).await?, 1);
    harness.shutdown().await;
    Ok(())
}

/// `/v1`'s own rules run on a tool call's arguments, and its refusal keeps
/// `/v1`'s `code` and `param`: a confirm on a payment-method type this
/// deployment has no rail for.
#[tokio::test]
async fn a_confirm_is_refused_for_exactly_what_v1_refuses() -> anyhow::Result<()> {
    let harness = harness().await?;
    let token = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let created = ok_object(
        &harness
            .call(
                &token,
                "create_payment_intent",
                create_args("k-confirm", 100),
            )
            .await?,
    );
    let refused = envelope(
        &harness
            .call(
                &token,
                "confirm_payment_intent",
                json!({ "args": {
                    "idempotency_key": "k-confirm-1",
                    "id": at(&created, "/id"),
                    "payment_method_type": "no_such_rail",
                    "payer_fields": [],
                } }),
            )
            .await?,
    );
    assert_eq!(at(&refused, "/code"), json!("BAD_REQUEST"), "{refused}");
    let message = text_at(&refused, "/message");
    assert!(
        message.contains("payment_method_data[type]"),
        "the refusal names /v1's own parameter: {refused}"
    );
    harness.shutdown().await;
    Ok(())
}

/// The refund tools reach `/v1`'s refund code: a stored refund reads back
/// as the `/v1` object reshaped, only to its own merchant, and a refund of
/// an intent that has taken no money is refused with nothing written.
///
/// A refund that a rail is instructed for is `refunds.rs`'s subject over
/// `/v1`; `create_refund` runs the same `create_with`.
#[tokio::test]
async fn the_refund_tools_reach_v1s_refund_code() -> anyhow::Result<()> {
    let harness = harness().await?;
    let a = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let b = harness.mcp_token(CLIENT_B, vpay_api::SCOPE_PAYMENTS_WRITE);
    let intent = ok_object(
        &harness
            .call(
                &a,
                "create_payment_intent",
                create_args("k-refund-intent", 900),
            )
            .await?,
    );
    let intent_id = at(&intent, "/id").as_str().expect("an id").to_owned();

    // A stored refund row, written directly: the only way to have one for an
    // intent no rail has paid.
    let refund_id = vpay_core::ids::refund_id();
    sqlx::query(
        "INSERT INTO refunds (id, payment_intent_id, amount, currency_code, status, metadata) \
         VALUES ($1, $2, 300, 'XAF', 'pending', $3)",
    )
    .bind(&refund_id)
    .bind(&intent_id)
    .bind(json!({ "case": "mcp" }))
    .execute(&harness.pool)
    .await
    .context("seeding a refund")?;

    let retrieved = ok_object(
        &harness
            .call(&a, "retrieve_refund", json!({ "id": refund_id }))
            .await?,
    );
    let v1 = SealedHttp::new()
        .get(format!("{}/v1/refunds/{refund_id}", harness.base_url))
        .signed_by(CLIENT_A)
        .send()
        .await?
        .json::<Value>()
        .await?;
    assert_eq!(
        retrieved,
        vpay_api::mcp::reshape_refund(v1.clone()),
        "the /v1 refund was {v1}"
    );

    let foreign = envelope(
        &harness
            .call(&b, "retrieve_refund", json!({ "id": refund_id }))
            .await?,
    );
    assert_eq!(at(&foreign, "/code"), json!("NOT_FOUND"), "{foreign}");

    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds")
        .fetch_one(&harness.pool)
        .await?;
    let refused = envelope(
        &harness
            .call(
                &a,
                "create_refund",
                json!({ "args": {
                    "idempotency_key": "k-refund-1",
                    "payment_intent": intent_id,
                    "destination_fields": [],
                    "metadata": [],
                } }),
            )
            .await?,
    );
    assert!(
        matches!(
            at(&refused, "/code").as_str(),
            Some("CONFLICT" | "BAD_REQUEST")
        ),
        "a refund of an intent that took no money is the caller's refusal: {refused}"
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(before, after, "a refused refund writes no row");
    harness.shutdown().await;
    Ok(())
}

/// ADR-0033 D2: an absent `deployment.surfaces` never mounts the endpoint.
#[tokio::test]
async fn an_absent_surfaces_key_mounts_no_mcp() -> anyhow::Result<()> {
    let harness = harness_with(None).await?;
    let token = harness.mcp_token(CLIENT_A, vpay_api::SCOPE_PAYMENTS_WRITE);
    let (status, body) = harness
        .rpc(Some(&token), None, "tools/list", None, json!({}))
        .await?;
    assert_eq!(status, 404, "{body}");
    harness.shutdown().await;
    Ok(())
}
