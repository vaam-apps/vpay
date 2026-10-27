//! The merchant MCP endpoint's two implementations (ADR-0033): how a tool
//! call reaches `/v1`'s own code ([`MerchantOps`]) and how a bearer token
//! becomes a merchant ([`MerchantMcpAuth`]).
//!
//! # A tool call is a `/v1` request with another origin
//!
//! [`MerchantOps`] does not reimplement a single rule. Each operation calls
//! the function the `/v1` route calls: the read handlers directly, and the
//! writes' `*_with` operations, which Phase 2 split out of the handlers for
//! exactly this. A write's parameters are built as the nested value
//! `crate::form::parse_form` produces from a form body, with string leaves,
//! and handed over as a [`PostRequest::for_tool`]. So the same `serde`
//! decode, the same validation, the same idempotency claim and the same
//! rail call run, and an agent is refused for exactly what a `/v1` caller
//! would be.
//!
//! # The answer is reshaped in three places, and only there
//!
//! The response is `/v1`'s JSON, and [`reshape_payment_intent`] and
//! [`reshape_refund`] turn it into the shape `schemas/vpay.cstack`'s MCP
//! output types declare. The three differences, and why, are in that file's
//! MCP section. `tests::the_mcp_objects_are_the_v1_objects_reshaped_in_three_places`
//! pins them.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::{Map, Value, json};
use vpay_db::DisabledClients;
use vpay_db::{
    ConfirmRequest, McpAuthRefusal, McpAuthenticator, McpCaller, MerchantCall, MerchantOperation,
    MerchantOperations, NewIntentRequest, NewRefundRequest, OperationOutcome, OperationRefusal,
    Repositories,
};
use vpay_provider::ProviderAdapter;

use crate::error::ApiError;
use crate::resource_auth::{AuthRejection, JwtValidator};
use crate::v1::payment_intents::{self, PostRequest};
use crate::v1::{self, MerchantScope, ResourceConfig, refunds};

/// [`vpay_db::MerchantOperations`] over `/v1`'s own functions.
pub struct MerchantOps {
    repositories: Arc<dyn Repositories>,
    config: Arc<ResourceConfig>,
    adapters: Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>,
}

impl std::fmt::Debug for MerchantOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MerchantOps")
            .field("adapters", &self.adapters.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl MerchantOps {
    /// Over the same three dependencies the `/v1` router holds.
    #[must_use]
    pub fn new(
        repositories: Arc<dyn Repositories>,
        config: Arc<ResourceConfig>,
        adapters: Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>,
    ) -> Self {
        Self {
            repositories,
            config,
            adapters,
        }
    }

    async fn run(&self, call: MerchantCall) -> Result<Response, ApiError> {
        let scope = MerchantScope::for_mcp(call.merchant_id);
        let tool = call.operation.tool_name();
        let repositories = self.repositories.as_ref();
        match call.operation {
            MerchantOperation::RetrievePaymentIntent { id } => {
                payment_intents::retrieve(State(Arc::clone(&self.repositories)), scope, Path(id))
                    .await
            }
            MerchantOperation::CreatePaymentIntent {
                idempotency_key,
                request,
            } => {
                let post = PostRequest::for_tool(&idempotency_key, tool, intent_params(request))?;
                payment_intents::create_with(
                    repositories,
                    &self.config,
                    &self.adapters,
                    &scope,
                    post,
                )
                .await
            }
            MerchantOperation::ConfirmPaymentIntent {
                idempotency_key,
                id,
                request,
            } => {
                let post =
                    PostRequest::for_tool(&idempotency_key, tool, confirm_params(&id, request))?;
                payment_intents::confirm_with(
                    repositories,
                    &self.config,
                    &self.adapters,
                    &scope,
                    &id,
                    post,
                )
                .await
            }
            MerchantOperation::CancelPaymentIntent {
                idempotency_key,
                id,
            } => {
                let post = PostRequest::for_tool(&idempotency_key, tool, json!({ "id": id }))?;
                payment_intents::cancel_with(repositories, &scope, &id, post).await
            }
            MerchantOperation::RetrieveRefund { id } => {
                refunds::retrieve(State(Arc::clone(&self.repositories)), scope, Path(id)).await
            }
            MerchantOperation::CreateRefund {
                idempotency_key,
                request,
            } => {
                let post = PostRequest::for_tool(&idempotency_key, tool, refund_params(request))?;
                refunds::create_with(repositories, &self.config, &self.adapters, &scope, post).await
            }
            MerchantOperation::CancelRefund {
                idempotency_key,
                id,
            } => {
                let post = PostRequest::for_tool(&idempotency_key, tool, json!({ "id": id }))?;
                refunds::cancel_with(repositories, &scope, &id, post).await
            }
        }
    }
}

#[async_trait]
impl MerchantOperations for MerchantOps {
    async fn perform(&self, call: MerchantCall) -> Result<OperationOutcome, OperationRefusal> {
        let reshape: fn(Value) -> Value = match &call.operation {
            MerchantOperation::RetrieveRefund { .. }
            | MerchantOperation::CreateRefund { .. }
            | MerchantOperation::CancelRefund { .. } => reshape_refund,
            _ => reshape_payment_intent,
        };
        let response = match self.run(call).await {
            Ok(response) => response,
            Err(error) => axum::response::IntoResponse::into_response(error),
        };
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), crate::V1_BODY_LIMIT_BYTES)
            .await
            .map_err(|_| internal_refusal())?;
        let body: Value = serde_json::from_slice(&bytes).map_err(|_| internal_refusal())?;
        if status.is_success() {
            return Ok(OperationOutcome {
                body: reshape(body),
            });
        }
        Err(refusal_of(status.as_u16(), &body))
    }
}

/// `/v1`'s error envelope, `{ "error": { "code", "message", "param"? } }`,
/// as a refusal. An envelope that is somehow not one is a `500`.
fn refusal_of(status: u16, body: &Value) -> OperationRefusal {
    let error = &body["error"];
    match (error["code"].as_str(), error["message"].as_str()) {
        (Some(code), Some(message)) => OperationRefusal {
            status,
            code: code.to_owned(),
            message: message.to_owned(),
            param: error["param"].as_str().map(str::to_owned),
        },
        _ => internal_refusal(),
    }
}

fn internal_refusal() -> OperationRefusal {
    OperationRefusal {
        status: 500,
        code: "api_error".to_owned(),
        message: "An internal error occurred.".to_owned(),
        param: None,
    }
}

fn metadata_params(metadata: Vec<(String, String)>) -> Value {
    Value::Object(
        metadata
            .into_iter()
            .map(|(key, value)| (key, Value::String(value)))
            .collect(),
    )
}

fn fields_object(fields: Vec<(String, String)>) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name, Value::String(value)))
            .collect(),
    )
}

/// `POST /v1/payment_intents`' body, as `parse_form` would decode it.
fn intent_params(request: NewIntentRequest) -> Value {
    let mut params = Map::new();
    params.insert(
        "amount".to_owned(),
        Value::String(request.amount.to_string()),
    );
    params.insert("currency".to_owned(), Value::String(request.currency));
    if !request.payment_method_types.is_empty() {
        params.insert(
            "payment_method_types".to_owned(),
            Value::Array(
                request
                    .payment_method_types
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(description) = request.description {
        params.insert("description".to_owned(), Value::String(description));
    }
    if let Some(customer) = request.customer {
        params.insert("customer".to_owned(), Value::String(customer));
    }
    if !request.metadata.is_empty() {
        params.insert("metadata".to_owned(), metadata_params(request.metadata));
    }
    Value::Object(params)
}

/// `POST /v1/payment_intents/{id}/confirm`'s body: `payment_method_data[type]`
/// and `payment_method_data[<type>][<name>]`, plus `id`, which `/v1` hashes
/// as part of the path and a tool call has no path for. The confirm's
/// decoder ignores a key it does not name, so `id` reaches the fingerprint
/// and nothing else.
fn confirm_params(id: &str, request: ConfirmRequest) -> Value {
    let mut data = Map::new();
    data.insert(
        "type".to_owned(),
        Value::String(request.payment_method_type.clone()),
    );
    if !request.payer_fields.is_empty() {
        data.insert(
            request.payment_method_type,
            fields_object(request.payer_fields),
        );
    }
    let mut params = Map::new();
    params.insert("id".to_owned(), Value::String(id.to_owned()));
    params.insert("payment_method_data".to_owned(), Value::Object(data));
    if let Some(return_url) = request.return_url {
        params.insert("return_url".to_owned(), Value::String(return_url));
    }
    Value::Object(params)
}

/// `POST /v1/refunds`' body: `destination[<type>][<name>]` when a type is
/// named.
fn refund_params(request: NewRefundRequest) -> Value {
    let mut params = Map::new();
    params.insert(
        "payment_intent".to_owned(),
        Value::String(request.payment_intent),
    );
    if let Some(amount) = request.amount {
        params.insert("amount".to_owned(), Value::String(amount.to_string()));
    }
    if let Some(reason) = request.reason {
        params.insert("reason".to_owned(), Value::String(reason));
    }
    if let Some(destination_type) = request.destination_type {
        let mut destination = Map::new();
        destination.insert(destination_type, fields_object(request.destination_fields));
        params.insert("destination".to_owned(), Value::Object(destination));
    }
    if !request.metadata.is_empty() {
        params.insert("metadata".to_owned(), metadata_params(request.metadata));
    }
    Value::Object(params)
}

/// `metadata`'s map as a `MetadataEntry[]`, in key order.
fn metadata_entries(metadata: Option<Value>) -> Value {
    let Some(Value::Object(map)) = metadata else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        map.into_iter()
            .map(|(key, value)| json!({ "key": key, "value": value }))
            .collect(),
    )
}

/// Renames the `type` key of an object, if there is one.
fn renamed_type(value: Option<Value>, to: &str) -> Value {
    match value {
        Some(Value::Object(mut object)) => {
            if let Some(kind) = object.remove("type") {
                object.insert(to.to_owned(), kind);
            }
            Value::Object(object)
        }
        _ => Value::Null,
    }
}

/// A `/v1` `payment_intent` in `McpPaymentIntent`'s shape.
///
/// `pub` because it is the contract between the two surfaces, and a suite
/// comparing an MCP object with the `/v1` one must apply exactly this.
#[must_use]
pub fn reshape_payment_intent(body: Value) -> Value {
    let Value::Object(mut object) = body else {
        return body;
    };
    object.remove("object");
    let metadata = metadata_entries(object.remove("metadata"));
    object.insert("metadata".to_owned(), metadata);
    let next_action = renamed_type(object.remove("next_action"), "kind");
    object.insert("next_action".to_owned(), next_action);
    let details = renamed_type(object.remove("payment_method_details"), "method_type");
    object.insert("payment_method_details".to_owned(), details);
    Value::Object(object)
}

/// A `/v1` `refund` in `McpRefund`'s shape. `pub` for
/// [`reshape_payment_intent`]'s reason.
#[must_use]
pub fn reshape_refund(body: Value) -> Value {
    let Value::Object(mut object) = body else {
        return body;
    };
    object.remove("object");
    let metadata = metadata_entries(object.remove("metadata"));
    object.insert("metadata".to_owned(), metadata);
    Value::Object(object)
}

/// [`vpay_db::McpAuthenticator`] over a [`JwtValidator`] whose audience is
/// the merchant MCP resource identifier (ADR-0033 D9).
///
/// Five checks, in this order: the token validates against the merchant MCP
/// audience; its client is registered to a merchant (from memory, so an
/// unregistered client costs no query); that client is not switched off in
/// `disabled_clients` (ADR-0037: a token minted before `disable_client` must not
/// outlive it, here as on `/v1`); and it holds a scope this surface serves, held
/// to the registration's current YAML `scopes` ([`ResourceConfig::token_holds`]).
/// `/v1` no longer takes a bearer token at all (VP-3 signs every request), so an
/// MCP token cannot reach `/v1`, and a token for any other audience fails the
/// first check here.
pub struct MerchantMcpAuth {
    validator: JwtValidator,
    config: Arc<ResourceConfig>,
    disabled: Arc<dyn DisabledClients>,
}

impl std::fmt::Debug for MerchantMcpAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MerchantMcpAuth").finish_non_exhaustive()
    }
}

impl MerchantMcpAuth {
    /// `validator` must be built with the MCP resource identifier as its
    /// audience; `disabled` is the kill-switch port `/v1`'s `MerchantKeyResolver`
    /// asks (the server hands both the same repositories).
    #[must_use]
    pub fn new(
        validator: JwtValidator,
        config: Arc<ResourceConfig>,
        disabled: Arc<dyn DisabledClients>,
    ) -> Self {
        Self {
            validator,
            config,
            disabled,
        }
    }
}

#[async_trait]
impl McpAuthenticator for MerchantMcpAuth {
    async fn authenticate(&self, token: &str) -> Result<McpCaller, McpAuthRefusal> {
        let claims = self
            .validator
            .validate(token)
            .await
            .map_err(|rejection| match rejection {
                AuthRejection::KeysUnavailable => {
                    McpAuthRefusal::Unavailable("the token signing keys are unreachable".to_owned())
                }
                other => McpAuthRefusal::InvalidToken(other.to_string()),
            })?;
        let Some(merchant_id) = self.config.merchant_id_for(&claims.subject) else {
            tracing::warn!(
                client_id = %claims.subject,
                "a validly signed MCP token names a client this deployment has no registration \
                 for; refusing rather than guessing a tenant"
            );
            return Err(McpAuthRefusal::InvalidToken(
                "the token's client is not registered".to_owned(),
            ));
        };
        // `/v1`'s kill switch, for the same reason: a lookup that fails is an
        // error, never "not disabled", or a database outage is a revocation
        // bypass.
        match self.disabled.is_client_disabled(&claims.subject).await {
            Ok(false) => {}
            Ok(true) => {
                tracing::warn!(
                    client_id = %claims.subject,
                    "refusing an MCP token: the client is disabled via the disabled_clients \
                     kill switch"
                );
                return Err(McpAuthRefusal::InvalidToken(
                    "the token's client is disabled".to_owned(),
                ));
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    client_id = %claims.subject,
                    "disabled_clients lookup failed; refusing an MCP token rather than assuming \
                     the client is enabled"
                );
                return Err(McpAuthRefusal::Unavailable(
                    "the kill switch could not be read".to_owned(),
                ));
            }
        }
        let can_write = self.config.token_holds(&claims, v1::SCOPE_PAYMENTS_WRITE);
        if !can_write && !self.config.token_holds(&claims, v1::SCOPE_PAYMENTS_READ) {
            return Err(McpAuthRefusal::InsufficientScope(
                "the token holds neither payments:read nor payments:write".to_owned(),
            ));
        }
        Ok(McpCaller {
            merchant_id: merchant_id.to_owned(),
            can_write,
        })
    }
}

/// Where the merchant MCP endpoint is mounted, under the deployment's
/// public base URL.
pub const MERCHANT_MCP_PATH: &str = "/mcp/v1";

/// The merchant MCP endpoint's protected-resource identifier (RFC 9728):
/// the deployment's public base URL plus [`MERCHANT_MCP_PATH`].
///
/// Derived, not configured, for the reason `deployment.public_base_url`
/// exists: it is already the one place this deployment says where it is,
/// and a second, independently written URL is one that can disagree with
/// it. It is also the token audience: a merchant opts in by listing it in
/// its client's `allowed_audiences` (ADR-0033 D9).
#[must_use]
pub fn merchant_resource(public_base_url: &str) -> String {
    format!(
        "{}{MERCHANT_MCP_PATH}",
        public_base_url.trim_end_matches('/')
    )
}

/// `scheme://host[:port]` of an absolute URL, which is the one browser
/// origin the endpoint accepts: the deployment's own. An agent is not a
/// browser and sends no `Origin`, which the transport accepts; a browser
/// page on any other origin is refused `403` (ADR-0033 threat model § 1).
fn origin_of(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            format!("{scheme}://{authority}")
        }
        None => url.to_owned(),
    }
}

/// What [`merchant_router`] needs.
#[derive(Debug)]
pub struct MerchantMcpParts {
    /// The same repositories `/v1` holds.
    pub repositories: Arc<dyn Repositories>,
    /// The same projection `/v1` holds.
    pub config: Arc<ResourceConfig>,
    /// The same adapters `/v1` holds.
    pub adapters: Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>,
    /// A validator whose audience is [`merchant_resource`].
    pub validator: JwtValidator,
    /// `deployment.public_base_url`.
    pub public_base_url: String,
    /// The merchant OP's issuer, which MCP clients get tokens from.
    pub issuer: String,
}

/// The merchant MCP endpoint and its RFC 9728 metadata, ready to merge into
/// [`crate::RouterDeps::merchant_mcp`].
///
/// # Errors
///
/// [`vpay_db::McpEndpointError`] when CrateStack refuses the endpoint, which
/// for a URL `Config::load` already validated means a defect, not an
/// operator's mistake.
pub fn merchant_router(parts: MerchantMcpParts) -> Result<axum::Router, vpay_db::McpEndpointError> {
    let resource = merchant_resource(&parts.public_base_url);
    let ops = Arc::new(MerchantOps::new(
        Arc::clone(&parts.repositories),
        Arc::clone(&parts.config),
        parts.adapters,
    ));
    let auth = Arc::new(MerchantMcpAuth::new(
        parts.validator,
        parts.config,
        Arc::clone(&parts.repositories) as Arc<dyn DisabledClients>,
    ));
    let endpoint = vpay_db::McpEndpoint {
        allowed_origins: vec![origin_of(&parts.public_base_url)],
        authorization_servers: vec![parts.issuer],
        scopes: vec![
            v1::SCOPE_PAYMENTS_READ.to_owned(),
            v1::SCOPE_PAYMENTS_WRITE.to_owned(),
        ],
        resource,
    };
    parts.repositories.merchant_mcp_router(ops, auth, &endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resource_and_origin_are_the_deployments_own() {
        assert_eq!(
            merchant_resource("https://pay.example/"),
            "https://pay.example/mcp/v1"
        );
        assert_eq!(
            origin_of("https://pay.example:8443/base/"),
            "https://pay.example:8443"
        );
    }

    #[test]
    fn the_mcp_objects_are_the_v1_objects_reshaped_in_three_places() {
        let v1 = json!({
            "id": "pi_1", "object": "payment_intent", "amount": 1000, "currency": "xaf",
            "status": "requires_action", "payment_method_types": ["orange_money"],
            "payment_method_details": { "type": "orange_wallet", "rail": "orange_money" },
            "next_action": { "type": "redirect_to_url",
                             "redirect_to_url": { "url": "https://r", "return_url": null } },
            "last_payment_error": null,
            "metadata": { "b": "2", "a": "1" },
            "description": null, "customer": null, "created": 1, "livemode": false
        });
        let mcp = reshape_payment_intent(v1);
        assert_eq!(
            mcp,
            json!({
                "id": "pi_1", "amount": 1000, "currency": "xaf",
                "status": "requires_action", "payment_method_types": ["orange_money"],
                "payment_method_details": { "method_type": "orange_wallet", "rail": "orange_money" },
                "next_action": { "kind": "redirect_to_url",
                                 "redirect_to_url": { "url": "https://r", "return_url": null } },
                "last_payment_error": null,
                "metadata": [ { "key": "a", "value": "1" }, { "key": "b", "value": "2" } ],
                "description": null, "customer": null, "created": 1, "livemode": false
            })
        );

        let refund = reshape_refund(json!({
            "id": "re_1", "object": "refund", "amount": 5, "currency": "xaf",
            "payment_intent": "pi_1", "status": "pending", "reason": null,
            "metadata": {}, "created": 2
        }));
        assert_eq!(
            refund,
            json!({
                "id": "re_1", "amount": 5, "currency": "xaf", "payment_intent": "pi_1",
                "status": "pending", "reason": null, "metadata": [], "created": 2
            })
        );
    }

    #[test]
    fn a_tool_calls_parameters_are_the_form_bodys_decoded_shape() {
        let params = confirm_params(
            "pi_1",
            ConfirmRequest {
                payment_method_type: "mtn_momo".to_owned(),
                payer_fields: vec![("msisdn".to_owned(), "237670000000".to_owned())],
                return_url: None,
            },
        );
        // What `parse_form` makes of
        // `payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=…`.
        assert_eq!(
            params,
            json!({ "id": "pi_1", "payment_method_data": { "type": "mtn_momo",
                                             "mtn_momo": { "msisdn": "237670000000" } } })
        );
        assert_eq!(
            intent_params(NewIntentRequest {
                amount: 1000,
                currency: "XAF".to_owned(),
                payment_method_types: Vec::new(),
                description: None,
                customer: None,
                metadata: vec![("order".to_owned(), "42".to_owned())],
            }),
            json!({ "amount": "1000", "currency": "XAF", "metadata": { "order": "42" } })
        );
    }

    /// The merchant MCP authenticator over a real validator, a real JWKS served
    /// from a local HTTP server and a kill switch the test controls. Only the
    /// database is replaced, as in `auth_methods_tests`.
    struct Harness {
        auth: MerchantMcpAuth,
        encoding_key: jsonwebtoken::EncodingKey,
        _jwks: wiremock::MockServer,
    }

    impl Harness {
        /// `yaml_scopes` is what the one fixture registration grants in the YAML.
        async fn new(yaml_scopes: &[&str], disabled: Arc<dyn DisabledClients>) -> Self {
            use crate::test_fixtures::tokens::{generate_keypair, jwks_server};
            use crate::test_fixtures::{PUBLIC_BASE_URL, TOKEN_ISSUER, config_with_scopes};

            let (encoding_key, jwk) = generate_keypair("tok-1");
            let jwks = jwks_server(&jwk).await;
            let validator = crate::resource_auth::mcp_token_validator(
                format!("{}/jwks.json", jwks.uri()),
                std::time::Duration::from_secs(300),
                TOKEN_ISSUER,
                PUBLIC_BASE_URL,
            )
            .expect("the vendored-roots JWKS client builds");
            let config = Arc::new(
                ResourceConfig::from_config(&config_with_scopes(None, yaml_scopes))
                    .expect("the fixture projects"),
            );
            Self {
                auth: MerchantMcpAuth::new(validator, config, disabled),
                encoding_key,
                _jwks: jwks,
            }
        }

        /// A token for the MCP resource, `sub` and `scope` as given.
        fn token(&self, sub: &str, scope: &str) -> String {
            use crate::test_fixtures::tokens::{mint_token, now_secs};
            use crate::test_fixtures::{PUBLIC_BASE_URL, TOKEN_ISSUER};
            mint_token(
                &self.encoding_key,
                "tok-1",
                &json!({
                    "iss": TOKEN_ISSUER,
                    "aud": merchant_resource(PUBLIC_BASE_URL),
                    "sub": sub,
                    "scope": scope,
                    "exp": now_secs() + 300,
                }),
            )
        }
    }

    const BOTH: &[&str] = &["payments:read", "payments:write"];

    /// `disable_client` reaches an MCP token as it reaches a signer and a `/v1`
    /// token: one minted before the switch flipped is otherwise good until it
    /// expires. The control row is the same token admitted while the client is
    /// enabled, and a kill switch that cannot answer is `Unavailable`, never
    /// "not disabled".
    #[tokio::test]
    async fn an_mcp_token_for_a_disabled_client_is_refused_and_an_unanswerable_switch_is_unavailable()
     {
        use crate::test_fixtures::MemoryKillSwitch;

        let enabled = Harness::new(BOTH, Arc::new(MemoryKillSwitch::disabling(&[]))).await;
        let token = enabled.token("acme-cameroon", "payments:write");
        assert_eq!(
            enabled.auth.authenticate(&token).await,
            Ok(McpCaller {
                merchant_id: "acme-cameroon-tenant".to_owned(),
                can_write: true,
            }),
            "the control"
        );

        let disabled = Harness::new(
            BOTH,
            Arc::new(MemoryKillSwitch::disabling(&["acme-cameroon"])),
        )
        .await;
        let token = disabled.token("acme-cameroon", "payments:write");
        assert!(
            matches!(
                disabled.auth.authenticate(&token).await,
                Err(McpAuthRefusal::InvalidToken(_))
            ),
            "a disabled client's token is a 401"
        );

        let unanswerable = Harness::new(BOTH, crate::test_fixtures::lazy_repositories()).await;
        let token = unanswerable.token("acme-cameroon", "payments:write");
        assert!(
            matches!(
                unanswerable.auth.authenticate(&token).await,
                Err(McpAuthRefusal::Unavailable(_))
            ),
            "a database outage must not be a revocation bypass"
        );
    }

    /// A client no registration names costs no query: the in-memory lookup
    /// comes first, so the kill switch is never asked, whatever it holds for it.
    #[tokio::test]
    async fn an_unregistered_mcp_client_is_refused_without_asking_the_kill_switch() {
        let switch = Arc::new(crate::test_fixtures::MemoryKillSwitch::disabling(&[
            "someone-else",
        ]));
        let harness = Harness::new(BOTH, Arc::clone(&switch) as Arc<dyn DisabledClients>).await;
        let token = harness.token("someone-else", "payments:write");
        assert!(matches!(
            harness.auth.authenticate(&token).await,
            Err(McpAuthRefusal::InvalidToken(m)) if m.contains("not registered")
        ));
        assert_eq!(switch.lookups(), 0, "an unregistered client costs no query");

        let token = harness.token("acme-cameroon", "payments:write");
        harness.auth.authenticate(&token).await.expect("registered");
        assert_eq!(switch.lookups(), 1, "a registered one asks exactly once");
    }

    /// The scopes are the token's held to the registration's current YAML
    /// `scopes`, the source the signed path reads: a scope removed from the YAML
    /// and applied by a restart reaches a token minted before it.
    #[tokio::test]
    async fn an_mcp_tokens_scopes_are_held_to_the_registrations_current_yaml() {
        use crate::test_fixtures::MemoryKillSwitch;
        let none_disabled = || Arc::new(MemoryKillSwitch::disabling(&[]));

        // The token was minted with both; the YAML now grants only read.
        let read_only = Harness::new(&["payments:read"], none_disabled()).await;
        let token = read_only.token("acme-cameroon", "payments:read payments:write");
        assert_eq!(
            read_only.auth.authenticate(&token).await,
            Ok(McpCaller {
                merchant_id: "acme-cameroon-tenant".to_owned(),
                can_write: false,
            }),
            "write was removed from the YAML, so the token no longer writes"
        );

        // Nothing in common: neither scope survives the intersection.
        let write_only_token = read_only.token("acme-cameroon", "payments:write");
        assert!(matches!(
            read_only.auth.authenticate(&write_only_token).await,
            Err(McpAuthRefusal::InsufficientScope(_))
        ));

        // The YAML grants nothing at all.
        let nothing = Harness::new(&[], none_disabled()).await;
        let token = nothing.token("acme-cameroon", "payments:read payments:write");
        assert!(matches!(
            nothing.auth.authenticate(&token).await,
            Err(McpAuthRefusal::InsufficientScope(_))
        ));

        // The control: the YAML grants both and the token asks for read only.
        let both = Harness::new(BOTH, none_disabled()).await;
        let token = both.token("acme-cameroon", "payments:read");
        assert_eq!(
            both.auth.authenticate(&token).await.map(|c| c.can_write),
            Ok(false),
            "a token cannot widen its own grant"
        );
    }

    #[test]
    fn a_v1_refusal_keeps_its_code_message_and_param() {
        let refusal = refusal_of(
            400,
            &json!({ "error": { "type": "invalid_request_error", "code": "parameter_invalid",
                                "message": "Bad amount.", "param": "amount" } }),
        );
        assert_eq!(
            refusal,
            OperationRefusal {
                status: 400,
                code: "parameter_invalid".to_owned(),
                message: "Bad amount.".to_owned(),
                param: Some("amount".to_owned()),
            }
        );
        assert_eq!(refusal_of(500, &json!({ "unexpected": true })).status, 500);
    }
}
