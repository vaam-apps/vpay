//! Create a `PaymentIntent` and confirm it against a push rail — the whole
//! merchant happy path, end to end.
//!
//! **No vpay serves `/v1` yet.** `vpay-server` answers `/healthz` and a
//! Stripe-shaped 404, and there is no OAuth2 token endpoint either
//! (`docs/status.md`, `docs/flows/merchant-auth.md`'s Status section). Run
//! against a real deployment this example will fail at step 1 — the token
//! exchange — with a transport error or a 404-shaped unexpected response.
//! It is here to show the API a merchant will write against, and to be the
//! thing that gets run first the day a token endpoint exists; it is not
//! evidence that any of this works against a server.
//!
//! ```text
//! VPAY_BASE_URL=https://api.vpay.example \
//! VPAY_CLIENT_ID=merchant_acme \
//! VPAY_PRIVATE_KEY_FILE=./merchant.pem \
//! VPAY_KID=key-1 \                      # only if several keys are registered
//! VPAY_MSISDN=237670000000 \
//! cargo run -p vpay-sdk --example create_and_confirm
//! ```

// An example is a CLI: printing is its output. See `verify_assertion.rs` for
// the same note.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeMap;
use std::process::ExitCode;

use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, PaymentMethodType,
};
use vpay_sdk::{Client, Credentials, Error, IntentStatus, RequestOptions};

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
    let key_file = env_var("VPAY_PRIVATE_KEY_FILE")?;
    let msisdn = env_var("VPAY_MSISDN")?;

    let pem = std::fs::read_to_string(&key_file)
        .map_err(|e| format!("reading the private key at {key_file}: {e}"))?;

    // The PEM is read into a `Credentials` and never touched again — it is
    // not logged, and `Credentials`' `Debug` is written to keep it out of
    // `{:?}` (see `tests/debug_redaction.rs`).
    let mut credentials =
        Credentials::rsa_pem(&client_id, &pem).map_err(|e| format!("bad private key: {e}"))?;
    if let Ok(kid) = std::env::var("VPAY_KID") {
        credentials = credentials.with_kid(kid);
    }

    let client = Client::builder(&base_url)
        .credentials(credentials)
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
/// naming which of the three failure kinds it is — the distinction that
/// decides whether retrying is correct.
fn describe(error: Error) -> String {
    match &error {
        Error::TokenEndpoint { .. } => {
            format!("authentication was refused (do not retry): {error}")
        }
        Error::Api { status, .. } => format!("vpay rejected the request ({status}): {error}"),
        Error::UnexpectedResponse { .. } => {
            format!("something other than vpay answered — check the URL: {error}")
        }
        Error::Transport(_) => format!("could not reach vpay: {error}"),
        other => other.to_string(),
    }
}
