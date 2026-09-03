# vpay-sdk

The Rust merchant SDK for vpay's `/v1` API. Implements the wire contract in
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md) exactly —
`private_key_jwt` client assertions, `client_credentials` token exchange and
caching, the form-encoded resource calls, and outbound-webhook verification.

Its sibling is [`sdks/nodejs`](../nodejs) (`@vpay/sdk`). The two implement one
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

## The handshake, in prose

`/v1` never accepts an API key
([ADR-0010](../../docs/adr/0010-merchant-auth-private-key-jwt.md)). Every
merchant is a statically registered OAuth2 client authenticating with
`client_credentials` (RFC 6749 §4.4) via a signed `private_key_jwt` assertion
(RFC 7523). Concretely:

1. **Mint an assertion.** The SDK signs a short-lived RS256 JWT with your
   private key: `iss`/`sub` are your `client_id`, `aud` is the token endpoint,
   `jti` is a fresh UUIDv4 (spent exactly once server-side — reusing one is
   indistinguishable from a replay), and `exp` is `now + assertion_lifetime`
   (default 60 s, hard-capped at 300 s because the OP refuses anything further
   out). A `kid` is stamped on only if you configured one; the OP requires it
   when you have registered more than one key and refuses to guess otherwise.
2. **Exchange it for an access token.** `POST` to the token endpoint with
   `grant_type=client_credentials`, `client_id`, `client_assertion_type`,
   `client_assertion` and `audience=vpay:v1`. No `client_secret` is ever
   sent — there is nothing to send; vpay stores only your **public** key, and
   the OP rejects a request that presents two authentication methods.
3. **Call `/v1` with the token.** Every request carries
   `Authorization: Bearer <token>`, `Accept: application/json` and
   `User-Agent: vpay-sdk-rust/<version>`; every `POST` also carries an
   `Idempotency-Key` (yours, or a UUIDv4 the SDK generates). The token is
   cached until `expires_in` minus a safety margin (30 s, or `expires_in / 2`
   for very short TTLs — integer arithmetic only), and concurrent callers
   share one in-flight refresh rather than each spending a `jti`.
4. **Re-authenticate on expiry or a `401`.** There is no refresh token, by
   design. On a `401` from a resource route the SDK discards the cached token,
   repeats steps 1–2 once, and retries the request exactly once; a second
   `401` is returned to you. A failure from the _token endpoint_ is never
   retried — it is a credential problem, and retrying it just spends another
   `jti`.

## Usage

```rust,no_run
use std::collections::BTreeMap;
use std::time::Duration;

use vpay_sdk::payment_intents::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, PaymentMethodType,
};
use vpay_sdk::{Client, CreateRefundParams, Credentials, IntentStatus, ListEventsParams,
               NextAction, RequestOptions};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let pem = std::fs::read_to_string("./merchant_a.key.pem")?;
let client = Client::builder("https://api.vpay.example")
    .credentials(Credentials::rsa_pem("merchant_a", &pem)?)
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
        },
        RequestOptions::new().with_idempotency_key("order_1234_attempt_1"),
    )
    .await?;

// `intent.client_secret` — not to be confused with the OAuth
// `client_secret` this SDK never sends (see "The handshake, in prose") — is
// the payer credential `/v1/browser` accepts for a browser-side confirm via
// `@vpay/stripe-js`. `create()` and `retrieve()` are the only calls that set
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
# Ok(())
# }
```

Runnable versions live in [`examples/`](examples):

| Example              | What it does                                                                                                                                     |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `create_and_confirm` | The flow above against a real deployment. **There is no deployment** — see the file's own header and "Status".                                   |
| `verify_assertion`   | Feeds an assertion to the real `authkestra_op` verifier. Backs `just sdk-conformance-node`, which checks the _Node_ SDK's assertions against it. |

## Configuration

`Client::builder(base_url)` plus these setters; `build()` validates and
returns `ConfigError` before anything touches the network.

| Setter                    | Default               | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------------------------- | --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.credentials(..)`        | —                     | Required. `Credentials::rsa_pem(client_id, pem)` (PKCS#1 or PKCS#8), then `.with_kid(..)` if you registered more than one key.                                                                                                                                                                                                                                                                                                                                   |
| `.issuer(..)`             | `{base_url}/v1/oauth` | This default is now what the server does: `vpay_api::op::issuer_for` builds the issuer as `{public_base_url}/v1/oauth` (from the deployment YAML), and `vpay_api::router` mounts the OP there. Override only if a deployment sits behind a path prefix.                                                                                                                                                                                                          |
| `.token_endpoint(..)`     | `{issuer}/token`      | The server's token route, and also the assertion's `aud` claim; overriding this moves the `aud` with it.                                                                                                                                                                                                                                                                                                                                                         |
| `.audience(..)`           | `vpay:v1`             | The OAuth2 `audience` request parameter. Load-bearing: without it the OP mints a token whose `aud` is the `client_id`, which every `/v1` route then rejects. Server-side the same string is `vpay_config::MERCHANT_AUDIENCE`, which both `Surface::Merchant::audience()` and each merchant's configured `allowed_audiences` check are derived from; this crate keeps its own copy so a merchant needs no vpay server crate, so the two must be changed together. |
| `.scope(..)`              | —                     | Omitted from the token request entirely unless set.                                                                                                                                                                                                                                                                                                                                                                                                              |
| `.assertion_lifetime(..)` | 60 s                  | Must be `1..=300` s; anything else is `ConfigError::InvalidAssertionLifetime` at `build()`, never silently clamped.                                                                                                                                                                                                                                                                                                                                              |
| `.timeout(..)`            | 30 s                  | Applies to the token exchange and to every resource call.                                                                                                                                                                                                                                                                                                                                                                                                        |

The resource base is always `{base_url}/v1`; overriding the issuer does not
move it.

`Credentials` and `Client` both have hand-written `Debug` implementations that
redact key material and the cached bearer token —
[`tests/debug_redaction.rs`](tests/debug_redaction.rs) fails if either is ever
replaced by a derive.

## Errors

Two types, deliberately:

- **`ConfigError`** — returned by `ClientBuilder::build` and
  `Credentials::rsa_pem`. Nothing has reached the wire yet, so it is not a
  variant of the network error enum; `Error: From<ConfigError>` for callers
  who want one `?`-able type.
- **`Error`** — everything that happens on, or instead of, the wire:

| Variant                     | When                                                                                                           | Carries                                                              | `@vpay/sdk` equivalent                         |
| --------------------------- | -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------- |
| `Error::Api`                | A non-2xx `/v1` response shaped like `vpay_api::error_envelope`.                                               | `status`, `kind` (the envelope's `type`), `code`, `message`, `param` | `VpayApiError`                                 |
| `Error::TokenEndpoint`      | The token endpoint refused (`invalid_client`, …). **Never retried.**                                           | `error`, `description`                                               | `VpayAuthError` (`error` / `errorDescription`) |
| `Error::UnexpectedResponse` | A response that is not that envelope — a proxy's HTML 502, an empty body, a success body that will not decode. | `status`, `body_prefix` (bounded to 500 bytes)                       | `VpayUnexpectedResponseError`                  |
| `Error::Transport`          | DNS, TLS, timeout, connection refused — no HTTP response to classify.                                          | the underlying message                                               | `VpayTransportError`                           |
| `Error::InvalidParams`      | A parameter this SDK refuses to send — an amount that is negative or past 2^53-1. Nothing reaches the wire.    | `param`, `message`                                                   | a `TypeError` from `assertIntegerAmount`       |
| `Error::Config`             | A `ConfigError` surfaced through a request path.                                                               | the `ConfigError`                                                    | `VpayConfigError`                              |
| `Error::Webhook`            | `webhooks::verify` rejected a delivery.                                                                        | a `WebhookError`                                                     | `WebhookSignatureError`                        |

The distinction is not cosmetic: retrying a `Transport` failure is correct and
retrying an `Api` `400` is a bug, and only a caller who can tell them apart
can get that right. Nothing is retried automatically except the single re-auth
on a `401`.

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

The header grammar is held byte-for-byte to `@vpay/sdk`'s, because a delivery
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

`just sdk-conformance-node` closes the loop in the other direction: the Node
SDK mints an assertion and this crate's `verify_assertion` example hands it to
the real OP verifier.

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

**Half of the server side of this contract exists.** `vpay-server` mounts the
merchant OP — `POST /v1/oauth/token`, `GET /v1/oauth/jwks.json`,
`GET /v1/oauth/.well-known/openid-configuration` — and gates every other `/v1`
path behind a merchant bearer token. What does not exist is a single `/v1`
_resource_ route: past that boundary, `payment_intents`, `refunds`, `events`
and `balance` all answer a Stripe-shaped `404 unknown_route`. So this SDK's
authentication half has completed real requests against a real vpay
(`backends/tests/integration/tests/merchant_token_flow.rs` drives this crate
against the real router over a real Postgres) and its resource half never has.
See [`docs/status.md`](../../docs/status.md) and
[`docs/flows/merchant-auth.md`](../../docs/flows/merchant-auth.md).

What the tests **do** prove — 113 tests, 0 ignored, run by
`just test-sdk-rust`:

- **The assertion this SDK mints is accepted by the real verifier.**
  `tests/op_conformance.rs` generates RSA keypairs, derives their public JWKs,
  and hands each minted assertion to
  `authkestra_op::client_assertion::verify_client_assertion` at the pinned
  `authkestra-op = "=0.7.1"` — the same code vpay will run — against a
  `ClientRegistration` holding the matching JWK, with `expected_audiences =
[token_endpoint, issuer]`. Both the single-key (no `kid`) and multi-key
  (`kid` selects the right one) cases. The negative controls matter as much:
  an assertion signed by a different keypair, one naming a `kid` it did not
  sign with, one minted for another audience, and one presented for a
  different `client_id` are each **refused** by that same verifier; a lifetime
  beyond 300 s is refused by the builder before minting.
- The token exchange, against a `wiremock` HTTP server (a real local server —
  the SDK's own transport and encoding code runs unchanged): the exact form
  fields and their order, the content type, the absence of `client_secret`,
  and the resulting `Authorization: Bearer` on the next call.
- Token caching (a second call makes no second token request), expiry
  (`expires_in: 1` does), single-flight refresh (eight cold-start callers,
  spawned before any is awaited, produce exactly one token request), the
  single `401`-triggered re-auth — proven by matching on the _new_ bearer
  value, not on call order — and that a second consecutive `401` surfaces to
  the caller instead of looping. A token-endpoint rejection is asserted to
  produce exactly one token request.
- That a `401`-triggered retry of a `POST` replays the **same**
  `Idempotency-Key` and the same body — for a caller-supplied key, for a
  generated one, and for a nested `confirm` body. A re-auth that minted a
  fresh key would turn one create into two charges.
- That two concurrent callers refused a moment apart do not each re-authenticate:
  the second one's `401` carries a token that is no longer cached, and
  discarding the cache on it would throw away the token the first caller just
  fetched. Exactly two token requests, asserted by mounting exactly two.
- That an amount which is negative or past `2^53-1` is refused before any
  request is built — including before the token exchange — and that
  `2^53-1` itself is still sent.
- That an id containing `/`, `?` or `#` is percent-encoded into the path
  rather than changing which route is addressed.
- That a `Client` builds in a process where no rustls `CryptoProvider` has
  been installed, and that the SDK does not install one itself
  (`tests/tls.rs`). Under reqwest 0.13's `rustls-no-provider` — the only
  feature set that keeps the banned `aws-lc-rs` out of the graph — reqwest's
  own builder panics in that situation.
- Every resource method: exact path, method, headers (including a generated
  UUIDv4 `Idempotency-Key` when the caller supplies none) and the exact
  encoded body string, plus the typed decode of the response — including
  `next_action`, `last_payment_error`, and an event carrying an object this
  SDK does not model.
- Error mapping: a Stripe-shaped `400` with all four fields, one without the
  optional fields, an HTML `502`, an oversized body truncated to 500 bytes, an
  oversized **multibyte** body cut on a character boundary rather than mid-
  character, a success body that will not decode, a token endpoint answering
  HTML, and a refused connection on both the token and resource paths.
- Webhook verification: a valid signature, the wrong secret, a timestamp
  outside tolerance, a timestamp exactly _on_ the tolerance boundary
  (accepted), a second `v1=` matching during rotation, malformed headers, a
  one-byte body change, and the header-grammar rules listed under "Webhook
  verification" — a `t` with a sign/decimal point/hex form is refused, a `t`
  with leading zeros verifies against an HMAC over its literal text, and an
  unparseable or unknown header part is ignored.
- That neither `Credentials`' nor `Client`'s `Debug` output — nor an
  `InvalidPrivateKey` error — can contain the private key, and that a cached
  bearer token does not appear either.
- That `PaymentIntent::client_secret` decodes when `create`/`retrieve`
  responses carry it, decodes to `None` when a `list()` item or an `Event`
  payload omits the key entirely, and never appears in `PaymentIntent`'s
  `Debug` output either way — `tests/debug_redaction.rs` fails this SDK the
  same way it would fail a `Credentials`/`Client` regression.

Each of these was checked to **fail** when the behaviour it names is broken —
by making the change and running the suite, not by reading the test. The list
is exact, because a mutation list nobody re-ran is worth less than no list:

| Mutation                                                          | What fails                                                                                                                     |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| release the cache mutex before the token fetch (no single-flight) | `concurrent_first_calls_share_one_token_request` — 8 token requests against `.expect(1)`                                       |
| regenerate the `Idempotency-Key` on the retry                     | all three `a_reauthed_*` tests                                                                                                 |
| send an empty body on the retry                                   | `a_reauthed_post_replays_the_callers_own_idempotency_key_and_body`, `a_reauthed_confirm_replays_its_nested_body_byte_for_byte` |
| clear the token cache unconditionally on a `401`                  | `a_second_concurrent_401_does_not_discard_the_token_the_first_one_just_fetched`                                                |
| HMAC over `t` re-rendered as a number                             | `the_hmac_covers_the_literal_t_text_not_a_re_rendered_number`                                                                  |
| accept any `t` that parses as an integer                          | `a_t_that_is_not_a_run_of_decimal_digits_is_malformed`, `a_malformed_header_is_rejected`                                       |
| hard-fail on a header part with no `=`                            | `an_unparseable_part_and_an_unknown_key_are_both_ignored`                                                                      |
| keep an empty `v1=` as a signature candidate                      | `an_empty_v1_is_never_treated_as_a_match`                                                                                      |
| tolerance `>` → `>=`                                              | `a_timestamp_exactly_on_the_tolerance_boundary_is_accepted`                                                                    |
| drop either amount check                                          | `an_amount_outside_the_cross_sdk_safe_range_is_refused_before_any_request`                                                     |
| interpolate a path id unescaped                                   | `an_id_with_url_metacharacters_is_percent_encoded_into_the_path`, `confirm_and_cancel_encode_the_id_too`                       |
| truncate a body prefix without backing off a character            | `bounded_prefix_cuts_on_a_character_boundary…`, `an_oversized_multibyte_error_body_is_cut_on_a_character_boundary`             |
| let reqwest configure TLS itself                                  | both `tests/tls.rs` tests — with the exact `No rustls crypto provider is configured` panic                                     |
| hard-code the assertion `jti`                                     | `mints_a_fresh_jti_on_every_call`                                                                                              |
| remove the token cache                                            | the caching test                                                                                                               |
| remove the `401` retry                                            | both re-auth tests                                                                                                             |
| change the assertion's `sub`, or drop its `kid`                   | the OP-conformance tests                                                                                                       |
| revert the escaping rule to RFC 3986's                            | the Node-parity tests                                                                                                          |

What this does **not** prove: **that TLS works.** Nothing in this repository
serves TLS — `wiremock` is plaintext HTTP and no test reaches the network — so
certificate verification against the vendored roots is exercised by no test at
all. `tests/tls.rs` proves only that the stack is built and reached (a
handshake is attempted against a plaintext listener and fails), and a unit
test proves the root store is populated and ALPN advertises `h2`. Also not
proven here: that any of this works against a real vpay deployment over TLS.

What _is_ proven elsewhere, and what still is not: this crate is driven
against the real `vpay_api::router` on a real socket over a real Postgres in
`backends/tests/integration/tests/merchant_token_flow.rs` — a client built
from nothing but a base URL and a credential obtains a token from
`/v1/oauth/token` and crosses the `/v1` boundary with it, and a replayed
`client_assertion` is refused the second time (the `ClientAssertionStore` is
wired: `vpay_db::SqlClientAssertionStore`). What that suite reaches on the
other side is a `404 unknown_route`, because vpay implements **no `/v1`
resource route** — so every `payment_intents`, `refunds`, `events` and
`balance` call in this crate is still unproven against a server, and the
request/response shapes they encode remain this SDK's own claim about an
API that does not answer yet. See `docs/status.md`.
