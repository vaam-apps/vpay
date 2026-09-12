# Verification log — 2026-09-12, the instrument register

Last verified: 2026-09-12, on `claude/instrument-register`, stacked on the
`@vaam-apps/ui` 0.1.2 upgrade — `InstrumentPanel` and `Card`'s `glow` do not
exist in 0.1.1.

`@vaam-apps/ui`'s "Surfaces and registers" doc names three registers and says
the choice is about what the reader is doing: a hairline for **diagnostic**
surfaces you work through row by row, a shadow for **floating** layers over a
ground they cannot know, and an aurora for **instrument** surfaces you scan to
find one number. This change puts each of the two instrument components on the
one surface in this repository that qualifies, and nowhere else.

## What this claims, and what it does not

It claims two surfaces changed, that both use only data already on screen, and
that the checkout's 22 screens still clear WCAG AA in a real browser with the
glow on.

It does **not** claim the dashboard's panel was measured in a browser. There is
no Storybook for the dashboard and no browser-level a11y gate for it at all;
the panel is covered by the jsdom axe suite, which computes no colour, and by a
source-level guard. **The mesh's own contrast is the library's measurement,
taken on trust here.**

## The panel that was not built

The doc's own illustration is a delivery-rate panel: `12,481 delivered, 98.2%
of 12,710 terminal`. **That cannot be built honestly in this repository, and
was not.** `/dash/v1` is a cursor-paged list and a get-by-id; `ListPage` is
`data`, `hasMore`, `cursor`. There is no count, total or summary route
anywhere, and deriving a rate from one page would describe the page rather than
the account. Inventing one is the failure `CLAUDE.md` names third, and the
dashboard's own `app/page.tsx` already records deleting a decorative screen for
exactly that reason.

So the panel carries three real fields off one object, with no arithmetic, and
its caption says so on the screen: _"This payment only — vpay exposes no
aggregates, so nothing here is a total."_

## Dashboard — `InstrumentPanel` on the payment detail

That page was **entirely diagnostic**: two divided `DetailList`s totalling
seventeen rows plus a refunds table. The two questions an operator opens it
with — how much, and did it move — were rows 3 and 2, rendered identically to
`Livemode: no`. The panel lifts amount, status and charged amount to a surface
you scan; every list below keeps its hairline and is unchanged.

`text-subtle-foreground` appears nowhere in it. The library measures that tier
at **4.41:1 dark / 4.45:1 light** on the mesh — below AA — while `muted` holds
at 5.29:1 and above, which is why the component renders its own caption at
muted and its doc says in as many words not to use subtle here.

`src/instrument-register.test.ts` makes that rule mechanical rather than a
comment: any file rendering an `<InstrumentPanel>` may not contain
`text-subtle-foreground`, nor a `text-state-*` status colour on a ground that
is deliberately meaningless. Three mutations, all reverted — the banned tier
inside the panel (1 failed), a status colour inside it (1 failed), and the
panel removed entirely (1 failed, after the check was tightened from the
identifier to the JSX: an import left by a partial edit satisfied the first
draft, which would have gone on guarding a surface nothing rendered).

## Checkout — `Card glow` on the amount

The payer's summary card is the only `<Card>` in the app, and it holds the one
number a payer checks before approving — already `text-metric`, already the
largest thing on the screen. Everything around it is a control rather than a
surface, so "a glow on every card is a glow on none" is satisfied by there
being exactly one card.

**It carries no meaning, and here that is load-bearing.** The card renders
unchanged on every state the machine can be in, so a payer cannot read an
outcome from it. Confirmed by looking: the `Succeeded` and `Failed` screens
carry the identical glow and differ only in the `InlineBanner` beneath it,
which is where the outcome has always been.

## Measured

`just test-storybook` — the 22 payer screens in a real Chromium, with the glow
— **22/22, 0 unhandled errors, exit 0** from a cold cache. The glow is four
`box-shadow`s outside the border box, so it changes no foreground/background
pair, and axe's verdict is unchanged.

`just test-web` **1 317 → 1 320 passing, 0 skipped**: checkout 523/523,
dashboard 258/258 (three new cases). `just verify-ui` exit 0 — every class
attribute added is a literal under the 60-character budget, and no status
colour is written in an app.

Both screens were opened and looked at. That is not a formality here: it is how
the `React is not defined` defect in the commit below was found, after every
gate was green.
