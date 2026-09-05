//! `merchant-demo` — a runnable walk through **everything vpay's `/v1`
//! surface answers today**, and a deliberate demonstration of where it stops.
//!
//! Five steps, the fourth of which is a table:
//!
//! 1. Read the OP's discovery document and JWKS, and show the issuer and the
//!    `kid` the server signs `/v1` access tokens with.
//! 2. Obtain an access token as the demo merchant and print its `iss`,
//!    `aud`, `sub` and `exp` — decoded, **not** verified, and never the token
//!    itself.
//! 3. Show that the same `/v1` path with no bearer token is a `401` carrying
//!    vpay's error envelope, so the authentication boundary is real.
//! 4. Drive [`OUTCOMES`] — **six payments, on both rails, to every outcome
//!    each rail documents**. Each one creates a PaymentIntent through the
//!    merchant SDK, reads it back, confirms it, waits for `vpay-worker` to
//!    settle it, and then reads the webhook that settlement produced out of
//!    the receiver's own request journal and verifies its `Vpay-Signature`
//!    with the shipping SDK — the same call a merchant's handler makes.
//! 5. Create **one hosted and one embedded Checkout Session** for a fresh
//!    intent each, and print what a merchant does with them: the `url` to
//!    send a payer to, and the two values `initEmbeddedCheckout` needs. It
//!    stops there — this program has no browser and does not pretend to. The
//!    proof that the page works is `frontends/tests/e2e` and a human
//!    following `docs/runbooks/demo.md` §7.
//!

//! **Step 4 used to be one payment: MTN, succeeded.** That under-specified
//! the contract an integrator writes against, which is what issue #11 asked
//! to fix. A merchant integrating against vpay has to handle a decline, an
//! expiry and a redirect, and until Step 8 none of the three had ever been
//! demonstrated end to end.
//!
//! # How an outcome is chosen, and why it is not chosen here
//!
//! Nothing in this program tells vpay what should happen. Every outcome is
//! selected at the **rail stub**, by a field of the request that a merchant
//! genuinely controls, and the stub is a WireMock host reached over HTTP
//! exactly as a real rail would be (ADR-0006). The two rails give a merchant
//! different handles, and that difference is why the table has a
//! `selected_by` column:
//!
//! * **MTN** — the payer's MSISDN. `confirm` mints the rail reference itself
//!   (`Uuid::new_v4()`) and MTN's status query is a `GET` carrying no body,
//!   so the number on the submit is the only field of the whole exchange a
//!   merchant can steer. The stub carries the choice forward to the status
//!   query with a WireMock scenario
//!   (`backends/tests/conformance/wiremock/mtn/mappings/`).
//! * **Orange** — the amount. Orange's status call is a `POST` whose body
//!   carries `amount` beside `order_id`, so the stub can select on the status
//!   request itself, with no scenario and no state
//!   (`backends/tests/conformance/wiremock/orange/mappings/`).
//!
//! **The MTN half is order-sensitive and this program is what keeps it
//! honest.** Its scenarios are armed by a submit and answer the *next* status
//! query whatever reference it carries, so two MTN charges in flight at once
//! against one stub could be answered the wrong way round. [`run_outcomes`]
//! therefore drives the table strictly sequentially: each charge reaches a
//! terminal state, and its webhook is verified, before the next confirm is
//! sent. Do not parallelise it without re-reading those mapping files.
//!
//! # What this demo still cannot show
//!
//! * **`amount_received`.** The settlement transaction writes that column
//!   (`vpay_db::settlement::apply_succeeded`), but the `payment_intent`
//!   object does not carry it, so a merchant's client cannot see it and
//!   neither can this demo. Printing it would mean reading the database
//!   behind the API this demo exists to demonstrate.
//! * **A payer actually visiting Orange's hosted page.** Outcome 4 prints the
//!   `next_action.redirect_to_url` a merchant would send a browser to, and
//!   then the rail stub answers the status query as though the payer had
//!   completed it. Nothing here opens that URL. The browser return trip is a
//!   named gap, not an oversight — see `docs/runbooks/demo.md`.
//! * **A rail calling *us*.** The route exists; nothing in this demo
//!   makes a rail use it. See [`CALLBACK_NOT_EXERCISED`].
//!
//! Nothing here prints a secret: not the access token, not the private key,
//! not a rail credential, and not the webhook signing secret step 4 verifies
//! with. Every payment prints the intent's own public fields.
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
    CheckoutPaymentStatus, CheckoutSession, CheckoutSessionStatus, CheckoutUiMode, Client,
    ConfirmPaymentIntentParams, CreateCheckoutSessionParams, CreatePaymentIntentParams,
    Credentials, IntentStatus, NextAction, PaymentIntent, PaymentMethodType, RequestOptions,
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
/// Every outcome reads this container's request *journal*
/// (`GET /__admin/requests`), which is the merchant-side view: what actually
/// arrived, headers and body, rather than what vpay believes it sent.
const DEFAULT_RECEIVER_URL: &str = "http://localhost:8083";

/// The webhook signing secret `compose.e2e.yml` gives both binaries, so the
/// demo can verify a delivery the same way a merchant's own handler would.
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

/// How long an outcome waits for its webhook before failing.
///
/// The settlement has already been observed through the API by the time this
/// wait starts, so the `events` row exists; what is left is the fan-out drain
/// (a singleton that reschedules every five seconds) and one
/// `deliver_webhook` job that runs immediately. Thirty seconds is several
/// times that, which makes a timeout here mean "the drain or the delivery is
/// broken" rather than "the demo was impatient".
const RECEIVER_WAIT: Duration = Duration::from_secs(30);

/// The id step 3 asks for without a token. Deliberately one no merchant
/// holds: step 3 is about the `401`, which is decided before any handler
/// looks anything up.
const DEMO_INTENT_ID: &str = "pi_demo";

/// The one sentence this demo exists to put on a terminal.
///
/// This replaced a constant that said no callback route existed. Since Step 8
/// one does — `vpay_api::provider_callback` mounts
/// `POST /provider/{code}/callback` — so the honest claim is no longer about
/// the route's absence but about what this run exercised: the demo's rail
/// stubs are request/response WireMock mappings that call nothing back, so
/// not one of the settlements above was prompted by a callback.
///
/// That is a statement about coverage, not a caveat about correctness. A
/// callback is only ever a hint that pulls an already-queued poll forward;
/// the authenticated `query_status` is the only thing that moves money
/// (`docs/flows/provider-port.md`). So the settlement path this demo did
/// exercise is the same one a callback would have reached, sooner.
///
/// Like its predecessor this is the one claim in this file no assertion
/// backs — the demo cannot prove a negative about traffic it never sent. If
/// a future stub is given a `postServeAction` that posts to the callback
/// nest, this line stops being true and the compiler will not catch it.
const CALLBACK_NOT_EXERCISED: &str = concat!(
    "the callback route exists — `POST /provider/{code}/callback` — but this demo's rail ",
    "stubs never call it, so every settlement above came from the worker's own ",
    "authenticated query_status; a callback would only have been a hint that pulled that ",
    "same poll forward (docs/flows/provider-port.md)"
);

/// What the demo asks the payer to do, and therefore what the rail stub keys
/// its answer on.
///
/// An enum rather than two `Option`s because the rails do not take the same
/// field and three of the four combinations would be nonsense — the same
/// reasoning as [`ConfirmPaymentIntentParams`]'s own shape.
enum Steering {
    /// MTN: the payer's MSISDN, which is the one field of a push exchange a
    /// merchant chooses. A documentation number in the `2376000000xx` block.
    Msisdn(&'static str),
    /// Orange: where the rail returns the payer afterwards. The *outcome* on
    /// this rail is steered by the amount instead (see [`Outcome::amount`]),
    /// because the return URL never reaches the status query.
    ReturnUrl(&'static str),
}

/// One row of the outcome table: a whole payment, from create to the webhook
/// it produced, and what must be true at each point.
///
/// A table rather than six functions because every row runs the *identical*
/// code path ([`run_outcome`]) and differs only in these values. Six
/// hand-written walk-throughs would let one of them quietly stop asserting
/// what the others do.
struct Outcome {
    /// What a human should understand happened, in a payer's terms.
    label: &'static str,
    /// The rail. Also the intent's single `payment_method_types` entry.
    rail: PaymentMethodType,
    /// Lowercase, as `/v1` wants it. **Per rail, not per taste**: `/v1`
    /// refuses a confirm whose intent currency is not the chosen rail's,
    /// before any charge exists (`vpay_api`'s `currencies_agree`).
    ///
    /// Every row below is `"xaf"`, because the overlay this demo runs against
    /// — the one `just gen-demo-keys` writes — settles **both** rails in XAF
    /// since 2026-09-04 (Step 9). It did not always: `config/application.yml`
    /// puts `mtn_momo` on EUR and still does, because **MTN's real sandbox
    /// rejects XAF** (`docs/flows/money.md`). What this stack talks to is a
    /// WireMock host that matches on no currency at all, and the demo shop
    /// beside this walkthrough prices its catalogue in XAF and offers a payer
    /// both rails — which one currency for both rails is the only way to
    /// make payable. The field stays per-row rather than becoming a constant
    /// precisely so a future deployment that splits them again has somewhere
    /// to say so.
    currency: &'static str,
    /// Minor units (`docs/flows/money.md`). On Orange this is also the field
    /// the stub selects the outcome on — see [`Outcome::selected_by`].
    amount: i64,
    /// What the confirm carries.
    steering: Steering,
    /// The sentence printed above the payment saying what makes this outcome
    /// happen, so a reader can go and check the mapping rather than trust
    /// the demo.
    selected_by: &'static str,
    /// What `confirm` must answer. `processing` on a push rail (the handset
    /// is prompting), `requires_action` on a redirect rail (the payer has
    /// somewhere to go). Anything else fails the run.
    after_confirm: IntentStatus,
    /// Where the worker must leave it. There is no `failed` status: a
    /// rail-reported failure returns the intent to `requires_payment_method`
    /// carrying `last_payment_error` (`docs/flows/payment-lifecycle.md`).
    settled: IntentStatus,
    /// The `charges.failure_code` this outcome must produce, as it reaches a
    /// merchant on `last_payment_error.code` — `None` for a payment that
    /// succeeded. The closed vocabulary is `docs/flows/failures.md`.
    failure_code: Option<&'static str>,
    /// The event the settlement must have written, as the receiver records
    /// it. Asserted against the *verified* event's `type`, not against the
    /// journal's raw text.
    event_type: &'static str,
}

/// Where a redirect rail sends the payer back to. Never fetched by anything:
/// it is handed to the rail, stored on the charge, and echoed to the merchant
/// as `next_action.redirect_to_url.return_url`. `example` is reserved for
/// documentation (RFC 2606), so this cannot resolve to anyone's host.
const DEMO_RETURN_URL: &str = "https://shop.example/orders/demo-1234/return";

/// The six payments this demo makes: **both rails, every outcome each rail
/// documents**.
///
/// The MTN rows come first and the order within a rail does not matter, but
/// the *sequencing* does — see this module's header and the mapping files.
///
/// # Why MTN's expiry is not spelled `EXPIRED`
///
/// Because MTN does not spell it that way. `EXPIRED` is Orange's status
/// string; MTN's status vocabulary is `PENDING`/`SUCCESSFUL`/`FAILED`, and
/// the reason on a prompt nobody answered is `COULD_NOT_PERFORM_TRANSACTION`
/// (`vpay_adapter_mtn_momo::mapping::FAILURE_REASONS`). Both land on the same
/// core code, `payer_timeout`, which is what a merchant integrates against —
/// so the outcome is demonstrated on both rails and neither stub is made to
/// claim something about a rail nobody has called.
const OUTCOMES: [Outcome; 6] = [
    Outcome {
        label: "mtn_momo · the payer approves on their handset",
        rail: PaymentMethodType::MtnMomo,
        currency: "xaf",
        amount: 5000,
        steering: Steering::Msisdn("237600000ce0"),
        selected_by: "MSISDN 237600000ce0 enters the `mtn-e2e-poll` scenario \
                      (requesttopay-scenario.json): PENDING on the first status query, \
                      SUCCESSFUL on the second",
        after_confirm: IntentStatus::Processing,
        settled: IntentStatus::Succeeded,
        failure_code: None,
        event_type: "payment_intent.succeeded",
    },
    Outcome {
        label: "mtn_momo · the payer has no balance",
        rail: PaymentMethodType::MtnMomo,
        currency: "xaf",
        amount: 5000,
        steering: Steering::Msisdn("237600000f01"),
        selected_by: "MSISDN 237600000f01 arms the `mtn-demo-decline` scenario \
                      (demo-outcomes.json), which answers the next status query \
                      FAILED/NOT_ENOUGH_FUNDS",
        after_confirm: IntentStatus::Processing,
        settled: IntentStatus::RequiresPaymentMethod,
        failure_code: Some("insufficient_funds"),
        event_type: "payment_intent.payment_failed",
    },
    Outcome {
        label: "mtn_momo · the prompt expires unanswered",
        rail: PaymentMethodType::MtnMomo,
        currency: "xaf",
        amount: 5000,
        steering: Steering::Msisdn("237600000f02"),
        selected_by: "MSISDN 237600000f02 arms the `mtn-demo-expiry` scenario \
                      (demo-outcomes.json), which answers FAILED with the OBJECT-shaped \
                      reason COULD_NOT_PERFORM_TRANSACTION — MTN's ~5-minute PIN window",
        after_confirm: IntentStatus::Processing,
        settled: IntentStatus::RequiresPaymentMethod,
        failure_code: Some("payer_timeout"),
        event_type: "payment_intent.payment_failed",
    },
    Outcome {
        label: "orange_money · the payer completes the hosted page",
        rail: PaymentMethodType::OrangeMoney,
        currency: "xaf",
        amount: 5000,
        steering: Steering::ReturnUrl(DEMO_RETURN_URL),
        selected_by: "5000 XAF is claimed by no amount-keyed mapping, so the status query \
                      falls through to transactionstatus.json's catch-all SUCCESS",
        after_confirm: IntentStatus::RequiresAction,
        settled: IntentStatus::Succeeded,
        failure_code: None,
        event_type: "payment_intent.succeeded",
    },
    Outcome {
        label: "orange_money · the hosted page expires before the payer finishes",
        rail: PaymentMethodType::OrangeMoney,
        currency: "xaf",
        amount: 5001,
        steering: Steering::ReturnUrl(DEMO_RETURN_URL),
        selected_by: "5001 XAF selects demo-outcomes.json's EXPIRED mapping — the amount \
                      travels on Orange's status body, so no scenario is needed",
        after_confirm: IntentStatus::RequiresAction,
        settled: IntentStatus::RequiresPaymentMethod,
        failure_code: Some("payer_timeout"),
        event_type: "payment_intent.payment_failed",
    },
    Outcome {
        label: "orange_money · the rail refuses, and documents no reason for it",
        rail: PaymentMethodType::OrangeMoney,
        currency: "xaf",
        amount: 5002,
        steering: Steering::ReturnUrl(DEMO_RETURN_URL),
        selected_by: "5002 XAF selects demo-outcomes.json's FAILED mapping. Orange \
                      documents no sub-reason vocabulary for FAILED, so the adapter \
                      refuses to guess: `provider_error` carrying the raw text",
        after_confirm: IntentStatus::RequiresAction,
        settled: IntentStatus::RequiresPaymentMethod,
        failure_code: Some("provider_error"),
        event_type: "payment_intent.payment_failed",
    },
];

/// How long an outcome waits for the worker to settle its charge.
///
/// The poll ladder's rungs are 10 s, 20 s, 30 s … (`vpay_worker::poll_delay`)
/// and the settling MTN outcome is answered `PENDING` first, so its earliest
/// possible settlement is about thirty seconds after the confirm; every other
/// outcome is terminal on the first rung. This is a *ceiling* on a wait that
/// normally ends well before it — generous enough that a cold compose stack
/// does not fail the demo, tight enough that a worker which is not running
/// fails it in under two minutes with a message saying so.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(120);

/// How often an outcome asks. A merchant integration polls; this is what that
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
            println!(
                "✔ all five steps behaved as expected — {} payments on 2 rails, every one \
                 settled by the worker asking the rail and evidenced by a signed webhook, \
                 plus one hosted and one embedded Checkout Session a browser can open.",
                OUTCOMES.len(),
            );
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

    // One SDK client for the whole table, configured the way
    // `docs/flows/merchant-auth.md` tells a merchant to configure one: a base
    // URL and a credential. The issuer, the token endpoint and the `vpay:v1`
    // audience are the SDK's own derivations, not values handed to it here —
    // which is what makes this a test of that derivation.
    let client = Client::builder(&base_url)
        .credentials(credentials(&client_id, &pem).context("step 4 (the outcome table)")?)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("step 4 (the outcome table): building the SDK client")?;

    run_outcomes(&client, &http, &receiver_url).await?;
    step_5_checkout_sessions(&client).await
}

/// A value unique to this run, which every `POST` below derives its
/// `Idempotency-Key`s from.
///
/// # Why not a fixed string
///
/// It was one, and that was a bug the moment the confirm started succeeding.
/// The keys are kept for 24 hours (`docs/api/README.md`), and `just demo`
/// does not tear the database down between runs — so a second run under a
/// fixed key *replayed* the create's stored response, which says
/// `requires_payment_method`, while the retrieve that followed it read the
/// row the confirm had since moved on. The demo then failed with "the
/// create's response and the stored row disagree", which was true and was
/// entirely the demo's own doing.
///
/// A per-run key makes each run a new set of payments, which is what a
/// merchant running this twice means. Wall-clock nanoseconds rather than a
/// UUID because that would be a dependency for one line; the value only has
/// to differ between runs on one machine, and it is never a security token.
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

    println!("[1/5] discovery + JWKS");

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
    println!("[2/5] access token (client_credentials + private_key_jwt)");

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
    println!("[3/5] the same path with no bearer token");

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

/// Drives every row of [`OUTCOMES`], one at a time, and prints a summary.
///
/// **Sequential on purpose, and not merely for readable output.** The MTN
/// stub's decline and expiry mappings are armed by a submit and answer the
/// next status query *whatever reference it carries* — the only steering a
/// push rail's `GET` status query allows (see this module's header and
/// `mtn/mappings/demo-outcomes.json`). Two MTN charges in flight against one
/// stub could therefore be answered the wrong way round, and the demo would
/// report the wrong outcome for both. Each row finishes — settled and its
/// webhook verified — before the next confirm is sent.
async fn run_outcomes(
    client: &Client,
    http: &reqwest::Client,
    receiver_url: &str,
) -> anyhow::Result<()> {
    let run = run_id()?;
    let total = OUTCOMES.len();

    println!();
    println!("[4/5] {total} payments, on both rails, to every outcome each rail documents");
    println!(
        "      each one: create → retrieve → confirm → the worker settles it → the signed \
         webhook it produced"
    );

    let mut settled = Vec::with_capacity(total);
    for (index, outcome) in OUTCOMES.iter().enumerate() {
        let intent = run_outcome(client, http, receiver_url, &run, index, outcome).await?;
        settled.push((outcome, intent));
    }

    println!();
    println!("      what just happened, in one table:");
    // The widths are the widest value each column can actually hold, not a
    // guess: `orange_money` is 12, and an intent id is `pi_` plus a 24-char
    // ULID-ish suffix. A column narrower than its content does not truncate
    // in Rust's formatter — it overflows and pushes every later column out of
    // line, which is what this table did until 2026-09-03.
    println!(
        "        {:<3} {:<12} {:<27} {:<24} failure_code",
        "#", "rail", "intent", "status"
    );
    for (index, (outcome, intent)) in settled.iter().enumerate() {
        println!(
            "        {:<3} {:<12} {:<27} {:<24} {}",
            index + 1,
            outcome.rail.as_wire_str(),
            intent.id,
            status_label(intent.status),
            intent
                .last_payment_error
                .as_ref()
                .map_or("—", |error| error.code.as_str()),
        );
    }

    println!();
    println!("      {CALLBACK_NOT_EXERCISED}");

    Ok(())
}

/// One payment, from create to the webhook it produced.
///
/// Returns the settled intent so [`run_outcomes`] can print the summary from
/// the objects it actually observed rather than from [`OUTCOMES`]'s
/// expectations — a table printed from what was *expected* would say the same
/// thing whether or not the run had gone that way.
async fn run_outcome(
    client: &Client,
    http: &reqwest::Client,
    receiver_url: &str,
    run: &str,
    index: usize,
    outcome: &Outcome,
) -> anyhow::Result<PaymentIntent> {
    println!();
    println!(
        "  ── {}/{}  {} ─────────────────────────",
        index + 1,
        OUTCOMES.len(),
        outcome.label
    );
    println!("     selected by: {}", outcome.selected_by);

    let key = format!("{run}-{index}");
    let created = create_and_retrieve(client, outcome, &key).await?;
    let confirmed = confirm(client, outcome, &created.id, &key).await?;
    let settled = await_settlement(client, outcome, &confirmed.id).await?;
    webhook(http, receiver_url, outcome, &settled.id).await?;

    Ok(settled)
}

/// Creates the intent through the SDK and reads it back.
///
/// This is a real write to a real database: the row exists after this, filed
/// under the demo merchant's tenant, and `just demo-down` is what throws it
/// away. The retrieve is not decoration — it is what proves the create
/// *persisted* rather than merely rendered an object, and comparing the two
/// is what would catch a retrieve that answered from somewhere else.
async fn create_and_retrieve(
    client: &Client,
    outcome: &Outcome,
    key: &str,
) -> anyhow::Result<PaymentIntent> {
    let step = format!("{} — create + retrieve", outcome.label);

    let params = CreatePaymentIntentParams {
        amount: outcome.amount,
        currency: outcome.currency.to_owned(),
        payment_method_types: vec![outcome.rail],
        metadata: BTreeMap::from([("order_id".to_owned(), format!("demo-{key}"))]),
        description: Some(format!("merchant-demo · {}", outcome.label)),
    };

    let created = client
        .payment_intents()
        .create(
            params,
            RequestOptions::new().with_idempotency_key(format!("{key}-create")),
        )
        .await
        .map_err(|error| describe(&step, "creating a payment intent", &error))?;

    println!("     ✔ POST /v1/payment_intents");
    print_intent(&created, "       ");

    let retrieved = client
        .payment_intents()
        .retrieve(&created.id)
        .await
        .map_err(|error| describe(&step, "retrieving the intent just created", &error))?;

    if retrieved != created {
        bail!(
            "{step}: the retrieve returned a different object than the create did. The create's \
             response and the stored row disagree, which means one of them is not what it says \
             it is."
        );
    }
    println!(
        "     ✔ GET /v1/payment_intents/{} — identical object",
        created.id
    );

    Ok(created)
}

/// Confirms the intent against its rail and asserts the flow's one success
/// state.
///
/// The request is real all the way down: vpay resolves the adapter, commits
/// the charge row with the reference it will submit under, records the
/// attempt, calls the rail over HTTP against the WireMock host
/// `config/application.yml` names, and commits what came back before
/// answering (`docs/flows/crash-safety.md`).
///
/// # What the two success states mean, and do not
///
/// `processing` on a push rail means the rail has the request and the payer's
/// handset should be prompting. `requires_action` on a redirect rail means
/// the rail minted a hosted page and vpay has **already committed** its URL
/// and key material — "the commit is the gate on the redirect". Neither means
/// money moved: only an authenticated `query_status` can say that, which is
/// what [`await_settlement`] waits for.
///
/// # Why no charge id is printed
///
/// There is none on the wire. `/v1` exposes payment intents, not charges —
/// there is no `/v1/charges` and the `payment_intent` object carries no
/// charge id (`docs/api/README.md`) — so a demo that printed one would have
/// had to read the database behind the API it is demonstrating.
async fn confirm(
    client: &Client,
    outcome: &Outcome,
    intent_id: &str,
    key: &str,
) -> anyhow::Result<PaymentIntent> {
    let step = format!("{} — confirm", outcome.label);

    let params = match outcome.steering {
        Steering::Msisdn(msisdn) => ConfirmPaymentIntentParams::mtn_momo(msisdn),
        Steering::ReturnUrl(return_url) => ConfirmPaymentIntentParams::orange_money(return_url),
    };

    let confirmed = client
        .payment_intents()
        .confirm(
            intent_id,
            params,
            RequestOptions::new().with_idempotency_key(format!("{key}-confirm")),
        )
        .await
        .map_err(|error| describe(&step, "confirming the payment intent", &error))?;

    if confirmed.status != outcome.after_confirm {
        bail!(
            "{step}: the confirm answered `{}` rather than `{}`. Each flow has exactly one \
             success state (docs/flows/payment-lifecycle.md); anything else means the response \
             and the stored row disagree about what happened.{}",
            status_label(confirmed.status),
            status_label(outcome.after_confirm),
            confirmed
                .last_payment_error
                .as_ref()
                .map_or_else(String::new, |error| format!(
                    " The rail refused it at submit: {} ({}).",
                    error.code, error.message
                )),
        );
    }

    println!(
        "     ✔ POST /v1/payment_intents/{intent_id}/confirm — HTTP 200, the rail accepted the \
         charge"
    );
    print_intent(&confirmed, "       ");

    match (&confirmed.next_action, outcome.after_confirm) {
        (None, IntentStatus::Processing) => {
            println!(
                "       next_action    null   (a push rail prompts the handset; there is \
                 nothing for a browser to do)"
            );
        }
        (Some(NextAction::RedirectToUrl { redirect_to_url }), IntentStatus::RequiresAction) => {
            println!("       next_action    redirect_to_url — send the payer here:");
            println!("         url          {}", redirect_to_url.url);
            println!(
                "         return_url   {}",
                redirect_to_url.return_url.as_deref().unwrap_or("(none)")
            );
            println!(
                "       (this demo does NOT open that URL. The rail stub answers the status \
                 query as though the payer had completed the page — the browser return trip is \
                 a named gap, docs/runbooks/demo.md)"
            );
        }
        (Some(_), IntentStatus::Processing) => bail!(
            "{step}: a push rail returned a next_action. There is nothing for a browser to do \
             while a payer types a PIN into their own handset, so a redirect here would be \
             pointing them somewhere invented."
        ),
        (None, IntentStatus::RequiresAction) => bail!(
            "{step}: the intent is `requires_action` and carries no next_action, so the payer \
             has nowhere to go. The charge's redirect_url is committed before this response is \
             built (docs/flows/crash-safety.md), so this cannot be a race — it is a defect."
        ),
        (_, other) => bail!(
            "{step}: this demo has no expectation for a confirm that lands in `{}`",
            status_label(other)
        ),
    }

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
        .map_err(|error| describe(&step, "re-reading the intent after the confirm", &error))?;
    let mut after_without_secret = after.clone();
    after_without_secret.client_secret = None;
    if after_without_secret != confirmed {
        bail!(
            "{step}: the confirm's response and a later retrieve are different objects (apart \
             from client_secret, which is expected to differ). One of them is not what the \
             database holds."
        );
    }
    println!(
        "     ✔ GET /v1/payment_intents/{intent_id} — identical object, so the `{}` a merchant \
         was told is the `{}` vpay stored",
        status_label(after.status),
        status_label(after.status),
    );

    Ok(confirmed)
}

/// Waits for `vpay-worker` to drive the charge to a terminal state, polling
/// the merchant API exactly as a merchant integration would, and asserts the
/// outcome the stub was steered to.
///
/// # What is actually happening while this loop waits
///
/// Nothing in this process. The work is in the `vpay-worker` container: it
/// claimed the `poll_charge` job the confirm committed *in the same
/// transaction as the charge*, asked the rail over HTTP, and either put the
/// job back on the ladder or committed the charge, the intent and one event
/// together.
///
/// # Why a merchant polls at all, when the next step shows a webhook
///
/// Because a poll is the fallback a merchant is told to keep
/// (`docs/api/README.md`): a delivery can be missed, and `GET
/// /v1/payment_intents/{id}` is the authoritative answer that cannot be. The
/// two observations are of the *same* settlement by the two routes a merchant
/// has — this one through the API, [`webhook`] through the event the same
/// transaction wrote.
async fn await_settlement(
    client: &Client,
    outcome: &Outcome,
    intent_id: &str,
) -> anyhow::Result<PaymentIntent> {
    let step = format!("{} — settlement", outcome.label);

    println!(
        "     … polling until it leaves `{}` (the worker is asking the rail; the ladder's \
         first rung is 10s)",
        status_label(outcome.after_confirm)
    );

    let deadline = std::time::Instant::now() + SETTLE_TIMEOUT;
    let mut polls = 0_u32;
    let settled = loop {
        let intent = client
            .payment_intents()
            .retrieve(intent_id)
            .await
            .map_err(|error| describe(&step, "re-reading the intent while it settles", &error))?;
        polls += 1;
        if intent.status != outcome.after_confirm {
            break intent;
        }
        if std::time::Instant::now() >= deadline {
            bail!(
                "{step}: the intent was still `{}` after {SETTLE_TIMEOUT:?} and {polls} polls. \
                 Nothing drove the charge to a terminal state — the usual cause is that \
                 vpay-worker is not running or cannot reach the rail. Try \
                 `docker compose logs vpay-worker`.",
                status_label(outcome.after_confirm),
            );
        }
        tokio::time::sleep(SETTLE_POLL_INTERVAL).await;
    };

    if settled.status != outcome.settled {
        bail!(
            "{step}: the charge resolved to `{}` rather than `{}`{}. {} — so anything else \
             means the stub, the mapping or the settlement path changed.",
            status_label(settled.status),
            status_label(outcome.settled),
            settled
                .last_payment_error
                .as_ref()
                .map_or_else(String::new, |error| format!(
                    " ({}: {})",
                    error.code, error.message
                )),
            outcome.selected_by,
        );
    }

    // The failure code is the merchant-visible half of `charges.failure_code`
    // — the closed vocabulary of docs/flows/failures.md, stamped onto the
    // intent by the settlement transaction. Asserting the *code* and not just
    // "it failed" is the difference between a demo that shows a decline and
    // one that shows the adapter's mapping table working.
    let observed = settled
        .last_payment_error
        .as_ref()
        .map(|error| error.code.as_str());
    if observed != outcome.failure_code {
        bail!(
            "{step}: expected last_payment_error.code = {:?}, got {observed:?}. The taxonomy code \
             is what a merchant integrates against (docs/flows/failures.md); a charge that \
             failed for the wrong stated reason is worse than one that failed loudly.",
            outcome.failure_code,
        );
    }

    println!("     ✔ settled after {polls} polls — the rail was asked, and answered");
    print_intent(&settled, "       ");
    match &settled.last_payment_error {
        Some(error) => {
            println!(
                "       failure_code   {}   (charges.failure_code, the closed vocabulary of \
                 docs/flows/failures.md)",
                error.code
            );
            println!("       message        {}", error.message);
            println!(
                "       the rail's own raw words are in charges.failure_raw and in the \
                 worker's log; only the taxonomy code and this generic message are public"
            );
        }
        None => {
            // Named, not printed, because it is genuinely not on the wire —
            // see this file's header. A demo that invented a number here, or
            // that read the database behind the API it is demonstrating,
            // would be worse than one that says what is missing.
            println!(
                "       amount_received  not on the wire — the settlement transaction writes \
                 payment_intents.amount_received (= amount, {}), but the payment_intent object \
                 does not carry it yet",
                settled.amount
            );
        }
    }

    Ok(settled)
}

/// Reads the webhook the settlement produced out of the receiver's own
/// request journal, and verifies it with the shipping SDK.
///
/// # Why the receiver's journal, and not vpay's own tables
///
/// Because the merchant-side view is the only one that can prove delivery.
/// `webhook_deliveries.state = 'succeeded'` is vpay's belief about what it
/// sent; `GET /__admin/requests` is what a receiver actually got, headers
/// and body, byte for byte. A demo that read the first would be quoting the
/// sender back to itself.
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
/// mirrors would verify in the SDK and fail in their code. And the verified
/// event's `type` is asserted against [`Outcome::event_type`], so a run in
/// which every payment was delivered as `payment_intent.succeeded` could not
/// pass.
async fn webhook(
    http: &reqwest::Client,
    receiver_url: &str,
    outcome: &Outcome,
    intent_id: &str,
) -> anyhow::Result<()> {
    let step = format!("{} — webhook", outcome.label);

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
            "{step}: no webhook was delivered for {intent_id} within {}s ({reason}). The \
             settlement was already observed through the API, so the `events` row exists and \
             this is the fan-out or the delivery failing, not a missing producer. Try \
             `docker compose logs vpay-worker`, and check that {receiver_url} is the port \
             `just demo` published wiremock-webhook on (VPAY_RECEIVER_URL).",
            RECEIVER_WAIT.as_secs()
        );
    };

    if delivery.stripe_signature != delivery.signature {
        bail!(
            "{step}: the delivery's Stripe-Signature and Vpay-Signature differ. They are the \
             same bytes by construction (vpay_worker::webhooks) so a merchant can keep a \
             Stripe-shaped handler; a drift here verifies in our SDK and fails in theirs."
        );
    }

    let secret = env_or("MERCHANT_WEBHOOK_SECRET", DEFAULT_WEBHOOK_SECRET);
    let event = vpay_sdk::webhooks::verify(
        delivery.body.as_bytes(),
        &delivery.signature,
        &secret,
        vpay_sdk::webhooks::DEFAULT_TOLERANCE,
    )
    .map_err(|error| {
        anyhow!(
            "{step}: a webhook arrived and its Vpay-Signature does not verify ({error}). vpay \
             signed something this merchant cannot check, which is worse than sending nothing. \
             The secret came from MERCHANT_WEBHOOK_SECRET (or the compose stub); check it \
             matches `webhooks[0].secrets` in .e2e/application-demo.yml."
        )
    })?;

    if event.kind != outcome.event_type {
        bail!(
            "{step}: the delivered event is `{}`, not `{}`. The event type is what a merchant's \
             handler branches on — a failed payment delivered as a success is the worst \
             possible defect in this system.",
            event.kind,
            outcome.event_type,
        );
    }

    println!(
        "     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk \
         (Stripe-Signature is byte-identical)"
    );
    println!("       event.id       {}", event.id);
    println!("       event.type     {}", event.kind);
    println!("       livemode       {}", event.livemode);
    println!(
        "       data.object.id {}",
        event
            .data
            .object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("(absent)")
    );
    println!(
        "       data.object.status {}",
        event
            .data
            .object
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("(absent)")
    );

    Ok(())
}

/// One delivery as the receiver recorded it.
///
/// A struct rather than a tuple because the two signature headers are the
/// same string and a tuple would let a careless caller compare a value with
/// itself and call it a check.
struct Delivered {
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
/// the container and holds every earlier outcome's delivery as well as every
/// previous run's — without it, outcome 2 would happily pass on outcome 1's
/// webhook, which is exactly the false green this repository is written
/// against.
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
        let (Some(_event_id), Some(signature), Some(stripe_signature)) = (
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
            body: body.to_owned(),
            signature: signature.to_owned(),
            stripe_signature: stripe_signature.to_owned(),
        }));
    }
    Ok(None)
}

// ------------------------------------------------------------------ step 5

/// The rails a session's intent is created with — both, because vpay's page
/// renders a rail selector and refusing to show one of them here would make
/// this demo's session narrower than the shop's.
const SESSION_RAILS: [PaymentMethodType; 2] =
    [PaymentMethodType::MtnMomo, PaymentMethodType::OrangeMoney];

/// XAF on both rails, which is what the overlay `just gen-demo-keys` writes
/// settles. See [`Outcome::currency`] for why, and for what
/// `config/application.yml` still says.
const SESSION_CURRENCY: &str = "xaf";

/// 5 000 FCFA. XAF is zero-decimal, so this integer is the whole price.
const SESSION_AMOUNT: i64 = 5_000;

/// Where a hosted session forwards a payer afterwards.
///
/// `{CHECKOUT_SESSION_ID}` is a literal template placeholder vpay substitutes
/// when it forwards (D5) — it is NOT a value this program fills in, and the
/// point of printing it unsubstituted is that a reader can see vpay do the
/// substitution in their browser's address bar. `example` is reserved for
/// documentation (RFC 2606), so neither URL can resolve to anyone's host.
const SESSION_SUCCESS_URL: &str = "https://shop.example/orders/demo/paid?cs={CHECKOUT_SESSION_ID}";
const SESSION_CANCEL_URL: &str = "https://shop.example/orders/demo/cancelled";
/// Where an embedded session's framed page forwards the payer at the end.
const SESSION_RETURN_URL: &str = "https://shop.example/orders/demo/return?cs={CHECKOUT_SESSION_ID}";

/// Creates one hosted and one embedded Checkout Session, on a fresh
/// PaymentIntent each, and prints what a merchant does with them.
///
/// # What this step proves, and what it deliberately does not
///
/// It proves that `POST /v1/checkout/sessions` answers — with a `url` for a
/// hosted session and with the `client_secret` an embedded one needs — for a
/// real intent this program has just created, against a running server, with
/// a merchant token this program minted. That is a genuine end-to-end
/// exercise of lane 1's route through the shipping Rust SDK.
///
/// It proves **nothing about the page**. This program has no browser. Nobody
/// has clicked the URL it prints, no rail has been asked for anything on
/// behalf of these two sessions, and both intents are still
/// `requires_payment_method` when this step returns. The browser proof is
/// `frontends/tests/e2e` and a human following `docs/runbooks/demo.md` §7;
/// this step's job is to hand that human a URL that works.
///
/// # Why a fresh intent each, and not one of step 4's
///
/// `checkout_sessions.payment_intent_id` is unique (migration `0028`: one open
/// session per intent), and every intent step 4 leaves behind already carries
/// a charge. `POST /v1/checkout/sessions` requires an intent in
/// `requires_payment_method` with no charge, so reusing one would be a `400`
/// this step could not distinguish from a real defect.
async fn step_5_checkout_sessions(client: &Client) -> anyhow::Result<()> {
    const STEP: &str = "step 5 (checkout sessions)";
    let run = run_id()?;

    println!();
    println!("[5/5] one hosted and one embedded Checkout Session (Step 9, D1/D6)");
    println!(
        "      each on its own fresh PaymentIntent: a session requires one in \
         requires_payment_method with no charge, and every intent above has a charge"
    );

    let hosted = create_session(
        client,
        &run,
        "hosted",
        CheckoutUiMode::Hosted,
        |payment_intent| CreateCheckoutSessionParams {
            payment_intent,
            ui_mode: Some(CheckoutUiMode::Hosted),
            success_url: Some(SESSION_SUCCESS_URL.to_owned()),
            cancel_url: Some(SESSION_CANCEL_URL.to_owned()),
            return_url: None,
        },
    )
    .await?;

    let url = hosted.url.as_deref().ok_or_else(|| {
        anyhow!(
            "{STEP}: a hosted session answered no `url`. That is the one field hosted mode \
             exists to produce, and without `checkout.public_base_url` in the loaded config the \
             route answers `checkout_not_configured` instead — check the `checkout:` block in \
             .e2e/application-demo.yml (`just gen-demo-keys` writes it)."
        )
    })?;

    println!();
    println!("      HOSTED — open this in a browser:");
    println!();
    println!("        {url}");
    println!();
    println!(
        "      That URL's #fragment IS the session's client_secret (D6). It is printed \
         here in full and NOWHERE else — it is not logged, and the SDK's own Debug for \
         CheckoutSession redacts it (`{:?}`).",
        RedactedUrl(url)
    );
    println!(
        "      A fragment never leaves the browser: it is not sent to a server, not written \
         to an access log, and not carried across the rail's redirect — which is why the \
         return page gets its own weaker `return_token` in a query string instead."
    );

    let embedded = create_session(
        client,
        &run,
        "embedded",
        CheckoutUiMode::Embedded,
        |payment_intent| CreateCheckoutSessionParams {
            payment_intent,
            ui_mode: Some(CheckoutUiMode::Embedded),
            success_url: None,
            cancel_url: None,
            return_url: Some(SESSION_RETURN_URL.to_owned()),
        },
    )
    .await?;

    let secret = embedded.client_secret.as_deref().ok_or_else(|| {
        anyhow!(
            "{STEP}: an embedded session answered no `client_secret`. That is the value \
             `initEmbeddedCheckout`'s `fetchClientSecret` must return, and an embedded session \
             without one cannot be mounted at all."
        )
    })?;

    println!();
    println!("      EMBEDDED — what a merchant's own page does with it:");
    println!();
    println!("        import {{ initEmbeddedCheckout }} from '@vaam-apps/vpay-stripe-js';");
    println!("        const checkout = await initEmbeddedCheckout({{");
    println!("          publishableKey: 'pk_test_demomerchantsandbox01',");
    println!(
        "          fetchClientSecret: async () => '{}',",
        redacted(secret)
    );
    println!("        }});");
    println!("        checkout.mount('#vpay-checkout');");
    println!();
    println!(
        "      The secret above is REDACTED on purpose — the same treatment step 2 gives the \
         access token. It is a live payer credential, this output ends up in CI logs and in \
         pasted terminal transcripts, and a demo that printed it would be teaching the habit."
    );
    println!(
        "      Read the real one with:  vpay.checkout().sessions().retrieve(\"{}\")",
        embedded.id
    );
    println!(
        "      It only mounts from an origin in this merchant's `checkout_origins` \
         (D4): vpay serves `Content-Security-Policy: frame-ancestors <that list>` on the \
         embedded page, and the page independently compares its own framer against the same \
         list. The second of those is the one a browser has been observed performing — see \
         docs/runbooks/checkout.md §5."
    );

    println!();
    println!("      what just happened, in one table:");
    println!(
        "        {:<10} {:<27} {:<27} {:<8} {:<14} url",
        "ui_mode", "session", "payment_intent", "status", "payment_status"
    );
    for session in [&hosted, &embedded] {
        println!(
            "        {:<10} {:<27} {:<27} {:<8} {:<14} {}",
            session.ui_mode.as_wire_str(),
            session.id,
            session.payment_intent,
            session_status_label(session.status),
            payment_status_label(session.payment_status),
            session
                .url
                .as_deref()
                .map_or("— (embedded sessions have none)", |_| "printed above"),
        );
    }

    println!();
    println!(
        "      NEITHER SESSION HAS BEEN PAID, and this program cannot pay one: both intents \
         are still requires_payment_method, no rail has been called for either, and no \
         browser has rendered either page. Open the hosted URL to change that."
    );

    Ok(())
}

/// Creates a fresh intent and one session on it.
///
/// The closure builds the params so the two call sites differ in exactly the
/// fields that make hosted and embedded different — `success_url`/`cancel_url`
/// against `return_url` — and in nothing else. `/v1` refuses the wrong pair
/// for a mode (`urls_match_ui_mode`, migration `0028`), so writing the two
/// out separately would be two places to get that wrong.
async fn create_session(
    client: &Client,
    run: &str,
    label: &str,
    mode: CheckoutUiMode,
    params: impl FnOnce(String) -> CreateCheckoutSessionParams,
) -> anyhow::Result<CheckoutSession> {
    let step = format!("step 5 (checkout sessions) — {label}");
    let key = format!("{run}-session-{label}");

    let intent = client
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount: SESSION_AMOUNT,
                currency: SESSION_CURRENCY.to_owned(),
                payment_method_types: SESSION_RAILS.to_vec(),
                metadata: BTreeMap::from([("order_id".to_owned(), key.clone())]),
                description: Some(format!("merchant-demo · {label} checkout session")),
            },
            RequestOptions::new().with_idempotency_key(format!("{key}-intent")),
        )
        .await
        .map_err(|error| describe(&step, "creating the session's payment intent", &error))?;

    println!();
    println!("      ✔ POST /v1/payment_intents   ({label})");
    print_intent(&intent, "        ");

    let session = client
        .checkout()
        .sessions()
        .create(
            params(intent.id.clone()),
            RequestOptions::new().with_idempotency_key(format!("{key}-session")),
        )
        .await
        .map_err(|error| describe(&step, "creating the checkout session", &error))?;

    println!("      ✔ POST /v1/checkout/sessions ({label})");

    // Read back, exactly as step 4's create does and for the same reason: it
    // is what proves the create PERSISTED rather than merely rendered an
    // object. `client_secret` is excluded from the comparison because
    // `retrieve` is documented to answer it too and a difference there would
    // be the interesting failure, not an expected one — so it is compared.
    let retrieved = client
        .checkout()
        .sessions()
        .retrieve(&session.id)
        .await
        .map_err(|error| describe(&step, "reading the session back", &error))?;
    if retrieved != session {
        bail!(
            "{step}: the create's response and the stored session disagree. Created \
             {session:?}, retrieved {retrieved:?}"
        );
    }
    println!(
        "      ✔ GET  /v1/checkout/sessions/{}  (identical)",
        session.id
    );

    if session.ui_mode != mode {
        bail!(
            "{step}: asked for ui_mode {} and got {}",
            mode.as_wire_str(),
            session.ui_mode.as_wire_str()
        );
    }

    Ok(session)
}

/// One `CheckoutSessionStatus` as its wire label.
///
/// Written out rather than `{:?}`-formatted, for the reason
/// [`status_label`] is: the wire spelling is lowercase, and a demo that
/// printed `Open` would be showing a Rust identifier where a reader is
/// trying to match a value they will see in an API response.
fn session_status_label(status: CheckoutSessionStatus) -> &'static str {
    match status {
        CheckoutSessionStatus::Open => "open",
        CheckoutSessionStatus::Complete => "complete",
        CheckoutSessionStatus::Expired => "expired",
    }
}

/// One `CheckoutPaymentStatus` as its wire label. Same reasoning.
fn payment_status_label(status: CheckoutPaymentStatus) -> &'static str {
    match status {
        CheckoutPaymentStatus::Unpaid => "unpaid",
        CheckoutPaymentStatus::Paid => "paid",
        CheckoutPaymentStatus::Failed => "failed",
    }
}

/// A credential rendered as its length and nothing else.
///
/// The same treatment step 2 gives the access token, and the same the Rust
/// SDK's own `Debug` for `CheckoutSession` gives `client_secret`. This
/// program's output is pasted into runbooks and captured by CI.
fn redacted(secret: &str) -> String {
    format!("[{} chars redacted]", secret.len())
}

/// A hosted `url` with its fragment replaced by a length marker, for the one
/// line that demonstrates the redaction rather than performing it.
///
/// A newtype with a `Debug` rather than a function, so the demonstration is
/// literally `{:?}` on a value — which is what a merchant's own logging would
/// do, and the thing this line is telling them is safe.
struct RedactedUrl<'a>(&'a str);

impl std::fmt::Debug for RedactedUrl<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.split_once('#') {
            Some((head, fragment)) => write!(f, "{head}#[{} chars redacted]", fragment.len()),
            None => write!(f, "{}", self.0),
        }
    }
}

// ------------------------------------------------------------------ helpers

/// The intent's public fields, as a merchant's own client sees them.
///
/// One function so every outcome prints the same fields in the same order: a
/// per-step `println!` block is how one of six payments quietly stops showing
/// the field that would have exposed a defect.
fn print_intent(intent: &PaymentIntent, indent: &str) {
    println!("{indent}id             {}", intent.id);
    println!("{indent}status         {}", status_label(intent.status));
    println!(
        "{indent}amount         {} {}   (integer minor units — docs/flows/money.md)",
        intent.amount,
        intent.currency.to_ascii_uppercase()
    );
    println!(
        "{indent}rails          {}",
        intent.payment_method_types.join(", ")
    );
    println!("{indent}livemode       {}", intent.livemode);
}

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
