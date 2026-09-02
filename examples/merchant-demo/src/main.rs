//! `merchant-demo` — a runnable walk through **everything vpay's `/v1`
//! surface answers today**, and a deliberate demonstration of where it stops.
//!
//! Four steps, one line each:
//!
//! 1. Read the OP's discovery document and JWKS, and show the issuer and the
//!    `kid` the server signs `/v1` access tokens with.
//! 2. Obtain an access token as the demo merchant and print its `iss`,
//!    `aud`, `sub` and `exp` — decoded, **not** verified, and never the token
//!    itself.
//! 3. Show that the same `/v1` path with no bearer token is a `401` carrying
//!    vpay's error envelope, so the authentication boundary is real.
//! 4. Call `payment_intents().retrieve` through the merchant SDK and print
//!    the typed `404 unknown_route` that comes back.
//!
//! Step 4 succeeding *as a 404* is the point of this program. Past the
//! bearer-token boundary vpay has no `/v1` resource route at all yet, so a
//! `200` there would mean someone had fabricated one — which is why this
//! demo treats a `200` as a hard failure rather than a pleasant surprise
//! (`CLAUDE.md`, "the failure mode to avoid"). When payment intents land,
//! step 4 becomes the first step of a real charge and this file changes with
//! it.
//!
//! # Why the token exchange in step 2 is not `Client`'s
//!
//! `vpay_sdk::Client` mints, caches and attaches access tokens, and
//! deliberately never hands one back: it has no accessor, and its `Debug`
//! redacts the cache (`sdks/rust/tests/debug_redaction.rs`). That is correct
//! for an SDK and unhelpful for a demo whose job is to *show* the claims. So
//! step 2 mints its assertion with the SDK's own
//! [`vpay_sdk::auth::mint_client_assertion`] — the security-carrying half,
//! not re-implemented here — and performs the one form POST itself. Step 4
//! then goes through `Client`, which does its own exchange, so the SDK's full
//! path is exercised too and neither step stands in for the other.

// This is a terminal program; stdout is its output medium, not stray
// debugging. Same allow, for the same reason, as `.xtask/src/main.rs`.
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use vpay_sdk::{Client, Credentials};

/// Where `just demo` publishes the stack, and what `config/application.yml`'s
/// `deployment.public_base_url` says — the two must agree or the OP's issuer
/// will not match the URL the SDK derives its endpoints from. Step 1 says so
/// out loud when they differ.
const DEFAULT_BASE_URL: &str = "http://localhost:8080";

/// The merchant `just gen-demo-keys` registers in `.e2e/application-demo.yml`.
const DEFAULT_CLIENT_ID: &str = "demo-merchant";

/// Where `just gen-demo-keys` puts that merchant's private key. Under
/// `.e2e/`, which is git-ignored and thrown away with the stack — this key is
/// a demo artefact and must never be reused for anything.
const DEFAULT_PRIVATE_KEY_FILE: &str = ".e2e/demo-merchant/oauth-signing-key.pem";

/// An id chosen to look like a real one and resolve to nothing, because
/// nothing is what `/v1/payment_intents/{id}` resolves to today.
const DEMO_INTENT_ID: &str = "pi_demo";

/// The one sentence this demo exists to put on a terminal.
const NOT_BUILT_YET: &str = "payment intents are not built yet — this is where the next step lands";

/// How long the demo waits on any single HTTP request. Short on purpose: a
/// stack that has not finished booting should fail this demo in seconds with
/// a transport error, not hang on a socket.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The assertion lifetime step 2 signs with. Well inside the OP's 300s
/// ceiling (`authkestra_engine::client_assertion::MAX_CLIENT_ASSERTION_LIFETIME_SECS`,
/// mirrored by `vpay_sdk::auth::MAX_ASSERTION_LIFETIME_SECS`).
const ASSERTION_LIFETIME: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> ExitCode {
    // The workspace pins reqwest 0.13 with `rustls-no-provider`, under which
    // `ClientBuilder::build()` panics if no process-wide `CryptoProvider` was
    // installed (root `Cargo.toml`'s reqwest note). Installing one is an
    // application's decision, so an application is where it happens —
    // `vpay-server`'s `main` does the identical thing. `Err` here means
    // something already installed one, which is the outcome we wanted anyway.
    let _ = rustls::crypto::ring::default_provider().install_default();

    match run().await {
        Ok(()) => {
            println!();
            println!("✔ all four steps behaved as expected.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!();
            eprintln!("✘ {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let base_url = env_or("VPAY_BASE_URL", DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_owned();
    let client_id = env_or("VPAY_CLIENT_ID", DEFAULT_CLIENT_ID);
    let key_file = PathBuf::from(env_or("VPAY_PRIVATE_KEY_FILE", DEFAULT_PRIVATE_KEY_FILE));

    println!("vpay merchant demo");
    println!("  base URL     {base_url}   (VPAY_BASE_URL)");
    println!("  client_id    {client_id}   (VPAY_CLIENT_ID)");
    println!(
        "  private key  {}   (VPAY_PRIVATE_KEY_FILE)",
        key_file.display()
    );
    println!();

    let http = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| anyhow!("step 1 (discovery): cannot build an HTTP client: {e}"))?;

    let pem = read_private_key(&key_file)?;

    let endpoints = step_1_discovery(&http, &base_url).await?;
    step_2_access_token(&http, &client_id, &pem, &endpoints).await?;
    step_3_unauthenticated(&http, &base_url).await?;
    step_4_payment_intents(&base_url, &client_id, &pem).await?;

    Ok(())
}

/// What step 1 learned from the server, so later steps use the OP's *own*
/// advertised endpoints rather than a URL this program guessed.
struct Endpoints {
    issuer: String,
    token_endpoint: String,
}

// ------------------------------------------------------------------ step 1

/// Reads the discovery document and the JWKS.
///
/// Both are unauthenticated by design (`docs/flows/merchant-auth.md`): a
/// merchant must be able to find the token endpoint before it holds a token,
/// and any verifier must be able to fetch the public keys that check vpay's
/// signatures.
async fn step_1_discovery(http: &reqwest::Client, base_url: &str) -> anyhow::Result<Endpoints> {
    const STEP: &str = "step 1 (discovery + JWKS)";

    println!("[1/4] discovery + JWKS");

    let discovery_url = format!("{base_url}/v1/oauth/.well-known/openid-configuration");
    let discovery = get_json(http, &discovery_url)
        .await
        .with_context(|| format!("{STEP}: GET {discovery_url}"))?;

    let issuer = string_field(&discovery, "issuer").with_context(|| STEP)?;
    let token_endpoint = string_field(&discovery, "token_endpoint").with_context(|| STEP)?;
    let jwks_uri = string_field(&discovery, "jwks_uri").with_context(|| STEP)?;

    println!("  ✔ GET /v1/oauth/.well-known/openid-configuration");
    println!("      issuer          {issuer}");
    println!("      token_endpoint  {token_endpoint}");
    println!("      jwks_uri        {jwks_uri}");

    // The SDK derives `{base_url}/v1/oauth` on its own and signs its
    // assertion for `{issuer}/token`. The server derives its issuer from
    // `deployment.public_base_url` in the YAML instead. When those two
    // disagree the OP rejects every assertion with a bare `invalid_client`
    // and nothing in that message points at the cause — so say it here,
    // before step 2 fails.
    let derived_issuer = format!("{base_url}/v1/oauth");
    if issuer != derived_issuer {
        println!(
            "      note: the SDK derives {derived_issuer} from VPAY_BASE_URL, which is not the \
             issuer above. `deployment.public_base_url` in the server's config disagrees with \
             VPAY_BASE_URL; step 2 will be refused with invalid_client."
        );
    }

    let jwks = get_json(http, &jwks_uri)
        .await
        .with_context(|| format!("{STEP}: GET {jwks_uri}"))?;
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{STEP}: {jwks_uri} has no `keys` array"))?;
    if keys.is_empty() {
        bail!(
            "{STEP}: {jwks_uri} published an empty key set — the server signs `/v1` tokens with a \
             key it does not publish, so nothing could verify them"
        );
    }
    println!("  ✔ GET /v1/oauth/jwks.json — {} key(s)", keys.len());
    for key in keys {
        println!(
            "      kid={}  alg={}  kty={}",
            key.get("kid").and_then(Value::as_str).unwrap_or("(none)"),
            key.get("alg").and_then(Value::as_str).unwrap_or("(none)"),
            key.get("kty").and_then(Value::as_str).unwrap_or("(none)"),
        );
    }

    Ok(Endpoints {
        issuer,
        token_endpoint,
    })
}

// ------------------------------------------------------------------ step 2

/// Performs one `client_credentials` + `private_key_jwt` exchange and prints
/// the resulting token's claims.
///
/// The token itself is never printed, logged or written anywhere: it is a
/// bearer credential for the whole of `/v1`, and a demo that pasted one onto
/// a terminal would teach exactly the wrong habit. Only `iss`, `aud`, `sub`
/// and `exp` are shown, and they are read straight out of the payload
/// segment **without verifying the signature** — this is a human-readable
/// peek, not a validation. The party that must verify is vpay's own resource
/// server, and step 4 is what proves it did.
async fn step_2_access_token(
    http: &reqwest::Client,
    client_id: &str,
    pem: &str,
    endpoints: &Endpoints,
) -> anyhow::Result<()> {
    const STEP: &str = "step 2 (access token)";

    println!();
    println!("[2/4] access token (client_credentials + private_key_jwt)");

    let credentials = credentials(client_id, pem).with_context(|| STEP)?;
    let assertion = vpay_sdk::auth::mint_client_assertion(
        &credentials,
        &endpoints.token_endpoint,
        ASSERTION_LIFETIME,
    )
    .with_context(|| format!("{STEP}: minting the client assertion"))?;

    let request_body = form_body(&[
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        (
            "client_assertion_type",
            vpay_sdk::auth::CLIENT_ASSERTION_TYPE_JWT_BEARER,
        ),
        ("client_assertion", assertion.as_str()),
        ("audience", vpay_sdk::DEFAULT_AUDIENCE),
    ]);

    let response = http
        .post(&endpoints.token_endpoint)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(request_body)
        .send()
        .await
        .with_context(|| format!("{STEP}: POST {}", endpoints.token_endpoint))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .with_context(|| format!("{STEP}: reading the token response body"))?;

    if !status.is_success() {
        bail!(
            "{STEP}: the token endpoint refused this merchant with HTTP {}: {}\n         \
             `just gen-demo-keys` registers `{client_id}` in .e2e/application-demo.yml, and \
             compose.demo.yml is what mounts that file into the server as the `demo` profile \
             overlay — check the server is running with VPAY_PROFILE=demo.",
            status.as_u16(),
            bounded(&body),
        );
    }

    let token_response: Value = serde_json::from_str(&body)
        .with_context(|| format!("{STEP}: the token response is not JSON: {}", bounded(&body)))?;
    let access_token = string_field(&token_response, "access_token").with_context(|| STEP)?;

    println!(
        "  ✔ POST {} — HTTP {}, token_type={}, expires_in={}",
        endpoints.token_endpoint,
        status.as_u16(),
        token_response
            .get("token_type")
            .and_then(Value::as_str)
            .unwrap_or("(none)"),
        token_response
            .get("expires_in")
            .map_or_else(|| "(none)".to_owned(), ToString::to_string),
    );

    let claims = peek_claims(&access_token).with_context(|| STEP)?;
    println!("      decoded (UNVERIFIED) claims — the token itself is never printed:");
    println!("        iss  {}", render(claims.get("iss")));
    println!("        aud  {}", render(claims.get("aud")));
    println!("        sub  {}", render(claims.get("sub")));
    println!("        exp  {}", render(claims.get("exp")));

    // `iss` is the one claim this program can check on its own: the OP is
    // supposed to stamp exactly the issuer it advertised a moment ago in step
    // 1, and a token issued under a different one would be rejected by any
    // verifier configured from that discovery document.
    let stamped_issuer = claims.get("iss").and_then(Value::as_str);
    if stamped_issuer != Some(endpoints.issuer.as_str()) {
        bail!(
            "{STEP}: the token's `iss` is {} but discovery advertised {} — a verifier configured \
             from that discovery document would refuse this token",
            render(claims.get("iss")),
            endpoints.issuer,
        );
    }

    Ok(())
}

// ------------------------------------------------------------------ step 3

/// The negative half of the boundary: the same `/v1` path, no bearer token.
///
/// The SDK cannot express this request — it always attaches a token — so this
/// one goes over the plain client. A `404` here instead of a `401` would mean
/// the authentication layer sits *behind* the `/v1` fallback rather than in
/// front of it, which is a hole worth failing the demo over: it would leak
/// which routes exist to an unauthenticated caller.
async fn step_3_unauthenticated(http: &reqwest::Client, base_url: &str) -> anyhow::Result<()> {
    const STEP: &str = "step 3 (unauthenticated /v1 is 401)";

    println!();
    println!("[3/4] the same path with no bearer token");

    let url = format!("{base_url}/v1/payment_intents/{DEMO_INTENT_ID}");
    let response = http
        .get(&url)
        .send()
        .await
        .with_context(|| format!("{STEP}: GET {url}"))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .with_context(|| format!("{STEP}: reading the response body"))?;

    if status != 401 {
        bail!(
            "{STEP}: expected HTTP 401 from an unauthenticated GET {url}, got HTTP {status}: {}",
            bounded(&body),
        );
    }

    let envelope: Value = serde_json::from_str(&body)
        .with_context(|| format!("{STEP}: the 401 body is not JSON: {}", bounded(&body)))?;
    let kind = envelope
        .pointer("/error/type")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!(
                "{STEP}: the 401 body is not vpay's error envelope (no `error.type`): {}",
                bounded(&body)
            )
        })?;
    let code = envelope
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("(none)");
    let message = envelope
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("(none)");

    if kind != "authentication_error" {
        bail!("{STEP}: expected `error.type` = authentication_error, got {kind}");
    }

    println!("  ✔ GET /v1/payment_intents/{DEMO_INTENT_ID} without a token — HTTP 401");
    println!("      error.type     {kind}");
    println!("      error.code     {code}");
    println!("      error.message  {message}");

    Ok(())
}

// ------------------------------------------------------------------ step 4

/// The authenticated call, through the SDK, and the honest answer it gets.
///
/// A `404 unknown_route` here is the success condition, and it proves more
/// than it looks: that envelope is only reachable *past* the bearer-token
/// boundary, so receiving it means the SDK minted an assertion, the OP
/// resolved this merchant out of the YAML registry, signed a token, and the
/// resource server fetched the JWKS and validated it. A `401` would mean the
/// token was refused; a `200` would mean a route was invented.
async fn step_4_payment_intents(base_url: &str, client_id: &str, pem: &str) -> anyhow::Result<()> {
    const STEP: &str = "step 4 (authenticated /v1 is the honest 404)";

    println!();
    println!("[4/4] payment_intents().retrieve(\"{DEMO_INTENT_ID}\") through vpay-sdk");

    // Configured the way `docs/flows/merchant-auth.md` tells a merchant to
    // configure it: a base URL and a credential. The issuer, the token
    // endpoint and the `vpay:v1` audience are the SDK's own derivations, not
    // values handed to it here — which is what makes this a test of that
    // derivation.
    let client = Client::builder(base_url)
        .credentials(credentials(client_id, pem).with_context(|| STEP)?)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .with_context(|| format!("{STEP}: building the SDK client"))?;

    match client.payment_intents().retrieve(DEMO_INTENT_ID).await {
        Ok(intent) => bail!(
            "{STEP}: the server returned a payment intent ({}) — vpay implements no `/v1` \
             resource route, so this response cannot be real. Do not trust anything else this \
             stack reports until you know where it came from.",
            intent.id,
        ),
        Err(vpay_sdk::Error::Api {
            status: 404,
            ref kind,
            ref code,
            ref message,
            ..
        }) if code.as_deref() == Some("unknown_route") => {
            println!("  ✔ HTTP 404 — authenticated, and then nothing to authenticate *for*");
            println!("      error.type     {kind}");
            println!("      error.code     unknown_route");
            println!("      error.message  {message}");
            println!();
            println!("      {NOT_BUILT_YET}");
        }
        Err(vpay_sdk::Error::Api { status: 401, .. }) => bail!(
            "{STEP}: the SDK obtained a token in step 2 but `/v1` refused it with 401. The token \
             is signed for `aud = {}`; check the demo merchant's `allowed_audiences` in \
             .e2e/application-demo.yml and the server's own resource-server audience.",
            vpay_sdk::DEFAULT_AUDIENCE,
        ),
        Err(other) => bail!(
            "{STEP}: expected the 404 `unknown_route` envelope past the authentication boundary, \
             got: {other}"
        ),
    }

    Ok(())
}

// ------------------------------------------------------------------ helpers

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// Loads the merchant's private key, with a message that names the recipe
/// that creates it — the overwhelmingly likely reason this file is missing is
/// that `just gen-demo-keys` has not been run in this checkout.
fn read_private_key(path: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(path).map_err(|e| {
        anyhow!(
            "step 0 (merchant key): cannot read {} ({e}). Run `just gen-demo-keys`, or point \
             VPAY_PRIVATE_KEY_FILE at the key registered for this client_id.",
            path.display(),
        )
    })
}

/// Built twice per run rather than cloned: [`Credentials`] holds private key
/// material and is deliberately neither `Clone` nor `Copy`, and re-parsing the
/// same PEM is cheaper than arguing with that decision.
fn credentials(client_id: &str, pem: &str) -> anyhow::Result<Credentials> {
    Credentials::rsa_pem(client_id, pem)
        .context("the private key is not a parseable RSA key (PKCS#1 or PKCS#8 PEM)")
}

/// Encodes an `application/x-www-form-urlencoded` body.
///
/// Hand-rolled rather than reached for from a crate, and not through
/// `reqwest`'s own `.form()`: that method lives behind reqwest's
/// `urlencoded` feature, which the workspace pin does not enable, and
/// enabling it here would put `serde_urlencoded` into the resolved graph of
/// every binary in the workspace — including the two that ship — to save
/// fifteen lines in a demo. `vpay-sdk` encodes its own bodies for a related
/// reason (byte-parity with the Node SDK; see `sdks/rust/src/form.rs`), but
/// that module is private to the SDK.
///
/// Percent-encodes everything outside RFC 3986's unreserved set, so no `+`
/// ever appears: this body carries a `client_assertion`, and a `+` that a
/// server decoded as a space would corrupt a signature.
fn form_body(fields: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, value) in fields {
        if !out.is_empty() {
            out.push('&');
        }
        out.push_str(&percent_encode(name));
        out.push('=');
        out.push_str(&percent_encode(value));
    }
    out
}

fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

async fn get_json(http: &reqwest::Client, url: &str) -> anyhow::Result<Value> {
    let response = http.get(url).send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!("HTTP {}: {}", status.as_u16(), bounded(&body));
    }
    serde_json::from_str(&body)
        .map_err(|e| anyhow!("response is not JSON ({e}): {}", bounded(&body)))
}

fn string_field(value: &Value, field: &str) -> anyhow::Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        // Names the offending field's JSON *type*, never any part of the
        // document: this runs on the token response, so echoing a
        // malformed-but-2xx body would put `access_token` on stdout.
        .ok_or_else(|| {
            anyhow!(
                "expected a string `{field}` in the response, found {}",
                json_type(value.get(field))
            )
        })
}

/// The JSON type of `value` as a fixed label, carrying none of its content.
///
/// Deliberately not `Display`/`to_string`: every caller is an error path on
/// a document that may hold a bearer token, and the type alone is what makes
/// the message diagnostic.
fn json_type(value: Option<&Value>) -> &'static str {
    match value {
        None => "no such field",
        Some(Value::Null) => "<null>",
        Some(Value::Bool(_)) => "<bool>",
        Some(Value::Number(_)) => "<number>",
        Some(Value::String(_)) => "<string>",
        Some(Value::Array(_)) => "<array>",
        Some(Value::Object(_)) => "<object>",
    }
}

/// Renders a claim for a human without asserting its JSON type: `aud` is a
/// string on some issuers and an array on others, and a demo that only
/// understood one of the two would print nothing for the other.
fn render(value: Option<&Value>) -> String {
    match value {
        None => "(absent)".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// The payload segment of a JWS, decoded and **not verified**.
///
/// Verification needs the issuer's public key and is the resource server's
/// job, not a demo's; presenting an unverified decode as if it were validated
/// is the kind of claim this repository exists to avoid making. The name says
/// "peek" for that reason.
fn peek_claims(jwt: &str) -> anyhow::Result<Value> {
    let payload = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| anyhow!("the access token is not a three-segment JWS"))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .context("the access token's payload segment is not base64url")?;
    serde_json::from_slice(&bytes).context("the access token's payload is not JSON")
}

/// Caps an upstream body before it reaches an error message. A proxy or a
/// misconfigured server can answer with an arbitrarily large page, and an
/// error value is not the place to hold one.
fn bounded(body: &str) -> String {
    const LIMIT: usize = 400;
    match body.char_indices().nth(LIMIT) {
        None => body.to_owned(),
        Some((cut, _)) => match body.get(..cut) {
            Some(prefix) => format!("{prefix}… ({} bytes total)", body.len()),
            None => body.to_owned(),
        },
    }
}
