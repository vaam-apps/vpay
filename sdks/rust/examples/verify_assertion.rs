//! Verifies a `private_key_jwt` client assertion with the **real** OP
//! verifier — `authkestra_op::client_assertion::verify_client_assertion` at
//! the pinned `=0.7.1` — and reports whether it would authenticate.
//!
//! This exists so the *Node* SDK's assertions can be checked against the same
//! verifier the Rust SDK's own test suite uses (`tests/op_conformance.rs`).
//! Two independent implementations of one wire contract will eventually
//! disagree; the cheapest place to find out is here, not in a merchant's
//! integration. Wired up as `just sdk-conformance-node`.
//!
//! **This is a manual check, not a CI gate**, and it proves nothing about
//! vpay's server side: no vpay serves a token endpoint (`docs/status.md`), so
//! a "verified" here means "the OP library would have accepted this", not
//! "vpay accepted this".
//!
//! # Usage
//!
//! ```text
//! # assertion and JWK Set as arguments
//! verify_assertion <jwt> <jwks-json> <client_id> <audience>...
//!
//! # or the JSON `sdks/nodejs/scripts/mint-assertion.mjs` prints, on stdin:
//! #   { "assertion": "<jwt>", "jwks": { "keys": [ ... ] } }
//! verify_assertion - <client_id> <audience>...
//! ```
//!
//! Exits `0` if the assertion verifies, `1` otherwise.

// An example is a CLI: its whole output is what it prints. `print_stdout` is
// a workspace-wide warn aimed at library and server code, where a stray
// `println!` bypasses `tracing`; there is no tracing subscriber here and
// nothing to bypass.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Read as _;
use std::process::ExitCode;

use authkestra_op::client_assertion::verify_client_assertion;
use authkestra_op::{ClientRegistration, GrantType, TokenEndpointAuthMethod};
use serde_json::Value;

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("verification failed: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().map(String::as_str).ok_or_else(usage)?;

    let (assertion, jwks, client_id, audiences) = if first == "-" {
        // stdin mode: `{ "assertion": "...", "jwks": { "keys": [...] } }`,
        // exactly what `sdks/nodejs/scripts/mint-assertion.mjs` prints.
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| format!("reading stdin: {e}"))?;
        let payload: Value =
            serde_json::from_str(&input).map_err(|e| format!("stdin is not JSON: {e}"))?;
        let assertion = payload
            .get("assertion")
            .and_then(Value::as_str)
            .ok_or("stdin JSON has no string `assertion` field")?
            .to_string();
        let jwks = payload
            .get("jwks")
            .cloned()
            .ok_or("stdin JSON has no `jwks` field")?;
        let client_id = args.get(1).cloned().ok_or_else(usage)?;
        let audiences: Vec<String> = args.iter().skip(2).cloned().collect();
        (assertion, jwks, client_id, audiences)
    } else {
        let assertion = first.to_string();
        let jwks: Value = args.get(1).ok_or_else(usage).and_then(|raw| {
            serde_json::from_str(raw).map_err(|e| format!("bad <jwks-json>: {e}"))
        })?;
        let client_id = args.get(2).cloned().ok_or_else(usage)?;
        let audiences: Vec<String> = args.iter().skip(3).cloned().collect();
        (assertion, jwks, client_id, audiences)
    };

    if audiences.is_empty() {
        return Err(usage());
    }

    let client = ClientRegistration {
        client_id: client_id.clone(),
        client_secret_hash: None,
        redirect_uris: Vec::new(),
        grant_types: vec![GrantType::ClientCredentials],
        scopes: Vec::new(),
        // Deprecated at 0.7.0 (PKCE is unconditional on the authorization-code
        // grant, which a merchant client never uses) but still a required
        // field of the struct.
        #[allow(deprecated)]
        require_pkce: false,
        allowed_audiences: vec!["vpay:v1".to_string()],
        token_endpoint_auth_method: Some(TokenEndpointAuthMethod::PrivateKeyJwt),
        jwks: Some(jwks),
    };

    let verified = verify_client_assertion(&assertion, &client, &audiences)
        .map_err(|e| format!("{e:?} (client_id={client_id}, audiences={audiences:?})"))?;

    Ok(format!(
        "verified: the pinned authkestra-op verifier accepted this assertion \
         for client_id={client_id}\n  jti={} (still to be spent through a \
         ClientAssertionStore)\n  exp={}",
        verified.jti, verified.expires_at,
    ))
}

fn usage() -> String {
    concat!(
        "usage:\n",
        "  verify_assertion <jwt> <jwks-json> <client_id> <audience>...\n",
        "  verify_assertion - <client_id> <audience>...   (reads ",
        "{\"assertion\":..,\"jwks\":..} from stdin)"
    )
    .to_string()
}
