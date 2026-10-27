//! The sealing path, free of `wasm-bindgen` so a host `cargo test` runs it.
//!
//! [`Sealer`] is what `vpay-sdk`'s `send_signed` does up to the HTTP request
//! and from the HTTP answer on, with the transport left to the caller.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cratestack_client_rust::cose::{
    CoseEnvelope, CoseMode, CoseSigner, CoseVerifyKey, Ed25519Signer, StaticVerifierResolver,
};
use cratestack_client_rust::{
    ClientEnvelope, ClientError, EnvelopeError, PendingResponse, RouteRef, SealCall,
};
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::contracts;

/// The payload type of every `/v1` request body.
const REQUEST_PAYLOAD_TYPE: &str = "application/x-www-form-urlencoded";
/// The payload type every `/v1` answer is sealed under.
const RESPONSE_PAYLOAD_TYPE: &str = "application/json";

/// Why a call could not be sealed. Nothing was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    /// The method and path address no `/v1` operation. The message names them.
    Unbound(String),
    /// The signer failed, or the call could not be encoded. The message is the
    /// envelope's own and never quotes a key.
    Seal(String),
}

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbound(message) | Self::Seal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SealError {}

/// Why an answer was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The answer is not a sealed message: a proxy's page, or vpay's coarse
    /// `401` (which is never sealed). Nothing in its body was read.
    Unsigned {
        /// The status it came with.
        status: u16,
    },
    /// The answer claims to be sealed and does not verify under either pinned
    /// server key, or under this request.
    Unverified,
    /// Anything else, including a header that could not be read.
    Other(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned { status } => {
                write!(
                    f,
                    "the answer ({status}) is not sealed under a pinned server key"
                )
            }
            Self::Unverified => f.write_str("the answer does not verify under a pinned server key"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for OpenError {}

/// Where the signature comes from.
pub enum SealerKey {
    /// An Ed25519 seed this crate signs with.
    Seed([u8; 32]),
    /// A signer that lives elsewhere (`ExternalSigner::ed25519` over a JS
    /// callback). Built by the caller; `public_key` is the registered half.
    Signer {
        /// The registered public key, 32 bytes.
        public_key: [u8; 32],
        /// The signer.
        signer: Arc<dyn CoseSigner>,
    },
}

impl std::fmt::Debug for SealerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the seed.
        match self {
            Self::Seed(_) => f.write_str("SealerKey::Seed([redacted])"),
            Self::Signer { public_key, .. } => f
                .debug_struct("SealerKey::Signer")
                .field("public_key", &URL_SAFE_NO_PAD.encode(public_key))
                .finish_non_exhaustive(),
        }
    }
}

/// A merchant's sealing identity: the key it signs with, the server keys it
/// trusts, and the audience it addresses.
#[derive(Debug)]
pub struct Sealer {
    envelope: ClientEnvelope,
    public_key: [u8; 32],
}

impl Sealer {
    /// Builds the sealer. `server_current` and `server_next` are the two
    /// response keys the client trusts (`next` may be absent for a first
    /// deployment that has not prepared a rotation); `audience` is the
    /// server's configured audience, which is its public base URL.
    ///
    /// # Errors
    ///
    /// [`SealError::Seal`] if a key is not a point or the envelope cannot be
    /// built.
    pub fn new(
        key: SealerKey,
        server_current: &[u8; 32],
        server_next: Option<&[u8; 32]>,
        audience: &str,
    ) -> Result<Self, SealError> {
        let (signer, public_key): (Arc<dyn CoseSigner>, [u8; 32]) = match key {
            SealerKey::Seed(seed) => {
                let signer = Ed25519Signer::from_seed(&seed);
                let public_key = signer
                    .verify_key()
                    .ed25519_bytes()
                    .ok_or_else(|| SealError::Seal("the seed has no Ed25519 public key".into()))?;
                (Arc::new(signer), public_key)
            }
            SealerKey::Signer { public_key, signer } => (signer, public_key),
        };
        let mut resolver = StaticVerifierResolver::new().with_key(
            CoseVerifyKey::ed25519(server_current).map_err(|_| {
                SealError::Seal("the current server key is not an Ed25519 key".into())
            })?,
        );
        if let Some(next) = server_next {
            resolver = resolver.with_key(CoseVerifyKey::ed25519(next).map_err(|_| {
                SealError::Seal("the next server key is not an Ed25519 key".into())
            })?);
        }
        let cose = CoseEnvelope::client(CoseMode::Sign1, signer, Arc::new(resolver))
            .build()
            .map_err(|error| SealError::Seal(error.to_string()))?;
        let envelope = ClientEnvelope::new(cose, audience.trim_end_matches('/').to_owned())
            .map_err(|error| SealError::Seal(error.to_string()))?;
        Ok(Self {
            envelope,
            public_key,
        })
    }

    /// The registered public half, 32 bytes.
    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// Seals one call.
    ///
    /// `path` is the path **under `/v1`**, with its parameters in place
    /// (`/payment_intents/pi_1/confirm`); `body` is the form-encoded text of a
    /// write, `query` the canonical query string of a read, and
    /// `idempotency_key` the `Idempotency-Key` a write is sent with. The
    /// operation, and with it the template, parameters and contract digest the
    /// signature binds, is found from the vendored table; a path no operation
    /// serves is refused before anything is signed.
    ///
    /// # Errors
    ///
    /// [`SealError::Unbound`] or [`SealError::Seal`].
    pub async fn seal_call(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        query: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<Sealed, SealError> {
        let bound = contracts::operation_for(method, path)
            .map_err(|error| SealError::Unbound(error.to_string()))?;
        let params: Vec<&str> = bound.params.iter().map(String::as_str).collect();
        let digest = contracts::contract_digest(method, bound.template);
        let query = query.filter(|query| !query.is_empty());
        let mut call = SealCall::new(method, RouteRef::new(bound.template, &params), digest)
            .query(query)
            .payload(body.unwrap_or("").as_bytes(), REQUEST_PAYLOAD_TYPE)
            .accept(RESPONSE_PAYLOAD_TYPE);
        if let Some(key) = idempotency_key {
            call = call.idempotency_key(key);
        }
        let sealed = self
            .envelope
            .seal_call(call)
            .await
            .map_err(|error| SealError::Seal(error.to_string()))?;
        Ok(Sealed {
            body: sealed.body.to_vec(),
            headers: sealed
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            pending: Some(sealed.pending),
        })
    }
}

/// A sealed call: the bytes and headers to send as given, and what the answer
/// is opened with.
#[derive(Debug)]
pub struct Sealed {
    /// The request body: the COSE message.
    pub body: Vec<u8>,
    /// Every header the seal depends on (`Content-Type`, `Accept`,
    /// `Cratestack-Contract`, the payload-type selectors, the bound
    /// `Idempotency-Key`). Anything else is the caller's to add.
    pub headers: Vec<(String, String)>,
    pending: Option<PendingResponse>,
}

impl Sealed {
    /// Verifies the answer against the request this was made for and returns
    /// its payload (the JSON text). Spent by the call: a second one is an
    /// [`OpenError::Other`].
    ///
    /// # Errors
    ///
    /// See [`OpenError`].
    pub async fn open_response(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<Vec<u8>, OpenError> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| OpenError::Other("this call's answer was already opened".into()))?;
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| OpenError::Other("a response header name is not valid".into()))?;
            let value = HeaderValue::from_str(value)
                .map_err(|_| OpenError::Other("a response header value is not valid".into()))?;
            map.append(name, value);
        }
        match pending.open(status, &map, body.into()).await {
            Ok(opened) => Ok(opened.body.to_vec()),
            Err(ClientError::Envelope(EnvelopeError::Unsigned { status })) => {
                Err(OpenError::Unsigned { status })
            }
            Err(ClientError::Envelope(EnvelopeError::Unverified)) => Err(OpenError::Unverified),
            Err(other) => Err(OpenError::Other(other.to_string())),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const AUDIENCE: &str = "https://pay.example.test";

    fn sealer() -> Sealer {
        let server = Ed25519Signer::from_seed(&[9u8; 32]);
        let server_key = server.verify_key().ed25519_bytes().unwrap();
        Sealer::new(SealerKey::Seed([7u8; 32]), &server_key, None, AUDIENCE).unwrap()
    }

    fn header<'a>(sealed: &'a Sealed, name: &str) -> Option<&'a str> {
        sealed
            .headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[tokio::test]
    async fn a_write_is_sealed_with_the_idempotency_key_it_was_given() {
        let sealed = sealer()
            .seal_call(
                "POST",
                "/payment_intents",
                Some("amount=1000&currency=xaf"),
                None,
                Some("idem-1"),
            )
            .await
            .unwrap();
        assert!(!sealed.body.is_empty());
        assert!(
            header(&sealed, "content-type")
                .unwrap()
                .starts_with("application/cose"),
            "{:?}",
            sealed.headers
        );
        assert_eq!(header(&sealed, "idempotency-key"), Some("idem-1"));
    }

    #[tokio::test]
    async fn the_request_body_is_a_cose_message_that_carries_the_form_signed() {
        let sealed = sealer()
            .seal_call("POST", "/customers", Some("name=Ada"), None, Some("idem-2"))
            .await
            .unwrap();
        // Sign1 signs, it does not encrypt: the form rides inside the message,
        // which is more than the form alone.
        assert_ne!(sealed.body, b"name=Ada");
        assert!(
            sealed.body.windows(8).any(|window| window == b"name=Ada"),
            "the signed payload is the form"
        );
    }

    #[tokio::test]
    async fn two_seals_of_one_call_differ_so_neither_is_a_replay_of_the_other() {
        let sealer = sealer();
        let first = sealer
            .seal_call("GET", "/payment_intents/pi_1", None, None, None)
            .await
            .unwrap();
        let second = sealer
            .seal_call("GET", "/payment_intents/pi_1", None, None, None)
            .await
            .unwrap();
        assert_ne!(first.body, second.body);
    }

    #[tokio::test]
    async fn a_path_no_operation_serves_is_refused_before_anything_is_signed() {
        let error = sealer()
            .seal_call("GET", "/balance", None, None, None)
            .await
            .unwrap_err();
        assert!(matches!(error, SealError::Unbound(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_known_path_under_the_wrong_method_is_unbound() {
        let error = sealer()
            .seal_call("DELETE", "/payment_intents", None, None, None)
            .await
            .unwrap_err();
        assert!(matches!(error, SealError::Unbound(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_coarse_401_is_unsigned_and_its_body_is_not_read() {
        let mut sealed = sealer()
            .seal_call("GET", "/payment_intents/pi_1", None, None, None)
            .await
            .unwrap();
        let error = sealed
            .open_response(
                401,
                &[("content-type".into(), "application/json".into())],
                br#"{"code":"UNAUTHORIZED"}"#.to_vec(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, OpenError::Unsigned { status: 401 });
    }

    #[tokio::test]
    async fn an_answer_is_opened_once() {
        let mut sealed = sealer()
            .seal_call("GET", "/payment_intents/pi_1", None, None, None)
            .await
            .unwrap();
        let _ = sealed.open_response(401, &[], Vec::new()).await;
        let error = sealed
            .open_response(401, &[], Vec::new())
            .await
            .unwrap_err();
        assert!(matches!(error, OpenError::Other(_)), "{error:?}");
    }

    #[test]
    fn a_seed_is_never_printed() {
        let text = format!("{:?}", SealerKey::Seed([0xAB; 32]));
        assert!(!text.contains("171") && !text.contains("ab"), "{text}");
        assert!(text.contains("redacted"));
    }

    #[test]
    fn a_server_key_that_is_not_a_point_is_refused() {
        // y = 2 has no x on the curve (x^2 = (y^2 - 1) / (d y^2 + 1) is a
        // non-residue), so these 32 bytes decompress to nothing.
        let mut not_a_point = [0u8; 32];
        not_a_point[0] = 2;
        let error = Sealer::new(SealerKey::Seed([1u8; 32]), &not_a_point, None, AUDIENCE)
            .expect_err("a non-point key must be refused");
        assert!(matches!(error, SealError::Seal(_)));
    }
}
