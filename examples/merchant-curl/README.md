# Raw HTTP — and why it is not `curl` any more

> **`curl` cannot call `/v1` since VP-3 (2026-10-26).** Every request is a
> COSE_Sign1 message — the signature is the _body_, even for a `GET` — and every
> answer is one too, so there is no `-H "Authorization: …"` to add to an ordinary
> request. This directory used to be the curl-shaped walkthrough; it is now the
> same walkthrough over a **minimal sealing CLI that ships with the Node SDK**,
> [`sdks/nodejs/scripts/vpay-call.mjs`](../../sdks/nodejs/scripts/vpay-call.mjs)
> (a documented SDK script, not a second client: it calls the SDK's own sealing
> path). The form fields, the `Idempotency-Key` and every answer below are
> exactly what they were; only the transport changed. What goes on the wire, and
> why, is [signed-transport.md](../../docs/flows/signed-transport.md).

> **What runs, and what does not.** Every call below — including **confirm**
> — executes against a running `vpay-server`. Confirm reaches the rail over
> HTTP and, when the rail accepts, moves the intent to `processing` (push) or
> `requires_action` (redirect). ~~What is _not_ built is everything after
> that: nothing asks the rail whether the payer approved, so an intent that
> reaches `processing` stays there until a human looks. The reconciler is a
> later step.~~ _(Struck 2026-09-23: wrong since Step 4 (2026-09-03). The
> worker's authenticated status query drives the intent to `succeeded` or
> back to `requires_payment_method`, so poll
> `GET /v1/payment_intents/{id}` or take the webhook; see
> [the reconciler flow](../../docs/flows/reconciler.md).)_ See
> ../../docs/status.md and
> [ADR-0033](../../docs/adr/0033-v1-is-a-signed-message-not-a-token.md).
>
> `GET /v1/balance` is not routed at all and answers the honest
> `404 unknown_route`. _(This line also named `POST /v1/refunds` until
> 2026-09-23. That route has been mounted since 2026-09-16, and a refund it
> creates stays `pending`, because nothing settles one.)_ `GET /v1/events`, `GET /v1/events/{id}` and
> — since 2026-09-05 (issue #45) — `GET /v1/refunds/{id}` **are** served. This
> line said all three of `/v1/refunds`, `/v1/balance` and `/v1/events` were
> unrouted; it had been wrong about `/v1/events` since Step 5 (2026-09-03).
>
> **Every `POST` under `/v1` requires an `Idempotency-Key` header.** The
> _server_ never defaults one: a `POST` without the header is refused with a
> `400` naming `idempotency_key`, before anything is created. Both SDKs do
> default one (a UUIDv4 per call), which is why this only matters to a client that builds its own request. Reuse the same key to retry the _same_ request safely; use a new
> key for a new request. See "Idempotency" below.
>
> Array parameters are shown in the bare style Stripe's own docs use (`key[]=v`); the SDKs
> send the indexed form (`key[0]=v`) Stripe's own SDKs use, and the server
> accepts both, as Stripe does.

## Authenticate — sign the call

`/v1` does not accept an API key, and it does not accept a token. A merchant's
backend holds an Ed25519 seed whose **public** half vpay has on file
(`merchant_clients[].cose_keys`, registered via a YAML config PR — see
[ADR-0003](../../docs/adr/0003-yaml-configuration.md)), and pins vpay's two
public response keys. The CLI takes all of it from the environment:

```bash
pnpm --filter @vaam-apps/vpay-sdk build     # the script imports ../dist

export VPAY_BASE_URL=https://api.vpay.example
export VPAY_MERCHANT_SIGNING_SEED_PATH=./merchant-a.seed      # never leaves your systems
export VPAY_SERVER_KEY_CURRENT_PATH=./vpay-current.pub        # vpay's response keys,
export VPAY_SERVER_KEY_NEXT_PATH=./vpay-next.pub              # public, from the operator
alias vpay-call='node sdks/nodejs/scripts/vpay-call.mjs'
```

`vpay-call METHOD /path [--query q] [--data form] [--key idempotency-key]` takes
the path **under `/v1`**, prints the verified JSON answer to stdout and
`status: <n>` to stderr, and exits `0` for a 2xx. A body that did not verify is
an error and is never printed. `--data` is the form body exactly as `curl -d`
would send it. For the other languages: the Rust SDK is `vpay-sdk`, and
`sdks/rust/examples/create_and_confirm.rs` is the same walk in Rust.

An unsigned call, a call signed by a key vpay does not know, a replay and a
call for the wrong audience are the **same** bare `401`, with nothing in it about
why. If you get one: check the key is the registered one, the clock is right,
and the base URL is the one vpay names itself by (`deployment.public_base_url`).

## Idempotency

Every `POST` under `/v1` **must** carry an `Idempotency-Key`:

```text
--key 5f0d5e4c-9f4e-4d2a-9c1b-4a2a1e0f7f31
```

(The key is part of what the signature binds. The CLI draws a fresh UUID for a
write that names none, and prints it to stderr so a retry can reuse it.)

**Both SDKs send one for you, and the default is a fresh UUIDv4 per call** —
`vpay_sdk::RequestOptions` (Rust) and the `RequestOptions` argument (Node)
generate one when you do not supply a key, so a `POST` from either SDK is
never the `400` below. That default is the right one for the common case: it
makes the SDK's own network-level retry safe.

Override it — `RequestOptions::new().with_idempotency_key("…")` in Rust,
`{ idempotencyKey: "…" }` in Node — when the thing you must not do twice is
bigger than one HTTP call: a key derived from your order id (say
`order_1234_create`) makes the _whole operation_ idempotent across process
restarts, a crashed job runner, or a queue that delivers the same message
twice. A per-call UUID cannot protect you there, because the retry is a
different call.

Keys are scoped to your merchant and kept for 24 hours. Two rules follow from
that, and both matter if you derive keys: a derived key must be unique per
logical operation (`order_1234_create` and `order_1234_confirm`, never
`order_1234` for both), and reusing one for a genuinely different request is
an error rather than a new object — see below.

What the server does with a key:

- **Missing or empty** → `400`, `{"error":{"type":"invalid_request_error",
"param":"idempotency_key", …}}`. Nothing is created.
- **Same key, same body** → the _stored_ response is replayed, byte for byte.
  Nothing is created a second time. This is what makes a network retry safe.
- **Same key, different body** → `400`,
  `{"error":{"type":"idempotency_error","code":"idempotency_key_in_use", …}}`.
- **Same key, first request still running** → `400`,
  `{"error":{"type":"idempotency_error","code":"idempotency_key_in_flight", …}}`
  — _"A request with this Idempotency-Key is still in progress; retry
  shortly."_ Retry the same call after a moment. Do **not** switch to a new
  key until you know how the first one ended: a new key is a new operation,
  and the first one may still be about to succeed.

  Branch on `error.code`, not on the status or the sentence:
  `idempotency_key_in_flight` is the one error here that clears itself, while
  `idempotency_key_in_use` needs you to change something. Both SDKs surface
  the envelope's `code` (`vpay_sdk::Error::Api { code, .. }` in Rust,
  `VpayApiError.code` in Node); neither maps it to a distinct exception type.

Which answers are _stored_ for a replay, and which hand the key back:

- **`2xx` and most `4xx` are stored.** A `4xx` is your request's own outcome —
  re-running it would produce it again — so the retry is answered identically
  rather than re-executed. (Stripe behaves the same way.)
- **A validation failure on `POST /v1/payment_intents` is the exception**: the
  key is released, because what is valid can change. If your currency was not
  configured, an operator configures it, and you retry under the same key, you
  get the intent you asked for — not a 24-hour-old refusal that is no longer
  true. Nothing was written, so re-executing is exactly as if the request had
  never been made.
- **`5xx` is not stored, and the key is released** — including the `502` a
  confirm returns when the rail could not be reached. "We do not know whether
  the rail saw it" is the only honest thing to say, and freezing that for 24
  hours would answer a merchant retrying after the deployment was fixed with
  the old outage. The retry therefore _re-executes_; that is safe because it
  is not the key that prevents a double charge — the unique index behind "one
  charge per intent, forever" is, and a re-executed confirm meets it and
  answers `409`.

## Create a PaymentIntent

Bodies are **form-encoded**, not JSON — that is what the Stripe SDKs send, and
it is unchanged by the auth model above (the form travels inside the signed
message).

```bash
vpay-call POST /payment_intents \
  --key order_1234_attempt_1 \
  --data 'amount=5000&currency=xaf&payment_method_types[]=mtn_momo&payment_method_types[]=orange_money&metadata[order_id]=1234&description=Order %231234'
```

```json
{ "id": "pi_…", "object": "payment_intent", "amount": 5000, "currency": "xaf",
  "status": "requires_payment_method",
  "payment_method_types": ["mtn_momo", "orange_money"],
  "next_action": null, "last_payment_error": null,
  "metadata": { "order_id": "1234" }, "description": "Order #1234",
  "created": 1753401600, "livemode": false }
```

`amount=5000` is **5,000 FCFA**. XAF is zero-decimal.

The parameters, exactly:

| Parameter                | Required | Notes                                                                                                                                         |
| ------------------------ | -------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `amount`                 | yes      | Integer minor units, `1..=2^53-1`.                                                                                                            |
| `currency`               | yes      | Case-insensitive on the way in, lowercase on the way out. Must be one this deployment configures.                                             |
| `payment_method_types[]` | yes, ≥ 1 | Rail codes. Each must be enabled on this deployment, or the create is refused — an intent naming a rail that is off could never be confirmed. |
| `metadata[k]`            | no       | ≤ 50 keys, key ≤ 40 chars, value ≤ 500 chars.                                                                                                 |
| `description`            | no       | ≤ 1000 chars. Shown to you, never to the payer.                                                                                               |

## Retrieve, and list

```bash
vpay-call GET /payment_intents/pi_xxx

# Newest first. `limit` defaults to 10 and is capped at 100.
vpay-call GET /payment_intents --query 'limit=10&starting_after=pi_xxx'   # next page
# ... or ending_before=pi_yyy for the previous one.
```

```json
{ "object": "list", "data": [ /* payment_intent objects */ ],
  "has_more": true, "url": "/v1/payment_intents" }
```

An id belonging to another merchant answers exactly the same `404` as an id
that never existed. That is deliberate: this API is not an oracle for which
ids exist.

## Confirm — push rail (MTN)

```bash
vpay-call POST /payment_intents/pi_xxx/confirm \
  --key order_1234_confirm_1 \
  --data 'payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000'
```

→ `"status": "processing"`, `"next_action": null`; the payer gets a prompt on
their handset, and you wait for the webhook — **do not ship on `processing`**.

The `currency` of the intent must be the one the rail settles in. vpay's own
`config/application.yml` puts `mtn_momo` on `EUR` (its sandbox rejects XAF),
so confirming an XAF intent on it is a `400` naming
`payment_method_data[type]` — before any charge exists, so the intent is
still confirmable on a rail that does settle in its currency.

Three other answers this call can give, and what each means for your
bookkeeping:

| answer                                                                                                                                  | what happened                                                                                                                                              | what to do                                                                                                                                                                    |
| --------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `409` `charge_declined`                                                                                                                 | the rail **decided**: the charge is `failed` and the intent carries `last_payment_error`                                                                   | read `last_payment_error.code` against [failures.md](../../docs/flows/failures.md); a retry is a **new** PaymentIntent                                                        |
| `502` `provider_unavailable`                                                                                                            | the rail could not be reached (or answered unreadably), and vpay does not know whether it saw the request. Nothing moved: the charge is still `submitting` | retry the same call under the same `Idempotency-Key`. If the first attempt did land, you get the **first** `409` below — poll, do not open a second intent                    |
| `409`, message _"…is being resolved with the rail; poll `GET /v1/payment_intents/{id}` — do not create a new PaymentIntent."_           | this intent already has a charge and that charge is **live** (`submitting`/`submitted`/`pending`/`unresolved`)                                             | **do not open a second intent.** Poll the `GET`; the charge will resolve one way or the other. This is what you get when your `502` retry raced a submit the rail did receive |
| `409`, message _"This payment intent already has a charge. One charge per intent, forever — create a new payment intent to try again."_ | this intent's charge is **terminal** (`succeeded`/`failed`)                                                                                                | nothing is in flight; a retry is a new PaymentIntent                                                                                                                          |

**Nothing polls a `processing` intent yet.** The rail has the request; the
worker that asks it how the payment ended is a later step
(`../../docs/status.md`). In practice that means an intent confirmed on a
push rail stays `processing` indefinitely and **never reaches `succeeded`**
— and no rail vpay has ever confirmed against was a real one: every outcome
described here has been observed against a WireMock stub, not against MTN or
Orange.

## Confirm — redirect rail (Orange)

```bash
vpay-call POST /payment_intents/pi_xxx/confirm \
  --key order_1234_confirm_1 \
  --data 'payment_method_data[type]=orange_money&return_url=https://shop.example/order/1234/return'
```

→ `"status": "requires_action"` with:

```json
{ "next_action": { "type": "redirect_to_url",
                   "redirect_to_url": { "url": "https://webpayment.orange-money.com/…",
                                        "return_url": "https://shop.example/order/1234/return" } } }
```

— send the payer's browser to `url`. `return_url` is echoed back as you sent
it; the rail returns the payer there afterwards.

**If you get a `url`, vpay already has it.** The rail's token and the URL are
committed before the response is built, so a URL you were handed is one vpay
can still query the charge by — that ordering is the whole of
[crash-safety.md](../../docs/flows/crash-safety.md)'s redirect section. The
same `next_action` comes back from `GET /v1/payment_intents/{id}`, so losing
this response does not strand the payment.

The same failure answers as the push rail apply, plus one shape rule: a
redirect confirm whose `return_url` is missing, is not `http`/`https`, or is
longer than **2048** characters is a `400` naming `return_url` — because a
payer must not be sent somewhere they cannot come back from, and because the
column that stores it is bounded the same way.

An intent in `requires_action` also stays there: nothing polls the charge
behind it, so it never reaches `succeeded` on its own.

## Cancel

Legal only while the intent is `requires_payment_method` — once a confirm has
handed a charge to a rail, cancelling would tell you a payment was withdrawn
while the payer's handset was still prompting.

```bash
vpay-call POST /payment_intents/pi_xxx/cancel --key order_1234_cancel_1
```

→ `"status": "canceled"`, or `409` if the status no longer allows it.

## Retrying a failed payment

Create a **new** PaymentIntent. An intent gets exactly one charge for its entire
life — that is what makes double-charging structurally impossible.
