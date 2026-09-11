# Hosted checkout — the page's state machine, and the outcome screens

_Split out of [docs/flows/hosted-checkout.md](../hosted-checkout.md) on 2026-09-11 by exp57, which broke a 937-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## The page's state machine

`frontends/apps/checkout` keeps its logic in a **pure reducer** — no `fetch`, no
timer, no DOM — with the React layer as wiring. That is what makes every refusal
and every transition a test rather than a branch inside an effect nobody can
reach twice.

```
                    ┌──────────────────────────────────────────┐
   load             │                                          │
   ────► loading ───┤ credentials missing → invalid link       │
                    │ framed when it must not be, or framer    │
                    │   not on the merchant's list → refused   │
                    │ session past its horizon → expired       │
                    │ no rail this page can drive → refused    │
                    └───────────────┬──────────────────────────┘
                                    │ session read ok
                                    ▼
                         select_rail  (only when the intent
                                    │  offers more than one)
                    ┌───────────────┴───────────────┐
                    │ MTN (push)                    │ Orange (redirect)
                    ▼                               ▼
              collect_msisdn                  ready_redirect
                    │ confirm                       │ confirm (redirect: 'if_required')
                    ▼                               ▼
               confirming                      redirecting ──► the rail's own page
                    │                               │            (top-level, or the
                    ▼                               │             parent navigates
                 waiting  ◄─────────────────────────┘             when framed)
                    │ poll  /v1/browser/payment_intents/{id}
                    ▼
                 outcome  (succeeded │ failed │ canceled)
                    │ the payer presses "Back to {merchant}"
                    ▼
               forwarding ──► success_url / cancel_url (hosted)
                              return_url + vpay:complete (embedded)
```

**Nothing on that diagram happens on a timer.** The one arrow out of
`outcome` is a button press. See "The outcome screens" below.

The return page is its own document with its own smaller machine: it holds no
intent secret, so it has **no confirm transition at all** — it polls the return
route until the intent is terminal, shows the outcome, and forwards.

Three properties worth stating because each is a test:

- **The embed check runs before the credential is read.** A framer that is not
  on the merchant's list is refused even when the URL carries no key and no
  secret, so the refusal cannot be used to probe which half of a link is wrong.
- **The language switch does not navigate.** A `?lang=fr` link has no fragment,
  and resolving a fragment-less relative URL _drops_ the current one — which on
  this page is the session's credential. The server picks the initial locale
  from `Accept-Language` (French by default: Cameroon first, and Orange's own
  page is French by default), and the switch swaps the dictionary in place.
- **No floating point anywhere in the money path.** `5000 XAF` becomes the
  string `"5000"` by moving a decimal point through the integer's digits, and
  `Intl.NumberFormat` formats that string. `minor / 100` would be float
  arithmetic in a money path, which the Rust half of this repository denies
  workspace-wide ([money.md](../money.md)).

## The outcome screens

_Changed 2026-09-06 (the maintainer's requirement of 2026-09-05)._

Every outcome — succeeded, failed, canceled — ends in **one button, named
"Back to {merchant}"**, and nothing else. There is no countdown, no timer,
and no navigation this page performs on its own. Both modes render the same
screen; what differs is what the button does, which is what already differed:

|                          | Hosted                                                                     | Embedded                                                            |
| ------------------------ | -------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| The button               | `location.assign(success_url \| cancel_url)`, top-level                    | posts `vpay:complete` to the framer, then navigates to `return_url` |
| Where the URL comes from | `forwardKindFor` — `success_url` on a paid session, `cancel_url` otherwise | `return_url`, for every outcome                                     |
| Session names nowhere    | no button; "This payment is finished. You can close this page."            | the same                                                            |

The button carries the merchant's name — `Back to Boutique Test` — and falls
back to "Back to the shop" where the session read carried no usable name, the
same `_unnamed` treatment every other merchant sentence on this page gets.

It replaced a **five-second auto-forward** that had been here since Step 9. A
page that navigates on its own takes the outcome away from the payer who is
reading it, and on the failure screen it takes away the only text that says
why — on a handset, in a shop, with someone waiting. That the button was
always there beside the countdown did not fix it: it made the countdown a
race a slow reader loses.

**A failed outcome is red.** _Corrected 2026-09-07._ The screen took its
colour from `@vpay/tokens`' `statusTone` through the intent status each
outcome implies, and a failed attempt leaves the intent at
`requires_payment_method` — which is `neutral`, because on a **dashboard**
that status means "awaiting a payment method" rather than "this failed". The
mapping was accurate and the screen was wrong: a payer whose payment failed
read a grey box while a payer who cancelled read a red one. An operator's
status palette is not a payer's outcome palette, so `@vpay/tokens` now carries
both — `statusTone` unchanged, and `checkoutOutcomeTone` for these three
screens. _Updated 2026-09-07 (decision D4,
[2026-09-07-ui-revamp.md](../../plans/2026-09-07-ui-revamp.md) §9):_ `canceled`
now tones `warning`, not `error` — the design call this paragraph used to
defer is taken, on the reasoning that a payer's own cancellation is not the
same event as a payment that failed for a reason outside their control.
`failed` still tones `error`. The token moved in `@vpay/tokens`
(`frontends/packages/tokens/src/index.ts`); `OutcomePanel`
(`frontends/apps/checkout/src/components/screens.tsx`) already carries a
`warning` entry in its `TONE_CLASS` lookup, so the colour renders correctly
with no change to this app — the migration of that lookup onto `@vpay/ui`'s
`Alert` component is separate work, not yet done (`docs/plans/2026-09-07-ui-
revamp.md` §4.1).

**A failed outcome also shows the rail's own words** where the API gave any.
`last_payment_error.message` is rendered as _data_, under the translated
sentence and labelled as the provider's ("What the payment provider said"),
never in place of it: it arrives in whatever language the rail writes in,
it is outside the closed `FailureCode` vocabulary the page translates, and
this page controls not a word of it. `providerReason` strips control
characters, collapses whitespace and bounds it at 300 characters; React
escapes the rest, so it reaches the DOM as a text node and never as markup,
an attribute or a URL.

<!-- Ported during the exp57 docs-split rebase: this is issue #73's contrast answer, which was written against the pre-split hosted-checkout.md. -->

measured. **Still true, more precisely, after 2026-09-11 (b1e review):**
the bumblebee theme's contrast is checked by nobody, and now there is a
real-browser harness that has been proven **unable** to check it, rather
than one nobody had built. `frontends/tests/e2e`'s `shop-hosted.cy.ts`
runs axe-core's `color-contrast` rule against four of the outcome screens
in a real browser (not `@vpay/ui`'s Storybook, which still runs it against
`corporate`/`business`, which this app does not use, and is still not part
of `just ci`) — and every one of the four checks comes back `incomplete`,
not a verdict, because daisyUI 5's own base CSS puts an unconditional
`background-image` on `:root` (its scrollbar-gutter-stability mechanism),
and axe-core's `color-contrast` rule refuses to evaluate any element with
such an ancestor. This was confirmed by mutation, three times, including a
plain-hex, foreground-equal-to-background pair that any measurement should
catch — see the Status section's b1e entry and `shop-hosted.cy.ts`'s own
header comment for the full trace. It reaches every daisyUI-5 page, not
only this one.
**Updated 2026-09-11 (b1e, corrected in review): the jsdom draft was
replaced with a real browser check — which was then proven unable to check
anything, for a reason outside this repository.** This entry first said a
new vitest suite (`outcomes.axe.test.tsx`, 10 tests) ran axe's
`color-contrast` rule against the four outcome screens and passed 10/10 —
true, and stated alongside its own jsdom caveat, but jsdom parses no CSS and
computes no layout, so that suite could only ever report zero violations.
Ten tests that cannot fail are worse than none: a reader sees "contrast" in
the test names and believes the theme was checked. **The review deleted
that suite** (and the now-dead `axeContrastViolations` helper it was the
only caller of, in `@vpay/ui`'s testing export) and built a real-browser
replacement in `frontends/tests/e2e`'s `shop-hosted.cy.ts` — axe-core's
`color-contrast` rule, run in a real Chrome against the compiled
`bumblebee` stylesheet, on four outcome screens as they are actually
reached by that spec's own payment trips: `CheckoutView`'s `succeeded` (MTN
push) and `failed` (MTN decline), and `ReturnView`'s `succeeded` and
`failed` (both through Orange's redirect).

**It does not use the `cypress-axe` package plan §7 row 6 names.** Measured,
in order: `cy.origin()`'s callback runs in its own realm and shares neither
`Cypress.Commands.add` registrations nor `cy.readFile()`/`cy.task()` with
the support file cypress-axe's commands depend on — `Cypress.require()`
(Cypress's own escape hatch) fixes the first, nothing fixes the second, so
the checks run axe-core directly (`window.eval`, then `win.axe.run()`) and
hand the result back across the `cy.origin()` boundary as its return value.

**And three rounds of a decisive mutation on `@vpay/ui/src/styles.css`
found that the check cannot produce a `violation` verdict at all — only
ever `incomplete`.** Reverting `--color-error-content`'s fix to daisyUI's
own default (3.53:1, previously measured below AA) did not fail it. Setting
it identical to `--color-error` (confirmed via `getComputedStyle`, ~1:1)
did not either. Replacing both with plain, identical, axe-parseable hex —
`--color-error: red; --color-error-content: red` — still did not. Every
round's `incomplete` result names the same cause: `messageKey: "bgImage"`
on the alert element itself. `daisyui@5.7.28`'s own
`base/rootscrollgutter.css` puts an unconditional `background-image` (a
scrollbar-gutter-stability trick for locking scroll behind an open
`<dialog>`/`Drawer`) on `:root`, whether or not a dialog is open — and
axe-core's `color-contrast` rule gives up the moment any ancestor carries a
`background-image`. `:root` is an ancestor of everything, so this reaches
every element on every page built with daisyUI 5's `@plugin 'daisyui'`, not
only these four screens and not only this repository. The full trace,
including the three mutations' exact values and results, is
`shop-hosted.cy.ts`'s own header comment.

**So issue #73's contrast half is still open**, and this is why: the
harness is real (real browser, real axe-core, correctly wired across
`cy.origin()`), it is exercised on every CI run and logs what it finds
(`incomplete`, not silently), and it still cannot confirm or deny WCAG AA
compliance on this theme. Closing it for real needs one of — an axe-core
release that tolerates a fully transparent `background-image` ancestor;
removing or conditioning daisyUI's scroll-lock `background-image` (a
behavioural change to a third-party base style, and if scroll-locking a
`<dialog>`/`Drawer` matters here, a real cost); or a measurement that does
not walk the DOM for a background colour at all. **Not covered: the
`canceled` outcome kind** (`PaymentIntent.status === "canceled"`, distinct
from the rail declines the `failed` cases above drive) — no spec anywhere
cancels an intent out from under an open checkout, so nothing has rendered
that screen in a browser, on top of the gap above. Locale is whatever
`shop-hosted.cy.ts` already runs under (English); the French strings on
these screens ride the same gap. The theme's own tone palette is still
measured for real, by a method with no DOM-background-walk to trip over:
`@vpay/ui`'s `theme-contrast.test.ts` compiles the stylesheet and computes
WCAG ratios from the resolved OKLCh values directly, verifying every
rendered tone clears AA (4.5:1), including the error/info overrides at
lines 887–899 above. That check is real, always was, and is unrelated to
the vitest suite this paragraph corrects.
