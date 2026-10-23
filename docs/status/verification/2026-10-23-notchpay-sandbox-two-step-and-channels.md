# 2026-10-23 — Notch Pay's sandbox, measured for ADR-0032 and RFC-0012 step 8

## What this is, and what it is not

This page reports one sandbox session that measured Notch Pay's side of two
changes. Both were built on 2026-10-23 against a WireMock stub:

- **[ADR-0032](../../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md):**
  initialize and process as two port calls, with a commit between them.
- **[RFC-0012](../../rfc/0012-notchpay-adapter.md) step 8:** `cm.mtn` and
  `cm.orange` chosen from the payer's method type.

It also tried, for the first time with a URL that could receive, to get a
webhook delivered (§ 10 items 7 and 17), and it re-ran the refunds probe
(item 6). It adds § 10 items 22 to 25.

**vpay itself made no call.** `vpay-server` cannot reach Notch Pay from a
cloud session: its client trusts only the vendored `webpki-roots`, and
ADR-0003 forbids adding the gateway's CA. Every call went through
`cargo run -p vpay-notchpay --example sandbox_probe`. That example sends the
same bodies with the same headers as the client, over the same
`vpay_provider::http` configuration plus its cloud-only `--ca-bundle`. Four of
its phases are new in this change: `two-step`, `channels`, `webhook-receive`
and `webhook-update`. **The adapter, the core and the worker are unchanged.**

The evidence is the probe's redacted report,
[2026-10-23-notchpay-sandbox-two-step-and-channels.report.json](2026-10-23-notchpay-sandbox-two-step-and-channels.report.json):
ten runs, every request body and every answer. Three things were replaced by
hand:

- Each `authorization_url` (a test-mode hosted-page link) became the
  placeholder the earlier reports use.
- The capture inbox's address became `<a webhook.site inbox>`.
- Its token became `<inbox token>`.

No key, hash key or token is in the report: each was searched for, by
value, and is absent.

## How it was run

- **Who approved it.** The maintainer, in the running session's own chat,
  2026-10-23. Asked "May I run a Notch Pay sandbox session from this cloud
  session now (test keys only, nothing printed or committed)…", they
  answered "Yes, run it". Asked how to receive a real webhook, they answered
  "A third-party capture service".
- **From where.** A Claude Code cloud session, on `master` at `82a68f4e`
  plus this change's probe, with `--ca-bundle /root/.ccr/ca-bundle.crt`.
- **Keys.** `NOTCHPAY_TEST_PUBLIC_KEY` and `NOTCHPAY_TEST_PRIVATE_KEY`,
  checked by the probe's `test.`/`test_` refusal. No value was printed.
  `NOTCHPAY_TEST_HASH_KEY` was never used, because no webhook arrived to
  check.
- **When.** 2026-10-23, from 18:59:15 to 20:02:52 UTC. Base URL
  `https://api.notchpay.co`.
- **Mode.** Every payment answer carried `"sandbox": 1`, the endpoint
  `"sandbox": true`. Every identifier carried `.test_`.
- **Concurrency.** `two-step` ran for an hour beside the others, each phase
  writing its own report, merged here in start order.

## 1. Two steps: initialize, then process later (ADR-0032; § 10 item 22)

Two payments on MTN's success number were initialized at 18:59:25 UTC and
neither was processed:

| UTC      | Request                                                     | Answer                                                                                                     |
| -------- | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| 18:59:25 | `POST /payments`, A: 151 XAF                                | `201` "Payment initialized", `trx.test_1mYws8ndvCvR0nMVrWGs4cWg`                                           |
| 18:59:25 | `POST /payments`, B: 157 XAF                                | `201`, `trx.test_VEi6b2qyxlnNzvXvMVLrx8oE`                                                                 |
| 18:59:25 | `POST /payments` with **A's reference again**               | **`409`** `{"errors":{"reference":["Reference already existing"]},"message":"Reference already existing"}` |
| 18:59:25 | `GET /payments/{A's trx}` at 0, 60, 305, 900, 1800, 3600 s  | **`pending` every time**                                                                                   |
| 19:04:26 | `POST /payments/{B's trx}`, `cm.mobile`, **300 s after** it | `202` "Payment is being processed"                                                                         |
| 19:04:31 | `GET` B                                                     | **`complete`**, `amounts.total` 157                                                                        |
| 19:59:26 | `POST /payments/{A's trx}`, **3600 s after** it             | `202` "Payment is being processed"                                                                         |
| 20:02:52 | `GET` A                                                     | `processing` (§ 5 below)                                                                                   |

What this says about ADR-0032:

- **A never-processed payment stays `pending` for at least an hour.** That
  is what vpay leaves at Notch Pay when it stops between `prepare` and
  `submit`. The resubmit then processes the committed `trx.…`, and nobody is
  prompted twice.
  - The payment does not expire on its own within the hour, so it stays open
    to whoever holds the public key (ADR-0031's accepted exposure) for at
    least that long.
  - Its eventual expiry was not seen.
- **A process 300 s late is accepted and completes**, as it does at once.
  This is what the worker does with a `prepare`d charge whose prompt never
  left.
- **A process 3600 s late is accepted (`202`).** Whether it would complete is
  not shown, because it fell into § 5's stall.
- **The `409` is what `mapping.rs` already reads as `AlreadyExists`.** It has
  the same status and message as 2026-10-21, and it now also carries
  `errors.reference`.

## 2. `cm.mtn` and `cm.orange` with the outcome numbers (RFC-0012 step 8; § 10 item 23)

From 18:59:50 to 19:06:50 UTC, each number was initialized, processed by its
`trx.…` on the channel shown, and read every 5 s to a terminal status:

| Channel     | Number          | Testing page says        | Process | Read                      | `amounts.total` of `amount` |
| ----------- | --------------- | ------------------------ | ------- | ------------------------- | --------------------------- |
| `cm.mtn`    | `+237670000000` | success                  | `202`   | `processing` → `complete` | 110 of 110                  |
| `cm.mtn`    | `+237670000001` | insufficient funds       | `202`   | `pending` → `complete`    | **111 of 113**              |
| `cm.mtn`    | `+237670000002` | failure (other reason)   | `202`   | `processing` → `failed`   | 116 of 116                  |
| `cm.mtn`    | `+237670000003` | timeout (no response)    | `202`   | `processing` → `expired`  | 119 of 119                  |
| `cm.mtn`    | `+237670000004` | canceled by user         | `202`   | `processing` → `canceled` | 122 of 122                  |
| `cm.orange` | `+237690000000` | success                  | `202`   | `processing` → `complete` | 125 of 125                  |
| `cm.orange` | `+237690000001` | insufficient funds       | `202`   | `pending` → `complete`    | **126 of 128**              |
| `cm.orange` | `+237690000002` | failure (other reason)   | `202`   | `processing` → `failed`   | 131 of 131                  |
| `cm.orange` | `+237690000003` | timeout (no response)    | `202`   | `processing` → `expired`  | 134 of 134                  |
| `cm.orange` | `+237690000004` | canceled by user         | `202`   | `processing` → `canceled` | 137 of 137                  |
| `cm.orange` | `+237670000000` | MTN's number, wrong rail | **422** | stays `pending`           | 140 of 140                  |
| `cm.mtn`    | `+237690000000` | Orange's, wrong rail     | **422** | stays `pending`           | 143 of 143                  |

Each terminal status was read about 5 s after the process. **The statuses
are the ones `cm.mobile` gave on 2026-10-21 and 2026-10-22**, Orange's
failure, timeout and cancel numbers included, which had not been run before.
The adapter's mapping applies unchanged:

- `complete` is `Succeeded`;
- `failed` is `provider_error`;
- `expired` is `payer_timeout`;
- `canceled` is `payer_declined`.

**The "Testing page says" column contradicts four of this repository's
pages.** The runbook, `adapter-notchpay.md`, `providers.md` and the 2026-10-22
B3b record said Notch Pay documents no Orange failure number. The testing
page, read again on 2026-10-23, gives the five suffixes `…000000` to `…000004`
"for each operator", with Orange's prefix `+23769`. The earlier reading took
Orange's success number only. Whether the page changed since 2026-10-20 is
not known. The living pages are corrected; the B3b record is left as it was
written.

The "insufficient funds" numbers again read `complete` for 2 XAF less than
asked. The adapter refuses that as `NeedsReconciliation`, which the worker
escalates to `unresolved` (§ 10 item 18).

**A number from the other network is refused at `process`**:

```json
{"code":"422","errors":{"phone":["Invalid CM Orange Money"]},"message":"Invalid CM Orange Money","status":"Unprocessable Content"}
{"code":"422","errors":{"phone":["Invalid CM Mobile Money"]},"message":"Invalid CM Mobile Money","status":"Unprocessable Content"}
```

The first is MTN's number on `cm.orange`, the second Orange's on `cm.mtn`.
In both, the initialize had succeeded, so the payment stays `pending` at
Notch Pay (item 22).

In vpay, `process_refusal` maps this `422` to `Rejected { provider_error }`,
carrying the field name `phone`. The charge fails, and the payer can retry
with the other network. `mapping.rs` says why it is not `invalid_payer`: "the
errors page names fields only by example". The `errors.phone` above is the
first measured example. **Not changed here.**

## 3. Refunds (§ 10 item 6): not re-measured

The `refunds` phase initialized and processed its payment at 19:13:46 UTC,
on MTN's success number. The payment read `processing` for 120 s and the
probe, as written, then sent no `POST /refunds`. It was in § 5's stall. The
2026-10-22 answer (`403` "Forbidden" to every `POST /refunds`) stands
unrepeated.

## 4. Webhooks (§ 10 items 7, 17, 20; new item 24): nothing arrived

**The receiver.** A webhook.site inbox, created at 19:01 UTC to answer `500`
on purpose, so that a retry would show as a second request.

| UTC      | Request                                                                         | Answer                                                                                     |
| -------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| 19:07:14 | `GET /webhooks`                                                                 | `200`, `totals: 0`                                                                         |
| 19:07:14 | `POST /webhooks`: the inbox URL, the five payment events + `payment.processing` | `201`, `ep_test.unEBL3JnHZF0fF5B`, all six events, **`"url": null`**, `"status": "active"` |
| 19:07–10 | two payments (MTN success, MTN failure), processed                              | both stayed `processing` (§ 5)                                                             |
| 19:16:19 | `GET /webhooks/ep_test.unEBL3JnHZF0fF5B`                                        | **`404`** "Resource Not Found"                                                             |
| 19:16:19 | `PUT /webhooks/ep_test.unEBL3JnHZF0fF5B`, the URL again                         | **`404`** "Resource Not Found"                                                             |
| 19:28:19 | `DELETE /webhooks/ep_test.unEBL3JnHZF0fF5B`                                     | **`404`** "Resource Not Found"                                                             |
| 19:28:19 | `GET /webhooks`                                                                 | `200`, the endpoint still listed, `"url": null`                                            |

**Nothing reached the inbox** between 19:07:14 and 19:28:25, 21 minutes. That
included no `payment.created`, which the initialize at 19:07:14 should have
fired if the endpoint had a URL. The inbox was then deleted (`204`, and `404`
on a read afterwards).

What this changes:

- **Item 24, contradicted.** `POST /webhooks` with the documented body
  (`url`, `events`, `description`) stores no URL in the sandbox. vpay's
  `ensure_callbacks` registers its endpoint with exactly this call. It
  recognises its own endpoint by description, because the URL is never
  echoed. So on this evidence, an endpoint vpay registers delivers nothing.
  Callbacks are hints and the poll ladder settles every charge without one,
  so no money depends on them.
- **Item 20, widened.** `GET`, `PUT` and `DELETE` by id all answer `404`. An
  endpoint created through the API can be neither repaired nor removed
  through it.
- **Items 7 and 17, still not measured.** No delivery, so no signature, no
  header, no retry policy and no `payment.processing` event were seen. The
  hash key was not used.

## 5. The `processing` stall (§ 10 item 25)

| Payment                        | Processed (UTC) | Read at 20:02:52 |
| ------------------------------ | --------------- | ---------------- |
| MTN success, `webhook-receive` | 19:07:14        | `processing`     |
| MTN failure, `webhook-receive` | 19:10:16        | `processing`     |
| MTN success, `refunds`         | 19:13:46        | `processing`     |
| A, `two-step`, at 3600 s       | 19:59:26        | `processing`     |

**Every payment whose `process` was accepted after 19:07:14 is still
`processing`**, 3 to 55 minutes later. Every one of the eleven accepted
between 18:59 and 19:06 reached a final status within about 5 s. The only
change seen between the two groups is the null-URL endpoint, created at
19:07:14. **That it is the cause is not shown.** It cannot be removed through
the API to test it.

**For the next sandbox run:** delete `ep_test.unEBL3JnHZF0fF5B` in the
dashboard first. Then run `read` to see an empty list, and re-read these
four payments.

## Cleanup

- **The Notch Pay endpoint `ep_test.unEBL3JnHZF0fF5B` is still on the
  sandbox account.** `DELETE` answered `404`. It must be deleted in the
  dashboard, as the 2026-10-21 run's three were.
- The webhook.site inbox was deleted. Nothing had been delivered to it.
- The orphan payments are sandbox ones and are left as they are: the two
  wrong-network `pending` ones and the four `processing` ones.

## What was not measured

- **vpay-server against the sandbox.** Neither the two-step split nor
  `cm.mtn`/`cm.orange` has been sent by vpay itself. That needs a machine
  whose egress presents Notch Pay's own certificate (runbook Part B).
- **When a never-processed payment expires**, beyond the hour watched.
- **Whether a process an hour late completes**, because § 5 confounds it.
- **Webhook delivery, signature format and retries** (items 7, 17), because
  of § 4.
- **Refunds** (item 6), because of § 5.
- **Production.** Nothing here touched a live key.

## Gates

Run on this branch, on the code as committed.

| Check                                                             | Result                                                       |
| ----------------------------------------------------------------- | ------------------------------------------------------------ |
| `cargo fmt --all --check`                                         | clean                                                        |
| `cargo clippy -p vpay-notchpay --all-targets -- -D warnings`      | clean                                                        |
| `cargo test -p vpay-notchpay`                                     | 42 passed (35 unit, 6 probe, 1 doctest), 0 failed, 0 ignored |
| `just verify`                                                     | ok, all fifteen                                              |
| `prettier@3.9.6 --check` over the changed Markdown and the report | clean                                                        |
| vpay-skills `node tools/verify-coverage.mjs` against this tree    | ok: 21 skills, 265 vpay paths, 48 feature pages, 0 exempt    |
