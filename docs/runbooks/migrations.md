# Migrations: the immutability rule

**Applied migrations cannot be edited.** Every migration file's SHA256 hash is recorded in
`backends/migrations/MANIFEST.sha256`, and `verify-migrations` fails the build if any file
has been modified since it was added to the manifest. This rule holds because:

- A migration's SQL was run against production databases; if the file changes, every
  database that applied the original will fail at boot with a checksum mismatch.
- The only way forward is to apply a new migration that fixes what the old one broke.

## Adding a new migration

1. Write the migration file, e.g. `backends/migrations/0099_my_change.sql`.
2. Run `just migrations-manifest` to add its line to the manifest.
3. The manifest will refuse to change existing lines, so this operation is safe: it either
   appends a new line or fails if you tried to run it after editing a file.
4. Commit both the migration file and the updated `MANIFEST.sha256` in the same commit.

## Fixing a broken migration

If a migration has a bug and was already applied to production databases:

1. **Do not edit the original file.** That would break every production database.
2. **Create a new migration that fixes the problem**, e.g. `0100_fix_0099.sql`.
3. Run `just migrations-manifest` to record its hash.
4. Document why the fix was needed (comment in the new migration).

## For operators: repairing applied migrations

### Background

Migration 0028 (`0028_create-checkout-sessions.sql`) was modified after it shipped (PR #39,
the npm rename from `@vpay` to `@vaam-apps`). Every database that applied the original
checksum now fails boot with:

```
migration 28 was previously applied but has been modified
```

This is the **only** situation in which an applied migration's checksum needs to be
manually repaired in a database. The repair is a one-line SQL update to the `_sqlx_migrations`
table, and it applies only to databases that were created before the file was edited
and are still running the old checksum.

### The repair

If you are running a database that applied 0028 before the file was edited (i.e., you
saw this error when the binary tried to boot), run this SQL once:

```sql
UPDATE _sqlx_migrations
SET checksum = decode('f4d1a8e11606df3e3d0b3fb2a0a0483668b53813fb1baa6598ca3fdc2e105db162c4d43e8ea0e2790a4baecbce8ae252'::text, 'hex')
WHERE version = 28;
```

**Important:** This SQL is specific to migration 0028. Never run it for any other
migration. If you see a checksum mismatch for a different migration, your migration file
has been edited after it was applied to production, and this is not a bug — it is a
violation of the immutability rule.

### Explanation

`sqlx::migrate!` computes a SHA-384 checksum (96 hex characters) of the entire migration
file and stores it in the `_sqlx_migrations.checksum` column, a `bytea`. The value above is
the SHA-384 of `0028_create-checkout-sessions.sql` **in its original, unedited state** before
the npm rename (commit d0b602e).

To verify this value is correct:

1. Check out commit d0b602e or earlier (before the file was edited in 479ecfe).
2. Read the file: `git show d0b602e:backends/migrations/0028_create-checkout-sessions.sql`.
3. Compute its SHA-384:

   ```bash
   git show d0b602e:backends/migrations/0028_create-checkout-sessions.sql | sha384sum
   ```

   This will output: `f4d1a8e11606df3e3d0b3fb2a0a0483668b53813fb1baa6598ca3fdc2e105db162c4d43e8ea0e2790a4baecbce8ae252  -`

4. Compare this value to the one in the SQL above to confirm they match.

After running this SQL, the database will accept the current binary with the modified 0028
file, and subsequent upgrades will apply new migrations as normal.
