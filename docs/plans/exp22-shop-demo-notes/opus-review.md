# exp22 — sabotage review of `claude/exp22-shop-demo`

Review of `ce88aae` (12 commits on `06e27f9`), 2026-09-06. The implementer's
own account is [opus.md](opus.md); this file records what a second pass
could and could not confirm, and what it changed.

The review's own bias, stated first: it was looking for the failure mode
[CLAUDE.md](../../../CLAUDE.md) names — a repository made to *look* more
finished than it is. Two of the findings below are exactly that, and both are
of the same shape as one the branch itself found and wrote up honestly (D5,
`cancelled`).

## Findings

Severity: **gate-hole** / **correctness** / **rule-break** /
**misleading-claim** / **nit**.

### R1 — `237600000400` cannot make an order `failed` (misleading-claim)

The MTN test-number table — in `examples/shop/README.md`, in
`src/lib/test-numbers.ts`, and on the panel a buyer reads on `/checkout` —
promised that paying with `237600000400` makes the shop's order `failed`
with `invalid_payer`.

It cannot. MTN refuses that MSISDN on the **submit** (`requesttopay.json`,
`400 PAYER_NOT_FOUND`), so vpay commits the failure through
`vpay_api::v1::payment_intents::persist_decline`, which writes the charge and
`last_payment_error` on the intent and **emits no event**.
`payment_intent.payment_failed` is written by
`vpay_db::settlement::apply_failed` and by nothing else, and only
`vpay_worker`'s poll path calls it. A shop that settles from signed events —
which is this example's entire argument — therefore never learns of the
decline, and the order stays `unpaid`.

This is the same defect class as the branch's own D5 (`cancelled`), found for
one transition and missed for the other. The other four MTN numbers are
decided by the **status query**, which is the worker's path, and are correct
as documented.

Fixed (commit `e56e44a`, which also carries R8): the row now reads `unpaid` and carries a `note` rendered both in the
README and in the panel; `TestNumber.orderStatus` gained `"unpaid"` and a
required `note` for any row that does not settle, pinned by a new case in
`test-numbers.test.ts`. The fact itself is now a **gate**:
`a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read` asserts
the `events` table stays empty for such a decline, so the day vpay emits one,
that test fails and every claim that it does not gets corrected with it.
Surfaced as **D6** below.

### R2 — the checkout panel still showed `cancelled` as if it worked (misleading-claim)

`orders.ts`, `order-actions.tsx`, `README.md` and `opus.md` all say plainly
that vpay emits no `payment_intent.canceled` and that the status is
unreachable. `TestNumbersPanel`'s closing paragraph did not: it said the
order "becomes `cancelled` when the shop cancels its PaymentIntent … and vpay
delivers `payment_intent.canceled`", full stop. So did the pre-click prose on
`/orders/{id}/cancelled` ("the order becomes `cancelled` when the signed
`payment_intent.canceled` event arrives") — the correction was only in the
note shown *after* the button was pressed.

Fixed in both places.

### R3 — a reused popup window threw a `SecurityError` instead of navigating (correctness)

`openCheckoutPopup` navigated with `popup.location.assign(url)`. That is legal
only while the window is still on the `about:blank` this side opened. The
default `windowName` makes reuse the *documented* behaviour, and
`win.open('', name, …)` with an empty url returns an existing window
**without navigating it** — so the second `openCheckoutPopup` of a session
reached that line holding a window already on vpay's origin, where the only
`Location` members an opener may touch are the `href` setter and `replace()`.

The failure landed after `fetchCheckoutUrl` had resolved: `examples/shop` had
already created an order and cleared the cart, the rejection was not a
`CheckoutPopupBlockedError` so the fallback did not fire, and the call's
`message` listener and close poll leaked because `stop()` is only reached
above that line.

Fixed with the `href` setter (commit `38aa6cd`, which also carries R4 and R5's documentation). `stubPopup` in `popup.test.ts` now models a
cross-origin `Location` (`assign()` throws), so pointing the code back at
`assign` fails 17 of the 27 cases rather than none.

### R4 — a comment that said the opposite of its own code (nit)

`windowFeatures` carried "Withheld deliberately: `location=yes` keeps the
address bar visible…" immediately above `"location=yes"`. Reworded.

### R5 — two limits of the popup completion channel, undocumented (correctness)

Neither is a security hole — both fail closed — and neither was written down:

* **"Has an opener" is not "is a vpay popup."** `PopupReturnNotifier` calls
  `notifyCheckoutOpener` on every `/return` load, and the default
  `close: true` closes the window. A shop tab that some *other* page opened
  with `window.open`, paying by ordinary redirect, therefore gets closed on
  the return page. The message itself is harmless there (it is pinned to
  `targetOrigin`, so a third-party opener receives nothing).
* **A `success_url` on a different origin from the opener is a silent
  no-op.** `targetOrigin` defaults to the return page's origin and
  `completionOrigin` to the opener's, so the browser drops the message —
  and `notifyCheckoutOpener` still answers `true`.

Fixed: both written into `sdks/stripe-js/README.md` (in `38aa6cd`), and the
shop's return page now passes `close: false` (`1b493af`) — the opener closes the window itself on a
message it actually recognised, which is the only case in which it should be
closed.

### R6 — "Cypress cannot be run from this branch's environment" is false (misleading-claim)

`opus.md` says so twice, and both times the sentence is load-bearing: it is
the reason D1's Orange race is left open and the reason the repaired specs
were never run. On this machine `pnpm exec cypress install` reports
`Cypress 15.21.1 is installed in ~/.cache/Cypress/15.21.1` and
`pnpm exec cypress verify` passes. See "Cypress" below for what was then
actually run.

### R7 — the demo-run account claims more than it records (misleading-claim)

`opus.md` D1 says "Those five numbers were driven end to end through the
browser on the demo stack and behave as the table says". Its own run table
records three of the five (`…0101`, `…0000`, `…0503`); `…0102` and `…0400`
were not driven — and `…0400` does not behave as the table said (R1).
Corrected in `opus.md`.

### R8 — nothing checks the test-number table against the stub that honours it (nit)

`test-numbers.test.ts` proves the README and the module agree. It does not
prove either agrees with the WireMock mappings, which is where the outcome
actually comes from — the table could name a number no mapping steers and
both copies would still agree. Added a case that reads the rail stubs' own mapping
files and asserts each failing number is keyed on by some mapping's
`request` (never by its `metadata`, which is prose). Decisive: renumbering
`…503` to `…504` in *both* the module and the README leaves the
README-agreement case green and fails this one.

## Maintainer decisions surfaced, not taken

### D6. A decline **at submit** is a terminal transition that emits no event

New with this review; see R1. `persist_decline` and
`vpay_db::settlement::apply_failed` write the same outcome by two different
routes and only the second emits. The consequence is concrete: a merchant
integrating the way this repository tells it to — settle only from a signed
event — cannot learn that a charge was declined at submit, and must poll
`GET /v1/payment_intents/{id}` to find out.

Three ways out, and the choice is not this review's:

1. **Emit it.** A decline at submit is terminal, which is the Step 4 plan's
   own rule for writing an event, and the settlement path already writes
   exactly this type in the same transaction as the status. This looks like
   the intended shape and is a `vpay-api` change.
2. **Say it is not coming**, in `docs/flows/webhooks.md` and `docs/status.md`,
   so a merchant knows to poll after a `409 charge_declined`.
3. **Leave it**, and let every integrator find it the way this review did.

It is the same question as D5 (`payment_intent.canceled`) with a different
transition, and answering one without the other would leave the taxonomy
half-emitted.

## What was run

### `just ci`, on `ce88aae` and on the final head

Green end to end on the review's final head, 2026-09-06, 19:01:49 → 19:16:39,
exit 0. Recipe by recipe:

| Recipe | Result |
|---|---|
| `fmt-check` | ok (`cargo fmt --all -- --check`) |
| `clippy` | ok |
| `verify` | the ten gates. `verify-sdk-parity`: 385 named proving tests all exist, 29 dated gaps, 14 SDK methods over 17 rows. `verify-links`: 811 links in 147 markdown files resolve. `verify-status`: 1 unimplemented item, declared. `verify-toolchain`: 1.98.0 in both places |
| `test-rust` | **1386 run, 1386 passed, 0 skipped** (838 s) |
| `test-doc` | **96 passed, 1 ignored** — the ignored one is `sdks/rust/src/lib.rs - ReadmeDoctests (line 464)` and predates this branch |
| `verify-ignored` | 0 ignored (expected 0), 43 test binaries (expected 43), 1386 total |
| `lint-web` | ok, every package |
| `test-web` | **797 passed, 0 skipped** over 8 packages: `frontends/apps/checkout` 302, `sdks/nodejs` 180, `sdks/stripe-js` 146, **`examples/shop` 96**, `frontends/packages/config` 63, `tokens` 3, `ui` 3, `api-client` 4 |
| `deny` | advisories ok, bans ok, licenses ok, sources ok |

The shop's 96 is 93 plus the three cases this review added; `sdks/stripe-js`'s
146 is 145 plus one.

The one thing else worth recording is the failure that is **not** this
change:
`just test-rust` failed twice on `ce88aae` with
`failed to create a container: Timeout error` and
`Client(CreateContainer(RequestTimeoutError))` — a 120 s bollard timeout, on
two *different* tests, on a host carrying a load average of ~24 with other
agents' suites and compose stacks running. Eight `Created` containers were
left stranded by those timeouts and pruned. This is the same failure the
implementer reported three times and it is environmental: nothing in this
branch touches container start-up, and every test that timed out passed in a
later run.

### Cypress — it runs, and everything passes

The claim that it could not be run here is R6. What actually happened:

* `pnpm exec cypress install` → `Cypress 15.21.1 is installed in
  ~/.cache/Cypress/15.21.1`; `pnpm exec cypress verify` → `Verified Cypress!`
* `just demo_project=exp22-review demo_port=18280 demo_receiver_port=18283
  demo_orange_port=18282 demo_checkout_port=18285 demo_shop_port=18286
  test-e2e` — one compose stack of the review's own, on its own project and
  its own ports, torn down with `down -v` by the recipe.
* **11 tests, 11 passing, 0 failing, 0 pending, 0 skipped, exit 0.**
  `checkout.cy.ts` 1, `dashboard.cy.ts` 3, `shop-hosted.cy.ts` 3 (MTN → paid
  via the webhook; **Orange redirect → paid**, 64.6 s; a payment that does
  not succeed lands on `cancel_url` and never becomes `paid`),
  `shop-embedded.cy.ts` 4 (frame `src` and `frame-ancestors`, MTN inside the
  frame → paid, Orange breaking out to `return_url`, and a refusal to be
  framed by an unregistered origin).

So `ce88aae`'s spec repairs are correct, and its "**Neither spec was run**"
is now "both were, and both pass". **No popup was opened by any of this**:
there is still no spec that opens one, so `docs/sdks/parity.md`'s dated ⛔ and
the 🟡 on the popup row are untouched and remain accurate.

### ZenStack 3, against a real Postgres

Run against a throwaway `postgres:16-alpine` on a port of the review's own,
removed afterwards. Every cell of the implementer's v2→v3 table reproduced:

| Call | Result |
|---|---|
| `product.create` | throws `operation is rejected by access policies` |
| `product.deleteMany({})` | `{count: 0}`; 5 catalogue rows before and after |
| `order.deleteMany({})`, `orderItem.deleteMany({})`, `webhookEvent.updateMany`, `webhookEvent.deleteMany` | each `{count: 0}`; the order row survives and the event's `type` is unchanged |
| one payer reading another's order by id | **allowed** — there is no principal, exactly as `schema.zmodel` and the README say |

`zen migrate deploy` applied all three migrations from empty, and
`zen migrate dev --create-only` afterwards produced a 30-byte "This is an
empty migration" — so the zmodel and the committed migrations agree. The
silent `{count: 0}` on a denied bulk write is documented where a developer
will meet it: the header of `policies.test.ts`, the "ZenStack, honestly"
section of `examples/shop/README.md`, and `docs/status.md`'s ZenStack 3 row.

The brief asked whether "a customer reading another customer's order is
refused". It is not, and that is not a defect of this branch: the shop is
guest checkout with no principal, the zmodel says so in a comment, the README
says so in prose, and `docs/status.md` carries it as a named 🟡 gap. A
`@@allow('all', auth() != null)` there would be a claim of protection this
shop cannot make.

### The worker's first poll (the Orange race)

Confirmed by reading the code, not by re-measuring:
`vpay_api::v1::payment_intents::insert_charge` enqueues `poll_charge` with
`run_at = OffsetDateTime::now_utc()` in the same transaction as the charge —
so `poll_delay(0)` really is the delay before the *second* attempt, and the
implementer's corrected account of D1 (T+449 ms against ~12 s to type) is
right. The stub was left alone, as the brief directs.

## Mutations

Each was applied, the named suite run, and the mutation reverted.

| # | Mutation | Suite | Result |
|---|---|---|---|
| M1 | delete the `event.origin !== completionOrigin` check in `popup.ts` | `sdks/stripe-js` `popup.test.ts` | **2 failed** / 24 passed |
| M2 | delete the `event.source !== popup` check | same | **1 failed** / 25 passed |
| M3 | drop `$use(new PolicyPlugin())` from `src/server/db.ts` | shop `policies.test.ts` | **1 failed** / 5 passed |
| M4 | widen `Product` to `@@allow('all', true)` and regenerate | same | **1 failed** / 5 passed |
| M5 | change the README's `…503` row to `provider_error` | shop `test-numbers.test.ts` | **1 failed** |
| M6 | `provider_unavailable.retryable = false` | shop `failures.test.ts` | **1 failed** / 7 passed |
| M7 | put `popup.location.assign(url)` back | `popup.test.ts` | **17 failed** / 10 passed — 0 before the fix |
| M8 | renumber `…503` → `…504` in **both** the module and the README | shop `test-numbers.test.ts` | README-agreement case **passes**, the new stub cross-check **fails** — which is the whole reason R8 exists |

## Not checked

* **No popup was opened by a real browser**, by this review or by anything
  else. The parity ⛔ is accurate and stays.
* **`ZenStackShopStore` still has no automated test of its own.** The review
  drove it indirectly through the Cypress specs and directly by hand through
  the policy probe above, and asserted nothing about the class itself. The 🟡
  in `docs/status.md` is unchanged and honest.
* **The Orange test numbers still do not work from a browser**, and the
  review did not try to make them: fixing the stub is a maintainer decision
  (D1), and `shop-hosted.cy.ts`'s Orange case reaching `paid` in 64.6 s is
  exactly the race the branch documents, not evidence against it.
* **`payer_declined` is still emitted by no adapter** (the branch's D2);
  nothing here changed that.
* `frontends/apps/checkout` was not touched or reviewed — exp21 owns it.
