# 2026-09-23 — Stale claims found by re-verifying the skills, and one test gap

## Why this page exists

Re-verifying the agent skills in
[vaam-apps/vpay-skills](https://github.com/vaam-apps/vpay-skills) against
vpay at `b747e5d5` turned up fourteen places where this tree's own docs, doc
comments or test titles disagree with its code. Each was re-checked against
`master` at `9184e42` (#252, the tip when this branch was cut) before it was
changed. Thirteen were real; one was not, as stated (item 3). Every
correction keeps what the text said before, with this date.

**One change is code.** The route-probe test in `vpay-db`'s `schema.rs`
listed nineteen model tables and had missed `manual_payments`, which
migration `0049` added in #251; it now lists twenty and checks its list
against the schema. Everything else is a document, a doc comment, a comment
or a test title — no route, migration, gate, `NotImplemented` token or
behaviour moved. One status row changed:
[infrastructure.md](../infrastructure.md)'s `schemas/*.cstack` row was given
a dated re-measurement (item 9), because its "twelve of seventeen" was a
status claim.

## The claims, and what the code says

1. **`vpay-db` `schema.rs`, `no_generated_model_route_is_mounted_only_the_one_procedure_is`**
   - Was: `MODEL_TABLES: [&str; 19]`, no `manual_payments`; doc said "nineteen
     models" and "seventy-six assertions"
   - Now: `[(&str, &str); 20]` of `(model, table)`, and before any request the
     model column is compared, sorted, with `cratestack_schema::MODELS` — the
     macro's own list of every `model` in `schemas/vpay.cstack`. That fails in
     both directions: a model added and not listed, or listed and no longer
     declared. The doc comment's count is 160 (twenty models × two paths ×
     four methods); "seventy-six" was nineteen × four and forgot the paths,
     and "seventy-two" on the day it was written was eighteen × four. That
     count is arithmetic from the loop bounds; no sabotage run was made to
     watch 160 turn red, and the loop panics on its first failure anyway
   - Same count fixed beside it: `dashboard_procedure_router`'s doc,
     `search_payment_intents.rs`'s header and `schemas/vpay.cstack`'s
     procedure note said "nineteen models". **Not** fixed:
     `backends/migrations/0045_ledger-entries-merchant-id.sql`'s comment calls
     it "the nineteen-name list", and a shipped migration's bytes are
     checksummed (`verify-migrations`)
2. **`frontends/apps/dashboard/src/layout.test.tsx`**
   - Was: case titled "the only theme @vaam-apps/ui actually compiles"; header
     said the package's theme is "the only theme the package compiles"
   - The package: `@vaam-apps/ui` 0.2.4 in `node_modules`,
     `dist/styles/theme.css` registers two `@plugin "daisyui/theme"` blocks,
     `dark` (`default: true`) and `light` (`default: false`), since 0.1.2
   - Now: the case reads the registered names off the package's `theme.css`
     and asserts they are `dark` and `light`, that the layout pins exactly one
     `data-theme`, `dark`, and that it is one of them. Mutation:
     `data-theme="corporate"` in `app/layout.tsx` fails the case (1 failed / 7
     passed); reverted
   - Same claim fixed beside it: [AGENTS.md](../../../AGENTS.md) ("its one
     theme"), the dashboard README, `app/globals.css`'s comment, and
     `.storybook/preview.ts` ("`@vaam-apps/ui` 0.1.2 registers")
3. **`frontends/apps/checkout/src/a11y-gate.test.ts` — not real, as stated.**
   That file names no `@vaam-apps/ui` version at all
   (`git grep -n "0\.1\." -- frontends/apps/checkout` finds only
   `styling-gate.test.ts` and `testing/contrast.ts`, both saying "since
   0.1.2", which is true). The **dashboard's** `a11y-gate.test.ts` does name
   `@vaam-apps/ui@0.1.2`, as the version the `SideNav` `landmark-unique`
   defect (vaam-apps/ui#16) was found in. That attribution is history rather
   than a claim about today, and whether 0.2.4 still has the defect needs a
   browser at four widths; `side-nav.js:507-509` in 0.2.4 still has no
   unprefixed `hidden` beside the `xl:` utilities, which is consistent with it
   being present, not proof. Left alone
4. **[provider-port.md](../../flows/provider-port.md) § The interface**
   - Was: a table of six methods and "the three network methods"
   - The trait (`vpay-provider/src/lib.rs`): nine — `code`, `capabilities`,
     `submit`, `query_status`, `parse_callback`, `parse_destination`,
     `refund`, `account_holder_name` (2026-09-06, #49), `payer_fields`
     (2026-09-16, #186); four are `async`
   - Now: rows for `account_holder_name`, `payer_fields` (the `RailSpec` and
     `resolve_payer_fields` it drives, the empty-slice default, MTN's declared
     MSISDN) and `code`, and "four network methods"
5. **`vpay-api/src/model.rs`, "twelve keys"**
   - The pin: `every_documented_key_is_present_including_the_null_ones` lists
     thirteen keys and asserts `object.len() == 13`
   - Since: `customer`, 2026-09-06 (`0fc5dcf`)
   - Now: all six "twelve"s in `model.rs` say thirteen, as do three in
     `v1/payment_intents.rs` and one in
     `backends/tests/integration/tests/webhooks.rs`. Found while checking:
     `sdks/stripe-js/src/types.ts`'s `PaymentIntent` declares twelve of the
     thirteen — no `customer` — and said "the twelve keys of
     `PaymentIntentObject`". Its comment now says so; the interface itself is
     unchanged (a follow-up, below)
6. **[frontends/apps/dashboard/README.md](../../../frontends/apps/dashboard/README.md),
   "two" handlers**
   - The tree: six `route.ts` under `app/api/dash/` — `payment_intents` and
     `payment_intents/[id]` (`face3da`, 2026-09-11), `refunds`, `deliveries`
     and `customers` (`f9c7d2c`, 2026-09-13), `checkouts` (`8be7850`,
     2026-09-14)
   - Now: the BFF section's "Two `GET` route handlers" and its two-row table,
     and the layout table's "The BFF's two route handlers", both say six.
     Found while checking: the #243 correction a few lines below dated all
     four later handlers 2026-09-14; three are 2026-09-13, narrowed in place
7. **[resource-contract.md](../../flows/merchant-auth/resource-contract.md)**
   - Was: `DELETE /v1/customers/{id}` is "the only `DELETE` on this API"; no invoice route listed
   - `V1_ROUTES`: 37 method-and-path pairs, three `DELETE`s — customers,
     invoices, invoice items — and fifteen invoice and invoice-item pairs,
     mounted 2026-09-07 (S4b, `c49ca96`)
   - Now: a paragraph naming the fifteen and pointing at
     [invoices.md](../../flows/invoices.md) § "The surface", the way the page
     already points at hosted-checkout.md for the session routes; the row's
     claim struck through. It was true for one day, 2026-09-06
8. **`InvoiceStatus` (`vpay-core/src/state.rs`) and `void_in_tx` (`vpay-db/src/invoices.rs`)**
   - Was: a `draft ──void──> void` edge; "Voids a `draft` or `open` invoice"
   - The statement:
     `WHERE merchant_id = $1 AND id = $2 AND status = 'open' AND …`; a draft
     is deleted, and `number_is_assigned_at_finalize` would refuse a voided
     draft
   - Now: the diagram's second line is `DELETE` and nothing else; the first doc line says `open`
9. **`schemas/vpay.cstack`'s header, "FIVE models"**
   - Re-counted by `.run(..)`/`.run_in_tx(..)` on a delegate in non-test
     `vpay-db/src` code (a `#[cfg(test)]` module excluded, `migrations.rs`'s
     `sqlx::migrate!` run excluded), attributed to the accessor that starts
     the chain: **36 statements, fourteen models** — CheckoutSession 4,
     Credential 4, Currency 2, Customer 2, DisabledClient 3, Event 1, Invoice
     1, InvoiceItem 1, ManualPayment 1, OauthAuthorizationCode 3, Provider 1,
     StaffMember 5, StaffSession 7, WebhookDelivery 1. None: PaymentIntent,
     Charge, Refund, LedgerTransaction, LedgerEntry, RateLimitWindow. Matches
     the skills audit's 14 of 20 and 36
   - Now: the header box, its title line and its "every other model below"
     sentence; the root [README.md](../../../README.md)'s "Thirteen of the
     file's nineteen" (one merge behind `ManualPayment`); and a dated
     re-measurement on [infrastructure.md](../infrastructure.md)'s row
10. **[cratestack-what-runs-through-it.md](../../reference/vpay-db/cratestack-what-runs-through-it.md)**
    - The page is a verbatim move and says so, so its 2026-09-10 registry was
      not rewritten. It now opens with a dated banner: the registry is a
      snapshot, today's figures are item 9's, and what moved since —
      `credentials` (+4), `manual_payments` (+1), `staff_members` 7 → 5
      (`enrol_totp`, `record_totp_step`, `set_password` gone, `delete` new),
      `staff_sessions` 6 → 7 (`delete_others`), and `customers`' `delete` now
      `hard_delete` on `run_in_tx`. 32 + 4 + 1 − 2 + 1 = 36
11. **[errors.md](../../flows/errors.md) § Binaries**
    - Was: "`find_in_chain::<ConfigError>` first, then `DbError`"
    - `exit_code_for` (`vpay-server/src/main.rs:217`): `StartupError`,
      `ConfigError`, `SigningKeyError`, `DbError`, then `Category::Internal`.
      `StartupError` and `SigningKeyError` were already there at `312098c`
      (Step 1, 2026-09-02); the sentence is from `b6b06d6`, earlier the same
      day
12. **[stripe-sdk-compat.md](../../flows/stripe-sdk-compat.md)**
    - Was: `customer` among the accepted-and-dropped create parameters
    - `git log -S` on `v1/payment_intents.rs`: `CreateParams::customer` since
      `0fc5dcf`, 2026-09-06, stored and rendered, and a foreign `cus_…` is a
      `400`
13. **[parity.md](../../sdks/parity.md)'s gap ledger**
    - Was: "iOS and macOS hosts … compiled by nobody"
    - The matrix row above it and
      [mobile-flutter-plugin.md](../mobile-flutter-plugin.md) (corrected
      2026-09-20): iOS built and run on an iPhone 17 Pro Simulator since
      2026-09-16 — which
      [2026-09-16-flutter-browser-cutover.md](2026-09-16-flutter-browser-cutover.md)
      records as carried over from the change, not re-run by its own pass.
      macOS: still compiled by nobody
    - Now: the ledger row struck and narrowed to macOS, the row kept open
14. **"both binaries"**
    - One binary, `vpay-server`, with `serve`, `worker` and `staff add`, since
      2026-09-07 (issue #77)
    - Now: the root README (three places) and `open_migrated_database`'s doc
      comment, which is called from all three modes (`main.rs:557`,
      `main.rs:818`, `worker.rs:255`); the two that bind a port call it first

## Left alone, on purpose

- The out-of-scope list this pass was given: `erase_in_tx`'s doc comment,
  `vpay-api/src/provider_callback.rs`'s module header, anything about job
  scheduling or clocks, and ADR files.
- Other "both binaries" sentences, each in a file this pass had no other
  reason to open: `.config/nextest.toml`, `.env.example`, `.github/workflows/ci.yml`,
  `.xtask/src/main.rs:2542`, `Cargo.toml`, `vpay-api/Cargo.toml` (three),
  `vpay-api/src/lib.rs:34` and `vpay-server/src/main.rs:492`.
- [cratestack.md](../../reference/vpay-db/cratestack.md)'s "sixteen of the
  thirty-two statements" and "twelve tables wide as of 2026-09-10": dated,
  on a verbatim-moved page, and the registry page now carries the current
  figures.
- `docs/flows/dashboard/status-styling-and-demo.md`'s "compiles one theme":
  a dated narrative of the 2026-09-12 cutover.
- The skills. No parity trigger in AGENTS.md § "Docs↔skills parity" moved:
  no flow page, route, token, gate, toolchain pin or path.

## Follow-ups, not done

- `sdks/stripe-js/src/types.ts`'s `PaymentIntent` does not declare
  `customer`. Adding it is an SDK type change with its own parity row.
- `MODEL_TABLES`' table column is still hand-written. The model column is
  now checked; a wrong table name beside a right model would still probe the
  wrong path. `models::<NAME>_MODEL.table_name` is the generated source a
  stricter version could read.

## What was run

On macOS, branch `docs/retire-stale-claims-2026-09-23` off `master` at
`9184e42`, with `CARGO_INCREMENTAL=0` and line-tables-only debug info:

- **Item 1's test.** `cargo nextest run -p vpay-db --lib`: 128 run, 128
  passed, 0 skipped, including
  `no_generated_model_route_is_mounted_only_the_one_procedure_is`. Mutation:
  the `("ManualPayment", "manual_payments")` pair deleted (and the array
  length set back to 19) fails it on the new check, `left` listing nineteen
  models and `right` twenty with `ManualPayment` among them; restored.
- **Rust tests for touched files.**
  `cargo nextest run -p vpay-core -p vpay-api -p vpay-db`: 723 run, 723
  passed, 0 skipped.
  `--test webhooks` in `vpay-tests-integration` (one doc comment touched):
  the **first** run was 18 passed, 3 failed, all three panicking on
  `the fan-out job is claimable`
  (`a_cancel_emits_one_payment_intent_canceled_and_it_reaches_the_receiver`,
  `a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan`,
  `a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled`), on a
  host with a load average near 12 from other builds. `master` at `9184e42`,
  same host, straight after: 21 of 21. This branch, twice more: 21 of 21 both
  times. Nothing this branch changes is on that path. The panic is the one
  #254 (`d08dafd`) diagnoses and fixes — `claim_fanout_job` enqueues at the
  host's clock and claims at the Postgres container's — which landed on
  `master` while this branch was being checked and is **not** in it; so the
  flake is expected here until the branch is rebased, and was not
  reproduced on purpose.
- `just clippy`: exit 0, no warnings.
- `just test-doc`: exit 0 — 124 passed, 0 failed, 1 ignored (`sdks/rust`'s
  README doctest at line 503, which predates this branch).
- `just verify`, with every change staged: exit 0, "the fifteen gates above
  passed". `verify-links` 2 051 links in 427 files (2 050 on the first run, before this page was finished); `verify-doc-counts` 13
  counts in 11 of 274 files, including the `files-with-suffix` marker in
  [../README.md](../README.md), moved from 65 to **66** by this page;
  `verify-status` 1 unimplemented item; `verify-sdk-parity` 767 / 45 / 35 /
  40; `verify-migrations` 49 files; `verify-versions` 24 references at 0.5.0;
  `check-schema` OK on the edited `vpay.cstack`.
- `just test-web`: exit 0 — 102 test files, 1 506 passed, 2 skipped. The two
  skips are each app's `a11y-gate.test.ts` case that reads a built
  `storybook-static/`, which `ctx.skip()`s when there is none.
  `frontends/apps/dashboard/src/layout.test.tsx` alone: 8 passed; with
  `data-theme="corporate"` in `app/layout.tsx`, 1 failed / 7 passed;
  reverted.
- `just lint-web`: exit 0.
- `just fmt`: prettier re-padded the tables in `resource-contract.md`,
  `provider-port.md`, `parity.md`, `infrastructure.md` and the dashboard
  README, and wrapped one regex in `layout.test.tsx`. `cargo fmt` moved
  nothing.

**Not run:** `just ci` as a whole, `just test-rust` beyond the packages
above, `just test-storybook`, `just test-e2e` and `just docs-check-citations`.
