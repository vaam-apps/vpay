# Migrations: the immutability rule, and repairing a database that applied the original 0028

**Trigger:** `vpay-server` or `vpay-worker` exits 78 at boot with
`migration <n> was previously applied but has been modified`.

**Also read this before adding a migration.** The rule and the mechanics of
adding one live in
[`backends/migrations/README.md`](../../backends/migrations/README.md); this
page is the operational half — what to do when a database is already in the
broken state.

---

## 1. The rule

`sqlx::migrate!` stores a **SHA-384 of each migration file's whole bytes**,
comments included, in `_sqlx_migrations.checksum`. At every boot it re-hashes
the files on disk and refuses to run if any applied migration's hash has moved.

So **a migration file that has shipped is never edited**. Not for a typo, not
for a comment, not for a rename. Editing one does not change a database that
already applied it — it stops that database booting the new binary, and the
only way back is a hand-written `UPDATE` against the migrator's own bookkeeping
table, which is what the rest of this page is.

To correct an applied migration, write a **new** migration that corrects it.

`backends/migrations/MANIFEST.sha256` and `cargo xtask verify-migrations`
(`just verify-migrations`, a gate in `just verify` and in CI's `self-checks`
job) make the rule enforceable: a migration file whose SHA-256 has moved fails
the build, and `just migrations-manifest` refuses to rewrite an existing line,
so the gate cannot be silenced by regenerating.

**What the gate does not stop, stated plainly.** A contributor who edits a
migration *and* hand-edits its line in `MANIFEST.sha256` passes the gate. That
is by construction, and hashing the manifest would not fix it: a manifest whose
own hash is checked has to pin that hash somewhere, and whoever can edit two
files can edit three. The manifest was not added to make the edit impossible —
it was added to make it **visible**, as a one-line diff on a file whose only
purpose is to be reviewed. A comment reflowed inside a 400-line `.sql` file is
not visible; that is how issue #76 happened. Review the manifest line.

---

## 2. What happened to 0028 (issue #76)

PR #39 (the npm scope rename to `@vaam-apps`) rewrote **one line of comment**
inside `backends/migrations/0028_create-checkout-sessions.sql` after it had
shipped in commit `d0b602e` — an old package name in a note about `ui_mode`.
No SQL changed. The whole edit:

```bash
git diff d0b602e HEAD -- backends/migrations/0028_create-checkout-sessions.sql
```

sqlx hashes the file, comments included, so that one line is a different
migration as far as every database is concerned. (The retired package name is
not quoted on this page: `cargo xtask verify-npm-scope` fails the build on any
occurrence of it outside `docs/plans`, `docs/adr` and `docs/status.md`, and
this runbook is not one of the places that record history. Run the command
above to see the exact bytes.) Every stack brought up between
#37 and #39 now exits 78 on boot:

```
migration 28 was previously applied but has been modified
```

The decision (issue #76) was **not** to edit the file back: databases created
after #39 — including every one a new deployment will create — carry the new
checksum, and reverting would break those instead.

---

## 3. Which state is your database in?

There are two SHA-384 values in play and they are 96 hex characters of noise
each. Do not eyeball them; ask the database.

```bash
psql "$DATABASE_URL" -tAc \
  "SELECT encode(checksum, 'hex') FROM _sqlx_migrations WHERE version = 28;"
```

| What it prints | State | Action |
|---|---|---|
| `6eeb31ee…07b5ec` | applied the **current** file | nothing — this database boots |
| `f4d1a8e1…8ae252` | applied the **original** file (created between #37 and #39) | §4 |
| anything else | not a state this page describes | stop; escalate |

The two values in full:

- **Current** — the file in the tree today, and what the running binary
  demands:

  ```
  6eeb31eeaf5e8adfe033b01c9ae7a8c579e68e543fef93d8febd41d32f6234d02e8fb63f03cb2e4137e187f59007b5ec
  ```

  Obtained from a real database, not by hand:
  `the_0028_repair_in_the_runbook_fixes_a_database_that_applied_the_original`
  (`backends/tests/integration/tests/postgres_smoke.rs`) applies the current
  migrations to a fresh `postgres:16-alpine` and reads
  `_sqlx_migrations.checksum` for version 28. It **fails if this page does not
  carry that value**. The same digest is what
  `sha384sum backends/migrations/0028_create-checkout-sessions.sql` prints.

- **Original** — what a broken database already holds:

  ```
  f4d1a8e11606df3e3d0b3fb2a0a0483668b53813fb1baa6598ca3fdc2e105db162c4d43e8ea0e2790a4baecbce8ae252
  ```

  ```bash
  git show d0b602e:backends/migrations/0028_create-checkout-sessions.sql | sha384sum
  ```

**The repair writes the first value.** Writing the second — the original — is
the mistake this page was first published with: it is what the broken database
already holds, so the `UPDATE` reports `UPDATE 1`, changes nothing, and the
binary keeps exiting 78.

---

## 4. The repair

Only for a database that printed the **original** value in §3. Take a backup
first ([restore-from-backup.md](restore-from-backup.md)); this writes to the
migrator's bookkeeping table, and nothing else in vpay ever does.

Stop the server and worker, then, once:

```sql
UPDATE _sqlx_migrations
SET checksum = decode('6eeb31eeaf5e8adfe033b01c9ae7a8c579e68e543fef93d8febd41d32f6234d02e8fb63f03cb2e4137e187f59007b5ec', 'hex')
WHERE version = 28;
```

Expect `UPDATE 1`. `UPDATE 0` means this database never applied migration 28 at
all and the boot failure is something else.

**This changes no schema and no data.** Migration 0028's SQL is byte-identical
across the two versions of the file — only a comment differs — so the tables,
columns, indexes and constraints the database already has are exactly what the
current file would have created. That is what makes the repair safe here, and
it is a fact about *this* migration, not a general licence.

### Confirm it is fixed

```bash
psql "$DATABASE_URL" -tAc \
  "SELECT encode(checksum, 'hex') FROM _sqlx_migrations WHERE version = 28;"
```

must now print the current value, and the server must boot:

```bash
kubectl rollout restart deploy/vpay-server deploy/vpay-worker   # or: docker compose up -d
kubectl logs -l app.kubernetes.io/name=vpay --tail=50
```

A clean boot logs the migrator applying **no** migrations and the server
binding its listeners. Any remaining `previously applied but has been modified`
names a *different* version, which is not this incident: some other migration
was edited after it shipped, and §5 applies.

---

## 5. A checksum mismatch on any other migration

Do **not** copy §4's `UPDATE` with a different version number. §4 is safe only
because 0028's SQL is provably unchanged. For any other version:

1. Find the edit: `git log --follow -p backends/migrations/<file>`.
2. If only comments moved, the repair is the same shape — but compute the
   current value yourself (`sha384sum <file>`) and confirm the SQL is identical
   across the diff before writing anything.
3. If any **statement** changed, the databases are genuinely divergent: they
   ran different SQL. A checksum update would paper over a schema difference.
   Reconcile the schema with a new migration first, and escalate.
4. Either way, the edit itself is a defect. Revert the migration file to the
   bytes the manifest records and correct it with a new migration, so no
   further database diverges.

---

## Status

The rule, the gate and the manifest are built and exercised. `just verify` runs
`verify-migrations`; twelve unit tests in `.xtask` pin every way it can fail.

**§4's `UPDATE` has been executed, and this page's copy of it is the one that
ran.** `the_0028_repair_in_the_runbook_fixes_a_database_that_applied_the_original`
(`backends/tests/integration/tests/postgres_smoke.rs`) applies the current
migrations to a fresh `postgres:16-alpine`, rewinds `_sqlx_migrations.checksum`
for version 28 to the original value in §3, confirms `sqlx::migrate!` then
refuses with the message §2 quotes, parses the `UPDATE` out of *this markdown
file*, runs it, and confirms the migrator runs clean afterwards. So the trigger,
the repair and the confirmation are all measured, and the SQL cannot drift from
what was tested without failing the build.

**What has NOT been done:** no deployment exists, so §4 has never been followed
against one — the `kubectl` and `docker compose` commands in it have not been
run anywhere, and §3's `psql` invocation has been run only against a scratch
container. §5 is written from the same reasoning and has never been needed.
