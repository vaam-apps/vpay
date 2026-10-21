# 2026-10-21 — Notch Pay's sandbox, answered: RFC-0012 step 1

## What this is, and what it is not

This is delivery step 1 of [RFC-0012](../../rfc/0012-notchpay-adapter.md)
(epic [#291](https://github.com/vaam-apps/vpay/issues/291)): the first
sandbox session. It records what Notch Pay's real sandbox returned to § 10's
**(S)** items. The [2026-10-20 attempt](2026-10-20-notchpay-sandbox.md) made
no request; this one did.

**It is not step 5.** Nothing here went through `vpay-server` or the
adapter's `submit`/`query_status`. The calls were made by
`cargo run -p vpay-notchpay --example sandbox_probe`, which builds every body
from `vpay-notchpay`'s wire types and decodes every answer with them. Each
read was also made through `vpay_notchpay::Client`. So
[providers.md](../providers.md)'s `notchpay` Collect and Status read cells
are **not** "proven" by this page. Only step 5 can do that.

The evidence is the probe's redacted report,
[2026-10-21-notchpay-sandbox-run.report.json](2026-10-21-notchpay-sandbox-run.report.json).
It holds every request body and every answer. The `authorization_url`
values (test-mode hosted-page links) were removed by hand. No key, hash key
or signing secret is in it, and none was returned to remove.

## How it was run

- **Who approved it.** The maintainer, in the running session's own chat:
  "Approve all three phases". The approval request had named every call
  first. Before that answer, no request was sent.
- **From where.** A Claude Code cloud session, on `master` at `d14cbe9`,
  with the probe's `--ca-bundle /root/.ccr/ca-bundle.crt`
  ([#302](https://github.com/vaam-apps/vpay/pull/302)). That flag adds the
  egress gateway's CA for the probe's own client only; redirects stay
  refused. It is in no shipping code (ADR-0003).
- **Keys.** `NOTCHPAY_TEST_PUBLIC_KEY` and `NOTCHPAY_TEST_PRIVATE_KEY`,
  checked by name and by the probe's `test.`/`test_` refusal only. No value
  was printed. `NOTCHPAY_TEST_HASH_KEY` was present and unused: item 7 needs
  a received webhook.
- **When.** 2026-10-21, between about 03:30 and 03:55 UTC. Base URL
  `https://api.notchpay.co`; channel `cm.mobile`.
- **Order.** `read` (changes nothing), then `payments` (six payments with
  Notch Pay's documented outcome numbers, amounts 100 to 135 XAF, and one
  repeated reference), then `webhooks` (three endpoints created at a
  `.invalid` URL nobody can receive at, then deletion attempted).

**Mode.** Nothing indicated live mode. Every payment answer carried
`"sandbox": true` (initialize) or `"sandbox": 1` (process, read), and every
webhook endpoint `"sandbox": true`. Every identifier carried `.test_`
(`trx.test_…`, `cus.test_…`, `pm.test_…`, `ep_test.…`).

## The finding that matters most: the path takes Notch Pay's reference only

**RFC-0012 § 10 item 2 is contradicted.** On all six payments:

| Call                                                      | By our reference (a UUID)                                                      | By Notch Pay's `trx.test_…`          |
| --------------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------ |
| `POST /payments/{reference}`                              | `404` `"Payment Not Found"`                                                    | `202` `"Payment is being processed"` |
| `GET /payments/{reference}`                               | `404` `"Payment Not Found"`, every poll (36 per payment, 5 s apart, for 180 s) | `200` `"Transaction retrieved"`      |
| `vpay_notchpay::Client::retrieve_payment` (our reference) | refused, `404`, `"Payment Not Found"`                                          | —                                    |

Our reference is not lost: every payment answer carries it twice, as
`trxref` and as `merchant_reference`, and `reference` is Notch Pay's
`trx.test_…`. That is the direct-charge guide's shape, not
`openapi.yaml` 2.1.0's.

**What this means for the adapter on `master`.** It sends call 2 (process)
and call 3 (read) by `charge.reference_id`
(`vpay-adapter-notchpay/src/lib.rs`, `process` and `query_status`). Against
this sandbox, call 2 gets the `404`, and `mapping::process_refusal` turns a
`404` "Payment not found" on a process call into `Malformed`. **So `submit`
fails before any USSD prompt is sent, and no money moves.** The rail does
not work as written, but it fails closed. The fix (process and read by
Notch Pay's reference, carried in `ref_extra`) is not in this change. Its
crash window is an open decision, below.

## The second finding: a "complete" that collected less

**The "insufficient funds" number read `complete`.** `+237670000001` is
documented as "insufficient funds". Its final read was:

```json
{
  "amount": 107,
  "amounts": { "converted": 107, "currency": "XAF", "rate": 1, "total": 105 },
  "charge": "business",
  "fees": [],
  "status": "complete"
}
```

`amounts.total` equalled `amount` on **all 17 other** answers that carried a
payment: every initialize and process answer, both real successes (MTN
`…000`, 100/100; Orange `…000`, 135/135) and the `failed`, `expired` and
`canceled` reads. It differed only here. The same payment's own initialize
and process answers had `total` 107. With `fees` empty and `charge`
`business`, `total` is not net of a fee. **This is a short collection
reported as `complete`.** It contradicts the number's documented meaning,
and it is now a written question for Notch Pay (below).

`amounts` is not in `openapi.yaml` 2.1.0, so `vpay_notchpay::wire::Payment`
did not model it. The adapter's `require_the_charges_amount` compared
`amount` only, so **once call 2 works, this read would have mapped to
`Succeeded`**: a false success on the money path. It cannot happen on
`master` today, only because call 2 fails first.

**Fixed in this change.** `Payment` now carries `amounts.total`, and a
success requires `amount`, `amounts.total` and `currency` all to be the
charge's. Anything else, including a missing `amounts` or a non-integer
`total`, is `Malformed`, as a wrong `amount` already was. Pinned by
`a_complete_that_collected_less_than_the_charge_is_never_a_success` and
`a_success_with_no_collected_total_is_never_a_success` (unit), and by the
conformance case
`notchpay_a_complete_that_collected_less_than_the_charge_is_never_a_success`
over a WireMock stub transcribed from that read (`REF_SHORT_COLLECTION`).

## RFC-0012 § 10, item by item

| Item | Question (short)                                              | Result                                                                                                                                                                                                               |
| ---- | ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2    | Which reference the payments path takes                       | **Contradicted.** Notch Pay's `trx.…` only; ours is `404` on process and read (above)                                                                                                                                |
| 3    | `cm.mobile` resolves the network; outcome numbers work        | **Answered: yes.** Every MTN (`+23767…`) and Orange (`+23769…`) number was processed on `cm.mobile` and reached the outcome below. The testing page lists them under `cm.mtn`/`cm.orange`; `cm.mobile` honoured them |
| 4    | A repeated `reference` is a `409`; the read returns the first | **Answered, half.** `409`, `"Reference already existing"`, `errors.reference: ["Reference already existing"]`. The read after it, by our reference, is `404`, as every read by our reference is (item 2)             |
| 5    | What the read returns for each outcome number                 | **Answered; one contradicts the documentation.** See the table below. No read names a failure reason                                                                                                                 |
| 7    | Signature over the raw body; header lower-case                | **Could not be measured: no public URL.** No webhook was received, and no third-party capture service was used. Notch Pay's API exposes no delivery log that the probe reads                                         |
| 11   | An `incomplete` payment expires as `pending` does             | **Not measured.** No outcome number produces `incomplete`; it stays a written question                                                                                                                               |
| 13   | `POST /webhooks` returns the endpoint's signing secret        | **Contradicted.** None of the three create answers, nor the list, carries a secret or any field named like one                                                                                                       |
| 14   | `POST /webhooks` refuses a URL the account already has        | **Answered: no.** The same URL twice made two endpoints, both `201`                                                                                                                                                  |
| 15   | Payload under `transaction`/`endpoint`/`endpoints` or `data`  | **Answered: the specification's keys.** `transaction` on every payment answer, `endpoint` on create, `endpoints` on list. No `data`                                                                                  |
| 16   | A webhook endpoint carries `active`                           | **Contradicted, differently.** No `active` flag. There is a `status`: `null` in the create answer, `"active"` in the list                                                                                            |
| 17   | `payment.processing` is a real event                          | **Half answered.** `POST /webhooks` accepted it (`201`). Whether it is ever delivered needs a received webhook (item 7)                                                                                              |

Items 1, 6, 8, 9, 10 and 12 are written questions, not sandbox ones.
ADR-0031's condition 6 keeps item 1 open; nothing here answers it.

### Item 5: what each number read

The final read, by Notch Pay's reference, about 180 s after process:

| Number          | Documented as          | Process answered | Final read                       | What vpay maps it to, after this change            |
| --------------- | ---------------------- | ---------------- | -------------------------------- | -------------------------------------------------- |
| `+237670000000` | MTN success            | `processing`     | `complete`, total = amount       | `Succeeded`                                        |
| `+237670000001` | MTN insufficient funds | `pending`        | **`complete`, total 105 of 107** | `Malformed` (it would have been `Succeeded`)       |
| `+237670000002` | MTN failure (other)    | `processing`     | `failed`                         | `Failed { provider_error }`                        |
| `+237670000003` | MTN timeout            | `processing`     | `expired`                        | `Failed { payer_timeout }`                         |
| `+237670000004` | MTN canceled by user   | `processing`     | `canceled`                       | `Failed { payer_declined }` (was `provider_error`) |
| `+237690000000` | Orange success         | `processing`     | `complete`, total = amount       | `Succeeded`                                        |

`canceled` for `…000004` is the evidence RFC-0012 § 4 named in advance for
moving that row to `payer_declined`, and it moved. The caveat there stands:
the public key can cancel a payment too (ADR-0031). `PRODUCED_FAILURE_CODES`
gained `payer_declined`, and the conformance decline row for `canceled`
moved with it. Nothing maps to `insufficient_funds`, because nothing read as
one.

## Other things the answers showed

- **No `id` on any payment answer.** `openapi.yaml` 2.1.0's payment has one,
  and the adapter recorded `id` as `provider_txn_id`, which would always have
  been `None`. A success now records `id`, or `reference` (`trx.…`) where
  there is none (`a_success_with_no_id_records_notch_pays_reference`).
- **A read's success status is `200`**, `"Transaction retrieved"`. The
  specification documents `202`. The client accepts any 2xx, and decoded
  every one (`decoded_by_vpay: true`).
- **Error bodies carry `code` as a string** (`"404"`, `"409"`); success
  bodies carry a number (`200`, `201`, `202`). The client does not read
  `code`.
- **A process answer carries `"action": "confirm"`**, which is not in the
  specification. Nothing reads it.
- **A webhook endpoint's `url` came back `null`** in the create answers and
  in the list, although every create sent one.
- **`DELETE /webhooks/{id}` answered `404` `"Resource Not Found"`** for all
  three endpoints just created and listed. The path is the one Notch Pay's
  API reference documents. So the probe could not clean up (below).

## Webhook endpoints left on the maintainer's account

The probe created three endpoints and could delete none of them. **The
maintainer must delete them in Notch Pay's dashboard:**

- `ep_test.Afe5yXKW1X4Dvnmb`
- `ep_test.dNa3XwecBWRSXxdw`
- `ep_test.PDfftDv55NkKhPQB`

All three point at `https://vpay-sandbox-probe.invalid/notchpay/webhook`, a
name that cannot resolve, so nothing can be delivered to anyone meanwhile.

## The open decision: the crash window

The fix for item 2 is to keep Notch Pay's `trx.…` reference from the
initialize answer in `ref_extra`, and to process and read by it. Its hole is
a crash between initialize and `ref_extra` becoming durable. The retry's
initialize then gets `409`, and with only our reference vpay can never find
the payment again. If the USSD prompt had already gone out, the payer could
pay and vpay would never read it.

Notch Pay documents `GET /payments?search=` as "Search by reference". If it
finds a payment by our reference, the hole closes. **It was not called**: it
was not in the calls the maintainer approved. Until it is measured, this is
a residual risk of the fix, not a solved problem, and the fix is not in this
change.

## Questions for Notch Pay, in writing

Added to the written list (RFC-0012 § 10, items 18 to 21):

- Why does the "insufficient funds" test number read `complete`, with
  `amounts.total` short of `amount`? On the live API, can a payment read
  `complete` having collected less than `amount`, and is `amounts.total` the
  collected amount?
- Does `GET /payments?search=` find a payment by the merchant's own
  reference (`trxref`)?
- Why does `DELETE /webhooks/{id}` answer `404` for an endpoint the list
  returns?
- Where is a webhook endpoint's signing secret, since `POST /webhooks` does
  not return one?

## Gates and tests

Run on 2026-10-21 on this branch, cut from `master` at `d14cbe9`, on Linux.

- `cargo test -p vpay-notchpay -p vpay-adapter-notchpay`: **32 + 35 + 5
  passed, 0 ignored** (`vpay-notchpay`'s lib, `vpay-adapter-notchpay`'s
  lib, the probe's own tests). Doctests (`cargo test --doc`): 2 passed,
  0 ignored. The three new mapping cases are among the 35.
- `cargo nextest run -p vpay-tests-conformance`: **93 passed, 0 skipped**,
  against WireMock in Docker. That includes 26 `notchpay` rows, among them the
  new `notchpay_a_complete_that_collected_less_than_the_charge_is_never_a_success`
  and the `canceled` decline row, now `payer_declined`.
- `cargo clippy -p vpay-notchpay -p vpay-adapter-notchpay -p
vpay-tests-conformance --all-targets -- -D warnings`: clean.
  `cargo fmt --all -- --check`: clean.
- `just verify`: **all fifteen gates passed**.
- **`verify-ignored` was not run**: a full workspace test build did not fit
  this container's remaining disk. Instead, `cargo metadata --no-deps`
  target lists on this branch and on `master` were compared and are
  identical, so `expected_suites` (53) cannot move. The new conformance case
  is a function in an existing binary, and no test is ignored. CI runs the
  gate.
- **Not run here:** `just ci` as a whole. CI runs it.
- markdownlint-cli 0.49.0 and prettier 3.9.6 on every changed Markdown and
  JSON file: no new findings (MD013 only on tables and on lines that were
  already there).
