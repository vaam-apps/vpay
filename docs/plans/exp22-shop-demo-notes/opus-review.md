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

Fixed: the row now reads `unpaid` and carries a `note` rendered both in the
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

Fixed with the `href` setter. `stubPopup` in `popup.test.ts` now models a
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

Fixed: both written into `sdks/stripe-js/README.md`, and the shop's return
page now passes `close: false` — the opener closes the window itself on a
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
both copies would still agree. Added a case that reads the rail stubs' own
mapping files.

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
