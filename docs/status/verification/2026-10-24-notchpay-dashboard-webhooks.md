# 2026-10-24 — Notch Pay delivers to a dashboard endpoint, and its signature is the one vpay checks

## What this is, and what it is not

The first webhooks Notch Pay has delivered, measured in its sandbox. They went
to a **capture endpoint registered by hand in Notch Pay's dashboard**, not to
vpay. The measurement answers three questions:

- **RFC-0012 § 10 item 7:** what `x-notch-signature` is computed over.
- **Item 25:** why payments stalled on 2026-10-23.
- **Whether vpay's own code accepts what Notch Pay really sends.**

That last one found a defect: vpay could not parse a real payment webhook.
This change fixes it.

**No webhook has reached a running vpay.** `vpay-server` cannot reach Notch
Pay from a cloud session, and Notch Pay cannot reach one either (see
[2026-10-23-notchpay-sandbox-two-step-and-channels.md](2026-10-23-notchpay-sandbox-two-step-and-channels.md)).
Every call to Notch Pay went through
`cargo run -p vpay-notchpay --example sandbox_probe`. vpay's adapter was run
**offline**, over the bodies and headers the capture endpoint recorded.

No key, Hash Key, signature or capture-inbox address is on this page or in
any committed file.

## How it was run

- **Who approved it.** The maintainer, in the running session's own chat,
  2026-10-24. They registered the endpoint themselves, test mode, for the
  five payment events, at the third-party capture inbox they had approved on
  2026-10-23 ("A third-party capture service"). Asked whether the endpoint's
  Hash Key was the one already configured as `NOTCHPAY_TEST_HASH_KEY`, they
  answered "I added the webhook and it's the same hash". That answer was also
  the approval for one successful and one failed test payment. They then said
  "I also deleted the stuck webhook `ep_test.unEBL3JnHZF0fF5B`" (§ 10 items 20
  and 25), which the API had refused to delete.
- **From where.** A Claude Code cloud session, on `master` at `630c8a34` plus
  this change, with `--ca-bundle /root/.ccr/ca-bundle.crt`.
- **Keys.** `NOTCHPAY_TEST_PUBLIC_KEY`, `NOTCHPAY_TEST_PRIVATE_KEY` and
  `NOTCHPAY_TEST_HASH_KEY` from the environment, behind the probe's
  `test.`/`test_` refusal. No value was printed.
- **Mode.** Every payment answer carried `"sandbox": 1`; the probe's mode
  check printed `sandbox=true`. Every identifier carried `.test_`.

```text
sandbox_probe read                 --out <report> --ca-bundle /root/.ccr/ca-bundle.crt
sandbox_probe dashboard-deliveries --out <report> --ca-bundle /root/.ccr/ca-bundle.crt
sandbox_probe verify-signature     --body <captured body> --signature <captured header>
```

`dashboard-deliveries` is new in this change. It **creates and deletes no
endpoint**:

1. it lists the account's endpoints;
2. it makes the same two payments as `webhook-receive`, MTN's success and
   other-failure numbers on `cm.mobile`, and polls each to a final status;
3. it waits `NOTCHPAY_PROBE_DELIVERY_WAIT` seconds (default 120);
4. it lists the endpoints again.

## 1. The account's endpoints

`read`, about 08:25 UTC: `GET /webhooks` answered `200` with **one**
endpoint, `ep_test.lBuAjask69aD7YBm`: the maintainer's, with five events.
`ep_test.unEBL3JnHZF0fF5B` is gone.

**The list shows `"url": null` for this endpoint too**, and it delivers
(§ 3). So a null URL in Notch Pay's listing is how the API shows every
endpoint. It is not evidence that an endpoint has no URL.

_That corrects the 2026-10-23 reading of item 24._ Two things were
established that day and still stand:

- the endpoint `POST /webhooks` created delivered nothing in 21 minutes;
- that endpoint could not be read, updated or deleted by id (`404`).

What was not established is that its URL was stored as null. Why it
delivered nothing is not known.

## 2. The registration ping

At **08:19:45 UTC**, when the maintainer saved the endpoint, the dashboard
sent it one `POST`:

- `User-Agent: Notch-Webhook-Verification/1.0`;
- **no `x-notch-signature`**;
- body
  `{"id":"evt_verification_…","type":"verification.test","created":…,"data":{"message":"This is a check of your webhook endpoint",…}}`.

It names itself under **`type`**.

vpay's adapter refuses it: it is not a payment event, and it names no charge.
So vpay's route would answer it `400`, as it would any body it cannot read. It
is not known whether Notch Pay needs a `2xx` here to keep an endpoint. The
capture endpoint answered `200`.

## 3. Two payments and four deliveries

`dashboard-deliveries`, 08:26:53 to 08:29:04 UTC, on `cm.mobile`:

| Payment                               | `trx`                               | Reads                         |
| ------------------------------------- | ----------------------------------- | ----------------------------- |
| MTN success, `+237670000000`, 151 XAF | `trx.test_vwTAcbfxYr5cgcnpCQpYzglh` | `processing`, then `complete` |
| MTN failure, `+237670000002`, 157 XAF | `trx.test_KylQnASZmLFJs4VUJhyGAzEm` | `processing`, then `failed`   |

Each reached its final status by the probe's second read, within about 5 s.

The capture endpoint received, from `User-Agent: GuzzleHttp/7` (a PHP
client):

| UTC      | Event              | Payment |
| -------- | ------------------ | ------- |
| 08:26:54 | `payment.created`  | 1       |
| 08:26:59 | `payment.complete` | 1       |
| 08:26:59 | `payment.created`  | 2       |
| 08:27:05 | `payment.failed`   | 2       |

**Every delivery carried `x-notch-signature`**: 64 lower-case hex characters.

**The body is not the documented shape.** RFC-0012 § 6 and Notch Pay's
Webhooks API show `{ "id": "evt_…", "type": "payment.complete",
"created_at": "…", "data": {…} }`. What arrived was compact JSON in this
form:

```json
{"id":"whc_test.…","event":"payment.complete","data":{"amount":151,"fees":[],
 "amounts":{"total":151,"converted":151,"currency":"XAF","rate":1},"sandbox":1,
 "payment_method":"pm.test_…","description":"vpay sandbox probe (ADR-0032 \/ RFC-0012 step 8)",
 "reference":"trx.test_…","merchant_reference":"<vpay's reference>","trxref":"<vpay's reference>",
 "status":"complete","currency":"XAF","customer":"cus.test_…","charge":"business",
 "callback":null,"created_at":"2026-10-24T08:26:53.000000Z"}}
```

The event names itself under **`event`**, not `type`. There is no top-level
`created_at`. `data.trxref` and `data.merchant_reference` are vpay's
reference; `data.reference` is Notch Pay's `trx.…`, as on every payment
answer. Slashes are escaped as `\/`, as PHP's `json_encode` writes them.

## 4. The signature (§ 10 item 7, answered)

For each of the four deliveries, the probe's `verify-signature` and an
independent check found:

| HMAC-SHA256 under the Hash Key, over…                       | Matches `x-notch-signature` |
| ----------------------------------------------------------- | --------------------------- |
| the **raw body**, byte for byte                             | **yes, 4 of 4**             |
| a `JSON.stringify(JSON.parse(body))`-style re-serialisation | **no, 0 of 4**              |

The re-serialisation differs from the raw body because it writes a bare `/`
where Notch Pay wrote `\/`. So **the signature is over the raw body**, which
is what vpay checks (RFC-0014). The Node example on Notch Pay's setup page,
which signs `JSON.stringify(req.body)`, would refuse genuine webhooks.

The probe's own second line, "whitespace-free re-serialisation", also
matched. That is because it removes whitespace and keeps escapes, and a
compact body has none to remove. Its comment is corrected in this change.

Negative controls:

- one byte of a body changed (`complete` → `cOmplete`): no match;
- one delivery's body checked against another delivery's signature: no match.

**vpay's own check.** A temporary test, ignored by default, ran
`NotchpayAdapter::verify_callback` over the four recorded bodies and headers.
It was removed afterwards and never committed, because it reads the Hash Key
from the environment.

| Configured secret               | Verdict, each of the 4 |
| ------------------------------- | ---------------------- |
| the real Hash Key               | `Verified`             |
| the test suite's fixture secret | `Invalid`              |

This is the first time vpay's check has met a webhook Notch Pay sent. It is
an offline check of recorded requests. No delivery went through vpay's route.

## 5. The defect it found, and the fix

The same temporary test called `parse_callback` on each body. **It refused
all four** as `Malformed`, because the adapter read the event's name from
`type` only. The route calls `parse_callback` first, so vpay would have
answered every genuine Notch Pay webhook `400` and ignored it. No money
depended on it: the poll ladder settles every charge without a callback.

Fixed in this change:

- `vpay_notchpay::wire::Event` keeps `type` and adds `event`;
- `Event::name()` prefers `event`;
- the adapter's `parse_callback` reads `name()`.

The tests are built from the delivered bodies, byte for byte:

| Test                                                             | Crate                   |
| ---------------------------------------------------------------- | ----------------------- |
| `a_delivered_payment_webhook_is_named_under_event`               | `vpay-notchpay`         |
| `the_registration_ping_is_named_under_type`                      | `vpay-notchpay`         |
| `event_wins_when_a_body_names_itself_both_ways`                  | `vpay-notchpay`         |
| `a_webhook_the_sandbox_delivered_names_our_charge` (by `trxref`) | `vpay-adapter-notchpay` |
| `the_registration_ping_is_not_a_payment_event`                   | `vpay-adapter-notchpay` |

The documented `type` shape still parses: the existing tests are unchanged.

**Mutation:** with `name()` reading `type` only, the adapter's
`a_webhook_the_sandbox_delivered_names_our_charge` fails (67 passed,
1 failed); restored, 68 of 68.

## 6. The stall (§ 10 item 25)

On 2026-10-23, every payment processed after the API-created endpoint
existed stayed `processing` for 3 to 55 minutes. Today, with that endpoint
deleted and the dashboard one in its place, both payments were final within
about 5 s.

That fits the endpoint being the cause. It is two payments, and a deleted
endpoint is not the only thing that changed between the days, so it remains
a correlation.

## What was not measured

- **A delivery to a running vpay**, through its route, its counter and its
  poll pull-forward. That needs a vpay a public URL can reach, and a vpay
  that can reach Notch Pay (runbook Part B).
- **Whether Notch Pay needs a `2xx` on `verification.test`** to keep an
  endpoint. vpay answers it `400`.
- **Retries.** The capture endpoint answered `200` to everything, so no
  retry was provoked.
- **`payment.processing`** (item 17). It was not subscribed, and none was
  seen.
- **Replay.** The signature carries no timestamp. A captured webhook would
  verify again; this was not tried.
- **Production, and a live key.**

## Cleanup

- The dashboard endpoint `ep_test.lBuAjask69aD7YBm` is the maintainer's and
  is left in place.
- The capture inbox is kept until it expires.
- The two sandbox payments are final.

## Gates

On this branch, on the code as committed.

| Check                                                                                                | Result                                                                                                                                   |
| ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test -p vpay-notchpay -p vpay-adapter-notchpay --lib`                                         | 38 and 68 passed, 0 failed, 0 ignored                                                                                                    |
| the mutation above                                                                                   | the delivered-webhook test fails, then passes                                                                                            |
| `cargo clippy -p vpay-notchpay -p vpay-adapter-notchpay --all-targets -- -D warnings`                | clean                                                                                                                                    |
| `rustfmt --check` on the probe                                                                       | clean                                                                                                                                    |
| `cargo test -p vpay-notchpay -p vpay-adapter-notchpay -p vpay-config --lib`                          | 38, 68 and 148 passed, 0 failed, 0 ignored                                                                                               |
| `cargo test -p vpay-notchpay --example sandbox_probe`                                                | 6 passed, 0 failed, 0 ignored                                                                                                            |
| `cargo test --doc -p vpay-notchpay`                                                                  | 1 passed, 0 failed, 0 ignored                                                                                                            |
| `cargo clippy -p vpay-notchpay -p vpay-adapter-notchpay -p vpay-config --all-targets -- -D warnings` | clean                                                                                                                                    |
| `cargo fmt --all --check`                                                                            | clean                                                                                                                                    |
| `just verify`                                                                                        | "verify: ok — the fifteen gates above passed"                                                                                            |
| `npx prettier@3.9.6 --check` on the sixteen changed and new Markdown files                           | clean                                                                                                                                    |
| `markdownlint-cli@0.45.0`, MD013 off, before and after this change on the same files                 | 13 findings before and 13 after, the same rules in the same files (12 `MD029` in RFC-0013, 1 `MD018` in `backend.md`); none on this page |
