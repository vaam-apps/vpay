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
| what the Flutter checkout plugin's gate/recipes/pin actually do, and what is still unbuilt            | [mobile-flutter-plugin.md](mobile-flutter-plugin.md)           |
| what would have to be true before any of this is an MVP                                               | [mvp.md](mvp.md)                                               |
| how the "vpay is a scaffold" banner was narrowed, step by step                                        | [overall-history.md](overall-history.md)                       |
| what `just ci` actually printed on a given day                                                        | [verification/](verification/)                                 |

## The verification log, newest first

Named by the newest date in each block, not by a single date: entries were
appended to `docs/status.md` in the order branches landed, which is not always
the order they were measured.

- [verification/2026-09-16-adr-0022.md](verification/2026-09-16-adr-0022.md) —
  ADR-0022 (surface isolation and independent scaling): `deployment.surfaces`
  and conditional `/v1`/`/dash/v1` mounting, the dashboard runner stage's real
  `docker run --read-only --tmpfs /tmp --user 1000:1000` observation and the
  `HOSTNAME`-binding defect it found, and the Helm chart's `-management`
  Deployment, HPA and `connection-budget` guard (23/23 guards fire,
  kubeconform validates 49 resources). Carries the review pass's own section:
  five mutations of the surface gate, four of which the new tests caught and
  one — `validate_all`'s boot-refusal wiring — which nothing did, plus the
  fix; the chart's rendered overlay fed to the real `Config::load`; and the
  `networkpolicy-management-ingress` guard added because an empty
  `podSelector` matches every pod in the namespace. `just ci`'s final `deny`
  step fails on a pre-existing, unrelated advisory — no `Cargo.lock` change
  on this branch.
- [verification/2026-09-16-confirm-poll-job-latency.md](verification/2026-09-16-confirm-poll-job-latency.md) —
  the first CI run that compiled `--test live_refunds` went red, and what it
  had found was **not** in the SDK: a charge confirmed while the worker's
  claim loop was busy waited up to sixty-one seconds for its first status
  query, on every rail, in every deployment, since Step 4. Reproduced against
  a real stack (`attempts = 1`, `run_at = created_at + 61 s`, `state =
submitted` from six seconds in, no status query on the rail's journal),
  fixed in `vpay-api` with a grace on the enqueue and a pull-forward in the
  confirm's own compare-and-swap, and mutation-proven on two existing
  `confirm_rails` cases. With settlement fixed, both live refund suites then
  failed on a `cancel` that `4bcf6491` had deliberately made impossible;
  the tests were wrong and were corrected, not the guard
- [verification/2026-09-16-w3-seam.md](verification/2026-09-16-w3-seam.md) —
  RFC-0003 wave 3 / arm H, the seam pass, run after every arm and every review
  had merged. Eleven findings, of which one was not a documentation bug: **CI
  ran `--test live_invoices` and not `--test live_refunds`**, so three
  `sdks/parity.md` rows were ✅ citing live refund cases no job compiled —
  `verify-sdk-parity` proves a test **name** exists, never that anything runs
  it. Also: a merge had pasted seventeen lines of prose into
  `docs/status.md`'s gate table and deleted its `verify-status` row, and
  prettier had aligned the wreckage into a plausible-looking table; two
  paragraphs on that page contradicted each other about how many
  `NotImplemented` tokens there are; `docs/flows/ledger.md`'s heading claimed
  four invariants are "asserted nightly" when nothing schedules any of them;
  and three flow pages had no Status entry for the day their subject changed.
  Twelve gates, each invoked on its own; **no banner addendum was earned and
  none was added**
- [verification/2026-09-16-w3-merge.md](verification/2026-09-16-w3-merge.md) —
  wave 3 merged into the refunds feature branch: arm F with its money review,
  arm F with its contract review, and arm G with its review. One conflict (the
  boot banner, rewritten by two reviews at once) and the resolution that names
  all five refund routes; migration `0048`'s "no deployment holds" wording
  corrected **before** it landed, because a migration's bytes are immutable
  once they ship; one case red on `review/w3f-money` before the merge ever
  happened, repaired by building its premise rather than weakening it; and the
  stale claims that only became false once both arms were in one tree — among
  them `docs/flows/errors.md`'s "no `/v1` caller can provoke a `501` at all",
  which a test has contradicted since arm F (806 crate + 73 integration + 67
  conformance, **0 skipped**)
- [verification/2026-09-16-w3-sdk-refunds.md](verification/2026-09-16-w3-sdk-refunds.md) —
  RFC-0003 § 2 wave 3 / arm G and its review: the refund surface in both
  merchant SDKs, driven against a real `vpay-server` over HTTP, plus the
  `gen-demo-keys` fix without which every refund on the demo stack was a 500.
  The `202`s in that run came from a `wiremock/wiremock` container, and no
  refund it created ever left `pending`
- [verification/2026-09-16-w3-refund-routes.md](verification/2026-09-16-w3-refund-routes.md) —
  RFC-0003 § 2 wave 3: the four `/v1` refund routes and the first
  `charge.refunded` / `charge.refund.updated` this repository has ever emitted
  (384 `vpay-api` + 243 `vpay-db` + 260 port/adapter/core + 40 integration
  incl. eleven new WireMock-backed refund cases + 168 `vpay-sdk`, **0
  skipped**). The decisive mutation — deleting the `RefundDestination::Origin`
  refusal — run, and the finding recorded: it fails **one** test in the
  workspace, because vpay carries no `Origin` rail for any integration suite
  to reach. Three things still stand between these routes and a refund a payer
  receives, and the page names all three
- [verification/2026-09-15-refunds-write-path-money-review.md](verification/2026-09-15-refunds-write-path-money-review.md) —
  the adversarial money/concurrency review of the entry below: six mutations
  with their numbers, eight claims confirmed TRUE, and one finding — the
  narrowing that followed the over-refund guard moving to `Refunds::create`
  dropped the only assertion that `apply_refund_succeeded` is one transaction,
  measured by splitting it in two and watching all 25 refund and ledger cases
  still pass (328 `vpay-db`/`vpay-ledger`/`vpay-core` + 52 `postgres_smoke`,
  **0 skipped**; `EXPECTED_DRIFT_CHANGES` re-measured off-pin at cratestack
  0.11.1 and therefore **not** verified here — CI is the arbiter)
- [verification/2026-09-15-refunds-write-path.md](verification/2026-09-15-refunds-write-path.md) —
  RFC-0003 §§ 3-4 wave 2: the first `refunds` INSERT this repository has ever
  issued from Rust, the intent counters `apply_refund_succeeded` never moved,
  and the first ledger postings a call site ever made (241 `vpay-db` + 51
  `postgres_smoke` + 86 `vpay-core`/`vpay-ledger` tests, **0 skipped**; the
  decisive mutation — deleting the `amount_refund_pending` increment — run,
  failed the race test as it must, and reverted)
- [verification/2026-09-13-flutter-lane-b-gate.md](verification/2026-09-13-flutter-lane-b-gate.md) —
  Lane B of the Flutter plugin brief: `verify-sdk-parity` learns Dart
  (245 xtask tests, 0 ignored; clippy `--all-targets -D warnings` clean; the
  real `docs/sdks/parity.md` unaffected, still 469 proving tests), and the
  decisive `skip: true` mutation run against `verify_sdk_parity` itself on a
  synthetic fixture, with the literal error text it returned
- [verification/2026-09-13-dash-cratestack-transport.md](verification/2026-09-13-dash-cratestack-transport.md) —
  mounting CrateStack's read-only procedure transport (nav plan Lane C):
  `POST /dash/v1/$procs/searchPaymentIntents` answers over a real Postgres, an
  unauthenticated caller is refused, the tenant-mismatch mutation reddens the
  procedure body's own container test, and a routing-table walk proves no
  generated model CRUD route is reachable
- [verification/2026-09-13-dashboard-storybook.md](verification/2026-09-13-dashboard-storybook.md) —
  a Storybook for `frontends/apps/dashboard`, mirroring the checkout's: 25
  stories, both `@vaam-apps/ui` themes (21 `dark`, 4 `light`), a measured
  router/`next/link` fix, one real `@vaam-apps/ui` accessibility defect found
  and suppressed by rule id rather than hidden, and the checkout's own
  theme.css alias measured as unnecessary here — plus the same day's
  adversarial review, which found three stories rendering the wrong theme,
  an `AppShell` with no axe coverage at all, and a built-stylesheet gate that
  never ran in CI
- [verification/2026-09-13-dash-shell-drawer.md](verification/2026-09-13-dash-shell-drawer.md) —
  the dashboard shell's "More" drawer (nav plan Lane A): the theme switcher
  reachable below the ≥1280px sidebar, the signed-in identity kept visible in
  `<main>` rather than moved behind it, the bottom-sheet-below-`md` /
  right-panel-above split, and the review that reversed the first
  implementation's "a bottom sheet cannot pass `verify-ui`" finding
- [verification/2026-09-13-storybook-reverified.md](verification/2026-09-13-storybook-reverified.md) —
  the 2026-09-12 Storybook restoration re-run on `origin/master`, the six
  places that still said the gap was open, and the measured answer to the
  question `justfile` left about the new `.storybook`'s tsconfig
- [verification/2026-09-12-erasure-idempotency-window.md](verification/2026-09-12-erasure-idempotency-window.md) —
  issue #111: the 24-hour window in which an update that lost a race to a
  `DELETE` put the payer back into `idempotency_keys.response_body` —
  reproduced, then closed — and the merchant's own copy, which is a contract
  question and is **not** decided
- [verification/2026-09-12-worker-claim-latency.md](verification/2026-09-12-worker-claim-latency.md) —
  what a single worker's claim latency actually is (issue #100): one
  `IDLE_SLEEP` plus milliseconds, the same at two workers, the bound now
  pinned, and the loop left alone
- [verification/2026-09-12-customer-debug-redaction.md](verification/2026-09-12-customer-debug-redaction.md) —
  the customer **write** path's `Debug`, which printed the payer's name, email,
  phone, street and GPS point until this date; issue #113's two questions about
  the coordinate are prepared and **not taken**
- [verification/2026-09-12-issue-88.md](verification/2026-09-12-issue-88.md) —
  issue #88 item by item: the proactive re-mint, the outage-tolerant cookie,
  the paging test — which a same-day adversarial review found was green under
  the very mutation this log calls decisive, and which gained the two
  assertions that fix that — the CSRF forwarded-host guard, and why item 5 is
  moot. Amended in place the same day; one residual is named and not closed
- [verification/2026-09-12-dashboard-refine.md](verification/2026-09-12-dashboard-refine.md) —
  the dashboard on Refine (plan lanes 3 and 4): one array for the rail and the
  router, and the `401`-only sign-out rule carried into `authProvider`
- [verification/2026-09-12-instrument-register.md](verification/2026-09-12-instrument-register.md) —
  `InstrumentPanel` on the payment detail and `Card glow` on the payer's
  amount, and the aggregate panel that could not be built honestly
- [verification/2026-09-12-vaam-apps-ui-012.md](verification/2026-09-12-vaam-apps-ui-012.md) —
  `@vaam-apps/ui` 0.1.1 → 0.1.2: a second theme, and the three places this
  repository assumed there was only one
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
- [cratestack/2026-09-13-credentials.md](cratestack/2026-09-13-credentials.md) — migration 0044, `model Credential`
- [cratestack/2026-09-13-dashboard-procedure-transport.md](cratestack/2026-09-13-dashboard-procedure-transport.md) — the first transport, mounting it

## What is _not_ here

The declaration `cargo xtask verify-status` reads — the list of every
`ProviderError::NotImplemented` token in shipping code — is still on
[docs/status.md](../status.md), under the heading the gate looks up by name. So
are the banner, the table of areas and the gate table. Everything else on that
page came from here.
