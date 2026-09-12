# The status archive

[docs/status.md](../status.md) was 6 151 lines on 2026-09-11. It is 275 now.
**The other 5 996 lines are here, verbatim.** Nothing was summarised away,
nothing was tidied, and in particular nothing that recorded a mistake was
dropped: this repository's habit of writing "this said X until date Y and was
wrong" is the reason the page is worth trusting, and a compression that lost one
of those would have cost more than the length did.

Only the relative links changed, because the files moved down a directory. The
32 lines that are neither here nor on `docs/status.md` are blank lines and `---`
rules that used to separate two sections which now live in two files.

## What answers what

| If you want to know                                                                                   | Read                                                           |
| ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| what each `just verify` gate checks, what it used to miss, and the mutation that proved the hole shut | [gates.md](gates.md)                                           |
| which backend capability is real, and what its test actually asserts                                  | [backend.md](backend.md)                                       |
| why the Orange stub's hosted page grew a payer's window                                               | [backend-orange-hosted-page.md](backend-orange-hosted-page.md) |
| what the checkout page, the dashboard and the demo shop do                                            | [frontend.md](frontend.md)                                     |
| what boots, in compose and in CI, and what has never run in a pod                                     | [infrastructure.md](infrastructure.md)                         |
| how Docker, `cargo deny`, the toolchain pin and the migration manifest got fixed                      | [infrastructure-history.md](infrastructure-history.md)         |
| what the sqlx 0.8 → 0.9 bump cost, and which OP stores pinned 0.8                                     | [sqlx-and-op-stores.md](sqlx-and-op-stores.md)                 |
| how far CrateStack adoption has got, and what each step measured                                      | [cratestack.md](cratestack.md) and [cratestack/](cratestack/)  |
| what each merchant SDK ships, and which gaps are dated                                                | [merchant-sdks.md](merchant-sdks.md)                           |
| what would have to be true before any of this is an MVP                                               | [mvp.md](mvp.md)                                               |
| how the "vpay is a scaffold" banner was narrowed, step by step                                        | [overall-history.md](overall-history.md)                       |
| what `just ci` actually printed on a given day                                                        | [verification/](verification/)                                 |

## The verification log, newest first

Named by the newest date in each block, not by a single date: entries were
appended to `docs/status.md` in the order branches landed, which is not always
the order they were measured.

- [verification/2026-09-12-storybook-restored.md](verification/2026-09-12-storybook-restored.md) —
  Storybook restored in `frontends/apps/checkout`, and the dropped `@import`
  that had both apps' stylesheets silently losing their entire theme
- [verification/2026-09-12.md](verification/2026-09-12.md) —
  the `@vaam-apps/ui` cutover: both apps move off `@vpay/ui`, and it is
  deleted along with its Storybook
- [verification/2026-09-12-browser-a11y.md](verification/2026-09-12-browser-a11y.md) —
  the browser a11y gate PR #133 built against `@vpay/ui`, the four contrast
  violations it found, and the deletion that superseded it hours later
- [verification/2026-09-11.md](verification/2026-09-11.md) —
  `claude/exp51-demo-tenant`, the header of `claude/exp45-worker-pool-bound`,
  and `claude/exp46-customer-address`
- [verification/2026-09-10.md](verification/2026-09-10.md) —
  `claude/exp39-readme-gaps`, `claude/exp38-sigterm-scenario`
- [verification/2026-09-07.md](verification/2026-09-07.md) —
  `claude/exp30-single-binary`, `claude/exp29-migration-manifest`,
  `claude/exp21-checkout-page`
- [verification/2026-09-04.md](verification/2026-09-04.md) — Steps 9 and 8
- [verification/2026-09-03-steps-5c-to-7.md](verification/2026-09-03-steps-5c-to-7.md) — Steps 7, 5c and 6
- [verification/2026-09-03-steps-4-to-6.md](verification/2026-09-03-steps-4-to-6.md) — Steps 6, 5 and 4
- [verification/2026-09-02-steps-0-to-3.md](verification/2026-09-02-steps-0-to-3.md) — Step 3 and,
  chained beneath it, every note before it

## The CrateStack adoption log, oldest first

- [cratestack/drift.md](cratestack/drift.md) — the measured migration/model drift
- [cratestack/2026-09-06-first-read.md](cratestack/2026-09-06-first-read.md)
- [cratestack/2026-09-06-outbox.md](cratestack/2026-09-06-outbox.md) — the transaction seam
- [cratestack/2026-09-06-first-writes.md](cratestack/2026-09-06-first-writes.md)
- [cratestack/2026-09-06-currencies-and-providers.md](cratestack/2026-09-06-currencies-and-providers.md) — migration 0032
- [cratestack/2026-09-06-providers-and-d7.md](cratestack/2026-09-06-providers-and-d7.md) — migration 0033, decision D7
- [cratestack/2026-09-06-customers.md](cratestack/2026-09-06-customers.md)
- [cratestack/2026-09-07-money-tables.md](cratestack/2026-09-07-money-tables.md) — migration 0037
- [cratestack/2026-09-11-search-payment-intents.md](cratestack/2026-09-11-search-payment-intents.md) — the first `procedure`

## What is _not_ here

The declaration `cargo xtask verify-status` reads — the list of every
`ProviderError::NotImplemented` token in shipping code — is still on
[docs/status.md](../status.md), under the heading the gate looks up by name. So
are the banner, the table of areas and the gate table. Everything else on that
page came from here.
