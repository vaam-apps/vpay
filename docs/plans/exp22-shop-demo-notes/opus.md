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

### D1. Orange's ten-second window on the demo's test numbers

Orange is a redirect rail: the number never reaches vpay, so the demo steers
its outcome from a form on the **stub's** hosted page, which arms a WireMock
scenario the later `transactionstatus` query reads.

vpay's first status query is `vpay_worker::poll_delay(0)` — ten seconds —
after the submit commits. A payer who has not submitted that form by then is
answered by `transactionstatus.json`'s priority-10 catch-all, `SUCCESS`, the
charge settles, vpay stops polling, and **the demo shows a paid order for a
number the operator chose to make fail**. That is the false green this
repository is written against, and it is not closed here.

Three ways to close it, each a decision rather than a defensible default:

1. **Answer `PENDING` while a payer is on the page.** Arm on the hosted
   page's `GET` and disarm on the way out. It requires the `#pay` link to
   disarm, which means changing an href that
   `the_stub_hosted_page_links_to_the_return_url_the_submit_carried`
   (`backends/tests/integration`) and `shop-hosted.cy.ts` both assert on
   byte for byte — and if the disarm ever fails to fire, the failure mode is
   a charge that polls `PENDING` for ever, which is worse than the race.
2. **Lengthen the first rung of the poll ladder for a demo profile.**
   `poll_delay` is a pure function in `vpay-worker`, not configuration, so
   this is a code change to make a demo comfortable — the class of change
   ADR-0003 exists to refuse.
3. **Leave it, and document it.** What this branch did. The window is stated
   in `demo-outcomes.json`, in `examples/shop/README.md`'s Orange table and
   in `docs/status.md`; the operator's check is the order's `failure_code`,
   not the walk appearing to work.

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
