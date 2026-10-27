# vpay-sdk

The Rust merchant SDK for vpay's `/v1` API. Implements the wire contract in
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md) — every
request sealed as a COSE_Sign1 message under your Ed25519 key, every answer
opened against the server keys you pinned, the same form-encoded requests and
JSON answers the SDK has always spoken, and outbound-webhook verification.
There are no access tokens.

Its sibling is [`sdks/nodejs`](../nodejs) (`@vaam-apps/vpay-sdk`). The two implement one
contract and are held to producing **byte-identical** request bodies; see
"Cross-SDK parity" below.

**This crate is `publish = false` and is not on crates.io.** Publishing a
client for an API nobody can reach would be actively misleading — see
"Status".

## Install

Inside this workspace it is a member crate:

```toml
[dependencies]
vpay-sdk = { path = "sdks/rust" }
```

```bash
cargo nextest run -p vpay-sdk    # or: just test-sdk-rust
```

Requires a `tokio` runtime (every request method is `async`) and Rust as
pinned in `rust-toolchain.toml`.

TLS is rustls with the `ring` provider and Mozilla's **vendored** root bundle
(`webpki-roots`), never `openssl`/`native-tls`
([ADR-0005](../../docs/adr/0005-rustls-only.md)). The `rustls::ClientConfig` is
built by this crate and handed to `reqwest`, rather than left to `reqwest` to
assemble — see `src/client.rs`'s `rustls_client_config` for why (in short:
under the only reqwest feature set that keeps the banned `aws-lc-rs` provider
out of the graph, `reqwest`'s own builder panics unless the _application_ has
installed a process-wide provider, and a library may neither panic in its
caller's process nor make that choice for it).

One consequence worth knowing before you deploy: because the roots are
vendored, a merchant behind a TLS-intercepting proxy with a private CA is not
trusted by this client. There is no setting for that today.

## The signed transport, in prose

`/v1` never accepts an API key
([ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md)) and, since
the signed-transport flag day (VP-3/VP-5), no bearer token from this SDK either
(a deployment may accept one from other callers, ADR-0037). A merchant
is a registered **Ed25519 public key** (`merchant_clients[].cose_keys` in
vpay's YAML); its server holds the matching seed. Concretely:

1. **Seal the call.** For each request the SDK builds a COSE_Sign1 message
   whose payload is the form body and whose signature binds the audience, the
   HTTP method, the route template and its path parameters, the canonical
   query, the digest of the operation (`src/contracts.rs`), the payload type
   and the `Idempotency-Key` exactly as sent, plus a fresh `cti` and `iat`.
   cratestack's `ClientEnvelope::seal_call` does the sealing; this crate sends
   the bytes with its own `reqwest`.
2. **Send it.** `Content-Type: application/cose`, the payload-type selectors
   (`application/x-www-form-urlencoded` in, `application/json` out), the bound
   `Idempotency-Key`, and `User-Agent: vpay-sdk-rust/<version>`. Every `POST`
   and `DELETE` carries an `Idempotency-Key` (yours, or a UUIDv4 the SDK
   generates). There is no `Authorization` header: vpay does not read one.
3. **Open the answer.** `PendingResponse::open` verifies the response under
   the pinned server keys `{current, next}` and its binding to _this_ request
   and status. A response that is not a COSE message is `Error::Unsigned`
   (whatever its status, and its body is never read); one that is but does
   not verify is `Error::Unverified`. Only a verified answer is decoded as the
   `Error::Api` envelope or the typed object.
4. **Nothing is retried.** A sealed request is single-use: vpay spends its
   `cti` when it opens it. The old "re-authenticate and send once more on a
   401" has no meaning — the 401 is the same bytes for an unknown key, a bad
   signature, a clock more than five minutes off, a wrong audience and a
   replay (cratestack ADR 0006 §10), and sending again spends another `cti`.
   `Error::Unsigned { status: 401 }` means: check the key registered under
   `cose_keys`, this host's clock, and the audience.

## Usage

```rust,no_run
use std::collections::BTreeMap;
use std::time::Duration;

use vpay_sdk::checkout::{
    CheckoutUiMode, CreateCheckoutSessionParams, ListCheckoutSessionsParams,
};
use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, PaymentMethodType,
};
use vpay_sdk::{Client, CreateRefundParams, Credentials, IntentStatus, ListEventsParams,
               NextAction, RequestOptions, ServerKeys};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
// 32 bytes, unpadded base64url — what `cargo xtask gen-envelope-key` writes.
let seed = std::fs::read_to_string("./merchant_a.seed")?;
// The two response keys vpay's operator published (the boot log of
// `vpay-server`): the one it seals with now, and the next.
let server_keys = ServerKeys::from_base64url(
    "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE",
    "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI",
)?;
let client = Client::builder("https://api.vpay.example")
    .credentials(Credentials::ed25519_seed_base64url("merchant_a", &seed)?)
    .server_keys(server_keys)
    .timeout(Duration::from_secs(30))
    .build()?;

let mut metadata = BTreeMap::new();
metadata.insert("order_id".to_string(), "1234".to_string());

// 5000 = 5,000 FCFA. Amounts are integer minor units and XAF is
// zero-decimal (docs/flows/money.md); the type is `i64`, so there is no
// float to round. A negative amount, or one past 2^53-1, is refused before
// any request — see "Cross-SDK parity".
let intent = client
    .payment_intents()
    .create(
        CreatePaymentIntentParams {
            amount: 5000,
            currency: "xaf".to_string(),
            payment_method_types: vec![PaymentMethodType::MtnMomo],
            metadata,
            description: Some("Order 1234".to_string()),
            // A `cus_…` from `client.customers()`, or `None`. The payment is
            // recorded against that payer, and the customer's twelve-month
            // retention clock is stamped by the create.
            customer: None,
        },
        RequestOptions::new().with_idempotency_key("order_1234_attempt_1"),
    )
    .await?;

// `intent.client_secret` — not to be confused with the OAuth
// `client_secret` this SDK never sends (see "The handshake, in prose") — is
// the payer credential `/v1/browser` accepts for a browser-side confirm via
// `@vaam-apps/vpay-stripe-js`. `create()` and `retrieve()` are the only calls that set
// it: a `list()` item and an `Event`'s payload always decode it as `None`,
// so it never reaches a merchant's listing view or a stored/forwarded
// webhook body. Hand it to your frontend; never log it — `PaymentIntent`'s
// `Debug` redacts it for exactly that reason.
let _client_secret: Option<String> = intent.client_secret.clone();

// Push rail: prompts the payer's handset. `ConfirmPaymentIntentParams` is
// one variant per rail — a push rail takes an msisdn and no return URL, a
// redirect rail the reverse — so the two cannot be mixed up:
//   ConfirmPaymentIntentParams::orange_money("https://m.example/return")
let confirmed = client
    .payment_intents()
    .confirm(
        &intent.id,
        ConfirmPaymentIntentParams::mtn_momo("237670000000"),
        RequestOptions::new(),
    )
    .await?;

// `Processing` means NOT YET. Wait for a `payment_intent.succeeded` webhook
// rather than treating a confirm response as settlement. There is no
// `failed` status — a rail refusal returns the intent to
// `RequiresPaymentMethod` with `last_payment_error` set.
match confirmed.status {
    IntentStatus::Processing => {}
    IntentStatus::RequiresAction => {
        // Redirect rail (Orange Money): send the payer to the URL.
        if let Some(NextAction::RedirectToUrl { redirect_to_url }) = &confirmed.next_action {
            println!("redirect to {}", redirect_to_url.url);
        }
    }
    IntentStatus::RequiresPaymentMethod => {
        if let Some(err) = &confirmed.last_payment_error {
            println!("refused: {} — {}", err.code, err.message);
        }
    }
    IntentStatus::Succeeded | IntentStatus::Canceled => {}
}

client.payment_intents().retrieve(&intent.id).await?;
client.payment_intents().cancel(&intent.id, RequestOptions::new()).await?;
client.payment_intents().list(Default::default()).await?;

client
    .refunds()
    .create(
        CreateRefundParams {
            payment_intent: intent.id.clone(),
            // Omit `amount` entirely for a full refund.
            reason: Some("requested_by_customer".to_string()),
            ..Default::default()
        },
        RequestOptions::new(),
    )
    .await?;

client
    .events()
    .list(ListEventsParams {
        limit: Some(20),
        event_type: Some("payment_intent.succeeded".to_string()),
        ..Default::default()
    })
    .await?;
client.balance().retrieve().await?;

// Checkout Sessions (Step 9). A session references an intent you already
// created; it never creates one, and it carries where the payer goes next.
let hosted = client
    .checkout()
    .sessions()
    .create(
        CreateCheckoutSessionParams {
            payment_intent: intent.id.clone(),
            ui_mode: Some(CheckoutUiMode::Hosted),
            // vpay substitutes {CHECKOUT_SESSION_ID} when it forwards the
            // payer, and appends nothing else. Omit it and the return has
            // no correlation.
            success_url: Some("https://shop.example/ok?sid={CHECKOUT_SESSION_ID}".to_string()),
            cancel_url: Some("https://shop.example/cancel".to_string()),
            ..Default::default()
        },
        RequestOptions::new(),
    )
    .await?;
// Redirect the payer to `hosted.url` — the session secret is in its
// fragment, which is why `{:?}` on this object redacts it.

client.checkout().sessions().retrieve(&hosted.id).await?;
client
    .checkout()
    .sessions()
    .list(ListCheckoutSessionsParams {
        payment_intent: Some(intent.id.clone()),
        ..Default::default()
    })
    .await?;
client
    .checkout()
    .sessions()
    .expire(&hosted.id, RequestOptions::new())
    .await?;
# Ok(())
# }
```

Runnable versions live in [`examples/`](examples):

| Example              | What it does                                                                                                                |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `create_and_confirm` | The flow above against a real deployment: prints your public key (to register), then creates and confirms a payment intent. |

## Configuration

`Client::builder(base_url)` plus these setters; `build()` validates and
returns `ConfigError` before anything touches the network.

| Setter             | Default  | Notes                                                                                                                                                                                                                                                           |
| ------------------ | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.credentials(..)` | —        | Required. `Credentials::ed25519_seed(client_id, &seed)?`, `ed25519_seed_base64url(client_id, text)`, or `ed25519_external(client_id, &public_key, sign)` for a key in a KMS. The `client_id` is **not sent**: vpay finds the merchant by the key that verified. |
| `.server_keys(..)` | —        | Required. `ServerKeys::new(current, next)` or `from_base64url(..)`: the response keys to trust. An answer under any other key is `Error::Unverified`. There is no default and no trust-on-first-use.                                                            |
| `.audience(..)`    | base URL | The name every request is addressed to: vpay's `deployment.public_base_url` without a trailing slash (`vpay_api::envelope::audience_for`). Set it when this process reaches vpay by another URL than payers do.                                                 |
| `.timeout(..)`     | 30 s     | Applies to every call.                                                                                                                                                                                                                                          |

### Reaching vpay by an internal URL

`base_url` is **where this process sends the HTTP request**; `.audience(..)` is
**what vpay calls itself**. They coincide only when you reach vpay at the URL it
publishes. A merchant server calling `http://vpay-server:8080` inside a compose
network, or a private DNS name in production, sets the audience to the public
one — the signature binds a configured _name_, never the `Host`, so that a
request signed for one deployment cannot be replayed at another that shares a
key. A mismatch has no symptom but the coarse `401`.

```rust
# use vpay_sdk::{Client, Credentials, ServerKeys};
# fn demo(credentials: Credentials, keys: ServerKeys) -> Result<(), vpay_sdk::ConfigError> {
let client = Client::builder("http://vpay-server:8080")
    .credentials(credentials)
    .server_keys(keys)
    .audience("https://api.vpay.example")
    .build()?;
# let _ = client;
# Ok(())
# }
```

`Credentials`, `Client` and `ClientBuilder` have hand-written `Debug`s that
print the `client_id` and the **public** key and nothing that could sign —
[`tests/debug_redaction.rs`](tests/debug_redaction.rs) fails if any is ever
replaced by a derive.

## Errors

Two types, deliberately:

- **`ConfigError`** — returned by `ClientBuilder::build` and the `Credentials` /
  `ServerKeys` constructors. Nothing has reached the wire yet, so it is not a
  variant of the network error enum; `Error: From<ConfigError>` for callers
  who want one `?`-able type.
- **`Error`** — everything that happens on, or instead of, the wire:

| Variant                     | When                                                                                                                                                                                                                  | Carries                                                              | `@vaam-apps/vpay-sdk` equivalent                               |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | -------------------------------------------------------------- |
| `Error::Api`                | A non-2xx `/v1` response shaped like `vpay_api::error_envelope`.                                                                                                                                                      | `status`, `kind` (the envelope's `type`), `code`, `message`, `param` | `VpayApiError`                                                 |
| `Error::Unsigned`           | The answer was not a COSE message: the envelope layer's bare `401` (unknown or disabled key, bad signature, clock, wrong audience, replay — one answer for all), a `426`, or a proxy's `502`. Its body is never read. | `status`                                                             | `VpayUnsignedError`                                            |
| `Error::Unverified`         | A COSE answer that did not verify under any pinned server key: a rotation outran this client's `ServerKeys`, or something other than vpay answered.                                                                   | —                                                                    | `VpayUnverifiedError`                                          |
| `Error::Seal`               | The request could not be sealed (a KMS callback failed). Nothing was sent.                                                                                                                                            | the message                                                          | `VpaySealError`                                                |
| `Error::UnexpectedResponse` | A response that is not that envelope — a proxy's HTML 502, an empty body, a success body that will not decode.                                                                                                        | `status`, `body_prefix` (bounded to 500 bytes)                       | `VpayUnexpectedResponseError`                                  |
| `Error::Transport`          | DNS, TLS, timeout, connection refused — no HTTP response to classify.                                                                                                                                                 | the underlying message                                               | `VpayTransportError`                                           |
| `Error::InvalidParams`      | A parameter this SDK refuses to send — an amount that is negative or past 2^53-1, or (since 2026-10-21) a path `id` that is empty or only dots. Nothing reaches the wire.                                             | `param`, `message`                                                   | `VpayInvalidParamsError` (a bare `TypeError` until 2026-10-21) |
| `Error::Config`             | A `ConfigError` surfaced through a request path.                                                                                                                                                                      | the `ConfigError`                                                    | `VpayConfigError`                                              |
| `Error::Webhook`            | `webhooks::verify` rejected a delivery.                                                                                                                                                                               | a `WebhookError`                                                     | `WebhookSignatureError`                                        |

The distinction is not cosmetic: retrying a `Transport` failure is correct and
retrying an `Api` `400` is a bug, and only a caller who can tell them apart
can get that right. Nothing is retried automatically: a sealed request is
single-use.

**`Error::Api` is one variant for every envelope**; the envelope's `type` is
carried in `kind` and is _not_ mapped to a variant of its own. Branch on
`code`, which is the field the server treats as the machine-readable answer.
The case where that matters today is idempotency, where two errors share a
status (`400`) and a `type` (`idempotency_error`) and mean opposite things:

```rust,ignore
match error {
    // The first request under this key has not finished. Wait and send the
    // same call again — do not mint a new key.
    vpay_sdk::Error::Api { ref code, .. }
        if code.as_deref() == Some("idempotency_key_in_flight") => retry_later(),
    // The key was already used with a *different* body. Retrying cannot
    // help; the caller has a bug.
    vpay_sdk::Error::Api { ref code, .. }
        if code.as_deref() == Some("idempotency_key_in_use") => give_up(),
    other => return Err(other),
}
```

## Webhook verification

```rust,no_run
use std::time::Duration;
use vpay_sdk::webhooks::{self, DEFAULT_TOLERANCE};

# fn handle(raw_body: &[u8], signature_header: &str, secret: &str)
#     -> Result<(), Box<dyn std::error::Error>> {
// The RAW request body must be used. A parsed-and-reserialised body breaks
// the HMAC — do not run a JSON body parser before this.
let event = webhooks::verify(raw_body, signature_header, secret, DEFAULT_TOLERANCE)?;

// Delivery is at-least-once and this verifier does NOT dedupe by event.id —
// that is your job. Keep a unique index on the ids you have processed.
match event.kind.as_str() {
    "payment_intent.succeeded" => {
        let intent = event.payment_intent()?;
        println!("{} succeeded", intent.id);
    }
    _ => {}
}
# let _ = Duration::from_secs(1);
# Ok(())
# }
```

`verify_at(.., now)` takes the clock as an argument, for tests that should not
race a real five-minute window. Both accept more than one `v1=` in the header
(so a secret rotation does not drop deliveries), compare in constant time via
`subtle`, and reject a malformed header, a timestamp outside the tolerance, or
a body whose signature does not match — in that order.

The header grammar is held byte-for-byte to `@vaam-apps/vpay-sdk`'s, because a delivery
one SDK accepts and the other rejects is a defect neither side can see alone:

- `t` must be a bare run of decimal digits. `+1753401600`, `-1`, `1753401600.0`
  and `0x65566CC0` are **malformed**, not "some other timestamp" — each of them
  parses under a looser rule, and the HMAC would then cover bytes the sender
  never signed.
- the HMAC covers the **literal** `t` text from the header, so `t=017…` hashes
  `"017…"` and a sender that writes leading zeros still verifies.
- a part with no `=`, an unknown `k=v`, and an empty `v1=` are each ignored, so
  a future scheme element cannot break today's verifier. A header whose only
  signature is an empty `v1=` carries no signature and is malformed.
- `|now - t| == tolerance` is **inside** the window; only strictly greater is
  rejected.

## Cross-SDK parity

The Rust and Node SDKs must put the _same bytes_ on the wire for the same
call. The form encoder here therefore reproduces JavaScript's
`encodeURIComponent` escaping rule exactly rather than RFC 3986's or the
WHATWG serializer's (see `src/form.rs`'s module documentation for why). The
same function escapes path ids, so `retrieve("pi_a/b")` addresses
`/v1/payment_intents/pi_a%2Fb` in both SDKs rather than a different route in
one of them. The expected strings in `src/form.rs`'s `node_parity` tests were
produced by running the Node SDK, not by reading it:

```console
$ node -e 'import("./dist/form.js").then(({encodeForm}) => console.log(encodeForm({
    amount: 5000, currency: "xaf",
    payment_method_types: ["mtn_momo","orange_money"],
    metadata: {order_id: "1234", note: "a b&c=d"},
    description: "Order #42 (rush)"})))'
amount=5000&currency=xaf&payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money&metadata[order_id]=1234&metadata[note]=a%20b%26c%3Dd&description=Order%20%2342%20(rush)
```

The sealed contract is shared the same way: `src/contracts.rs` is this crate's
copy of `vpay-wire`'s operation table and digest derivation, and the workspace
test `v1::contracts::tests::the_sdks_vendored_table_is_the_wire_crates_table`
(in `vpay-api`) fails when the two disagree. The Node SDK seals through a wasm
build of the same code (`sdks/rust/wasm`).

Where the two type systems land differently, and why:

| Thing                             | Rust                                                | Node                                                 | Why                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| --------------------------------- | --------------------------------------------------- | ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `amount`                          | `i64`, refused if `< 0` or `> 2^53-1`               | `number`, refused unless a non-negative safe integer | Rust could send an `i64` past `2^53-1` exactly; JavaScript could not represent it. Refusing it in both keeps one wire contract instead of two.                                                                                                                                                                                                                                                                                                                                                                          |
| `payment_method_types` (request)  | closed `PaymentMethodType`                          | closed `PaymentMethodType` union                     | Both close it: a request naming a rail that does not exist is a typo, not forward compatibility.                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `payment_method_types` (response) | `Vec<String>`                                       | `string[]`                                           | Both leave it open: a rail this SDK version predates must still decode.                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `confirm` params                  | two-variant enum + `mtn_momo()`/`orange_money()`    | discriminated union                                  | A push rail takes an msisdn, a redirect rail a `return_url`; neither takes both.                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `last_payment_error.code`         | **open `String`**                                   | closed `FailureCode` union                           | The deliberate divergence. The vocabulary is owned by `vpay_core::failure` and can grow a code this SDK predates; in Rust a closed enum would make such a response a **decode failure** for the whole `PaymentIntent`, and even a `#[serde(other)]` fallback would discard the original string. TypeScript's unions are erased at runtime, so Node's narrower type costs its callers nothing at decode time. Rust callers match on `&str` against the list in [`docs/flows/failures.md`](../../docs/flows/failures.md). |
| `event.type`                      | `Event::kind: String` (`#[serde(rename = "type")]`) | `type: string`                                       | Same reason; `type` is a Rust keyword, hence the field rename.                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| errors                            | one `enum Error`                                    | one class per case                                   | See the table under "Errors" for the variant-by-variant mapping.                                                                                                                                                                                                                                                                                                                                                                                                                                                        |

## Status

**Behaviour change, 2026-10-21: an empty or all-dots `id` is refused before
any request.** Every one of the twenty-one methods that put an `id` in the
path — `retrieve`, `update`, `del`, `confirm`, `cancel`, `expire`, `finalize`,
`void`, `mark_uncollectible`, `pay` — now returns `Error::InvalidParams` with
`param == "id"` for `""`, `.`, `..` or any longer run of dots, without
signing or sending anything. No signature changed: each already returned
`Result<_, Error>`, `Error` is `#[non_exhaustive]`, and `InvalidParams`
already existed. What changes is the outcome for those inputs. They used to
be sent, and the URL parser resolved them before the request left, so
`customers().del("..")` was a `DELETE /v1/` and
`payment_intents().retrieve("")` a `GET /v1/payment_intents/`. What vpay
answered for each is on
[the dated verification page](../../docs/status/verification/2026-10-21-sdk-dot-segment-ids.md).
A caller matching on `Error::Api { status: 404, .. }` for those inputs now
gets `InvalidParams`. `sdks/nodejs` refuses the same ids with a `TypeError`
(a `VpayInvalidParamsError` since later the same day, carrying `param` as
`InvalidParams` does).

**Breaking, 2026-09-23 (RFC-0004 § 5).** `ListPaymentIntentsParams`,
`ListCheckoutSessionsParams` and `ListRefundsParams` each gained a public
`customer: Option<String>`. Code that builds one of the three with a struct
literal naming every field and no `..Default::default()` no longer compiles
(`E0063`, missing field `customer`); add `customer: None` or
`..Default::default()`. `ListPaymentIntentsParams { .. }` literals were the
likeliest to break, because that struct had only three fields and naming
all three was natural. Code using `Default::default()`, field assignment
or `..Default::default()` is unaffected. The filter itself needs a vpay
server that has it (no release up to and including 0.5.0 does, as of
2026-09-23); an older server ignores it and answers the unfiltered list.

**The server side of this contract is built, and has run against Postgres, not
against a rail.** `vpay-server` mounts `/v1` behind the signed transport
(`vpay_api::envelope`, `EnvelopeMode::Required`): an unsigned request to any
operation is the layer's bare `401`, and every answer, errors included, is
sealed. What this crate's tests prove, and against what:

- **The SDK's requests are accepted by cratestack's real verifier.**
  [`tests/signing.rs`](tests/signing.rs) and the 90 byte-level cases of
  [`tests/resources.rs`](tests/resources.rs) run the SDK against the real
  `EnvelopeLayer` (`tests/support/front.rs`) in front of a `wiremock` upstream,
  so a bug in sealing or binding is refused by the verifier itself.
- **A response under an unpinned key, an unsigned response, a wrong key and a
  wrong audience are each refused**, and a refused call is never retried.
- **Against the real `vpay-api` router and Postgres**:
  `backends/tests/integration/tests/signed_transport.rs` and the `customers`
  suite. Against the compose stack: `tests/live_invoices.rs` and
  `tests/live_refunds.rs` (`just sdk-live`).
- **What is not proven:** that a real rail moves money. No rail has ever
  returned money to anyone (`docs/status.md`); see `refunds` above.

`BalanceResource::retrieve` is refused by the client itself: `GET /v1/balance`
is declared in the wire contract and mounted nowhere, so there is no operation
to sign for it.
