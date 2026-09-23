# 2026-09-23 — `just migrations-manifest` on macOS and Linux

Earlier the same day the `0049` line in `backends/migrations/MANIFEST.sha256`
was appended by hand with `shasum -a 256`, because the recipe failed on macOS
([2026-09-23-manual-payments.md](2026-09-23-manual-payments.md)). This page
records the fix and the output of both platforms. Nothing here changes a gate,
the manifest or any migration. The only change is how the recipe lists files
and which tool it hashes them with.

## What was wrong

Reproduced on this branch's base (`origin/master` at `b747e5d`), macOS 26.5.1:

```text
$ just migrations-manifest
find: -printf: unknown primary or operator
error: recipe `migrations-manifest` failed with exit code 1
```

`set -euo pipefail` stopped the recipe before it wrote the manifest, so the
failure was loud and left the tree clean. It never corrupted anything. It just
could not be used. `sha256sum` was **not** the blocker on this machine:
macOS 26.5.1 ships `/sbin/sha256sum`, and it prints the same digest as
`shasum -a 256`. There is no `sha256sum` in `/usr/bin`, though, and an older
macOS or a trimmed `PATH` may have none at all. The recipe now handles that
case too, and the case is tested below.

## What changed

The listing and the hash, in the `justfile`. Before:

```bash
find "$migrations_dir" -maxdepth 1 -type f -name '*.sql' -printf '%f\n' \
    | LC_ALL=C sort \
    | while IFS= read -r filename; do
        printf '%s  %s\n' "$(sha256sum "$migrations_dir/$filename" | cut -d' ' -f1)" "$filename"
    done > "$current"
```

After (the loop also checks that the digest is 64 lowercase hex characters
before it writes the digest):

```bash
if command -v sha256sum >/dev/null 2>&1; then
    digest() { sha256sum < "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    digest() { shasum -a 256 < "$1" | cut -d' ' -f1; }
else
    echo "migrations-manifest: neither sha256sum nor shasum is on PATH." >&2
    exit 1
fi
find "$migrations_dir" -maxdepth 1 -type f -name '*.sql' -exec basename {} \; \
    | LC_ALL=C sort \
    | while IFS= read -r filename; do
        hash=$(digest "$migrations_dir/$filename")
        # ... 64-hex check ...
        printf '%s  %s\n' "$hash" "$filename"
    done > "$current"
```

- **The selection is the same.** It still uses `find -maxdepth 1 -type f`
  (regular files only, dotfiles included), and only the output step changed. A
  shell glob would have skipped dotfiles and followed symlinks, which is a
  different set.
- **The order is the same.** It is still `LC_ALL=C sort`, and the committed
  manifest is in that order (`LC_ALL=C sort -c` on its filenames passes).
- **The hash reads stdin.** With a path argument, GNU `sha256sum` puts a `\`
  before the line if the name contains a backslash. From stdin, the name
  cannot change the output.
- **The rest of the recipe is unchanged.** It keeps the header verbatim,
  refuses a changed line, refuses a line whose file is gone, writes mode `644`,
  and writes `<64 hex>  <filename>\n` with a trailing newline. The one other
  edit: `added` goes through `tr -d ' '`, because BSD `wc -l` pads its count
  with spaces.

## macOS 26.5.1 (BSD `find`, `/bin/bash` 3.2)

On the clean tree:

```text
$ just migrations-manifest
migrations-manifest: ok — 0 line(s) appended to backends/migrations/MANIFEST.sha256
$ git diff --stat -- backends/migrations/MANIFEST.sha256
$ just verify-migrations
verify-migrations: ok — 49 migration file(s) in backends/migrations/ all match their entries in backends/migrations/MANIFEST.sha256
```

The append path used two copies of `backends/migrations/` in a scratch
directory, each with an extra `0050_dummy.sql`. The recipe ran against each
copy through its variables
(`just migrations_dir=… migrations_manifest=…/MANIFEST.sha256 migrations-manifest`).
The dummy was never in the worktree.

```text
== sha256sum path (/sbin/sha256sum)
migrations-manifest: ok — 1 line(s) appended to …/m1/MANIFEST.sha256
$ diff backends/migrations/MANIFEST.sha256 m1/MANIFEST.sha256
66a67
> e4051b0eb391e77068301fbea00bef0444d9f597662438a40d04afb645cc42ea  0050_dummy.sql
== shasum fallback (PATH=~/.cargo/bin:/usr/bin:/bin, so no sha256sum)
migrations-manifest: ok — 1 line(s) appended to …/m2/MANIFEST.sha256
m1 == m2 byte-identical
$ shasum -a 256 m1/0050_dummy.sql
e4051b0eb391e77068301fbea00bef0444d9f597662438a40d04afb645cc42ea  …/m1/0050_dummy.sql
== rerun is a no-op
migrations-manifest: ok — 0 line(s) appended to …/m1/MANIFEST.sha256
```

The refusals still fire, and the manifest stays unchanged after each one
(checked with `cmp` against a copy taken first):

```text
# after appending '-- edited' to m1/0028_create-checkout-sessions.sql
migrations-manifest: REFUSED — 0028_create-checkout-sessions.sql has been edited.
  ...
error: recipe `migrations-manifest` failed with exit code 1
manifest unchanged after edit refusal

# after restoring 0028 and deleting m1/0050_dummy.sql
migrations-manifest: REFUSED — 0050_dummy.sql is in the manifest but not on disk.
  ...
error: recipe `migrations-manifest` failed with exit code 1
manifest unchanged after deletion refusal
```

## Linux: Debian 13 (trixie) aarch64, GNU tools

This ran in `debian:stable-slim` with `just` 1.40.0 from apt. The worktree was
mounted read-only and each run worked on a copy inside the container. It ran
both the new recipe and the old one (`git show origin/master:justfile`).

```text
== userland
Debian GNU/Linux 13 (trixie) aarch64
find (GNU findutils) 4.10.0
sha256sum (GNU coreutils) 9.7
just 1.40.0
== new recipe, clean copy of the committed migrations
migrations-manifest: ok — 0 line(s) appended to /t/new/MANIFEST.sha256
byte-identical to the committed MANIFEST.sha256
db850a9a9c8ea9e8145d1d22261b5b1f3fe8aa3317aed0b7fb1bd5cb26ce9452  /src/backends/migrations/MANIFEST.sha256
db850a9a9c8ea9e8145d1d22261b5b1f3fe8aa3317aed0b7fb1bd5cb26ce9452  /t/new/MANIFEST.sha256
== old recipe (origin/master), same input, GNU tools
migrations-manifest: ok — 0 line(s) appended to /t/old/MANIFEST.sha256
old == new on Linux
== append path: committed migrations + 0050_dummy.sql
migrations-manifest: ok — 1 line(s) appended to /t/app/MANIFEST.sha256
66a67
> e4051b0eb391e77068301fbea00bef0444d9f597662438a40d04afb645cc42ea  0050_dummy.sql
Linux append output byte-identical to the macOS (shasum fallback) append output
migrations-manifest: ok — 1 line(s) appended to /t/appold/MANIFEST.sha256
old == new on the append path too
```

The new recipe gives the same bytes on both platforms, with either hasher, for
both the no-op case and the append case. Under GNU tools its output also
matches the old recipe's, byte for byte.

## What this does not cover

- It was not run on x86_64 Linux or on an older macOS without
  `/sbin/sha256sum`. The `shasum` fallback was tested by removing `/sbin` from
  `PATH`.
- `grep -F -- "  $existing_filename"` still matches a name as a substring, so
  it would also match a file whose name starts with another file's full name
  (`a.sql` and `a.sql.sql`). This change did not touch that, and no migration
  name hits it.
