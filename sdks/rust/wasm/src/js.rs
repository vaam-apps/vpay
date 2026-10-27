//! The JavaScript surface: `Sealer` and `SealedCall`, over [`crate::core`].
//!
//! Errors cross as a plain object `{ kind, message, status? }` with `kind` one
//! of `"unbound" | "seal" | "unsigned" | "unverified" | "invalid"`, so the
//! TypeScript side maps them to its own error classes without parsing text.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cratestack_client_rust::cose::{CoseSigner, ExternalSigner};
use cratestack_core::CratestackError;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::core::{OpenError, SealError, Sealed, Sealer, SealerKey};

fn error(kind: &str, message: &str, status: Option<u16>) -> JsValue {
    let object = Object::new();
    // `Reflect::set` on a fresh plain object cannot fail.
    let _ = Reflect::set(&object, &"kind".into(), &kind.into());
    let _ = Reflect::set(&object, &"message".into(), &message.into());
    if let Some(status) = status {
        let _ = Reflect::set(&object, &"status".into(), &JsValue::from(status));
    }
    object.into()
}

fn invalid(message: &str) -> JsValue {
    error("invalid", message, None)
}

fn seal_error(error_: &SealError) -> JsValue {
    match error_ {
        SealError::Unbound(message) => error("unbound", message, None),
        SealError::Seal(message) => error("seal", message, None),
    }
}

fn open_error(error_: &OpenError) -> JsValue {
    match error_ {
        OpenError::Unsigned { status } => error("unsigned", &error_.to_string(), Some(*status)),
        OpenError::Unverified => error("unverified", &error_.to_string(), None),
        OpenError::Other(message) => error("seal", message, None),
    }
}

/// 32 bytes of unpadded base64url, whitespace ignored. The text is never echoed.
pub(crate) fn key_from_base64url(text: &str, what: &str) -> Result<[u8; 32], JsValue> {
    URL_SAFE_NO_PAD
        .decode(text.trim())
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
        .ok_or_else(|| invalid(&format!("{what} must be 32 bytes of unpadded base64url")))
}

fn pairs_from_js(headers: &JsValue) -> Result<Vec<(String, String)>, JsValue> {
    let entries = Array::from(headers);
    let mut out = Vec::with_capacity(entries.length() as usize);
    for entry in entries.iter() {
        let pair = Array::from(&entry);
        match (pair.get(0).as_string(), pair.get(1).as_string()) {
            (Some(name), Some(value)) => out.push((name, value)),
            _ => {
                return Err(invalid(
                    "headers must be an array of [name, value] string pairs",
                ));
            }
        }
    }
    Ok(out)
}

fn pairs_to_js(pairs: &[(String, String)]) -> JsValue {
    pairs
        .iter()
        .map(|(name, value)| {
            let pair = Array::new();
            pair.push(&name.into());
            pair.push(&value.into());
            JsValue::from(pair)
        })
        .collect::<Array>()
        .into()
}

/// Wraps a JS `(tbs: Uint8Array) => Uint8Array | Promise<Uint8Array>` as the
/// future-returning callback `ExternalSigner::ed25519` takes.
fn external_signer(public_key: &[u8; 32], sign: Function) -> Result<Arc<dyn CoseSigner>, JsValue> {
    let signer = ExternalSigner::ed25519(public_key, move |tbs: Vec<u8>| {
        let sign = sign.clone();
        async move {
            let failed = |what: &str| CratestackError::Internal(what.to_owned());
            let returned = sign
                .call1(&JsValue::NULL, &Uint8Array::from(tbs.as_slice()))
                .map_err(|_| failed("the signing callback threw"))?;
            let settled = JsFuture::from(Promise::resolve(&returned))
                .await
                .map_err(|_| failed("the signing callback rejected"))?;
            Ok(Uint8Array::new(&settled).to_vec())
        }
    })
    .map_err(|_| invalid("publicKey is not an Ed25519 public key"))?;
    #[expect(
        clippy::arc_with_non_send_sync,
        reason = "a JS callback's future is not Send, and on wasm32 cratestack's MaybeSend is empty: \
                  the module is single-threaded, so the Arc never crosses one"
    )]
    let signer: Arc<dyn CoseSigner> = Arc::new(signer);
    Ok(signer)
}

/// A merchant's sealing identity. See the crate docs for the flow.
#[wasm_bindgen(js_name = "Sealer")]
#[derive(Debug)]
pub struct JsSealer {
    inner: Sealer,
}

#[wasm_bindgen(js_class = "Sealer")]
impl JsSealer {
    /// Signs with the Ed25519 key whose seed is `seed_base64url` (32 bytes,
    /// unpadded base64url). `server_current` and `server_next` are the pinned
    /// response keys in the same spelling; `server_next` may be `undefined`.
    /// `audience` is the server's public base URL.
    #[wasm_bindgen(js_name = "fromSeed")]
    pub fn from_seed(
        seed_base64url: &str,
        server_current: &str,
        server_next: Option<String>,
        audience: &str,
    ) -> Result<JsSealer, JsValue> {
        let seed = key_from_base64url(seed_base64url, "the seed")?;
        Self::build(SealerKey::Seed(seed), server_current, server_next, audience)
    }

    /// Signs through `sign`, a callback `(tbs: Uint8Array) => Uint8Array |
    /// Promise<Uint8Array>` returning the 64-byte pure-Ed25519 signature over
    /// exactly `tbs` (`crypto.sign(null, tbs, key)`). The key never enters this
    /// module. A signature that does not verify under `public_key_base64url` is
    /// refused here.
    #[wasm_bindgen(js_name = "fromCallback")]
    pub fn from_callback(
        public_key_base64url: &str,
        sign: Function,
        server_current: &str,
        server_next: Option<String>,
        audience: &str,
    ) -> Result<JsSealer, JsValue> {
        let public_key = key_from_base64url(public_key_base64url, "the public key")?;
        let signer = external_signer(&public_key, sign)?;
        Self::build(
            SealerKey::Signer { public_key, signer },
            server_current,
            server_next,
            audience,
        )
    }

    fn build(
        key: SealerKey,
        server_current: &str,
        server_next: Option<String>,
        audience: &str,
    ) -> Result<JsSealer, JsValue> {
        let current = key_from_base64url(server_current, "the current server key")?;
        let next = server_next
            .as_deref()
            .map(|text| key_from_base64url(text, "the next server key"))
            .transpose()?;
        let inner =
            Sealer::new(key, &current, next.as_ref(), audience).map_err(|e| seal_error(&e))?;
        Ok(JsSealer { inner })
    }

    /// The public half, unpadded base64url: what `merchant_clients[].cose_keys`
    /// holds.
    #[wasm_bindgen(getter, js_name = "publicKey")]
    #[must_use]
    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.inner.public_key())
    }

    /// Seals one call. `path` is under `/v1`; `body` is the form text of a
    /// write; `query` the canonical query of a read.
    #[wasm_bindgen(js_name = "sealCall")]
    pub async fn seal_call(
        &self,
        method: String,
        path: String,
        body: Option<String>,
        query: Option<String>,
        idempotency_key: Option<String>,
    ) -> Result<JsSealedCall, JsValue> {
        let sealed = self
            .inner
            .seal_call(
                &method,
                &path,
                body.as_deref(),
                query.as_deref(),
                idempotency_key.as_deref(),
            )
            .await
            .map_err(|e| seal_error(&e))?;
        Ok(JsSealedCall { inner: sealed })
    }
}

/// One sealed call: send `body` with `headers`, then `openResponse` the answer.
#[wasm_bindgen(js_name = "SealedCall")]
#[derive(Debug)]
pub struct JsSealedCall {
    inner: Sealed,
}

#[wasm_bindgen(js_class = "SealedCall")]
impl JsSealedCall {
    /// The request body: the COSE message.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn body(&self) -> Uint8Array {
        Uint8Array::from(self.inner.body.as_slice())
    }

    /// The headers to send as given, as `[name, value]` pairs.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn headers(&self) -> JsValue {
        pairs_to_js(&self.inner.headers)
    }

    /// Verifies the answer and returns its payload as text. `headers` is the
    /// response's header list as `[name, value]` pairs
    /// (`[...response.headers]`).
    #[wasm_bindgen(js_name = "openResponse")]
    pub async fn open_response(
        &mut self,
        status: u16,
        headers: JsValue,
        body: Uint8Array,
    ) -> Result<String, JsValue> {
        let pairs = pairs_from_js(&headers)?;
        let payload = self
            .inner
            .open_response(status, &pairs, body.to_vec())
            .await
            .map_err(|e| open_error(&e))?;
        String::from_utf8(payload).map_err(|_| error("seal", "the answer is not UTF-8", None))
    }
}
