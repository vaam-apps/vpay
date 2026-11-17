# Issue #79 item 4: the payer mask is written, backfilled and constrained

_Written 2026-11-17 on `impl/79-payer-mask`, branched from local `batch3`
(`fdc9a965`: `master` + PR #403 + the integrated #396 work, not yet merged).
What changed is in [backend.md](../backend.md)'s payer-mask row; this page is
what was run._

## Items 1–3 of #79, checked rather than assumed

Commits `e9e9a845` and `fa8f6629` are **not ancestors** of `origin/master`
(`git merge-base --is-ancestor` exits 1): `master`'s history was squashed, and
its oldest commits (`3633ca0c`, #345) already carry their content —
`staff_auth.trusted_proxies` in `vpay_api::staff::client_address`,
`rate_limit_windows` (migration `0038`) behind `vpay_db::rate_limits`, and
`PasswordRequest::current_password` in `vpay_api::staff`. `f885170c` (#383,
VP-6a) then **deleted the staff session plane** those three items hardened;
VPA-2's `0058` brought the sign-in tables back. So items 1–3 landed, and their
subject has since been retired and partly restored. This commit does not
touch them.

## The decision, and what "first three" was read as

"`charges.payer_ref_masked` is written at `open_attempt` from the normalised
MSISDN (first three and last two digits)". Read literally: the normalised
MSISDN is `payer_fields`' digits-only E.164 (`237671234567`), so the first
three digits are the country code. The mask is `+237••••67`. Representation
(leading `+`, a **fixed** four U+2022 bullets) is the one #396's
`account_holders::masked` already used; its digit counts (four and three,
`+2376••••200`) are not the decision's. **The log mask and the stored mask are
deliberately separate until RFC-0002 D3 decides the log and telemetry
shape**: #79 decided only the stored column, so `account_holders::masked`
and the refund log line are unchanged from `batch3`, and only the column
uses `vpay_core::privacy::payer_msisdn_mask`. (A first version of this
commit moved the log mask onto the new function too; review reverted that as
deciding D3 by stealth. #396's tests are byte-for-byte `batch3`'s; its docs
and two code comments changed only where they said nothing writes the
column or that the log shape is the column's.)

`open_attempt` built the charge row when the decision was written; since
ADR-0032 (#345) the row is built by `open_charge`, one step earlier, and that
is where the mask is written — in the same `INSERT` as `payer_ref`.

Per rail, from the code: `mtn_momo` and `notchpay` are `ProviderFlow::Push`
with a declared `msisdn` payer field, so they get a mask; `orange_money` is
`ProviderFlow::Redirect`, `payer_ref` is `None`, so the mask is `NULL`.

## Tests, each broken and watched fail

| Test                                                                                                                                                                 | Mutation                                                                | Result under the mutation                                                                                                                                       |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vpay-core` `the_payer_mask_keeps_the_first_three_and_last_two_digits`, `the_payer_mask_does_not_reveal_how_many_digits_it_hides`                                    | `PAYER_MASK_TAIL_DIGITS = 3`                                            | both FAIL (the third, `…refuses_anything_but_a_normalised_number`, passes, as it should)                                                                        |
| `confirm_rails.rs` `a_push_confirm_writes_the_payer_mask_from_the_normalised_number`                                                                                 | `open_charge` writes `payer_ref_masked: None` again                     | FAIL at the first assertion                                                                                                                                     |
| `vpay-db` `migration_0062_backfills_the_payer_mask_and_leaves_the_rest_null`                                                                                         | backfill without the `~ '^[0-9]{8,15}$'` predicate                      | FAIL: `check constraint "payer_ref_masked_derives_from_payer_ref" … is violated by some row` — the redacted and verbatim rows were masked                       |
| same                                                                                                                                                                 | migration without the CHECK                                             | FAIL: `the unmasked number in the masked column: PgQueryResult { rows_affected: 1 }`                                                                            |
| same (two rows carry a pre-existing non-derived mask: the unmasked number, and a mask beside `[redacted]`)                                                           | migration without the clearing `UPDATE` before `ADD CONSTRAINT`         | FAIL: `migration 0062 applies to a table that already has rows, garbage masks included` — `check constraint … is violated by some row`, i.e. boot would fail    |
| same, **first run, unmutated**                                                                                                                                       | — (a real defect)                                                       | FAIL: `a mask on a charge that has no number` was accepted: `NULL ~ '…'` is NULL and a CHECK passes on NULL. Fixed with `payer_ref IS NOT NULL`, then PASS      |
| `customers.rs` `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`, `an_erasure_reaches_a_payment_whose_only_link_to_the_payer_is_a_checkout_session` | `erase_in_tx`'s charge `UPDATE` stops setting `payer_ref_masked = NULL` | both FAIL: the erasure answers `503 database_query_failed` — the CHECK refuses a mask beside the marker and the whole erasure rolls back rather than leaving it |

Every mutation was reverted; `0062`'s bytes were restored from a copy and its
hash re-checked against `MANIFEST.sha256`
(`8d8a138d…5ba6`).

## Runs (all against real Postgres 16 via testcontainers; Docker was up)

| Command                                                                                                                    | Result                                                  |
| -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| `cargo nextest run -p vpay-core`                                                                                           | 98 passed, 0 skipped                                    |
| `cargo test --doc -p vpay-core privacy`                                                                                    | 5 passed (incl. `payer_msisdn_mask`'s), 0 ignored       |
| `cargo nextest run -p vpay-api --lib`                                                                                      | 570 passed, 0 skipped                                   |
| `cargo nextest run -p vpay-db`                                                                                             | 303 passed, 0 skipped                                   |
| `cargo nextest run -p vpay-tests-integration --test dashboard_read_surface --test postgres_smoke`                          | 75 passed, 0 skipped (drift test unchanged at 232 / 31) |
| `cargo nextest run -p vpay-tests-integration --test customers`                                                             | 28 passed, 0 skipped                                    |
| `cargo nextest run -p vpay-tests-integration --test confirm_rails` (MTN, Orange and Notch Pay WireMock stubs)              | 21 passed, 0 skipped                                    |
| `pnpm run typecheck` / `lint` / `test` in `frontends/apps/dashboard`                                                       | clean / clean / 580 passed, 1 skipped (pre-existing)    |
| `cargo clippy -p vpay-core -p vpay-db -p vpay-api -p vpay-provider -p vpay-tests-integration --all-targets -- -D warnings` | clean                                                   |

After the review amendment (log mask reverted to `batch3`, the clearing
`UPDATE` and two pre-existing-mask rows added, the `PAYER_MASK_MIN_DIGITS`
comment reworded) these were re-run: `vpay-core` 98 passed + 5 doctests,
`vpay-api --lib` 570 passed (#396's own mask tests among them, unchanged),
`vpay-db` 303 passed, `postgres_smoke` + `dashboard_read_surface` 75 passed,
all 0 skipped; clippy on `vpay-core`, `vpay-db`, `vpay-api` clean. The
`confirm_rails` and `customers` suites were not re-run: neither the confirm
path nor erasure changed, and a fresh database has no row the clearing
`UPDATE` touches.

Two environment events, recorded because they explain two red runs that are
not about this change: the disk filled during the first full `vpay-db` run
and the Docker daemon exited; the daemon was restarted and the suite re-run
in full (the 303 above). And the shared `CARGO_TARGET_DIR` once handed a
build another worktree's `vpay_core` (`payer_msisdn_mask` not found); touching
the changed sources and rebuilding cleared it. It did the same once to `just
verify` after the amendment: a cached `xtask` built in another worktree
counted 61 migrations there; touching `.xtask/` rebuilt it, and the `just
verify` output above is from this tree (62 migrations, 547 tracked markdown
files).

## Gates

`just verify`, run with every change staged, exit 0:

```text
verify-migrations: ok — 62 migration file(s) in backends/migrations/ all match their entries in backends/migrations/MANIFEST.sha256
verify-privacy-inventory: ok — 347 database column(s) classified in schemas/privacy-inventory.yaml across 30 elements (20 personal-data, 21 necessary), …
verify-doc-counts: ok — 9 documented count(s) in 9 of 314 markdown file(s) agree with what this tree measures
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.16.0
check-schema: ok — frontends/packages/dash-client matches schemas/vpay.cstack
verify-links: ok — 3318 repository link(s) in 547 tracked markdown file(s) resolve to a tracked path …
verify: ok — the seventeen gates above passed; the verify-docs report is advisory
```

The other eleven printed `ok` as well (`verify-ui` prints nothing on
success). `cargo fmt --all -- --check` is clean. `charges.payer_ref_masked`
was already classified (`payer_msisdn`, control `redact`) in
`schemas/privacy-inventory.yaml`, so the inventory did not change; it is the
same personal-data element as `payer_ref`, which is what erasure treats it as.

## Not done

- **The dashboard application** renders nothing from `/dash/v1` until VP-6b.
  `PaymentDetailView` already renders the column (its comments and tests now
  say the column is written, and the test fixture uses the real shape), but
  no page mounts it, so no person sees a mask yet.
- **No normalisation in SQL.** A pre-`payer_fields` row whose `payer_ref` was
  stored verbatim keeps a `NULL` mask; how many such rows exist in any
  deployment was not measured (no deployment data is reachable from here).
- **RFC-0002 D3 stays open** for every other identifier and surface; this is
  #79's decision for one column.
- The full workspace suite and `just ci` were not run (GitHub CI runs them).
