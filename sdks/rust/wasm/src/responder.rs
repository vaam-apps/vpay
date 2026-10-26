//! The server half of the transport, for the Node SDK's own tests.
//!
//! Built only with the `responder` feature, into a package the npm tarball does
//! not list. It is what a vpay server does to a sealed `/v1` request —
//! find the operation, rebuild the binding the client signed, verify, and seal
//! the answer under the server's key — written against cratestack's own server
//! envelope (`CoseEnvelope::server`) so the SDK's vitest suites run against a
//! verifier that is not a copy of the SDK's sealing code. The real
//! `EnvelopeLayer` is exercised against the same SDK by `sdks/rust/tests` and
//! by the integration suites.

use std::borrow::Cow;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use cratestack_client_rust::cose::{
    CoseEnvelope, CoseMode, CoseVerifyKey, Ed25519Signer, StaticVerifierResolver, request_digest,
};
use cratestack_core::{
    Binding, BoundHeaders, InMemoryNonceStore, PAYLOAD_TYPE_HEADER, PathParams, ResponseBinding,
    canonical_query,
};
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

use crate::contracts;
use crate::js::key_from_base64url;

const FORM: &str = "application/x-www-form-urlencoded";
const JSON: &str = "application/json";
const COSE_SIGN1: &str = "application/cose; cose-type=\"cose-sign1\"";

fn failure(message: &str) -> JsValue {
    JsValue::from_str(message)
}

/// A vpay server's envelope, for tests.
#[wasm_bindgen(js_name = "Responder")]
#[derive(Debug)]
pub struct JsResponder {
    server: CoseEnvelope,
    audience: String,
}

#[wasm_bindgen(js_class = "Responder")]
impl JsResponder {
    /// `server_seed` signs answers; `merchant_keys` are the registered merchant
    /// public keys (unpadded base64url), as `cose_keys` holds them.
    #[wasm_bindgen(constructor)]
    pub fn new(
        server_seed: &str,
        merchant_keys: Array,
        audience: &str,
    ) -> Result<JsResponder, JsValue> {
        let seed = key_from_base64url(server_seed, "the server seed")?;
        let mut resolver = StaticVerifierResolver::new();
        for key in merchant_keys.iter() {
            let text = key
                .as_string()
                .ok_or_else(|| failure("merchant keys must be strings"))?;
            let bytes = key_from_base64url(&text, "a merchant key")?;
            resolver = resolver.with_key(
                CoseVerifyKey::ed25519(&bytes).map_err(|_| failure("not an Ed25519 key"))?,
            );
        }
        let server = CoseEnvelope::server(
            CoseMode::Sign1,
            Arc::new(Ed25519Signer::from_seed(&seed)),
            Arc::new(resolver),
            Arc::new(InMemoryNonceStore::new()),
        )
        .build()
        .map_err(|error| failure(&error.to_string()))?;
        Ok(Self {
            server,
            audience: audience.trim_end_matches('/').to_owned(),
        })
    }

    /// The server's public key, unpadded base64url: what a client pins.
    #[wasm_bindgen(js_name = "serverPublicKey")]
    pub fn server_public_key(seed: &str) -> Result<String, JsValue> {
        let seed = key_from_base64url(seed, "the server seed")?;
        let public = Ed25519Signer::from_seed(&seed)
            .verify_key()
            .ed25519_bytes()
            .ok_or_else(|| failure("no public key"))?;
        Ok(URL_SAFE_NO_PAD.encode(public))
    }

    /// Opens one sealed request. `path` is under `/v1`; `headers` is the
    /// request's `[name, value]` pairs. Rejects with the coarse refusal text
    /// when the message does not verify: the caller answers an unsigned `401`.
    #[wasm_bindgen(js_name = "openRequest")]
    pub async fn open_request(
        &self,
        method: String,
        path: String,
        query: Option<String>,
        headers: Array,
        body: Uint8Array,
    ) -> Result<JsOpenedRequest, JsValue> {
        let bound = contracts::operation_for(&method, &path)
            .map_err(|error| failure(&error.to_string()))?;
        let mut idempotency_key = None;
        let mut payload_type = None;
        for entry in headers.iter() {
            let pair = Array::from(&entry);
            let (Some(name), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string())
            else {
                continue;
            };
            match name.to_ascii_lowercase().as_str() {
                "idempotency-key" => idempotency_key = Some(value),
                n if n.eq_ignore_ascii_case(PAYLOAD_TYPE_HEADER) => payload_type = Some(value),
                _ => {}
            }
        }
        let request_type = payload_type.unwrap_or_else(|| FORM.to_owned());
        // The client signs the *canonical* query (pairs sorted), so the server
        // must rebuild exactly that, whatever order the URL carried.
        let query = Some(canonical_query(query.as_deref())).filter(|query| !query.is_empty());
        let binding = Binding {
            audience: Cow::Owned(self.audience.clone()),
            method: Cow::Owned(method.clone()),
            route: Cow::Borrowed(bound.template),
            path_params: PathParams::Owned(bound.params.clone()),
            query: query.clone().map(Cow::Owned),
            contract_sha: contracts::contract_digest(&method, bound.template),
            payload_media_type: Cow::Owned(request_type),
            bound_headers: BoundHeaders {
                idempotency_key: idempotency_key.clone().map(Cow::Owned),
                if_match: None,
            },
            response: None,
        };
        let bytes = Bytes::from(body.to_vec());
        let opened = self
            .server
            .open_request(bytes.clone(), &binding)
            .await
            .map_err(|_| failure("refused"))?;
        Ok(JsOpenedRequest {
            server: self.server.clone(),
            binding: binding.into_owned(),
            request: bytes,
            payload: String::from_utf8_lossy(&opened.payload).into_owned(),
            idempotency_key,
        })
    }
}

/// A request that verified, and the means to answer it.
#[wasm_bindgen(js_name = "OpenedRequest")]
#[derive(Debug)]
pub struct JsOpenedRequest {
    server: CoseEnvelope,
    binding: Binding<'static>,
    request: Bytes,
    payload: String,
    idempotency_key: Option<String>,
}

#[wasm_bindgen(js_class = "OpenedRequest")]
impl JsOpenedRequest {
    /// The request payload, as text (the form body).
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn payload(&self) -> String {
        self.payload.clone()
    }

    /// The bound `Idempotency-Key`, if the request carried one.
    #[wasm_bindgen(getter, js_name = "idempotencyKey")]
    #[must_use]
    pub fn idempotency_key(&self) -> Option<String> {
        self.idempotency_key.clone()
    }

    /// Seals `body` (JSON text) as the answer with `status`; returns
    /// `{ headers: [name, value][], body: Uint8Array }`.
    #[wasm_bindgen(js_name = "sealResponse")]
    pub async fn seal_response(&self, status: u16, body: String) -> Result<JsValue, JsValue> {
        let mut binding = self.binding.clone();
        binding.payload_media_type = Cow::Borrowed(JSON);
        binding.response = Some(ResponseBinding {
            request: request_digest(&self.request),
            status,
        });
        let sealed = self
            .server
            .seal_response(body.as_bytes(), &binding)
            .await
            .map_err(|error| failure(&error.to_string()))?;
        let headers = Array::new();
        for (name, value) in [("content-type", COSE_SIGN1), (PAYLOAD_TYPE_HEADER, JSON)] {
            let pair = Array::new();
            pair.push(&name.into());
            pair.push(&value.into());
            headers.push(&pair);
        }
        let out = Object::new();
        let _ = Reflect::set(&out, &"headers".into(), &headers);
        let _ = Reflect::set(&out, &"body".into(), &Uint8Array::from(sealed.as_ref()));
        Ok(out.into())
    }
}
