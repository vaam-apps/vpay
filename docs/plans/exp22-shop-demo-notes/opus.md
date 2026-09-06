# exp22 — the shop example: three integration modes, first-class failure outcomes, fake test numbers, ZenStack 3

Working notes for the branch `claude/exp22-shop-demo` (base `06e27f9`).
Written as the work happened; the parts that are *claims about what runs* are
in `docs/status.md` and in the flow docs, not here.

## Requests to other tracks

### To `frontends/apps/checkout` (exp21): a popup has no framer

`src/lib/frame.ts`'s `createFrameChannel` returns `null` when
`win.parent === win`, and posts every message to `win.parent`. Both are
right for an iframe and both make the checkout page **silent inside a
popup**, where `window.parent === window` and the opener is `window.opener`.

The popup integration this branch adds therefore does **not** frame vpay's
page. It loads the **hosted** session's own `url` in a window the merchant
opened, and the completion signal comes from the merchant's own
`success_url` page calling `notifyCheckoutOpener` — see
`sdks/stripe-js/src/popup.ts` for the full reasoning.

If that track wants vpay's page to be able to report completion directly to
a popup opener, the change is in `createFrameChannel`: fall back to
`win.opener` when there is no framer, and resolve the target origin the same
way `resolveParentOrigin` does today (from the `frame-ancestors` the tenant
configured, which is the same allowlist a popup opener would have to be on).
**Nothing in this branch depends on that happening**, and the popup surface
is honest without it; it would simply let the popup use the embedded page
instead of the hosted one.

This is written down rather than done, because the brief gives that
directory to exp21.

## Maintainer decisions surfaced, not taken

### D1. Orange's test numbers lose a race to the worker, and do not work from a browser

Orange is a redirect rail: the number never reaches vpay, so the demo steers
its outcome from a form on the **stub's** hosted page, which arms a WireMock
scenario the later `transactionstatus` query reads.

That mechanism is right and is proven —
`a_test_number_typed_on_the_rails_hosted_page_reaches_the_documented_outcome`
drives the real page and the real form against a real container and gets the
documented outcome. **It nonetheless does not work from a browser**, and the
first draft of this document got the reason's size wrong.

The first draft said "about ten seconds", reading `poll_delay(0)` = 10 s as
the delay before the first status query. It is not.
`vpay_api::v1::payment_intents`'s confirm handler enqueues the `poll_charge`
job with `run_at = OffsetDateTime::now_utc()`, and `vpay_worker`'s idle sleep
is one second; the ladder governs the delay before the **second** attempt. So
the first `transactionstatus` lands about a second after the submit commits,
`transactionstatus.json`'s priority-10 catch-all answers `SUCCESS`, the charge
settles, and vpay stops asking.

**Measured on the running demo stack on 2026-09-06**, from `wiremock-orange`'s
own request journal:

| Event | Offset |
|---|---|
| `POST …/v1/webpayment` (the submit) | T |
| `GET /stub-hosted-page/{token}` (the payer lands) | T + 44 ms |
| `POST …/v1/transactionstatus` (the first poll) | **T + 449 ms** |
| `GET /stub-hosted-page/{token}/pay?msisdn=237600000400` (the payer chooses) | T + 11.96 s |

The order came back **paid**. That is the false green this repository is
written against, so it is on the shop's own checkout panel, in the README, in
the runbook, in `docs/status.md` and in the mapping's own metadata — none of
which now claims these numbers work.

Three ways to close it, each a maintainer's call:

1. **Answer `PENDING` while a payer is on the page.** Arm on the hosted page's
   `GET` — which the journal above shows arrives 405 ms *before* the first
   poll, so it would win — and disarm on the way out. The catch is the way
   out: a payer who leaves by the `#pay` **link** never touches the stub
   again, so nothing disarms and that charge polls `PENDING` for ever. A
   bounded chain of scenario states (`PENDING` a fixed number of times, then
   fall through) fixes that and costs the Orange happy path a rung or three of
   the ladder — roughly a minute — which `shop-hosted.cy.ts` and
   `shop-embedded.cy.ts` would absorb inside their 120 s outcome timeout, or
   would not. ~~**Cypress cannot be run from this branch's environment**
   (`CYPRESS_INSTALL_BINARY=0`; the binary needs a CDN this network does not
   reach)~~ — **wrong, corrected in review on 2026-09-06: the binary is
   already on this machine.** `pnpm exec cypress install` answers "Cypress
   15.21.1 is installed in ~/.cache/Cypress/15.21.1" and `cypress verify`
   passes; `just … test-e2e` ran all four specs green against a stack of its
   own (see "The review's Cypress run" at the end of this document). What
   remains true is the second half of the sentence: changing the timing of a
   stub that `compose.yml`, CI's e2e job and both Rust suites share is a
   maintainer's decision and not one to take inside this task, and the
   measurement below is unaffected either way.
2. **Enqueue the first poll with a delay.** One line in the confirm handler,
   and the wrong place for it: the immediate first poll is a deliberate
   property (`docs/flows/crash-safety.md` — a charge is asked about as soon as
   it exists), and slowing it to make a demo comfortable is the class of
   change ADR-0003 exists to refuse.
3. **Leave it, and say so everywhere a reader could be misled.** What this
   branch did.

MTN is unaffected and needs none of this: a push rail carries the MSISDN in
the merchant's own submit, so the steering happens before the charge exists
and there is no window to lose. ~~Those five numbers were driven end to end
through the browser on the demo stack and behave as the table says.~~
**Corrected in review, 2026-09-06.** Three of the five were driven — the run
table below records `…0101`, `…0000` and `…0503` and no others — and one of
the remaining two did **not** behave as the table said: `…0400` is refused on
the *submit*, which vpay commits through `persist_decline`, which emits no
event, so the shop's order stays `unpaid` where the table promised `failed`.
See R1 and D6 in [opus-review.md](opus-review.md).

### D2. `payer_declined` is a code no adapter emits

`vpay_core::FailureCode::PayerDeclined` — "the payer answered, and refused" —
is in the core's eleven, is `payer_actionable`, and is produced by **neither**
`vpay-adapter-mtn-momo` nor `vpay-adapter-orange-money`: MTN's nine-row
`FAILURE_REASONS` has no entry for it and Orange documents no sub-reason for
`FAILED` at all. So no test number can reach it and the shop's copy for it is
unreachable.

That is a fact about the two rails, not a defect, and it may be the right
state. It is written down here because the alternative readings — that MTN's
`COULD_NOT_PERFORM_TRANSACTION` should map to it (it should not: that row is
annotated "the payer never entered their PIN", which is `payer_timeout`), or
that the code should be removed from the core — are both maintainer calls.

### D3. Does the browser SDK belong in the ADR-0015 parity matrix?

`docs/sdks/parity.md` already carries a one-column `sdks/stripe-js` table,
added before this branch, and the popup rows were added to it. ADR-0015 is
about the **merchant** SDKs being at parity with each other; a browser SDK
has no peer to be at parity with, so those rows are a record rather than a
comparison. Whether the ADR should say so — or whether that table should move
out of this document — is not something this branch decided.

### D4. A popup that frames vpay's page rather than loading the hosted one

See the request to `frontends/apps/checkout` above. Nothing depends on it.

### D5. `payment_intent.canceled` is a documented event type nothing emits, so the shop's `cancelled` status is unreachable

**Measured on the running demo stack, 2026-09-06.** `examples/shop`'s
"Cancel this payment" button calls `POST /v1/payment_intents/{id}/cancel`
through `@vaam-apps/vpay-sdk`. It works: the intent's row in vpay's own
database read `canceled` immediately afterwards. **No event was written** —
the `events` table gained nothing, and the shop's order stayed `unpaid`.

That is not a defect in the call. `docs/status.md`'s "Events written by the
worker" row says outright that three types are written and only three:
`payment_intent.succeeded`, `payment_intent.payment_failed` and
`checkout.session.expired`. `payment_intent.canceled` is in migration `0018`'s
enum, in `docs/flows/webhooks.md`'s list, in both merchant SDKs'
`KnownEventType` vocabularies and in this shop's `SETTLING_EVENTS` — and is
produced by nothing.

The consequence for a merchant is concrete and is the reason this is written
down rather than left to be discovered: **there is no way for a merchant to
learn that an intent it cancelled was cancelled**, short of polling
`GET /v1/payment_intents/{id}`. A shop that shows a settled status only from
a signed event — which is the discipline this whole example exists to
demonstrate — therefore cannot show `cancelled` at all.

Three ways out, and the choice is not this branch's to make:

1. **Emit it.** A cancel is a terminal transition, which is exactly the rule
   the Step 4 plan's decision 4 gives for writing an event, and the two the
   settlement path writes are written inside the same transaction as the
   status. This looks like the intended shape and is a `vpay-api` change.
2. **Say it is not coming**, and strike `payment_intent.canceled` from
   `docs/flows/webhooks.md`'s list and from both SDKs' vocabularies — at
   which point a merchant knows to poll.
3. **Leave it**, and let every integrator find this the way this branch did.

What this branch did **not** do, deliberately: make the shop write
`cancelled` locally when its own cancel call returns. That would be the
example deciding a settled status from its own request rather than from a
signed event, and the shop's entire argument is that a merchant must not.
The button, the procedure and the copy are all still there, and all three now
say what actually happens.

## The demo run, 2026-09-06

One stack, brought up once and torn down: `just demo_project=exp22-demo
demo_port=18180 demo_receiver_port=18181 demo_orange_port=18184
demo_checkout_port=18182 demo_shop_port=18183 demo-up`, on its own project so
it shared nothing with the `vpay-demo` stack already on the machine. The first
`up` failed in buildkit on `Could not resolve host: index.crates.io` while
building `vpay-worker`; a plain `docker run alpine` resolved the same host, so
it was transient and the retry built cleanly.

**No PNG screenshots were captured**, and that is a limitation of this
environment rather than a choice: the browser available here renders into a
pane and returns images into the transcript, and nothing in the toolchain
writes an image file ~~(Cypress's binary is not installed —
`CYPRESS_INSTALL_BINARY=0`, and its CDN is unreachable from here)~~. What is
recorded instead is what each page actually said, which is the thing a
screenshot would have been evidence *of*. **The parenthesis is wrong and was
corrected in review on 2026-09-06:** Cypress 15.21.1 is installed on this
machine and runs. It writes screenshots only for *failing* tests, so a green
run still produces none — but "the toolchain cannot run Cypress" was not the
reason, and it was the reason given twice more in this document.

### `vpay-shop`'s first log lines — ZenStack 3 in a read-only container

```
vpay-shop: applying migrations
Prisma schema loaded from ../tmp/zenstack/~schema.prisma
Datasource "db": PostgreSQL database "shop", schema "public" at "postgres:5432"
3 migrations found in prisma/migrations
Applying migration `20260904091557_init`
Applying migration `20260904091600_seed_catalogue`
Applying migration `20260906120000_optional_email_and_failure_columns`
```

The `../tmp/` in the second line is the entrypoint's copy doing its job: the
CLI derives that Prisma schema beside the zmodel, and `/app` is read-only.

### What was driven, and what each page said

| # | Path | Result |
|---|---|---|
| 1 | `/checkout` | Surface switch renders all three (`hosted` marked as the configured one); test-number panel renders both rails, the five MTN rows, the three Orange rows and all four "no number produces X" notes |
| 2 | Redirect + MTN `237600000101` | vpay: "There was not enough money in the account." Order → `failed`, `failure_code = insufficient_funds`, buyer's sentence "Not enough money in the wallet", **"Try again" offered** |
| 3 | Redirect + MTN `237600000000` | vpay: "Payment received." Order → `paid` |
| 4 | Redirect + MTN `237600000503` (**new**) | vpay: "The payment provider could not be reached." Order → `failed`, `failure_code = provider_unavailable` |
| 5 | Redirect + Orange `237600000400` | Order → **`paid`**. The race in D1 above; this is where it was measured |
| 6 | "Try again" on #4 | New order, new session (`cs_mw9ms8…` against the failed order's `cs_zj8zf2…`), payer sent to vpay's page |
| 7 | Embedded | Frame `src` = `…/e/{cs_id}?key=pk_…#cs_…_secret_…`, `sandbox="allow-scripts allow-same-origin allow-forms"` (no `allow-top-navigation`), height grown to `179px` — so `vpay:resize` crossed the boundary. The payment **inside** the frame was not driven: vpay's `/e/` route correctly refuses to render top-level (`refused_embed`), and this browser cannot script a cross-origin frame. That is `shop-embedded.cy.ts`'s job |
| 8 | Popup | The synthetic click carries no user activation, so `window.open` returned `null`, `CheckoutPopupBlockedError` was raised and **the fallback ran**: the order was created in `hosted` mode and the tab navigated to vpay. Which is worth having — it is the path a merchant most needs to work — but it means **the popup itself was still not opened by anything, here or in any test** |
| 9 | "Cancel this payment" on an unpaid order | The call succeeded and the intent read `canceled` in vpay's database. No event, order still `unpaid`. D5 above |

Three orders were placed with **no e-mail at all**; every one stored `NULL`
and every page rendered "not given — optional, see the checkout page".

### The orders left behind

```
cmtpyhj6o000001o4cnfd1tg3|failed|insufficient_funds|(null)
cmtpyk48b000201o40y0ebkx3|paid|-|(null)
cmtpyuahc000001mhd0730va9|paid|-|(null)
cmtpyvsrb000201mh0cum53dl|unpaid|-|(null)
cmtpywqpf000401mh6gak72fy|failed|provider_unavailable|(null)
cmtpyxm34000601mhgrtr0mt6|unpaid|-|(null)
```

### `wiremock-orange`'s journal for the Orange run

```
…15285  POST /orange-money-webpay/dev/v1/webpayment
…15329  GET  /stub-hosted-page/pay-ba7bc37a-…?return=…&cancel=…
…15734  POST /orange-money-webpay/dev/v1/transactionstatus     <- 449 ms after the submit
…27241  GET  /stub-hosted-page/pay-ba7bc37a-…/pay?…&msisdn=237600000400
```

## The review's Cypress run, 2026-09-06

Added by the sabotage review (`opus-review.md`, R6). `pnpm exec cypress
install` found `Cypress 15.21.1` already in `~/.cache/Cypress`, `cypress
verify` passed, and

```
just demo_project=exp22-review demo_port=18280 demo_receiver_port=18283 \
     demo_orange_port=18282 demo_checkout_port=18285 demo_shop_port=18286 \
     test-e2e
```

built the stack and ran every spec against it. **11 tests, 11 passing, 0
failing, 0 pending, 0 skipped**, exit 0, stack torn down with `down -v`:

| Spec | Tests |
|---|---|
| `checkout.cy.ts` | 1 — the MTN push through `@vaam-apps/vpay-stripe-js` settles to `succeeded` |
| `dashboard.cy.ts` | 3 |
| `shop-hosted.cy.ts` | 3 — MTN on vpay's page → `paid` via the webhook; **Orange redirect → `paid`** (64.6 s); a payment that does not succeed lands on `cancel_url` and never becomes `paid` |
| `shop-embedded.cy.ts` | 4 — frame `src` and `frame-ancestors`, MTN inside the frame → `paid`, Orange breaking out to `return_url`, and a refusal to be framed by an unregistered origin |

So the two specs this branch repaired without running are green against a
real browser and a real stack, which is what `ce88aae`'s "**Neither spec was
run**" could not say. **Nothing changed about the popup**: there is still no
spec that opens one, so the ⛔ in `docs/sdks/parity.md` stands unaltered.
