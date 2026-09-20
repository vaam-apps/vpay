# 2026-09-20 — `verify-doc-counts`, the fifteenth gate

Adds `cargo xtask verify-doc-counts` and the **fifteenth** `just verify` gate:
every number a document marks as countable must still equal what this tree
counts. This page is the gate's own output, and each of its failure modes
driven as a mutation of the real tree rather than described.

## Why, measured rather than assumed

A documentation survey the same day found **35** checkably-false claims in the
live docs. **Thirteen were a number that was right when somebody measured it
and drifted afterwards** — two flow pages' test-case counts, the migration
count, the environment-variable count, an SDK method count, and `just verify`'s
own gate tally.

The other half of that measurement is what made this a gate rather than a
cleanup: **every claim a gate already reads came through the same survey
clean.** `verify-serde` parses ADR-0016's exemption table, `verify-sdk-parity`
resolves 662 test citations, and neither produced a finding. All 35 were in
prose no gate reads.

## What landed

- `verify_doc_counts` in `.xtask/src/main.rs`, with six measurers —
  `tokio-tests`, `files-with-suffix`, `env-vars`, `pub-async-fn`,
  `dir-entries` and `verify-gates` — dispatched from `main`, in `verify-all`
  and in `just verify`'s recipe, and a `verify-doc-counts` step in CI's
  `self-checks` job.
- 13 markers across 11 documents, listed under § What is annotated.
- 23 tests in `doc_count_tests`, one of which runs the gate against this
  repository's own documentation.

## The gate on this tree

```text
$ cargo run -q -p xtask -- verify-doc-counts
verify-doc-counts: ok — 13 documented count(s) in 11 of 262 markdown file(s) agree with what this tree measures
```

## Every failure mode, driven on the real tree

Each mutation below was applied to the real file, the gate run, and the
mutation reverted. Nothing is paraphrased.

**1. A cited number the tree no longer measures** — `docs/status.md`'s gate
tally set back to 14:

```text
xtask: 1 documented count(s) no longer measure what they claim:
  - docs/status.md:110: this line says 14, but `count:verify-gates` measures 15. A reader sizes their expectations by the number in the sentence, and this one stopped being true without anything going red — which is the whole reason the marker is there. Run `grep '^verify:' justfile` to see it, then write 15 here; do not delete the marker
```

**2. An unknown kind** — `docs/flows/README.md`'s `dir-entries` renamed to a
plausible synonym. **This is a hard error, not a skip**, which is the single
most important thing about this gate: one that ignored what it did not
understand would print `ok` for a marker nothing measured.

```text
xtask: 1 documented count(s) no longer measure what they claim:
  - docs/flows/README.md:42: unknown count kind `subdirectories`. The kinds are: dir-entries, env-vars, files-with-suffix, pub-async-fn, tokio-tests, verify-gates. A gate that skipped a marker it did not understand would print success for a number nothing measured, so this is a failure and not a warning: add a measurer for `subdirectories` to `measure_count`, or use a kind that already exists
```

**3. A path that does not exist** — one character dropped from
`staff_sign_in.rs` in `docs/flows/dashboard/slice-1-gaps.md`:

```text
xtask: 1 documented count(s) no longer measure what they claim:
  - docs/flows/dashboard/slice-1-gaps.md:19: `backends/tests/integration/tests/staff_signin.rs` cannot be read (No such file or directory (os error 2)). The number this line states was measured from that file, so nothing can confirm it any more: point the marker at whatever replaced the file, or delete the claim together with its marker
```

**4. A marker with no digits before it** — `backends/migrations/README.md`'s
**48** spelled out as a word, which is exactly the shape this gate cannot read
and says so:

```text
xtask: 1 documented count(s) no longer measure what they claim:
  - backends/migrations/README.md:6: `count:files-with-suffix` guards no number — there is no digit before it on this line. Move the marker onto the line that carries the figure, immediately after it, and write the figure in digits: a count spelled out in words cannot be checked. It measures 48 today (`ls backends/migrations/*.sql | wc -l`)
```

**5. The wrong number of arguments** — the same marker with its `.sql` suffix
dropped:

```text
xtask: 1 documented count(s) no longer measure what they claim:
  - backends/migrations/README.md:6: `count:files-with-suffix` takes 2 argument(s) — a directory and a filename suffix — and was given 1: `backends/migrations`
```

**6. The gate removed from the `verify` recipe.** The case the `verify-gates`
measurer exists for: one edit to the recipe line, and every document that
states the tally goes red at once — which is what did **not** happen on
2026-09-17, when `AGENTS.md` and `docs/status.md` said twelve for a day after
the recipe grew its thirteenth entry.

```text
xtask: 3 documented count(s) no longer measure what they claim:
  - AGENTS.md:22: this line says 15, but `count:verify-gates` measures 14. …
  - CLAUDE.md:31: this line says 15, but `count:verify-gates` measures 14. …
  - docs/status.md:110: this line says 15, but `count:verify-gates` measures 14. …
```

**7. No marker at all.** Observed before any document was annotated — the gate
was wired into the recipe and read 262 files without finding one:

```text
xtask: no `<!-- count:KIND ARG… -->` marker in any of the 262 markdown file(s) this gate reads. It is wired into `just verify` and protecting nothing, which is exactly the failure it exists to catch in the documentation: a check that passes because it checked nothing. Annotate a count — `docs/status.md`'s gate tally is the one this gate was built for — or take it out of the `verify` recipe
```

## What is annotated, and what each number was re-measured as

Every figure below was measured on this tree on 2026-09-20 before it was
written down. None was copied from the page it replaced.

| Document                                   | Claim                       | Kind                | Measured |
| ------------------------------------------ | --------------------------- | ------------------- | -------- |
| `AGENTS.md`                                | the gate tally              | `verify-gates`      | 15       |
| `CLAUDE.md`                                | the gate tally              | `verify-gates`      | 15       |
| [../../status.md](../../status.md)         | the gate tally              | `verify-gates`      | 15       |
| `docs/flows/dashboard-auth.md`             | `staff_sign_in.rs` cases    | `tokio-tests`       | 27       |
| `docs/flows/dashboard-auth.md`             | `staff_sign_in.rs` cases    | `tokio-tests`       | 27       |
| `docs/flows/dashboard-auth.md`             | `dashboard_read_surface.rs` | `tokio-tests`       | 21       |
| `docs/flows/dashboard/slice-1-gaps.md`     | `staff_sign_in.rs` cases    | `tokio-tests`       | 27       |
| `docs/flows/configuration.md`              | environment variables       | `env-vars`          | 10       |
| `docs/flows/README.md`                     | flow directories            | `dir-entries`       | 6        |
| `backends/migrations/README.md`            | migration files             | `files-with-suffix` | 48       |
| [../merchant-sdks.md](../merchant-sdks.md) | Rust SDK resource methods   | `pub-async-fn`      | 35       |
| `docs/api/README.md`                       | Rust SDK resource methods   | `pub-async-fn`      | 35       |
| [../README.md](../README.md)               | verification pages          | `files-with-suffix` | 59       |

**One of those re-measurements contradicted the pages it was added to, and is
recorded rather than papered over.** [../merchant-sdks.md](../merchant-sdks.md)
and `docs/api/README.md` both said `vpay_sdk` exposes **fourteen** resource
methods — a 2026-09-06 measurement, made before the `customers`, `invoices` and
`invoice_items` resources existed. It exposes **35** today, across nine
resources; `verify-sdk-parity`'s own output ("35 SDK method(s) enumerated
across 39 row(s)") agrees independently. Both pages now carry the new figure,
annotated and gated, beside the old paragraph kept as a dated record.

The arithmetic built on the old figure — "two of fourteen have no route",
"twelve of the fourteen are routed" — is **not** re-derived on either page.
How many of the 35 have no route is a separate measurement nobody has taken,
and writing a plausible one is precisely the defect this gate exists to
remove.

## What is not done

- **The routed-method count on both SDK pages.** "Two of the fourteen have no
  route" and "twelve of the fourteen are routed" are 2026-09-06 arithmetic over
  a fourteen-method SDK. Both pages now say so; neither says what the figure is
  for 35 methods, because that measurement was not taken here.
- **`README.md`'s gate tally is a word in a table cell** and is **not** gated.
  A marker in that cell would widen the whole column on every prettier run; the
  word was updated to "fifteen" by hand, and that file's own sentence already
  says the justfile is the number and it is not.
- **No marker is on a heading**, for the same reason: the syntax is legal there
  and the result is unreadable.
- **`docs/status/gates.md` and this page's own prose spell counts out in
  words** where they are historical. That is correct: a dated record is not
  re-measured.
