//! Create a `PaymentIntent` and confirm it against a push rail — the whole
//! merchant happy path, end to end.
//!
//! Every call is signed with the merchant's Ed25519 key and every answer is
//! opened against the server keys the operator published; there is no token.
//! Run against a deployment whose operator registered your public key
//! (`merchant_clients[].cose_keys`) — the example prints it first so you can
//! hand it over.
//!
//! ```text
//! VPAY_BASE_URL=https://api.vpay.example \
//! VPAY_CLIENT_ID=merchant_acme \
//! VPAY_SIGNING_SEED_FILE=./merchant.seed \   # 32 bytes, unpadded base64url
//! VPAY_SERVER_KEY_CURRENT=... VPAY_SERVER_KEY_NEXT=... \
//! VPAY_AUDIENCE=https://api.vpay.example \    # optional: defaults to the base URL
//! VPAY_MSISDN=237670000000 \
//! cargo run -p vpay-sdk --example create_and_confirm
//! ```

// An example is a CLI: printing is its output.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeMap;
use std::process::ExitCode;

use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, PaymentMethodType,
};
use vpay_sdk::{Client, Credentials, Error, IntentStatus, RequestOptions, ServerKeys};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn env_var(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("missing required environment variable: {name}"))
}

async fn run() -> Result<(), String> {
    let base_url = env_var("VPAY_BASE_URL")?;
    let client_id = env_var("VPAY_CLIENT_ID")?;
    let seed_file = env_var("VPAY_SIGNING_SEED_FILE")?;
    let msisdn = env_var("VPAY_MSISDN")?;

    let seed = std::fs::read_to_string(&seed_file)
        .map_err(|e| format!("reading the signing seed at {seed_file}: {e}"))?;

    // The seed is read into a `Credentials` and never touched again — it is
    // not logged, and `Credentials`' `Debug` is written to keep it out of
    // `{:?}` (see `tests/debug_redaction.rs`).
    let credentials = Credentials::ed25519_seed_base64url(&client_id, &seed)
        .map_err(|e| format!("bad signing seed: {e}"))?;
    println!(
        "register this public key under cose_keys: {}",
        credentials.public_key_base64url()
    );
    let server_keys = ServerKeys::from_base64url(
        &env_var("VPAY_SERVER_KEY_CURRENT")?,
        &env_var("VPAY_SERVER_KEY_NEXT")?,
    )
    .map_err(|e| format!("bad server key: {e}"))?;

    let mut builder = Client::builder(&base_url)
        .credentials(credentials)
        .server_keys(server_keys);
    if let Ok(audience) = std::env::var("VPAY_AUDIENCE") {
        builder = builder.audience(audience);
    }
    let client = builder
        .build()
        .map_err(|e| format!("client configuration: {e}"))?;

    // 1. Create. `amount` is integer minor units — 5000 on a `xaf` intent is
    //    5,000 FCFA, because XAF is zero-decimal (`docs/flows/money.md`).
    let mut metadata = BTreeMap::new();
    metadata.insert("order_id".to_string(), "1234".to_string());

    let intent = client
        .payment_intents()
        .create(
            CreatePaymentIntentParams {
                amount: 5000,
                currency: "xaf".to_string(),
                payment_method_types: vec![PaymentMethodType::MtnMomo],
                metadata,
                description: Some("Example order".to_string()),
                // `..Default::default()` is deliberately not used here: this
                // example is the one a merchant copies, and spelling every
                // field is what makes a new one visible to them rather than
                // silently defaulted. `customer` is S4a's — attach a `cus_…`
                // and the payment is recorded against that payer.
                customer: None,
            },
            // Supplying the key explicitly ties it to the merchant's own
            // order id, so a retry of *this* example cannot double-create.
            // Omit it and the SDK generates a fresh UUIDv4 per call instead.
            RequestOptions::new().with_idempotency_key("order-1234-create"),
        )
        .await
        .map_err(describe)?;
    println!("created {} ({:?})", intent.id, intent.status);

    // 2. Confirm with the payer's number. On a push rail this prompts the
    //    payer's handset; the intent goes to `processing`, and nothing is
    //    settled until a status query says so (`docs/flows/payment-lifecycle.md`).
    let confirmed = client
        .payment_intents()
        .confirm(
            &intent.id,
            ConfirmPaymentIntentParams::mtn_momo(msisdn),
            RequestOptions::new().with_idempotency_key("order-1234-confirm"),
        )
        .await
        .map_err(describe)?;
    println!("confirmed {} -> {:?}", confirmed.id, confirmed.status);

    match confirmed.status {
        IntentStatus::Processing => {
            println!("the payer is being prompted; wait for a webhook rather than polling");
        }
        IntentStatus::RequiresAction => {
            // Redirect rails only; a push rail never populates `next_action`.
            if let Some(action) = &confirmed.next_action {
                println!("send the payer to: {action:?}");
            }
        }
        IntentStatus::RequiresPaymentMethod => {
            if let Some(error) = &confirmed.last_payment_error {
                println!("rail refused it: {} — {}", error.code, error.message);
            }
        }
        IntentStatus::Succeeded | IntentStatus::Canceled => {}
    }

    Ok(())
}

/// Turns an [`Error`] into the sentence a merchant's on-call would want,
/// naming which kind of failure it is — the distinction that
/// decides whether retrying is correct.
fn describe(error: Error) -> String {
    match &error {
        Error::Unsigned { .. } | Error::Unverified => {
            format!(
                "the signature exchange failed (do not retry; check the key and the clock): {error}"
            )
        }
        Error::Api { status, .. } => format!("vpay rejected the request ({status}): {error}"),
        Error::UnexpectedResponse { .. } => {
            format!("something other than vpay answered — check the URL: {error}")
        }
        Error::Transport(_) => format!("could not reach vpay: {error}"),
        other => other.to_string(),
    }
}
