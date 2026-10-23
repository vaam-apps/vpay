# vpay-notchpay

A typed client for the Notch Pay REST API, transcribed on 2026-10-20 from
Notch Pay's OpenAPI specification **2.1.0** (`https://developer.notchpay.co/openapi.yaml`)
and its API reference pages. It is internal (`publish = false`) and exists to
be called by a `notchpay` adapter.

**What is built, and what is not.** The crate is built and unit-tested.
**Since 2026-10-20 `vpay-adapter-notchpay` sits on it, and `vpay-server`
links that adapter.** It is admitted under
[ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md),
which amended ADR-0029 D1's third rule; see
[adapter-notchpay.md](../flows/adapter-notchpay.md). **Notch Pay's sandbox
has been called through this crate**, by the step 1 probe and by the adapter
(step 5), both on 2026-10-21 ([step 1](../status/verification/2026-10-21-notchpay-sandbox-run.md),
[step 5](../status/verification/2026-10-21-notchpay-step5.md)). Its
production never has. _(This said "Nothing has called Notch Pay, through this
crate or any other" until 2026-10-21.)_

_(This paragraph read as follows until 2026-10-20, when the adapter landed.)_
"No adapter uses it, and no binary links it.
[RFC-0012](../rfc/0012-notchpay-adapter.md) designs the adapter and records
why it is not written: Notch Pay's status read is documented as
authenticated by the key Notch Pay calls public, and
[ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
D1's third hard rule refuses an adapter to such a provider until the
maintainer resolves it. Nothing in this crate has called Notch Pay."

## Why it is its own crate

The maintainer wrote on 2026-10-20: "From their official api-ref we can
create a new crate and use it internally." Notch Pay publishes SDKs for
JavaScript, PHP, Python, Laravel and mobile, and none for Rust. The two
existing adapters keep their wire inside themselves as `wire.rs`; this crate
is that `wire.rs` moved to a crate boundary, with the HTTP calls beside it.
The adapter keeps `mapping.rs`, which decides what an answer means for a
charge. The boundary is where it is so that no decision about money can live
in the client.

## What it covers

| Operation                           | Notch Pay endpoint           | Method                       |
| ----------------------------------- | ---------------------------- | ---------------------------- |
| Initialize a payment                | `POST /payments`             | `Client::initialize_payment` |
| Process a payment (the USSD prompt) | `POST /payments/{reference}` | `Client::process_payment`    |
| Retrieve a payment (the read)       | `GET /payments/{reference}`  | `Client::retrieve_payment`   |
| List webhook endpoints              | `GET /webhooks`              | `Client::list_webhooks`      |
| Create a webhook endpoint           | `POST /webhooks`             | `Client::create_webhook`     |
| A webhook event (inbound)           | —                            | `parse_event`                |

The two webhook calls are here because the maintainer decided on 2026-10-20
that vpay registers its own endpoints
([RFC-0013](../rfc/0013-vpay-registers-its-own-callbacks.md)). The client
only performs them; when, how often and how a duplicate is avoided is that
RFC's design. Since 2026-10-23 nothing calls them unless the deployment
writes `callbacks.register: check` or `ensure` for the rail: unset, Notch
Pay's adapter declares `off` (RFC-0013's amendment, after the sandbox stored
an endpoint created by `create_webhook` with `"url": null`).

`status::PaymentStatus` names the eleven payment statuses Notch Pay
documents, in Notch Pay's spelling (`partialy-refunded` included). It does
not say what any of them means for a charge; RFC-0012 § 4 does, and the
adapter's mapping table will.

## What it deliberately omits

- **Refunds.** `POST /refunds` appears in a Notch Pay guide and not in the
  specification or the API reference, and the guide's request carries no
  reference that would make a replay safe. RFC-0012 § 7 is the argument; the
  adapter will declare a `NotImplemented` token instead.
- **Cancel** (`DELETE /payments/{reference}`). The port has no cancel-at-rail
  operation, and a courtesy call in the money path is a call with no test
  behind it.
- **Webhook update and delete.** Never deleting an endpoint is deliberate: an
  account's other endpoints are not vpay's.
- **Transfers, beneficiaries, customers, balance, channels.** vpay calls none
  of them.
- **Webhook signature verification.** Notch Pay signs with HMAC-SHA256 in
  `x-notch-signature`. The check is not this client's: since 2026-10-23 it is
  the adapter's `verify_callback` (RFC-0014), over the HMAC helper in
  `vpay_provider::callback`, because the route hands the adapter the headers
  and the raw body and this client never sees a webhook. _(This said the
  port's `parse_callback` "receives no headers … so a verifier would be code
  nothing can call" until 2026-10-23.)_
- **A signing secret on `Webhook`.** The specification's schema has no field
  for one, although the API reference says the secret is shown "when creating
  a webhook via the API" (RFC-0012 § 10 item 13).

## Every call carries the private key

The specification requires only `Authorization` (the public key) on the
payment endpoints, and `X-Grant` (the private key) on transfers, balance,
beneficiaries and webhooks. `Client` sends **both on every request**. That is
what RFC-0012's way A would require of the read, and a server that ignores
the header loses nothing by receiving it. Both keys are held as header values
marked sensitive, so neither `reqwest`'s `Debug` of a request nor this crate's
own prints them.

**A per-request deadline, since 2026-10-20.** `Client::with_request_timeout`
puts a deadline on every request the client builds. The adapter sets it from
`ProviderConfig::request_timeout`, because the process's one
`reqwest::Client` is shared by every rail and carries only the default. The
conformance case `an_unavailable_rail_is_a_transport_error_never_a_decline`
was measured to fail on `notchpay` with the call removed.

**The `reqwest::Client` must not follow redirects.** A followed redirect
replays every header `reqwest` does not consider sensitive to the new host,
and `X-Grant` is one of them. `vpay_provider::http::client_with_timeouts` and
`vpay_provider::http::client` both refuse redirects; the binary builds its
client with them, and a caller must too.

## A `4xx` is an answer, not an error

A call returns `Result<Answer<T>, ProviderError>`:

| Notch Pay answered              | Result                                                          |
| ------------------------------- | --------------------------------------------------------------- |
| `2xx` with the documented shape | `Ok(Answer::Success(T))`                                        |
| `2xx` with anything else        | `Err(Malformed)`                                                |
| `408`, `429`, `5xx`, or nothing | `Err(Transport)` — the payment's fate is unknown                |
| any other `4xx`                 | `Ok(Answer::Refused(Refusal))`, with as much envelope as parses |
| `1xx`, `3xx`                    | `Err(Malformed)` — the client follows no redirect               |

What a `409` or a `422` means for a charge is the adapter's mapping. A client
that turned one into a decline would be deciding that in a library. Nor does
the crate define an error type of its own: the `Err` side is the port's
`ProviderError`, only ever `Transport`, `Malformed` or, at construction,
`Config`. A leaf enum would be a second place to classify the same failures,
which is why the two existing adapters do not define one either.

A `404` on the read is **not** necessarily "no such payment". An unknown
`X-Sync` account is a `404` too ("Sync account not found"), and so is every
call against a base URL with a wrong path. The adapter tells them apart from
Notch Pay's `message` (RFC-0012 § 3).

The error envelope is read field by field rather than as one struct. An API
built on Laravel answers an empty error list as `[]`, and an all-or-nothing
decode would then lose `message` too — the field that says "A payment with
this reference already exists".

## The guards, and the review that added them

An independent review of the first version, on 2026-10-20, found two
blocking defects and eight minor ones. All were fixed before the crate was
published on a branch:

- **A reference cannot choose the endpoint.** `vpay_provider::http::path_segment`
  escapes a segment byte by byte and passes `.` and `..` through, and URL
  parsing treats `%2E%2E` as `..` as well. `retrieve_payment("")` would have
  read `GET /payments/`, the list, and `process_payment("..")` would have
  posted the payer's number to the API root. The read's reference can be
  Notch Pay's own, from a stored answer, so the rail is a caller this guards
  against. An empty or dot-only reference is now `Malformed` before any
  request. MTN's account-holder path uses the same helper with a digits-only
  number, so it is not affected; the helper itself still passes dots, and a
  future caller must refuse them as this crate does.

  _(This bullet said "the helper itself still passes dots, and a future caller
  must refuse them as this crate does", and described the helper as one that
  "passes `.` and `..` through", until 2026-10-20; both were true of the
  helper the review read. `path_segment` now refuses an empty or all-dots
  segment itself, as `Malformed` with a fixed message that never quotes the
  input, and this crate's private `reference_segment` was deleted in favour
  of it: `Client::payment_request` calls `path_segment(reference)?`. The
  refusal is the helper's to prove, in `vpay-provider`; that this crate
  calls it is what
  `an_empty_or_dot_only_reference_is_refused_before_any_request` pins, by
  going through `payment_request` rather than through the helper. See
  `docs/reference/rails.md` § "A path segment the caller chose".)_

- **A payload is required.** Every envelope field was `Option`, so `{}` — or
  the `data`-shaped body some API reference pages show — decoded as an empty
  success. A webhook list read that way finds no vpay endpoint, and a caller
  registers a duplicate every run, because creation is not idempotent. The
  payload (`transaction`, `endpoint`, `endpoints`, an event's `data`) is now
  required; the fields inside it stay `Option`.
- **A base URL no call could use is refused at construction**, as `Config`,
  rather than becoming a `Transport` error on every charge. A `Transport` says
  a payment's fate is unknown, and a request that was never sent has none.
- **Transport errors carry no URL** (`reqwest::Error::without_url`), because a
  payment path carries a reference.
- **`408` is `Transport`**, not a refusal: its fate is unknown.
- **`Client`'s `Debug` prints the base URL through `SafeUrl`**, as every URL in
  this workspace's `Debug` output does.
- **The tests were made to fail when what they name breaks.** The canary for
  "never quotes the body" now uses a body serde's own message _does_ quote,
  and asserts that it does, so the test cannot pass vacuously. The path tests
  go through the client's own request builder for both payment methods.

## Where Notch Pay's pages disagree with the specification

The client follows the specification at each point below, and RFC-0012 § 10
lists every one for the first sandbox session.

| Question                               | Specification 2.1.0                    | Notch Pay's other pages                                                    |
| -------------------------------------- | -------------------------------------- | -------------------------------------------------------------------------- |
| Which reference the payment path takes | the merchant's (`order_123`)           | the direct-charge guide passes Notch Pay's `trx.…`, with ours as `trxref`  |
| Where the number goes on process       | `data.phone`, or `data.account_number` | one guide uses `account_number`                                            |
| A duplicate reference on initialize    | no `409`                               | the errors page shows `409` "A payment with this reference already exists" |
| The envelope key                       | `transaction`, `endpoint`, `endpoints` | API reference pages show `data`                                            |
| A webhook endpoint's `active` flag     | absent                                 | shown on the API reference page                                            |
| The `payment.processing` event         | absent from the enum                   | listed on the API reference page                                           |
| A webhook's signing secret             | absent from the schema                 | "when creating a webhook via the API"                                      |
| What a webhook signature covers        | —                                      | raw body on two pages; a re-serialised object in one example               |

## The sandbox probe

`examples/sandbox_probe.rs` asks Notch Pay's real sandbox RFC-0012 § 10's
sandbox questions, from
[runbooks/notchpay-sandbox-test.md](../runbooks/notchpay-sandbox-test.md).
It is an example whose `[[example]]` entry says `test = true`, so its own
unit tests are one of `verify-ignored`'s test binaries (52 → 53 on
2026-10-20); the probe itself runs only by hand. **Written 2026-10-20;
run against Notch Pay's sandbox on 2026-10-21**
([verification](../status/verification/2026-10-21-notchpay-sandbox-run.md)).
Two limits that run showed: its poll reads by our reference only, which the
sandbox always answers `404`, so it waits out the whole poll window and then
reads once by Notch Pay's reference; and its webhook cleanup uses the
documented `DELETE /webhooks/{id}`, which the sandbox answered `404`.
Its `search` phase (added the same day) calls `GET /payments?search=` for
each `--reference`, read-only, for item 19. Four phases were added on
2026-10-23, all addressing by Notch Pay's `trx.…`:

- `two-step` (ADR-0032, item 22);
- `channels` (`cm.mtn` and `cm.orange` with every outcome number, RFC-0012
  step 8, item 23);
- `webhook-receive` (one endpoint at a URL you can read, then the documented
  `DELETE`);
- `webhook-update` (the documented `GET` and `PUT /webhooks/{id}` for one
  endpoint it created).

They ran the same day
([verification](../status/verification/2026-10-23-notchpay-sandbox-two-step-and-channels.md)).
The webhook pair found that the sandbox stores `"url": null` and answers
`404` to every route that takes an endpoint's id.

`--ca-bundle <path>` is a cloud-only aid, off unless given: it adds that
file's PEM certificates to the vendored roots for the probe's own client,
so the probe can reach Notch Pay through a cloud session's TLS-terminating
egress gateway. That client still refuses redirects and ignores proxy
variables, and a bad path stops the probe before any request. It is not,
and must not become, an option of `vpay_provider::http`, `vpay-server` or
`vpay-worker` (ADR-0003). Its `rustls` and `webpki-roots` are
`[dev-dependencies]`, like the four below.

It sends every body from this crate's `wire` types and decodes every answer
with them. Each read also goes through `Client`. The other calls go out by
hand with `Client`'s headers, because `Client` keeps no raw body and the
evidence needs one. It refuses to start unless both keys contain `test.` or
`test_`. It redacts its report before writing it: a string holding a key
becomes the key's name, and a field named like a secret becomes its shape.
It deletes every webhook endpoint it creates. Its `uuid`, `hmac`, `sha2` and
`hex` are `[dev-dependencies]`, already in the workspace.

## Privacy

Every type that holds a payer's number, a reference, a callback URL or Notch
Pay's own prose has a hand-written `Debug` and is registered in
[schemas/privacy-inventory.yaml](../../schemas/privacy-inventory.yaml)'s
`debug_protections` (twelve types). `Keys` and `Client` hold rail credentials
and hide them with `Secret`, and are not registered: the inventory has no
element for a rail credential, which is `vpay_provider::ProviderConfig`'s
situation too. When an adapter first sends the payer's number to Notch Pay,
that adapter's change adds `notchpay` to the `payer_msisdn` element's
recipients; this crate sends nothing on its own.
