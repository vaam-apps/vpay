# 2026-10-22 — Notch Pay's `POST /refunds` answers `403` in the sandbox

## What this is, and what it is not

This page is one sandbox probe of the refunds endpoint described in
[RFC-0012](../../rfc/0012-notchpay-adapter.md) § 7. It bears on § 10 items 6
and 12 (epic [#291](https://github.com/vaam-apps/vpay/issues/291)). **It is
not delivery step 6.**

- The adapter is unchanged: `refund` still answers the declared token
  `NotImplemented("notchpay::refund")`.
- `vpay-server` was not involved.
- Every call was made by `cargo run -p vpay-notchpay --example sandbox_probe
-- refunds`. That is a new phase of the probe, added in this change.

The evidence is the probe's redacted report,
[2026-10-22-notchpay-refunds.report.json](2026-10-22-notchpay-refunds.report.json).
It holds every request body and every answer from two runs: `read`, then
`refunds`. One `authorization_url`, a test-mode hosted-page link, was
replaced by hand with the same placeholder the
[2026-10-21 report](2026-10-21-notchpay-sandbox-run.report.json) uses. No key,
hash key or signing secret is in the report: each of the three values was
searched for and is absent.

## How it was run

- **Who approved it.** The maintainer, in the running session's own chat:
  "let's probe their refund endpoint, just to see".
- **From where.** A Claude Code cloud session, on `master` at `45a244f` plus
  this change's probe. It used the probe's cloud-only
  `--ca-bundle /root/.ccr/ca-bundle.crt`, which is in no shipping code
  (ADR-0003).
- **Keys.** `NOTCHPAY_TEST_PUBLIC_KEY` and `NOTCHPAY_TEST_PRIVATE_KEY`,
  checked by name and by the probe's `test.`/`test_` refusal only. No value
  was printed. As the refunds guide says, every request carried both keys:
  `Authorization` and `X-Grant`. `NOTCHPAY_TEST_HASH_KEY` was not used.
- **When.** 2026-10-22, `read` at 05:10:50 UTC and `refunds` from 05:10:55
  to 05:11:16 UTC. Base URL `https://api.notchpay.co`; channel `cm.mobile`.
- **Mode.** Every payment answer carried `"sandbox": 1`. Every identifier
  carried `.test_` (`trx.test_…`, `cus.test_…`, `pm.test_…`).

## What was sent, and what came back

Twelve requests from the probe, then one by hand. Times are seconds after
the `refunds` run started.

| t    | Request                                    | Answer                                                           |
| ---- | ------------------------------------------ | ---------------------------------------------------------------- |
| 0    | `POST /payments`: 200 XAF, `+237670000000` | `201` "Payment initialized", `trx.test_Z32gAVoIsFnQKrDl0xDcFA1J` |
| 1    | `POST /payments/trx.test_…`                | `202` "Payment is being processed"                               |
| 1, 6 | `GET /payments/trx.test_…`, 5 s apart      | `200`, `processing`, then `complete`; `amounts.total` 200        |
| 6    | `POST /refunds`, plain                     | **`403` `{"code":"403","status":"Forbidden"}`**                  |
| 6    | the identical body again                   | **`403`**, the same body                                         |
| 6    | plain + a `reference` of ours              | **`403`**, the same body                                         |
| 6    | that `reference` again                     | **`403`**, the same body                                         |
| 21   | `GET /payments/trx.test_…/refunds`         | `404` "Payment Not Found"                                        |
| 21   | `GET /payments/trx.test_…`                 | `200`, `complete`, `amounts.total` 200                           |

Before these, the `read` phase made its two usual read-only calls (a
nonexistent reference, `404`; the webhook list, `200`, empty).

The four refund bodies were the guide's fields: `payment` (the `trx.test_…`),
`amount` 50, `reason`, and `metadata.vpay_refund`. The third and fourth also
carried a top-level `reference`, which the guide does not document.

**By hand, at 05:11:47 UTC:** one `GET /refunds?page=1`, with both keys,
through `curl` with the same CA bundle. Its output was filtered to
replace any key value before it was printed. It answered:

```json
{
  "code": 200,
  "status": "OK",
  "message": "Refunds retrieved",
  "totals": 0,
  "last_page": 1,
  "current_page": 1,
  "selected": 0,
  "items": []
}
```

That answer is not in the report: the probe did not make the call.

## What it shows

- **`/refunds` exists in the sandbox.** The list answers `200` in the same
  envelope as `/webhooks`, and it is empty.
- **Creating a refund is refused, every time, before anything is decided.**
  All four `POST /refunds` answered `403` with no `message`. Every other
  refusal in this report and the 2026-10-21 one carried one: 241 of them,
  all `404`s but one `409`. The answer names no field, which suggests the
  body was not the objection. Adding the undocumented `reference` changed
  nothing.
- **Nothing was refunded.** The payment read `complete` with `amounts.total`
  200 both before and after. The refund list was empty. No refund id came
  back to read at `GET /refunds/{id}`, so the probe made no such call.
- **`GET /payments/{id}/refunds` did not find the payment by its `trx.…`
  reference.** It answered `404` "Payment Not Found" for the reference that
  `GET /payments/{id}` had just read. The guide documents that path. Whether
  it wants a different id, or is not served at all, is not known. The
  payment answers carry no `id` to try.

## What it does not show

- **Item 6 stays open.** Replay safety was **not measured**. A `403` before
  any refund exists says nothing about what a repeated request does once
  refunds are allowed. Whether `POST /refunds` works on the live API is
  unknown.
- **Item 12 stays open.** A `403` with no message fits an IP allowlist,
  which item 12 asks about. It equally fits refunds being disabled for this
  account, or refunds needing a verified business, or refunds being absent
  from the sandbox. The answer does not say which. The cloud session's
  egress address is not a stable one an allowlist could name.
- **No refund event** (`refund.created`, `refund.complete`, `refund.failed`)
  was looked for. No webhook can be received without a public URL (runbook
  A4).

## What changes

- **Code:** only the probe, `backends/crates/vpay-notchpay/examples/sandbox_probe.rs`.
  - It has a new `refunds` phase.
  - It has a helper, `refund_ids`, that accepts an id only if it is safe to
    put in a path. Pinned by
    `refund_ids_are_found_under_any_envelope_and_only_if_path_safe`.
  - The adapter, its mapping and its conformance rows are unchanged.
  - `notchpay::refund` stays a declared `NotImplemented` in
    [docs/status.md](../../status.md).
- **Docs:**
  - RFC-0012 § 7 and § 10 items 6 and 12, and
    [adapter-notchpay.md](../../flows/adapter-notchpay.md) § Refunds and
    item 6, each carry a dated note.
  - The [runbook](../../runbooks/notchpay-sandbox-test.md) has an A6 for the
    phase, and a B3b. B3b runs the timeout (`…000003`), canceled (`…000004`)
    and Orange (`+237690000000`) numbers through vpay. That has never been
    done: step 5 ran only `…000000`, `…000001` and `…000002`.
- **A correction on the way:** the step-5 bullet in adapter-notchpay.md
  § Status said `provider_requests` records the 11 refused reads as "no
  answer", and "no change made".
  [#314](https://github.com/vaam-apps/vpay/pull/314) (`6d9dfbf`) changed that
  on 2026-10-21. A refused status read is now recorded as answered:
  `status_code` 0 with its `error_kind`
  ([verification](2026-10-21-provider-requests-refused-answers.md)). The
  bullet now says so. Its second observation, the 24 h before an operator
  hears of a short collection, is unchanged and still a design question.

## Next, for the maintainer

1. Look in the Notch Pay dashboard for an API IP allowlist and for a
   refunds setting.
2. Ask Notch Pay in writing why `POST /refunds` answers `403` for a test
   account, together with item 6's replay question. The probe's request
   bodies are in the report.

When either answer allows a refund, rerun A6. That run is what measures
replay safety.

## Gates

`cargo fmt --check`; `cargo clippy -p vpay-notchpay --examples -- -D
warnings`; `cargo test -p vpay-notchpay --example sandbox_probe`: 6 passed,
0 ignored. The xtask self-checks, with this page staged, are in the pull
request.

## Not run here

- **Nothing through vpay.** `vpay-server`, `vpay-worker` and the adapter
  made no call. This session's egress re-terminates TLS, so step 5's path
  cannot run here (ADR-0003). That includes runbook B3b.
- **No live call.** Production was not called and no live key was used.
- **No second refunds run.** The four requests went out once. Nothing
  suggested a retry would be answered differently.
