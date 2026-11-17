# Issue #91: the invoice number on the checkout page, and Stripe-compat invoice cases

_Written 2026-11-17 while the work was done, on `impl/91-invoice-number-checkout`
over `master` at `ea469b40`. Every number below was measured on the tree this
page was committed with, unless the line says otherwise._

What was built is in [backend.md](../backend.md) ("The checkout read names the
invoice a payment settles"), [frontend.md](../frontend.md) ("The checkout page
names the invoice it is paying"),
[invoices.md](../../flows/invoices.md) § Paying and § Status,
[hosted-checkout.md](../../flows/hosted-checkout.md) § Status and
[stripe-sdk-compat.md](../../flows/stripe-sdk-compat.md) § Status. This page is
what was read, what was run, what was broken on purpose, and what could not be
run.

## Issue #91's items, checked on `master` before relying on the triage

| Item                                                        | On `master`?                       | Evidence read                                                                                                                                                                                                                                                                                                          |
| ----------------------------------------------------------- | ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1. both SDKs: invoice + invoice-item resources, `invoice.*` | yes, except its Stripe-compat half | `sdks/rust/src/resources.rs` (`InvoicesResource`), `sdks/rust/src/model.rs` (`KnownEventType::Invoice{Created,Finalized,Paid,Voided}`), `sdks/nodejs/src/resources/invoices.ts`, `sdks/nodejs/src/invoices.live.test.ts`, the invoice rows of `docs/sdks/parity.md` (PR #95). `sdks/stripe-compat` had no invoice case |
| 2. the checkout page renders the invoice number (D6)        | **no** — this change               | `frontends/apps/checkout` rendered amount and merchant name only; `docs/flows/invoices.md` § Paying recorded the gap                                                                                                                                                                                                   |
| 3. a refund against a paid invoice: `amount_refunded` (D5)  | yes                                | `backends/migrations/0042_invoices-amount-refunded.sql` (PR #110). It does **not** amend `paid_means_nothing_remaining`, deliberately: its header records that as a deviation from the issue's wording and why                                                                                                         |
| 4. per-merchant default `success_url`/`cancel_url` (D2)     | yes                                | `vpay_config::oauth::InvoiceDefaults` (`merchant_clients[].invoices`), consumed in `vpay_api::v1::invoices` (PR #110)                                                                                                                                                                                                  |
| 5. a Cypress case following `hosted_invoice_url`            | **no** — not done here             | #78 is closed (PR #94, 2026-09-10), so its blocker is gone; see "What was not run"                                                                                                                                                                                                                                     |

## What was run

| Command                                                                                                       | Result                                                                                                                                                     |
| ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test -p vpay-tests-integration --test invoices --test checkout_sessions -- --test-threads=2`           | `invoices.rs` **32 passed, 0 failed, 0 ignored** (re-run after the cross-tenant case was added); `checkout_sessions.rs` **36 passed, 0 failed, 0 ignored** |
| `pnpm --filter @vpay/checkout test` (`vitest run`)                                                            | **569 passed, 1 skipped**, 30 files (13 new; the skip is `a11y-gate`'s pre-existing built-Storybook case)                                                  |
| `vitest run --config vitest.storybook.config.ts` (checkout; Vite cache cleared first), Chromium 1243 headless | **29 of 29 stories passed** axe incl. `color-contrast`, three new                                                                                          |
| checkout `tsc --noEmit`, `eslint . --max-warnings 0`                                                          | clean                                                                                                                                                      |
| `sdks/stripe-compat`: `pnpm run typecheck`, `pnpm run lint`                                                   | clean                                                                                                                                                      |
| `vitest run src/invoices.compat.test.ts` (`sdks/stripe-compat`) against a running `vpay-server` (below)       | **6 passed (6)**                                                                                                                                           |
| the same, with two expectations flipped (`uncollectible` → `open`, a voided number → `null`)                  | exactly those 2 failed, 4 passed — the assertions read the server's bodies                                                                                 |
| `cargo test -p vpay-db -p vpay-api --lib`                                                                     | `vpay_api` **558 passed, 0 ignored**; `vpay_db` **150 passed, 0 ignored** (incl. `sql_audit`)                                                              |
| `cargo clippy -p vpay-api -p vpay-db -p vpay-tests-integration --all-targets -- -D warnings`                  | clean                                                                                                                                                      |
| `rustfmt --check` on the five changed Rust files; `prettier --write` on every changed TS/Markdown file        | clean                                                                                                                                                      |
| `just verify`                                                                                                 | `verify: ok — the seventeen gates above passed` (incl. `verify-doc-counts`, `verify-privacy-inventory`, `verify-links`)                                    |

**The stack the Stripe-compat invoice file ran against.** `cargo build
--profile test -p vpay-server` (this branch), started natively with
`--bind 127.0.0.1:18981`, `VPAY_PROFILE=demo` over a copy of
`config/application.yml` and the `.e2e/application-demo.yml` that
`just demo_port=18981 gen-demo-keys` wrote, the environment
`compose.e2e.yml` gives `vpay-server`, and a `postgres:16-alpine` container
it migrated at boot. No worker, no WireMock rails, no receiver: no invoice
case confirms a payment or reads a delivery. It is the same code as the
compose image, not the same artefact.

The integration suites ran against `postgres:16-alpine` containers through
testcontainers. A first run of `checkout_sessions.rs` reported 7–8 failures,
every one of them `No space left on device` from Postgres while the shared
build host's disk was full; re-run once the disk was cleared, 36 of 36 passed.
The numbers in the table are the re-run's.

## What was broken on purpose

Each mutation applied, the suite re-run, the failure observed, the mutation
reverted.

| Mutation                                                                                                                           | Caught by                                                                                                                                                                                   |
| ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `for_payer` passes `invoice_number.filter(\|_\| false)` **and** `CheckoutSessionForPayer::invoice` loses its `skip_serializing_if` | both new integration cases: `left: Some(Null)` against `{"number": "AYDH8CWY-000001"}`; and `"invoice": null` on a plain session                                                            |
| `number_for_intent` keyed on `merchant_id` alone (`ORDER BY seq LIMIT 1`)                                                          | `the_payers_checkout_reads_name_the_invoice_number_and_nothing_else_about_it`: the second session rendered `…-000001` where `…-000002` was expected; the negative case stays green          |
| `number_for_intent` without `merchant_id = $1` (`$1::text IS NOT NULL` in its place)                                               | `the_invoice_number_lookup_is_scoped_to_the_merchant`: merchant B's lookup answered `Some("Q2KP9CJJ-000001")` where `None` was expected; `the_payers_checkout_reads_…` stays green under it |
| `PaymentSummary` never draws the line                                                                                              | the four "names it" cases in `checkout-view.test.tsx` (4 failed, 129 passed)                                                                                                                |
| `PaymentSummary` always draws the line                                                                                             | the two "draws no invoice line" cases (2 failed, 131 passed)                                                                                                                                |
| `return.ts` dispatches `invoice: null`                                                                                             | the return controller's invoice case (1 failed, 18 passed)                                                                                                                                  |
| `contextOf` drops what `invoiceOf` read                                                                                            | `machine.test.ts`'s `contextOf` invoice case and `controller.test.ts`'s (2 failed, 80 passed)                                                                                               |

## What was not run, and why

- **The Stripe-compat suite's other five files** (`lifecycle`, `webhooks`,
  `errors`, `idempotency`, `headers`): not re-run. They need the worker, the
  two WireMock rails and the receiver; the invoice file needs none of them,
  and the compose images (a release musl build of `vpay-server`) could not be
  built here in reasonable time.
- **The invoice file against the compose stack:** not run. It ran against
  the branch's own `vpay-server` started natively instead (below), which is
  the same code but not the same image.

- **Item 5, the Cypress case.** Not written. It needs the compose e2e stack —
  `vpay-server`'s image, the checkout page's Next.js image and the rails —
  and a browser spec nothing has run is a claim nothing checked. Its blocker
  (#78) is gone.
- **`just ci`** and the full workspace suite: not run here, per the task
  brief; CI runs them on the PR.
