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
