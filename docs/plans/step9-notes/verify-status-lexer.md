# exp3 — `verify-status` must count only code, not prose

Branch `claude/exp3-verify-status-opus`, base `master` at `c531407`.

## 1. What the defect actually was, once measured

The brief describes the scanner as purely textual, so that a `//` comment, a
`///`/`//!` doc comment, a `/* */` block, a string literal and a
`#[doc = "…"]` attribute all count as shipping code. **That was only half
true on this base**, and the half that was already fixed is worth naming so
the record is accurate:

`verify_status` did not scan the raw file. It scanned `searchable(&text)`,
which already stripped block comments (nesting- and string-aware) and dropped
any line whose *trimmed start* was `//`. So a leading `//`, `///` or `//!`
line, and a `/* */` block, were already prose — `docs/status.md` said so, and
`a_token_quoted_in_a_comment_is_not_a_shipping_claim` already pinned it.

Two shapes did get through, and both were still live:

* a **trailing** `// … ProviderError::NotImplemented("x") …` comment, because
  the stripper dropped whole *lines* and only when they *began* with `//`;
* any **string literal** whose content spells the token out — in practice a
  raw string, `r#"ProviderError::NotImplemented("x")"#`, since a plain string
  has to escape its inner quotes and the old argument reader required a bare
  `"` immediately after the paren. A `#[doc = r#"…"#]` attribute is the same
  shape by another syntax.

`searchable`'s own doc comment said why the compromise existed: a line-based
stripper cannot tell `// a comment` from the `//` in `"https://…"`, and
swallowing the rest of that line would delete live code and make the gate
pass by finding nothing. That is the right instinct and the wrong remedy — a
lexer removes the trade instead of the compromise.

Consequence, unchanged from the brief: a false positive forces a phantom
bullet into `docs/status.md`, the two-directional check then *requires* that
bullet to stay, and the cheapest way for an author to clear one is to delete
the honest sentence from the adapter's doc comment that explained the gap.

## 2. Characterising test — failed first, on purpose

`tests::a_token_outside_code_is_never_a_shipping_claim` in
`.xtask/src/main.rs`, feeding the scanner one snippet carrying the token in a
`//!`, a `///`, a leading `//`, a `/* */`, a trailing `//`, a plain string
literal and a raw string, and asserting **zero** occurrences.

On the unmodified scanner (`cargo nextest run -p xtask -E
'test(a_token_outside_code_is_never_a_shipping_claim)'`):

```
thread 'tests::a_token_outside_code_is_never_a_shipping_claim' panicked at .xtask/src/main.rs:3591:9:
assertion `left == right` failed: a token that is only ever mentioned in prose or data is not code
  left: ["trailing::comment", "raw::string"]
 right: []
Summary [0.004s] 1 test run: 0 passed, 1 failed, 83 skipped
```

The two names in `left` are exactly the two shapes §1 identifies. The other
five were already prose.

## 3. What was built

A hand-written lexer in `.xtask/src/main.rs`. No parser dependency was added;
`syn` is **not** in `.xtask/Cargo.toml` (its four deps are `rsa`, `rand`,
`sha2`, `base64`, all for `gen-signing-key`), so pulling one in was not open.

* `strip_comment_kinds` / `CommentKinds` — one lexer, two modes. `All`
  removes every comment (`//`, `///`, `//!`, `/* */`, `/** */`, nested);
  `BlocksOnly` removes block comments and leaves `//` lines verbatim.
* `strip_comments` (`All`) — what `searchable` now calls, replacing the old
  `strip_block_comments` + line filter.
* `strip_block_comments` (`BlocksOnly`) — kept, because `verify-docs`'
  `count_doc_and_code` *counts* `///` lines; a stripper that removed them
  would report every file as having no docs. `only_the_gates_stripper_removes_doc_lines`
  is the test that stops the two modes collapsing into each other.
* `end_of_literal` / `end_of_quoted` / `end_of_raw` / `end_of_char_literal` —
  string, raw string (any `#` count), byte and C-string prefixes, and
  character literals. A lifetime (`'a`, `'static`) is deliberately *not* a
  literal: reading one as a char literal would swallow everything up to the
  next `'` in the file, which is the delete-too-much failure that passes a
  gate silently.
* `scan_not_implemented` — now lexes instead of `match_indices`. It skips
  whole literals, so a `NotImplemented(` spelled inside one is never
  examined; when the needle is found *in code* it reads the following literal
  as the token. Literals are skipped rather than blanked, because the token
  this gate extracts **is** a string literal — blanking would turn
  `NotImplemented("mtn_momo::refund")` into `NotImplemented("")`.

Two-directional semantics: untouched. `verify_status`, `declared_tokens` and
`STATUS_TOKEN_HEADING` are unchanged, and
`the_status_check_fails_in_both_directions` still passes.

`STUB_ADAPTER_NAMES`: untouched, and still matched against the **raw** file
including comments — a binary that names a stub adapter in a comment is
describing a code path ADR-0006 says must not exist.

**The scanner is shared, and `verify-no-mocks` does use it.** The
`connect_lazy` half of `app_source_violations` calls `searchable`, so it
inherits the fix: a *trailing* `// … connect_lazy …` comment is now
documentation rather than a call, which is what that guard's own doc comment
always claimed. `a_binary_that_opens_its_pool_lazily_is_a_violation` was
extended with a trailing-comment line to pin it. `verify-errors` also calls
`searchable`; its output is byte-identical (§5).

## 4. Guard-failure proofs (measured, in this worktree)

A temporary method was added to `MtnMomoAdapter` in
`backends/crates/vpay-adapter-mtn-momo/src/lib.rs`, and reverted with
`git checkout HEAD --` afterwards.

**A — a real token in code, undeclared → FAILS.**

```rust
async fn guard_proof(&self) -> Result<(), ProviderError> {
    Err(ProviderError::NotImplemented("guard::proof_in_code"))
}
```
```
$ cargo run -q -p xtask -- verify-status
xtask: these unimplemented items are missing from docs/status.md under
  `### Unimplemented items tracked by `verify-status``:
  - guard::proof_in_code
EXIT_A=1
```

**B — the same token in prose only → PASSES.**

```rust
/// One day this returns ProviderError::NotImplemented("guard::proof_in_doc").
fn guard_proof(&self) -> u8 {
    let _trailing = 1; // ProviderError::NotImplemented("guard::proof_trailing")
    let _raw = r#"ProviderError::NotImplemented("guard::proof_raw")"#;
    0
}
```
```
$ cargo run -q -p xtask -- verify-status
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
EXIT_B=0
```

**C — the decisive one: the same prose-only file, on the OLD scanner.** With
the adapter left exactly as in B, `.xtask/src/main.rs` was reverted to
`HEAD`, rebuilt and re-run:

```
$ git checkout HEAD -- .xtask/src/main.rs && cargo run -q -p xtask -- verify-status
xtask: these unimplemented items are missing from docs/status.md under
  `### Unimplemented items tracked by `verify-status``:
  - guard::proof_raw
  - guard::proof_trailing
EXIT_C=1
```

Two tokens, both from prose, both gone after the fix — and note that
`guard::proof_in_doc` is absent from that list, which is the direct
measurement behind §1's claim that the `///` half was already fixed.

**D — the real occurrence is still found.** `mtn_momo::refund`
(`backends/crates/vpay-adapter-mtn-momo/src/lib.rs:547`) is still the one
declared item, before and after: `verify-status: ok — 1 unimplemented
item(s)`. The tree was restored (`git status --short` showed only
`.xtask/src/main.rs`) before any gate below was run.

## 5. Gates (every number measured here, 2026-09-05, `CARGO_BUILD_JOBS=4`)

| command | result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy -p xtask --all-targets -- -D warnings` | clean (the lexer was rewritten off `chars[i]` onto `chars.get(i)`; the workspace denies `indexing_slicing`) |
| `cargo nextest run -p xtask` | **89 tests run: 89 passed, 0 skipped** (was 83) |
| `cargo xtask verify-no-mocks` | ok — no test double reachable from a shipping binary |
| `just verify` | ok — `verify-status` **1** item (unchanged), `verify-errors` **15** types / **14** `#[from]` variants (unchanged), `verify-sdk-parity` **342** tests / **26** gaps (unchanged) |
| `just verify-ignored` | **0 ignored, 42 test binaries, 1153 total** (was 1147; +6, all in `xtask`) |

`verify-docs` and `verify-errors` output was diffed old-vs-new binary on this
same tree: **byte-identical** in both cases.

## 6. Documentation changed — verbatim

### `justfile`, new comment above the `verify-status` recipe

```
# AGENTS.md rule 2: every `ProviderError::NotImplemented("…")` token in
# shipping code is declared in docs/status.md, and every token declared there
# is still carried by shipping code. Both directions fail the build.
#
# "In shipping code" is lexical, not textual: since 2026-09-05 the scanner
# lexes rather than greps, so a token mentioned in a comment (`//`, `///`,
# `//!`, `/* */`, leading or trailing), in a `#[doc = "…"]` attribute, or
# inside a string, raw-string or character literal is prose and counts for
# nothing in either direction. Before that, a trailing comment or a raw
# string carrying the token forced a phantom bullet into docs/status.md —
# and the cheapest way to clear one was to delete the honest sentence from
# the adapter's doc comment that explained the gap.
```

`docs/reference/xtask.md` does **not** exist on this base (`docs/reference/`
holds `README.md`, `rails.md` and one file per shipping crate), so the
justfile comment is where the recipe-level description went.

### `docs/status.md`, header prose

Was:

> described cannot sit here unnoticed. The scanner is now comment-aware: a
> `NotImplemented("…")` written inside a doc comment while explaining
> something is no longer counted as a token (that blind spot is described in
> the Step 2 note below, where it was found).

Now:

> described cannot sit here unnoticed. The scanner reads *code*, not text:
> since 2026-09-05 it lexes, so a `NotImplemented("…")` written in a comment
> of any kind (`//`, `///`, `//!`, `/* */` nested or not — leading **or**
> trailing), in a `#[doc = "…"]` attribute, or inside any string, raw-string
> or character literal is prose, and prose declares nothing. It was
> comment-aware from 2026-09-03 (that blind spot is described in the Step 2
> note below, where it was found), but only for comments that *began* a line,
> and not at all for string literals; a trailing `// … NotImplemented("x")` or
> an `r#"…"#` carrying the token still forced a phantom bullet into this file
> — and because the check runs in both directions, the bullet then had to
> stay, so the docs→code half could be satisfied by prose alone.

### `docs/status.md`, the note under `### Unimplemented items tracked by `verify-status``

Was:

> leaving an item here that no shipping code carries any more. The scanner is
> comment-aware, so a token quoted in a doc comment neither declares anything
> nor counts as one (`a_token_quoted_in_a_comment_is_not_a_shipping_claim` in
> `xtask`).

Now:

> leaving an item here that no shipping code carries any more. The scanner
> counts only occurrences in code, so a token quoted in a comment of any kind,
> in a `#[doc = "…"]` attribute or in a string literal neither declares
> anything nor counts as one — you never have to strip an honest sentence from
> a doc comment to keep this gate green
> (`a_token_quoted_in_a_comment_is_not_a_shipping_claim`,
> `a_token_outside_code_is_never_a_shipping_claim`,
> `a_token_in_a_doc_attribute_is_not_a_shipping_claim` and
> `the_lexer_tells_the_four_states_apart` in `xtask`).

### `justfile`, `verify-ignored` counters comment

```
# Re-measured 2026-09-05 on `claude/exp3-verify-status-opus`, which made
# `verify-status` lex rather than grep: `just verify-ignored` reports **1153
# total, 42 test binaries, 0 ignored**. Six cases added, all in `xtask`
# (83 → 89) — the characterising test for the defect and five for the lexer's
# own edge cases. No new test binary, so `expected_suites` stays 42 and the
# floor stays 1080.
```

## 7. What was NOT done

* **`end_of_cfg_test_item` still has its own, separate literal scanner**
  (`.xtask/src/main.rs`), and it is **not raw-string aware**: a `#[cfg(test)]`
  item containing `r#"…{…"#` could still unbalance its brace count and delete
  more than the item. It was out of scope here, it predates this change, and
  no such literal exists in the tree today. Left visible rather than
  half-fixed.
* **The four other `verify-*` gates were not re-lexed.** `verify-errors` and
  the `connect_lazy` half of `verify-no-mocks` benefit because they share
  `searchable`; `verify-sdk-parity` and `verify-docs` do their own text
  matching and were left alone. `STUB_ADAPTER_NAMES` still matches raw text
  including comments, deliberately.
* **No escape decoding.** `literal_content` returns the token as written; a
  token containing a backslash escape would come back escaped. Tokens are
  identifier paths, so an escape in one is a bug worth seeing rather than
  silently normalising — but that is a choice, not a proof.
* **`just ci` was not run in full.** The gates in §5 are what was run;
  `test-web`, `lint-web` and `deny` were not, because nothing outside
  `.xtask`, `justfile` and `docs/` changed.
