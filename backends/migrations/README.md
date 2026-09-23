# `backends/migrations`

Every `.sql` file here is applied, in filename order, by
`sqlx::migrate!("../../migrations")` — from `vpay_db::migrations`, which both
binaries call at boot, and from every container-backed test suite. There are
**49** <!-- count:files-with-suffix backends/migrations .sql --> of them,
re-measured 2026-09-20 and gated from that day by
`cargo xtask verify-doc-counts`, which re-counts the directory on every
`just verify` — so this figure cannot drift the way the prose counts elsewhere
in this repository did until then.

## The one rule: a migration that has shipped is never edited

`sqlx::migrate!` records a **SHA-384 of each file's whole bytes, comments
included**, in `_sqlx_migrations.checksum`, and refuses to run against a
database whose recorded checksum no longer matches the file. Changing so much
as a word in a comment therefore stops every database that applied the original
from booting:

```text
migration 28 was previously applied but has been modified
```

That is not hypothetical. The `@vpay` -> `@vaam-apps` npm rename (PR #39)
rewrote one comment inside `0028_create-checkout-sessions.sql` after it had
shipped, and every stack brought up before it exited 78 on the next boot
(issue #76).

**To fix a mistake in an applied migration, write a new migration that
corrects it.** Never touch the old file.

That works for anything SQL can reach — a `COMMENT ON`, a constraint, a
default. It does **not** work for a `--` comment in the file's header, which
is in no database and which no statement can address. Those go in § Errata
below.

## Errata: statements in applied migrations that are now false

A migration's header is frozen prose. When the world moves under one, the
correction is a line here, dated, naming the file and the current record —
because a reader who opens the `.sql` file has this README one directory up,
and has nothing else.

A stale `COMMENT ON` is a different thing and does **not** belong here: it is
in every database that applied the migration and an operator sees it in
`\d+`. Correct one with a new migration, as
`0047_refunds-comments-mtn-refund-is-written.sql` corrects `0017`'s
`COMMENT ON TABLE refunds` and `0042`'s `COMMENT ON COLUMN
invoices.amount_refunded`, following `0020`'s precedent.

| File                                | What its header still says                                                                                                                                                                                                                                                                                                                        | The correction                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0017_create-refunds.sql`           | "no refunds repository in `vpay-db`, no `/v1/refunds` route, and no adapter `refund` implementation"                                                                                                                                                                                                                                              | False since **2026-09-15**, three times over, and **wholly false since 2026-09-16**. `vpay_db::Refunds` exists and has a `create` (RFC-0003 § 3); `GET /v1/refunds/{id}` is routed (issue #45); `mtn_momo::refund` makes MTN's Disbursements `transfer` call (RFC-0003 § 5); and `POST /v1/refunds` is routed (RFC-0003 § 2), so a deployment **can** create a row. `0048` restates the deployed `COMMENT`. What stands: no rail has returned money, and nothing settles a `pending` refund. |
| `0031_refunds-fee.sql`              | "`mtn_momo::refund` is the one remaining `NotImplemented` token"                                                                                                                                                                                                                                                                                  | False since **2026-09-15**: that token is retired, but it was not the last one — `orange_money::refund` became one the same day (RFC-0003 § 5), so `cargo xtask verify-status` prints one unimplemented item, not zero. The `fee` column is still never written, which is its real point.                                                                                                                                                                                                    |
| `0042_invoices-amount-refunded.sql` | "NO RAIL CAN REFUND YET (`ProviderAdapter::refund` is `NotImplemented` on MTN and `Unsupported` on Orange)", plus a GAP note reading "that insert does not exist anywhere in this repository — … `vpay_db::Refunds` exposes no create" and "`payment_intents.amount_refunded` and `amount_refund_pending` … are NOT maintained by the settlement" | All three false since **2026-09-15**, in the header as well as in the `COMMENT ON` that `0047` corrects. MTN's refund is written; Orange's is a `NotImplemented` token rather than `Unsupported`; `vpay_db::Refunds::create` inserts the row; and `vpay_db::settlement::apply_refund_succeeded` / `apply_refund_failed` do maintain both intent counters. `0047` restates all of it in the deployed `COMMENT`.                                                                               |

**A mounted route is not "everything is built."** `POST /v1/refunds` has been
routed since **2026-09-16** (RFC-0003 § 2) — this paragraph said it was routed
nowhere, and that "no refund can be created through `/v1` at all", until then.
A refund can be created now; what still has not happened is that **no rail has
ever returned money** (no REAL MTN Disbursements credential exists in this
project — the only subscription key anywhere is the e2e/demo stack's stub,
pointed at a WireMock container — and nothing in this repository has ever
called that product;
`orange_money::refund` is still a declared token) and **nothing settles a
`pending` refund**, because the port has no refund status read. `docs/status.md`
and `docs/flows/adapter-mtn-momo.md` are the current record for all three rows.

## Adding a migration

1. Write `NNNN_short-name.sql`, numbered one above the current highest.
2. `just migrations-manifest` — appends its SHA-256 to `MANIFEST.sha256`.
3. Commit the `.sql` file and `MANIFEST.sha256` **in the same commit**.

`MANIFEST.sha256` is what makes rule 1 enforceable: `cargo xtask
verify-migrations` (`just verify-migrations`, a gate in `just verify` and in
CI's `self-checks` job) fails if any file's SHA-256 has moved, if a `.sql` file
here has no manifest line, or if a manifest line names a file that is gone.
`just migrations-manifest` **refuses** to rewrite an existing line, so the gate
cannot be silenced by regenerating.

The manifest's SHA-256 is not the checksum sqlx stores; it is the digest
`sha256sum` writes, so any line here can be checked by hand:

```bash
sha256sum backends/migrations/0028_create-checkout-sessions.sql
```

The SHA-384 values sqlx actually stores, and the one-off repair for a database
that applied the original 0028, are in
[../../docs/runbooks/migrations.md](../../docs/runbooks/migrations.md).
