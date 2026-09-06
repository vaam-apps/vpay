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
