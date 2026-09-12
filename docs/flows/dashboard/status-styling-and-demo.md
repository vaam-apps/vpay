# The dashboard — Status: the restyle, the demo stack, and the second payments list

_Split out of [docs/flows/dashboard.md](../dashboard.md) on 2026-09-11 by exp57, which broke a 865-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

**Restyled, 2026-09-07 (exp26 Lane D):** the scaffold's `app/layout.tsx` and
`app/page.tsx` onto `@vpay/ui`'s components — `styling_files` 2 → 0,
`class_tokens_distinct` 17 → 0 in both files (`exp26-plan-count.sh`); Tailwind
4.3.3 + daisyUI 5.7.28 + `@base-ui/react` 1.8.0, replacing the `corporate`
theme with `bumblebee`.

**Reviewed, 2026-09-07** ([../plans/exp26-notes/lane-d-review.md](../../plans/exp26-notes/lane-d-review.md)).
Two of the four findings bear on this document's own claims. (1) The rewrite
**dropped the `<main>` landmark** the scaffold had: axe-core's `region` rule
went from 0 violations to 1 on the real rendered `<body>`, and every other
gate — 9/9 vitest, lint, typecheck, `next build`, `dashboard.cy.ts` — stayed
green. `<main>` is back and `frontends/apps/dashboard/src/a11y.test.tsx`
gates it. (2) **This document's own nav rule** — "the navigation is only ever
allowed to link to slices that exist" — was enforced by nothing: an
`<a href="/payments">` in the layout left the whole suite green. It is now a
test that resolves every internal `href` against `app/**/page.tsx` on disk,
and since exp28 it checks the `NAV_LINKS` constant as well, so a
conditionally-rendered link cannot slip past it. Neither finding changes what
this app claims to do; both are cases of a rule this repository states being
checked by nobody.

**The demo stack's dashboard port became a variable, 2026-09-10 (issue #78).**
`demo_dashboard_port` (default 3000) is threaded through the publication in
`compose.demo.yml` — still `!override` and still bound to `127.0.0.1`, because
what is behind it is a real staff sign-in form — the app's
`VPAY_DASHBOARD_REDIRECT_URI`, the generated overlay's registered
`redirect_uris`, Cypress's `baseUrl` and `test-e2e`'s readiness probe, so two
demo stacks can each serve a dashboard. It changes nothing about the flow: the
two OAuth legs still have to spell one string identically, and `gen-demo-keys`
regenerating the overlay when the variable moves is what keeps them doing so.
Measured in [../plans/exp35-dashboard-port-notes/opus-review.md](../../plans/exp35-dashboard-port-notes/opus-review.md).

**A second payments list exists in the schema and nothing serves it,
2026-09-11 (exp54).** `schemas/vpay.cstack` declares
`procedure searchPaymentIntents(page: PageInput, filter:
PaymentIntentListFilter): Page<PaymentIntentSummary>` and `vpay-db`
implements it — offset paging, the same three filters `GET
/dash/v1/payment_intents` accepts, a summary row carrying no `metadata` and
no `client_secret_suffix`, and a tenancy predicate read from the caller's
context rather than from its arguments. It is **not** wired to this app and
not to any route: no CrateStack router is mounted in this workspace, and the
only `CratestackContext` vpay mints carries no tenant. **`GET
/dash/v1/payment_intents` is unchanged and is still the only payments list
the dashboard can reach**, and its cursor paging stays cursor paging —
sharing `crate::v1::paging` with the merchant API is what makes `has_more`
mean one thing to an operator and to a merchant, and moving it is a
maintainer's decision, not a consequence of this. What the procedure changes
today is what a future `/payments` table _could_ be paged by; what it changes
right now is nothing a user can see.
[../status.md](../../status.md) § "The first `procedure`" has the measurements.

**And the thing to weigh before anyone wires it to a table
(2026-09-11, exp54 review): offset paging is not stable while payments are
being created.** `ORDER BY seq DESC` over a UNIQUE `seq` is a total order, so
a single page is never ambiguous — but `OFFSET` is counted afresh on every
call, and a payment created between page 1 and page 2 shifts the window by
one. An operator paging through a busy merchant's list sees the last row of a
page again at the top of the next one and misses a row at the tail for every
intent created behind their back; new rows land at offset 0, so this is worst
exactly where the list matters most. The cursor list `/dash/v1` already serves
does not have this property — `starting_after` asks for rows below a named
`seq`, which an insert cannot move. A numbered table is what `Page<T>` buys
and this is what it costs; neither the procedure nor anything else in vpay
compensates for it.

**The demo dashboard follows the SHOP's tenant, 2026-09-11 (exp51).** Nothing
in this flow changed; what changed is which tenant the demo binds it to. The
demo stack registers two merchant clients on two tenants — `shop-merchant`
(`examples/shop`, the clickable surface) and `demo-merchant`
(`just demo-walk`) — and this surface reads exactly one of them, as the
boundary above says it must. It was bound to `demo-merchant-tenant`, so a
payment a person made by hand through the shop was absent from the list and
its id answered the uniform cross-tenant `404` on the detail page. Both
answers were correct; the binding named the tenant nobody clicks anything in.
`demo_dashboard_merchant` is the binding now and defaults to the shop's
tenant, `demo_staff_merchant` is the same variable so the staff member cannot
end up in the other one, and `gen-demo-keys` regenerates an overlay whose
binding no longer matches. `dashboard.cy.ts` buys a tote in the shop, pays for
it on vpay's hosted page and asserts the payment is visible to the demo staff
member in the list **and** by id — point the variable back at
`demo-merchant-tenant` and that case fails on both. See
[../runbooks/demo.md](../../runbooks/demo.md) §6.

The by-id half is asserted **twice**, and the second one is the point: once by
clicking the row's link, and once by visiting `/payments/{id}` directly. Only
the second can fail on its own. Reached through the list, a broken by-id read
fails at the row selector and reads as a list problem — which is the shape the
report arrived in, and the shape a fix could restore only half of. Added in
this flow's review, 2026-09-11;
[../plans/exp51-demo-tenant-notes/opus-review.md](../../plans/exp51-demo-tenant-notes/opus-review.md)
carries what else that review found.

**Restyled again, 2026-09-12: `@vpay/ui` (this page's first paragraph, above)
was deleted from the repository, and the dashboard left it for the published
`@vaam-apps/ui`.** So did the checkout, in the same change — the maintainer's
own scope for this cutover was "both apps, delete `@vpay/ui` in this PR", not
the dashboard alone.

`@vaam-apps/ui` ships components but no layout or typography primitive at
all — no `PageShell`, `Stack`, `Heading`, `Text` or `List` — so unlike the
2026-09-07 restyle above, this one does not drive `styling_files` or
`class_tokens_distinct` to zero: both apps now write layout classes
directly, and `just verify-ui`'s old blanket "no `className` in an app" rule
was replaced by three narrower ones for exactly that reason (a plain
literal only, no raw status-colour token, a 60-character budget — see
[../../status/gates.md](../../status/gates.md)'s `verify-ui` entry for the
full derivation). `data-theme` moved `bumblebee` → `dark`: `@vaam-apps/ui`
compiles one theme, registered under daisyUI's built-in name, and
`src/layout.test.tsx` still pins the attribute directly for the same reason
as before — a wrong value renders the page completely unthemed, silently.

The two findings the 2026-09-07 review fixed both survive the cutover
unchanged in substance: `<main>` is still in `app/layout.tsx` (now wrapping
a `ScreenStack` rather than a `PageShell`), and the nav-honesty test still
resolves every `href` — rendered and declared — against `app/**/page.tsx` on
disk.

`statusTone` — daisyUI semantic colour per status, unused after this cutover
(its one consumer was `@vpay/ui`'s `StatusBadge`) — is deleted from
`@vpay/tokens`. The dashboard's payment-status colour and glyph now come
from `src/payment-status.ts`'s `defineStatusSystem` table, whose `label`
field still reads `@vpay/tokens`'s `statusLabel`, so the operator-facing copy
keeps one source. See `docs/status/verification/2026-09-12.md` for what was
run and measured.

## Contrast under the dark theme, measured 2026-09-12

Moving `data-theme` from `bumblebee` to `@vaam-apps/ui`'s dark theme changed
every foreground/background pair on every screen at once, and the repo had no
measurement of any of them: the checkout had `outcome-contrast.test.ts`, the
dashboard had nothing. `src/payment-status-contrast.test.ts` (10 cases) is
that measurement.

It checks two render paths, established by reading the package's compiled
source rather than inferred from its types:

- `PaymentStatusPill`'s glyph paints `--state-<hue>-fg` straight onto the page
  ground and never paints a `-bg` — both call sites are `quiet` attention,
  checked rather than assumed. Held to WCAG 1.4.11 non-text contrast, 3:1.
- `InlineBanner` paints its foreground over the alpha `-bg` composited over
  the ground. Held to WCAG 1.4.3 AA, 4.5:1.

The hue list is derived from `PAYMENT_STATUS_SYSTEM` itself rather than typed
out, so a status added without a contrast answer cannot pass unnoticed.

Three properties stop it reporting success without measuring anything — the
failure the deleted `@vpay/ui` helper would have produced here, since it
matched `oklch(L% C H)` only and returned silently on anything else while this
theme is authored in hex:

1. it parses hex and `rgb()`;
2. it composites the alpha tints over their actual ground, rather than
   measuring a token nothing paints;
3. it asserts 44 colours parsed before asserting any ratio, and pins one
   independently hand-computed value (the `InlineBanner` warning pair,
   10.2766:1).

Two mutations kill it and both were run and reverted: every parsed colour
forced to black fails 7 of 10 including the pin; the deleted helper's
oklch-only regex fails all 10, and in fact fails at module load, because the
theme is parsed at module scope.

**What this is not.** No browser has rendered these pages. Every number here
comes from the compiled stylesheet, and nobody has looked at the dashboard
since it went dark — the `processing` pill's hue in particular is a judgement
recorded in `src/payment-status.ts`, not an observation.
