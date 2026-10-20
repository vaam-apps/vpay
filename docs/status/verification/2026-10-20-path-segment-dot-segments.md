# 2026-10-20 — `path_segment` refuses a dot segment

Branch `claude/blissful-goodall-0c29a8`, from `master` at `0a86e83` (where
everything below was run), rebased before push onto `dbe8300` (#282), a
release commit that moves only version lines. `master` was later merged in
twice, for #285 and #295 (below). One port helper, its callers, and
documentation. **No route, no wire byte
for any number a route can send, no migration, no gate and no
`NotImplemented` token moved.**

## What was wrong

`vpay_provider::http::path_segment` percent-encoded everything outside the
RFC 3986 unreserved set, so `/`, `?` and `#` could not move a request to
another endpoint. `.` is unreserved, so `""`, `.` and `..` passed through
unchanged, and as whole segments they are dot segments. Measured against a
WHATWG parser (Node's `URL`, the same standard the `url` crate reqwest uses
implements) on MTN's lookup path,
`http://rail.test/collection/v1_0/accountholder/msisdn/{x}/basicuserinfo`:

```text
".."      /collection/v1_0/accountholder/basicuserinfo
"."       /collection/v1_0/accountholder/msisdn/basicuserinfo
""        /collection/v1_0/accountholder/msisdn//basicuserinfo
"%2E%2E"  /collection/v1_0/accountholder/basicuserinfo
".%2e"    /collection/v1_0/accountholder/basicuserinfo
"..."     /collection/v1_0/accountholder/msisdn/.../basicuserinfo
```

The fourth and fifth rows are why escaping could not be the fix. The last row
is why `...` needed a decision rather than a reading of the standard.

**Nothing reachable was exposed.** `GET /v1/account_holders` and
`POST /v1/refunds` validate the number as `237` + `6` + eight digits before the
port sees it. `vpay-notchpay` (RFC-0012), which landed on `master` the same
day in #285, had already worked round it with a private `reference_segment`
guard. The `notchpay` rail (#295, also the same day) reaches that client only
with vpay's own UUID reference.

## The change

- `path_segment` returns `Result<String, ProviderError>` and refuses an empty
  or all-dots input as `ProviderError::Malformed`, with one fixed context
  (`DOT_SEGMENT_REFUSAL`) that never quotes the input. `...` and longer runs
  are refused **by decision**: no value an adapter interpolates is
  legitimately all dots, and no hop in front of a rail has been read.
  `docs/reference/rails.md` § "A path segment the caller chose" argues it.
- No unchecked variant was kept, so a caller that ignores the refusal does not
  compile. That includes the `vpay-notchpay` branch, until it takes the
  companion commit described below.
- `vpay-adapter-mtn-momo`: `account_holder_url` returns a `Result` and calls
  `path_segment(msisdn)?`. `account_holder_name` builds it before
  `send_authorized`, so a refused number costs no token request.
- The port's error-surface table and `account_holder_name`'s `# Errors` name
  the new `Malformed` case, and `Malformed`'s `Display` prefix ("could not
  parse the rail's response") is stated as the same compromise
  `parse_destination` makes.

## What ran, on 2026-10-20 (macOS arm64, rustc 1.98.0, cargo-nextest 0.9.137)

```text
$ cargo test -p vpay-provider -p vpay-adapter-mtn-momo
vpay_adapter_mtn_momo  unit: 89 passed; 0 failed; 0 ignored
vpay_provider          unit: 42 passed; 0 failed; 0 ignored
Doc-tests vpay_adapter_mtn_momo: 1 passed; 0 failed; 0 ignored
Doc-tests vpay_provider:        12 passed; 0 failed; 0 ignored
```

That is four new tests and no new doctest. `path_segment`'s existing doctest
grew the dot cases.

- `vpay-provider`, three:
  - `url_parsing_resolves_an_escaped_dot_segment_like_a_plain_one` pins the
    premise against `reqwest::Url`.
  - `an_empty_or_all_dots_segment_is_refused_with_a_fixed_message`.
  - `every_admitted_segment_stays_exactly_one_segment` parses sixteen
    admitted inputs through `reqwest::Url` and asserts each is exactly one
    segment, in place, with no query or fragment. The inputs include `a..b`,
    `%2E%2E`, `.%2E` and `a\b`, which WHATWG treats as `a/b`.
- `vpay-adapter-mtn-momo`, one: `a_dot_segment_reference_is_refused_before_any_request`
  drives the method against `config()`, which points at a port nothing
  listens on. A request that left the process would therefore come back
  `Transport`, not `Malformed`.
- `the_lookup_url_escapes_the_payer_reference_it_interpolates` gained the four
  dot inputs.

The gates, on the same tree:

```text
$ just fmt-check-rust     exit 0
$ just clippy             exit 0   (cargo clippy --workspace --all-targets -- -D warnings)
$ just verify             verify: ok — the fifteen gates above passed; the verify-docs report is advisory
                          verify-links: ok — 2245 repository link(s) in 449 tracked markdown file(s) resolve to a tracked path
                          verify-doc-counts: ok — 13 documented count(s) in 11 of 284 markdown file(s) agree with what this tree measures
                          verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
$ just test-doc           128 passed, 0 failed, 1 ignored (14 doctest runs; the 1 ignored is unchanged)
$ just verify-ignored     verify-ignored: 0 ignored (expected 0), 50 test binaries (expected 50), 2202 total (minimum 1080)
$ just test-rust           2101/2202 run, 2100 passed, 1 failed, then cancelled (fail-fast)
$ cargo nextest run --workspace --no-fail-fast -E 'not (<the four staff_sign_in loopback cases>)'
                          Summary [1621.205s] 2198 tests run: 2198 passed (2 slow), 4 skipped
```

The one `just test-rust` failure was
`staff_sign_in::a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget`.
The test said it itself: "127.0.0.20 is not assigned to a local interface …
(EADDRNOTAVAIL, os error 49)". That is the macOS loopback-alias case
`CLAUDE.md` names. `just loopback-aliases` needs `sudo`, which this session
did not have, so the run was repeated without the four cases that bind those
addresses. The 4 skipped are exactly those four. The integration suites,
Docker-backed Postgres and WireMock included, ran and passed. Linux CI assigns
the whole `127.0.0.0/8`, so CI's `rust` job is the run that covers the four.

`check-schema` warned that `cratestack` 0.15.1 is on `PATH` while the
repository pins 0.15.0, and checked the schema under 0.15.1. That is this
machine's install, and unrelated to the change.

### Mutations, each reverted after the run

| Mutation                                                                              | Caught by                                                                                                                                                |
| ------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1: refuse only `""`, `.`, `..` (the standard's set; `...` admitted)                  | `an_empty_or_all_dots_segment_is_refused_with_a_fixed_message`, the `path_segment` doctest, `the_lookup_url_escapes_the_payer_reference_it_interpolates` |
| M2: no refusal at all                                                                 | the same two in `vpay-provider`, plus both MTN cases                                                                                                     |
| M3: the message quotes the input (`format!("{DOT_SEGMENT_REFUSAL}: {input}")`)        | `an_empty_or_all_dots_segment_is_refused_with_a_fixed_message`                                                                                           |
| M4: the MTN adapter swallows the refusal (`.unwrap_or_else(\|_\| msisdn.to_owned())`) | `the_lookup_url_escapes_the_payer_reference_it_interpolates`, `a_dot_segment_reference_is_refused_before_any_request`                                    |

M1 and M2 were first run with both crates together, and `cargo test` stopped
at MTN's failing binary. They were re-run on `vpay-provider` alone with
`--no-fail-fast`, which is where the provider's own tests are shown catching
them.

## The companion changes

- **`vpay-notchpay`, in this change since #285 merged.** It was first
  migrated on a side branch, because the crate was still RFC-0012 WIP. #285
  then landed it on `master` with the private guard intact and the old
  infallible call, `Ok(path_segment(reference))`, which does not compile
  against this signature. So merging `master` in brought the migration here.
  The commit deletes `reference_segment`, has `payment_request` call
  `path_segment(reference)?`, and adds a dated correction to
  `docs/reference/vpay-notchpay.md`. `master`'s crate was byte-identical to
  the RFC-0012 tip it had been migrated on. The results, re-run on the merged
  tree, are below.

  ```text
  $ cargo test -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-notchpay
  vpay_adapter_mtn_momo  unit 89 passed, 0 ignored; doctests 1 passed
  vpay_notchpay          unit 34 passed, 0 ignored; doctests 1 passed
  vpay_provider          unit 42 passed, 0 ignored; doctests 12 passed
  $ just clippy            exit 0
  $ just verify            verify: ok — the fifteen gates above passed
                           verify-links: ok — 2347 repository link(s) in 454 tracked markdown file(s)
  ```

  Replacing `path_segment(reference)?` with `reference.to_owned()` in
  `payment_request` failed three of the crate's tests:
  `an_empty_or_dot_only_reference_is_refused_before_any_request`,
  `the_public_payment_methods_refuse_a_dot_reference_without_sending` and
  `both_payment_paths_escape_the_reference_as_one_segment`.

- **The `notchpay` rail, after #295 merged.** #295 put a third adapter,
  `vpay-adapter-notchpay`, in `vpay-server`, reading and processing payments
  through the migrated client by vpay's own UUID reference. It needed no code
  change here: its `path_segment` calls all go through
  `Client::payment_request`. The second merge of `master` conflicted only in
  prose: `provider-port.md`'s Status, where both sections are kept, and this
  directory's index. Re-run on that tree:

  ```text
  $ cargo test -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-notchpay -p vpay-adapter-notchpay
  vpay_adapter_mtn_momo   unit 89 passed, 0 ignored; doctests 1 passed
  vpay_adapter_notchpay   unit 29 passed, 0 ignored; doctests 1 passed
  vpay_notchpay           unit 35 passed, 0 ignored; doctests 1 passed
  vpay_provider           unit 43 passed, 0 ignored; doctests 12 passed
  $ cargo nextest run -p vpay-tests-conformance --no-fail-fast
  Summary [ 293.381s] 92 tests run: 92 passed (1 slow), 0 skipped
  $ just clippy            exit 0
  ```

  The two counts that moved since the first merge, `vpay_notchpay` 34 → 35
  and `vpay_provider` 42 → 43, are #295's own tests.

- **`vaam-apps/vpay-skills`**: `vpay-provider-adapters` (`SKILL.md` and
  `references/adding-a-rail.md`) and `vpay-mtn-momo`
  (`references/account-holder-lookup.md`, which quotes `account_holder_url`).
  Its `tools/verify-coverage.mjs` passes against this tree.

## What did not run

- **No real rail.** Nothing here was sent to MTN or Notch Pay. The refusal
  happens before any request, so there is nothing for a rail to confirm.
- **The merchant SDKs were not changed.** `sdks/rust`'s private
  `path_segment` and the Node SDK's `encodeURIComponent` still pass `.`, `..`
  and `""`. That is a merchant's own request to vpay's own API under the
  merchant's credentials. What vpay answers for one has not been measured.
- `pnpm install --frozen-lockfile` failed on this machine (the
  `@next/swc-darwin-arm64` tarball download, then a missing `jiti`
  `package.json`), so `lint-web`, `test-web`, `audit-web` and the whole-tree
  `fmt-check-web` did not run. The eight Markdown files this change touches
  were checked with `npx prettier@3.9.6 --check` (clean). They were also
  checked with `npx markdownlint-cli2@0.20.0` under super-linter's default
  config: 0 errors.
