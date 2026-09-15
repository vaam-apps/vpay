# `backends/migrations`

Every `.sql` file here is applied, in filename order, by
`sqlx::migrate!("../../migrations")` — from `vpay_db::migrations`, which both
binaries call at boot, and from every container-backed test suite.

## The one rule: a migration that has shipped is never edited

`sqlx::migrate!` records a **SHA-384 of each file's whole bytes, comments
included**, in `_sqlx_migrations.checksum`, and refuses to run against a
database whose recorded checksum no longer matches the file. Changing so much
as a word in a comment therefore stops every database that applied the original
from booting:

```
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
`0046_refunds-comments-mtn-refund-is-written.sql` corrects `0017`'s
`COMMENT ON TABLE refunds` and `0042`'s `COMMENT ON COLUMN
invoices.amount_refunded`, following `0020`'s precedent.

| File                                | What its header still says                                                                                  | The correction                                                                                                                                                                                                                                                         |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0017_create-refunds.sql`           | "no adapter `refund` implementation — the port's refund path is still `ProviderError::NotImplemented`"      | False since **2026-09-15**. `mtn_momo::refund` makes MTN's Disbursements `transfer` call (RFC-0003 § 5). The rest of that paragraph stands: nothing writes or reads the table.                                                                                         |
| `0031_refunds-fee.sql`              | "`mtn_momo::refund` is the one remaining `NotImplemented` token"                                            | False since **2026-09-15**, twice over: that token is retired and there is now **no** `NotImplemented` token in the workspace at all (`cargo xtask verify-status` prints `0 unimplemented item(s)`). The `fee` column is still never written, which is its real point. |
| `0042_invoices-amount-refunded.sql` | "NO RAIL CAN REFUND YET (`ProviderAdapter::refund` is `NotImplemented` on MTN and `Unsupported` on Orange)" | False since **2026-09-15** for MTN, in the header as well as in the `COMMENT ON` that `0046` corrects. Orange is unchanged.                                                                                                                                            |

**Zero `NotImplemented` tokens is not "everything is built."** No deployment
holds an MTN Disbursements subscription key and nothing in this repository has
ever called that product. `docs/status.md` and
`docs/flows/adapter-mtn-momo.md` are the current record for all three rows.

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
