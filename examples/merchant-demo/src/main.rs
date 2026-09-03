//! `merchant-demo` — a runnable walk through **everything vpay's `/v1`
//! surface answers today**, and a deliberate demonstration of where it stops.
//!
//! Seven steps:
//!
//! 1. Read the OP's discovery document and JWKS, and show the issuer and the
//!    `kid` the server signs `/v1` access tokens with.
//! 2. Obtain an access token as the demo merchant and print its `iss`,
//!    `aud`, `sub` and `exp` — decoded, **not** verified, and never the token
//!    itself.
//! 3. Show that the same `/v1` path with no bearer token is a `401` carrying
//!    vpay's error envelope, so the authentication boundary is real.
//! 4. Create a PaymentIntent through the merchant SDK and read it back,
//!    printing its id, status, amount and currency.
//! 5. Confirm it against the push rail, which accepts it: the MTN adapter
//!    submits to the WireMock rail `compose.yml` configures, and the intent
//!    moves to `processing`.
//! 6. Wait for `vpay-worker` to drive it to `succeeded`, polling
//!    `GET /v1/payment_intents/{id}` exactly as a merchant integration would.
//! 7. Read the webhook that settlement produced out of the receiver's own
//!    request journal, and verify its `Vpay-Signature` with the shipping
//!    SDK — the same call a merchant's handler makes.
//!
//! **Each of the last three steps changed once the code behind it landed,
//! and that is the point.** Step 5 used to assert a `501 not_implemented`,
//! because no adapter implemented `submit`; step 6 did not exist, because
//! nothing polled a charge and the demo said so in place of a payment; step
//! 7 used to report an *absent* webhook, because nothing wrote an `events`
//! row and nothing drained one. All three now assert what the code actually
//! does, and all three are the first thing to break when that stops being
//! true.
//!
//! **Why the demo can end in `succeeded` at all.** Step 6 is not a poll that
//! hopes: the rail stub answers `PENDING` on the first status query and
//! `SUCCESSFUL` on the second, and it does so because the confirm's MSISDN
//! ([`DEMO_MSISDN`]) enters a WireMock scenario keyed on that number
//! (`backends/tests/conformance/wiremock/mtn/mappings/requesttopay-scenario.json`).
//! Nothing here fakes an approval; a real worker asks a real stub twice, over
//! HTTP, and the second answer is the one that moves the money.
//!
//! **What step 6 still cannot show:** `amount_received`. The settlement
//! transaction writes that column (`vpay_db::settlement::apply_succeeded`),
//! but the `payment_intent` object does not carry it, so a merchant's client
//! cannot see it and neither can this demo. Printing it would mean reading
//! the database behind the API this demo exists to demonstrate. It is named
//! here rather than quietly omitted.
//!
//! Nothing here prints a secret: not the access token, not the private key,
//! not a rail credential, and not the webhook signing secret step 7 verifies
//! with. Steps 4-6 print the intent's own public fields.
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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use vpay_sdk::{
    Client, ConfirmPaymentIntentParams, CreatePaymentIntentParams, Credentials, IntentStatus,
    PaymentMethodType, RequestOptions,
};

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

/// Where `just demo` publishes the `wiremock-webhook` receiver — the
/// container `.e2e/application-demo.yml` points this merchant's webhook
/// endpoint at. `just demo_receiver_port=…` moves it, and the recipe exports
/// the matching `VPAY_RECEIVER_URL`.
///
/// Step 7 reads this container's request *journal*
/// (`GET /__admin/requests`), which is the merchant-side view: what actually
/// arrived, headers and body, rather than what vpay believes it sent.
const DEFAULT_RECEIVER_URL: &str = "http://localhost:8083";

/// The webhook signing secret `compose.e2e.yml` gives both binaries, so step
/// 7 can verify a delivery the same way a merchant's own handler would.
///
/// A **stub** value for a stub receiver, and not a secret in any meaningful
/// sense: it is written in `compose.e2e.yml` in plain sight, and the demo
/// stack is `livemode: false` (a literal there under livemode is a refusal
/// to boot). It is still read from the environment first, so pointing this
/// demo at a stack configured differently needs no rebuild — and it is never
/// printed.
///
/// Over `vpay_config`'s 32-byte livemode floor even though this stack is
/// sandbox, so that the value an operator is most likely to copy is not one
/// the boot guard would then refuse for a reason unrelated to why it is
/// wrong.
const DEFAULT_WEBHOOK_SECRET: &str = "wiremock-stub-webhook-secret-32-bytes";

/// How long step 7 waits for the delivery before failing.
///
/// Step 6 has already seen the intent reach `succeeded`, so the `events` row
/// exists before this wait starts; what is left is the fan-out drain (a
/// singleton that reschedules every five seconds) and one `deliver_webhook`
/// job that runs immediately. Thirty seconds is several times that, which
/// makes a timeout here mean "the drain or the delivery is broken" rather
/// than "the demo was impatient".
const RECEIVER_WAIT: Duration = Duration::from_secs(30);

/// The id step 3 asks for without a token. Deliberately one no merchant
/// holds: step 3 is about the `401`, which is decided before any handler
/// looks anything up.
const DEMO_INTENT_ID: &str = "pi_demo";

/// The one sentence this demo exists to put on a terminal.
///
/// It is no longer about the adapters, and no longer about delivery — both
/// ship, and steps 6 and 7 prove it. What is still missing is the *inbound*
/// half: nothing accepts a rail's callback, so every settlement this demo
/// shows came from vpay asking rather than from being told.
const NOT_BUILT_YET: &str = concat!(
    "there is still no callback route — `POST /provider/{code}/callback` does not exist, ",
    "so a rail that calls back is ignored and every settlement above came from the ",
    "worker's own authenticated query_status (docs/status.md)"
);

/// The intent step 4 creates: 5,000 minor units on the push rail.
///
/// **EUR, not XAF, and that is a property of the rail rather than a
/// preference.** `config/application.yml` configures `mtn_momo` with
/// `currency: EUR` because MTN's sandbox rejects XAF (`docs/flows/money.md`),
/// and `/v1` refuses a confirm whose intent currency is not the chosen
/// rail's — before any charge exists (`vpay_api`'s `currencies_agree`). An
/// XAF intent here would therefore be a `400` at step 5 rather than a
/// payment, which is exactly the mistake the rule exists to catch, and
/// exactly the wrong thing for a demo to model. EUR has two decimals, so
/// `5000` is €50.00.
const DEMO_AMOUNT: i64 = 5000;
const DEMO_CURRENCY: &str = "eur";

/// The MSISDN step 5 prompts. A documentation number, not anyone's — and a
/// *specific* one, which is load-bearing rather than arbitrary.
///
/// `backends/tests/conformance/wiremock/mtn/mappings/requesttopay-scenario.json`
/// keys a WireMock scenario (`mtn-e2e-poll`, priority 5) on this value: a
/// `requestToPay` carrying it is accepted normally, and the two *status*
/// queries that follow answer `PENDING` then `SUCCESSFUL`. That is what lets
/// step 6 end in a settled payment instead of a demo that waits forever.
///
/// It has to be steered on the submit because it cannot be steered anywhere
/// else. `confirm` mints the rail reference itself (`Uuid::new_v4()`), and
/// MTN's status query is a `GET` carrying no body — so the payer's number,
/// which comes from the merchant's own request, is the only field of this
/// exchange a demo can choose. Change this constant and the demo stops
/// settling; that is the coupling, stated rather than buried.
const DEMO_MSISDN: &str = "237600000ce0";

/// How long step 6 waits for the worker to settle the charge.
///
/// The poll ladder's first rung is ten seconds (`vpay_worker::poll_delay`)
/// and the stub answers `PENDING` first, so the earliest possible settlement
/// is about eleven seconds after the confirm. This is a *ceiling* on a wait
/// that normally ends well before it — generous enough that a cold compose
/// stack does not fail the demo, tight enough that a worker which is not
/// running fails it in under a minute with a message saying so.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(90);

/// How often step 6 asks. A merchant integration polls; this is what that
/// looks like, at a rate that will not annoy the API.
const SETTLE_POLL_INTERVAL: Duration = Duration::from_secs(2);

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
            println!("✔ all seven steps behaved as expected.");
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
    let receiver_url = env_or("VPAY_RECEIVER_URL", DEFAULT_RECEIVER_URL)
        .trim_end_matches('/')
        .to_owned();

    println!("vpay merchant demo");
    println!("  base URL     {base_url}   (VPAY_BASE_URL)");
    println!("  client_id    {client_id}   (VPAY_CLIENT_ID)");
    println!(
        "  private key  {}   (VPAY_PRIVATE_KEY_FILE)",
        key_file.display()
    );
    println!("  receiver     {receiver_url}   (VPAY_RECEIVER_URL)");
    println!();

    let http = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| anyhow!("step 1 (discovery): cannot build an HTTP client: {e}"))?;

    let pem = read_private_key(&key_file)?;

    let endpoints = step_1_discovery(&http, &base_url).await?;
    step_2_access_token(&http, &client_id, &pem, &endpoints).await?;
    step_3_unauthenticated(&http, &base_url).await?;

    // One SDK client for both of the remaining steps, configured the way
    // `docs/flows/merchant-auth.md` tells a merchant to configure one: a base
    // URL and a credential. The issuer, the token endpoint and the `vpay:v1`
    // audience are the SDK's own derivations, not values handed to it here —
    // which is what makes this a test of that derivation.
    let client = Client::builder(&base_url)
        .credentials(credentials(&client_id, &pem).context("step 4 (create + retrieve)")?)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("step 4 (create + retrieve): building the SDK client")?;

    // One id per run, shared by step 4's and step 5's idempotency keys.
    let run = run_id()?;
    let intent_id = step_4_create_and_retrieve(&client, &run).await?;
    step_5_confirm(&client, &intent_id, &run).await?;
    step_6_await_settlement(&client, &intent_id).await?;
    step_7_webhook(&http, &receiver_url, &intent_id).await?;

    Ok(())
}

/// A value unique to this run, which the two `POST`s below derive their
/// `Idempotency-Key`s from.
///
/// # Why not a fixed string
///
/// It was one, and that was a bug the moment step 5 started succeeding. The
/// keys are kept for 24 hours (`docs/api/README.md`), and `just demo` does
/// not tear the database down between runs — so a second run under a fixed
/// key *replayed* step 4's stored response, which says
/// `requires_payment_method`, while the retrieve that follows it read the
/// row step 5 had since moved to `processing`. The demo then failed with
/// "the create's response and the stored row disagree", which was true and
/// was entirely the demo's own doing.
///
/// A per-run key makes each run a new payment, which is what a merchant
/// running this twice means. Wall-clock nanoseconds rather than a UUID
/// because that would be a dependency for one line; the value only has to
/// differ between runs on one machine, and it is never a security token.
fn run_id() -> anyhow::Result<String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("the system clock is before the unix epoch: {e}"))?
        .as_nanos();
    Ok(format!("demo-{nanos}"))
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

    println!("[1/7] discovery + JWKS");

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
    println!("[2/7] access token (client_credentials + private_key_jwt)");

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
    println!("[3/7] the same path with no bearer token");

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

/// Creates a PaymentIntent through the SDK and reads it back.
///
/// This is a real write to a real database: the row exists after this step,
/// filed under the demo merchant's tenant, and `just demo-down` is what
/// throws it away. The retrieve is not decoration — it is what proves the
/// create *persisted* rather than merely rendered an object, and comparing
/// the two is what would catch a retrieve that answered from somewhere else.
///
/// The `Idempotency-Key` is fixed rather than random on purpose: run this
/// demo twice against the same stack and the second run replays the first
/// run's stored answer instead of creating a second intent, which is the
/// property `docs/flows/merchant-auth.md` promises and the reason the header
/// is required at all. (`just demo-down` deletes the volumes, so a fresh
/// stack starts over.)
async fn step_4_create_and_retrieve(client: &Client, run: &str) -> anyhow::Result<String> {
    const STEP: &str = "step 4 (create + retrieve a payment intent)";

    println!();
    println!("[4/7] payment_intents().create(…) then .retrieve(…) through vpay-sdk");

    let params = CreatePaymentIntentParams {
        amount: DEMO_AMOUNT,
        currency: DEMO_CURRENCY.to_owned(),
        payment_method_types: vec![PaymentMethodType::MtnMomo],
        metadata: BTreeMap::from([("order_id".to_owned(), "demo-1234".to_owned())]),
        description: Some("merchant-demo order".to_owned()),
    };

    let created = client
        .payment_intents()
        .create(
            params,
            RequestOptions::new().with_idempotency_key(format!("{run}-create")),
        )
        .await
        .map_err(|error| describe(STEP, "creating a payment intent", &error))?;

    println!("  ✔ POST /v1/payment_intents — HTTP 200");
    println!("      id        {}", created.id);
    println!("      status    {}", status_label(created.status));
    println!(
        "      amount    {} {}   (minor units — {} has two decimals, so this is 50.00)",
        created.amount, created.currency, created.currency
    );
    println!(
        "      rails     {}",
        created.payment_method_types.join(", ")
    );
    println!("      livemode  {}", created.livemode);

    let retrieved = client
        .payment_intents()
        .retrieve(&created.id)
        .await
        .map_err(|error| describe(STEP, "retrieving the intent just created", &error))?;

    if retrieved != created {
        bail!(
            "{STEP}: the retrieve returned a different object than the create did. The create's \
             response and the stored row disagree, which means one of them is not what it says \
             it is."
        );
    }
    println!(
        "  ✔ GET /v1/payment_intents/{} — identical object",
        created.id
    );

    Ok(created.id)
}

// ------------------------------------------------------------------ step 5

/// Confirms the intent against the push rail, which accepts it.
///
/// `processing` is this step's success condition, and every other outcome
/// fails the demo — including the `501` that used to be the success
/// condition here. The request is real all the way down: vpay resolves the
/// adapter, commits the charge row with the reference it will submit under,
/// records the attempt, calls MTN's `requesttopay` over HTTP against the
/// `wiremock-mtn` host `config/application.yml` names, and commits what came
/// back before answering (`docs/flows/crash-safety.md`).
///
/// # What `processing` does and does not mean
///
/// It means the rail has the request and the payer's handset should be
/// prompting. It does **not** mean money moved: only an authenticated
/// `query_status` can say that, and nothing in this stack asks yet. The demo
/// prints that sentence rather than letting a reader infer a completed
/// payment from a green tick.
///
/// # Why no charge id is printed
///
/// There is none on the wire. `/v1` exposes payment intents, not charges —
/// there is no `/v1/charges` and the `payment_intent` object carries no
/// charge id (`docs/api/README.md`) — so a demo that printed one would have
/// had to read the database behind the API it is demonstrating. What it
/// prints instead is everything the merchant's own client can see.
async fn step_5_confirm(client: &Client, intent_id: &str, run: &str) -> anyhow::Result<()> {
    const STEP: &str = "step 5 (confirm reaches the rail and is accepted)";

    println!();
    println!("[5/7] payment_intents().confirm(\"{intent_id}\") through vpay-sdk");

    let confirmed = client
        .payment_intents()
        .confirm(
            intent_id,
            ConfirmPaymentIntentParams::mtn_momo(DEMO_MSISDN),
            RequestOptions::new().with_idempotency_key(format!("{run}-confirm")),
        )
        .await
        .map_err(|error| describe(STEP, "confirming the payment intent", &error))?;

    if confirmed.status != IntentStatus::Processing {
        bail!(
            "{STEP}: the rail accepted the charge and the intent is `{}` rather than \
             `processing`. A push rail's confirm has exactly one success state \
             (docs/flows/payment-lifecycle.md); anything else means the response and the \
             stored row disagree about what happened.",
            status_label(confirmed.status),
        );
    }
    if confirmed.next_action.is_some() {
        bail!(
            "{STEP}: a push rail returned a next_action. There is nothing for a browser to do \
             while a payer types a PIN into their own handset, so a redirect here would be \
             pointing them somewhere invented."
        );
    }

    println!("  ✔ HTTP 200 — the rail accepted the charge");
    println!("      id             {}", confirmed.id);
    println!(
        "      status         {}   (was requires_payment_method)",
        status_label(confirmed.status)
    );
    println!("      next_action    null   (push rails prompt the handset)");
    println!(
        "      charge         not on the wire — /v1 has no charges resource; the charge row, \
         its provider_reference_id and the provider_requests attempt are in Postgres"
    );

    // The response and the stored row agree. This is the assertion that
    // would fail if `confirm` rendered a status it had not committed.
    //
    // `client_secret` is excluded from that comparison, deliberately: it is
    // the one field that is *supposed* to differ between the two responses
    // (Step 5c's D2, `vpay_api::model::PaymentIntentWithSecret`) — `confirm`
    // omits it (the merchant already holds the credential from `create`,
    // and a browser never reaches this route), `retrieve` includes it. A
    // blanket comparison here would fail on that documented asymmetry
    // rather than on an actual disagreement between what the response said
    // and what the database holds.
    let after = client
        .payment_intents()
        .retrieve(intent_id)
        .await
        .map_err(|error| describe(STEP, "re-reading the intent after the confirm", &error))?;
    let mut after_without_secret = after.clone();
    after_without_secret.client_secret = None;
    if after_without_secret != confirmed {
        bail!(
            "{STEP}: the confirm's response and a later retrieve are different objects (apart \
             from client_secret, which is expected to differ). One of them is not what the \
             database holds."
        );
    }
    println!(
        "  ✔ GET /v1/payment_intents/{intent_id} — identical object, so the `{}` a merchant \
         was told is the `{}` vpay stored",
        status_label(after.status),
        status_label(after.status),
    );
    println!(
        "      the payer's handset is prompting; nothing here knows yet whether they \
         approved. Only an authenticated query_status can say, and that is step 6."
    );

    Ok(())
}

// ------------------------------------------------------------------ step 6

/// Waits for `vpay-worker` to drive the charge to a terminal state, polling
/// the merchant API exactly as a merchant integration would.
///
/// `succeeded` is this step's success condition and every other outcome fails
/// the demo — including `requires_payment_method`, which is what a *declined*
/// payment looks like and would mean the rail stub answered something other
/// than the scenario this MSISDN selects.
///
/// # What is actually happening while this loop waits
///
/// Nothing in this process. The work is in the `vpay-worker` container: it
/// claimed the `poll_charge` job the confirm committed *in the same
/// transaction as the charge*, asked MTN over HTTP, was told `PENDING`, put
/// the job back on the ladder ten seconds out, asked again, was told
/// `SUCCESSFUL`, and committed the charge, the intent and one
/// `payment_intent.succeeded` event together. This loop only observes the
/// result through the same `GET` any merchant has.
///
/// # Why a merchant polls at all, when step 7 shows a webhook
///
/// Because a poll is the fallback a merchant is told to keep
/// (`docs/api/README.md`): a delivery can be missed, and `GET
/// /v1/payment_intents/{id}` is the authoritative answer that cannot be. The
/// two steps deliberately observe the *same* settlement by the two routes a
/// merchant has — this one through the API, step 7 through the event the
/// same transaction wrote.
async fn step_6_await_settlement(client: &Client, intent_id: &str) -> anyhow::Result<()> {
    const STEP: &str = "step 6 (the worker drives the charge to a terminal state)";

    println!();
    println!(
        "[6/7] polling payment_intents().retrieve(\"{intent_id}\") until it is no longer \
         processing"
    );
    println!(
        "      (the vpay-worker container is asking the rail; the ladder's first rung is \
         10s, so this normally takes ~10-15s)"
    );

    let deadline = std::time::Instant::now() + SETTLE_TIMEOUT;
    let mut polls = 0_u32;
    let settled = loop {
        let intent = client
            .payment_intents()
            .retrieve(intent_id)
            .await
            .map_err(|error| describe(STEP, "re-reading the intent while it settles", &error))?;
        polls += 1;
        if intent.status != IntentStatus::Processing {
            break intent;
        }
        if std::time::Instant::now() >= deadline {
            bail!(
                "{STEP}: the intent was still `processing` after {SETTLE_TIMEOUT:?} and \
                 {polls} polls. Nothing drove the charge to a terminal state — the usual \
                 cause is that vpay-worker is not running or cannot reach the rail. Try \
                 `docker compose logs vpay-worker`."
            );
        }
        tokio::time::sleep(SETTLE_POLL_INTERVAL).await;
    };

    if settled.status != IntentStatus::Succeeded {
        bail!(
            "{STEP}: the charge resolved to `{}` rather than `succeeded`{}. The MSISDN this \
             demo confirms with ({DEMO_MSISDN}) selects a rail stub that answers PENDING \
             then SUCCESSFUL, so anything else means the stub, the mapping or the \
             settlement path changed.",
            status_label(settled.status),
            settled
                .last_payment_error
                .as_ref()
                .map_or_else(String::new, |error| format!(
                    " ({}: {})",
                    error.code, error.message
                )),
        );
    }

    println!("  ✔ settled after {polls} polls — the rail confirmed the payer approved it");
    println!("      id             {}", settled.id);
    println!(
        "      status         {}   (was processing)",
        status_label(settled.status)
    );
    println!(
        "      amount         {} {}   (integer minor units — docs/flows/money.md)",
        settled.amount,
        settled.currency.to_ascii_uppercase()
    );
    // Named, not printed, because it is genuinely not on the wire — see this
    // file's header. A demo that invented a number here, or that read the
    // database behind the API it is demonstrating, would be worse than one
    // that says what is missing.
    println!(
        "      amount_received  not on the wire — the settlement transaction writes \
         payment_intents.amount_received (= amount, {}), but the payment_intent object \
         does not carry it yet",
        settled.amount
    );
    println!(
        "      charge         succeeded in Postgres, with the rail's own provider_txn_id; \
         /v1 has no charges resource (docs/api/README.md)"
    );

    Ok(())
}

// ------------------------------------------------------------------ step 7

/// Reads the webhook the settlement in step 6 produced out of the receiver's
/// own request journal, and verifies it with the shipping SDK.
///
/// # Why the receiver's journal, and not vpay's own tables
///
/// Because the merchant-side view is the only one that can prove delivery.
/// `webhook_deliveries.state = 'succeeded'` is vpay's belief about what it
/// sent; `GET /__admin/requests` is what a receiver actually got, headers
/// and body, byte for byte. A demo that read the first would be quoting the
/// sender back to itself.
///
/// # Why a missing delivery fails the run
///
/// Every link now exists: the rail answered, the worker settled the charge
/// and wrote an `events` row in the same transaction, the `fanout:events`
/// drain turned it into a `webhook_deliveries` row and a `deliver_webhook`
/// job, and that job POSTed it. This step used to report an absent webhook
/// and pass, because the first two links were unbuilt; they are built, so an
/// absence here is a defect and is reported as one.
///
/// # What is checked, beyond "something arrived"
///
/// The signature is verified with `vpay_sdk::webhooks::verify` over the exact
/// recorded bytes — the same call a merchant's handler makes. A delivery that
/// arrives and does not verify fails the demo louder than one that never
/// arrives: at that point vpay is signing something a merchant cannot check.
/// `Stripe-Signature` is asserted to carry the *same* value as
/// `Vpay-Signature`, because that duplicate header exists so a merchant can
/// keep a Stripe-shaped handler, and one that drifted from the header it
/// mirrors would verify in the SDK and fail in their code.
async fn step_7_webhook(
    http: &reqwest::Client,
    receiver_url: &str,
    intent_id: &str,
) -> anyhow::Result<()> {
    const STEP: &str = "step 7 (the webhook the settled payment produced)";

    println!();
    println!("[7/7] polling {receiver_url}/__admin/requests for the delivery of {intent_id}");

    let deadline = std::time::Instant::now() + RECEIVER_WAIT;
    let mut delivered = None;
    let mut last_error = None;

    while std::time::Instant::now() < deadline {
        match recorded_webhook(http, receiver_url, intent_id).await {
            Ok(Some(found)) => {
                delivered = Some(found);
                break;
            }
            Ok(None) => {}
            // The receiver being briefly unreachable is not on its own a
            // failure — compose may still be publishing the port — so it is
            // remembered and only reported if the wait runs out.
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let Some(delivery) = delivered else {
        let reason = last_error.map_or_else(
            || "the journal was readable and held no POST for this intent".to_owned(),
            |error| format!("the last read of the journal failed: {error:#}"),
        );
        bail!(
            "{STEP}: no webhook was delivered for {intent_id} within {}s ({reason}). Step 6 \
             already saw the intent reach `succeeded`, so the `events` row exists and this \
             is the fan-out or the delivery failing, not a missing producer. Try \
             `docker compose logs vpay-worker`, and check that {receiver_url} is the port \
             `just demo` published wiremock-webhook on (VPAY_RECEIVER_URL).",
            RECEIVER_WAIT.as_secs()
        );
    };

    println!(
        "  ✔ the receiver recorded a POST carrying Vpay-Event-Id: {}",
        delivery.event_id
    );

    if delivery.stripe_signature != delivery.signature {
        bail!(
            "{STEP}: the delivery's Stripe-Signature and Vpay-Signature differ. They are the \
             same bytes by construction (vpay_worker::webhooks) so a merchant can keep a \
             Stripe-shaped handler; a drift here verifies in our SDK and fails in theirs."
        );
    }
    println!(
        "      Vpay-Signature   present, t=…,v1=… (not printed: it is a MAC, not a secret, but it is noise)"
    );
    println!("      Stripe-Signature present, and byte-identical to Vpay-Signature");

    let secret = env_or("MERCHANT_WEBHOOK_SECRET", DEFAULT_WEBHOOK_SECRET);
    let event = vpay_sdk::webhooks::verify(
        delivery.body.as_bytes(),
        &delivery.signature,
        &secret,
        vpay_sdk::webhooks::DEFAULT_TOLERANCE,
    )
    .map_err(|error| {
        anyhow!(
            "{STEP}: a webhook arrived and its Vpay-Signature does not verify ({error}). vpay \
             signed something this merchant cannot check, which is worse than sending nothing. \
             The secret came from MERCHANT_WEBHOOK_SECRET (or the compose stub); check it \
             matches `webhooks[0].secrets` in .e2e/application-demo.yml."
        )
    })?;

    println!("  ✔ Vpay-Signature verifies with vpay-sdk — the same call a handler makes");
    println!("      id             {}", event.id);
    println!("      type           {}", event.kind);
    println!("      livemode       {}", event.livemode);
    println!(
        "      data.object.id {}",
        event
            .data
            .object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("(absent)")
    );

    println!();
    println!("      {NOT_BUILT_YET}");

    Ok(())
}

/// One delivery as the receiver recorded it.
///
/// A struct rather than a tuple because the two signature headers are the
/// same string and a tuple would let a careless caller compare a value with
/// itself and call it a check.
struct Delivered {
    /// `Vpay-Event-Id` — the `evt_…` the delivery names.
    event_id: String,
    /// The recorded request body, verbatim. Never re-serialised.
    body: String,
    /// `Vpay-Signature`.
    signature: String,
    /// `Stripe-Signature`, which must equal [`Self::signature`].
    stripe_signature: String,
}

/// The POST in the receiver's journal that delivered an event about
/// `payment_intent_id`, if one has arrived yet.
///
/// Two filters, and both are load-bearing. The `Vpay-Event-Id` header,
/// because the receiver answers 200 to *anything* POSTed at it
/// (`backends/tests/webhook-receiver/wiremock/mappings/`) and a stray request
/// from something else on the machine must not be mistaken for a delivery.
/// The body's `data.object.id`, because the journal survives for the life of
/// the container and a previous run's delivery would otherwise satisfy this
/// run — a demo that passed on last run's evidence is exactly the false green
/// this repository is written against.
///
/// The body is taken as the journal's recorded text and never re-serialised:
/// the signature covers bytes, and a parse-and-reprint is the single most
/// common way a merchant breaks their own verification. It is *parsed* here
/// only to read the intent id, and the parse's result is thrown away.
async fn recorded_webhook(
    http: &reqwest::Client,
    receiver_url: &str,
    payment_intent_id: &str,
) -> anyhow::Result<Option<Delivered>> {
    let journal: Value = http
        .get(format!("{receiver_url}/__admin/requests"))
        .send()
        .await
        .context("reading the receiver's request journal")?
        .json()
        .await
        .context("the receiver's journal is JSON")?;

    let requests = journal
        .get("requests")
        .and_then(Value::as_array)
        .context("the journal has a `requests` array")?;

    for entry in requests {
        let Some(request) = entry.get("request") else {
            continue;
        };
        if request.get("method").and_then(Value::as_str) != Some("POST") {
            continue;
        }
        let headers = request.get("headers").and_then(Value::as_object);
        let header = |name: &str| {
            headers.and_then(|map| {
                map.iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .and_then(|(_, value)| value.as_str())
            })
        };
        let (Some(event_id), Some(signature), Some(stripe_signature)) = (
            header("Vpay-Event-Id"),
            header("Vpay-Signature"),
            header("Stripe-Signature"),
        ) else {
            continue;
        };
        let body = request
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let names_this_intent = serde_json::from_str::<Value>(body).is_ok_and(|parsed| {
            parsed
                .pointer("/data/object/id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == payment_intent_id)
        });
        if !names_this_intent {
            continue;
        }
        return Ok(Some(Delivered {
            event_id: event_id.to_owned(),
            body: body.to_owned(),
            signature: signature.to_owned(),
            stripe_signature: stripe_signature.to_owned(),
        }));
    }
    Ok(None)
}

// ------------------------------------------------------------------ helpers

/// One `IntentStatus` as its wire label.
///
/// Written out rather than `{:?}`-formatted: the wire spelling is
/// `requires_payment_method`, and a demo that printed `RequiresPaymentMethod`
/// would be showing a Rust identifier where a reader is trying to match the
/// value they will see in a webhook.
fn status_label(status: IntentStatus) -> &'static str {
    match status {
        IntentStatus::RequiresPaymentMethod => "requires_payment_method",
        IntentStatus::RequiresAction => "requires_action",
        IntentStatus::Processing => "processing",
        IntentStatus::Succeeded => "succeeded",
        IntentStatus::Canceled => "canceled",
    }
}

/// Turns an SDK error into a message that names the step and, for the two
/// failures an operator actually meets, what to go and look at.
///
/// A `401` past step 2 means the token was minted and then refused, which is
/// an audience mismatch nine times out of ten; a transport error means the
/// stack is not up. Neither is obvious from the SDK's own `Display`.
fn describe(step: &str, doing: &str, error: &vpay_sdk::Error) -> anyhow::Error {
    match error {
        vpay_sdk::Error::Api { status: 401, .. } => anyhow!(
            "{step}: {doing}: the SDK obtained a token and `/v1` refused it with 401. The token \
             is signed for `aud = {}`; check the demo merchant's `allowed_audiences` in \
             .e2e/application-demo.yml and the server's own resource-server audience.",
            vpay_sdk::DEFAULT_AUDIENCE,
        ),
        vpay_sdk::Error::Api {
            status: 404, code, ..
        } if code.as_deref() == Some("unknown_route") => anyhow!(
            "{step}: {doing}: the server answered `unknown_route`, so this deployment does not \
             route /v1/payment_intents at all. It is older than this demo — rebuild the stack \
             (`just demo-down && just demo`)."
        ),
        other => anyhow!("{step}: {doing}: {other}"),
    }
}

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
