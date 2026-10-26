# `verify-security-inventory`, the sixteenth gate (issue #150, ADR-0034)

Measured on 2026-10-26 on the review branch for
[PR #356](https://github.com/vaam-apps/vpay/pull/356), against the tree that
addresses the reviews of 2026-10-24 and 2026-10-26.

## What changed

- `schemas/security-inventory.yaml` was rewritten to match the code: the five
  trust tiers, the worker's real job kinds, the five `authkestra` tables, the
  corrected `vpay-config` paths, the real callback routes, the two published
  packages, the shipped CI jobs, and honest `gap`/`partial` statuses.
- Notch Pay's callback is recorded as verified (`x-notch-signature`, hex
  HMAC-SHA256 of the raw body under the endpoint's Hash Key); the gap is scoped
  to MTN, Orange's unbuilt `BodyToken` check and the missing per-account
  secret. The `dashboard_client.client_secret` class was removed because
  `vpay-config` refuses to boot with that key set.
- `cargo xtask verify-security-inventory` now deserializes the whole inventory
  with `deny_unknown_fields`, checks both directions of the table set, checks
  every cited path and test name, validates secret `config_keys` against a
  closed set, and refuses a gap whose issue number no document cites.
- The gate is gate 16 of `just verify` and a step in CI's `self-checks` job.
  Both `count:verify-gates` tallies and every current prose count moved from
  fifteen to sixteen.
- `docs/security/trust-boundaries.md`, `docs/security/data-flows.md`,
  `docs/security/README.md`, `docs/reference/security-asset-inventory.md` and
  ADR-0034 were corrected to describe what exists, not what the first draft
  hoped existed.

## Gate output

```text
$ cargo xtask verify-security-inventory
verify-security-inventory: ok — 5 service(s), 4 state store(s), 33 table(s),
10 secret class(es), 15 material control(s) and 7 tracked gap(s) verified in
schemas/security-inventory.yaml
```

## Failing cases

`cargo test -p xtask test_security_inventory` runs twelve cases. The real
inventory passes and eleven mutations fail:

| Mutation                                        | Assertion                           |
| :---------------------------------------------- | :---------------------------------- |
| Missing inventory                               | `the inventory is mandatory`        |
| Misspelt `tables:` key                          | `deny_unknown_fields` refuses it    |
| A table no migration creates                    | `ghost` is named                    |
| A live table in no state store                  | `jobs` is named                     |
| `rails.mtn_momo.api_key` as a config key        | the invented path is named          |
| A nonexistent implementation path               | `nope.rs` is named                  |
| A cited test no source declares                 | the test name is named              |
| A gap issue no document cites                   | `999999` is named                   |
| An issue number that is a prefix of a cited one | `#5` is refused against `issues/56` |
| A `gap` control with no `related_gap`           | `related_gap` is named              |
| A published manifest with `private: true`       | the manifest is refused             |

## Gate and doc checks

```text
$ cargo test -p xtask test_security_inventory
12 passed; 0 failed

$ cargo xtask verify-doc-counts
13 documented count(s) in 11 of 300 markdown file(s) agree

$ cargo fmt --all -- --check
clean
```

## `just verify`

```text
$ just verify
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 2 unimplemented item(s), all declared in docs/status.md
verify-errors: ok — 20 error type(s), all classified; anyhow confined to binaries
verify-sdk-parity: ok — 794 proving test(s), 46 dated gap(s), 35 SDK method(s)
verify-links: ok — 3223 repository link(s) in 512 tracked markdown file(s)
verify-npm-scope: ok — 3 publishable package(s) under sdks/
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.12.0
               (the local CLI is 0.12.0; this repository pins 0.16.0, and the
               recipe says so before it runs)
verify-serde: ok — 133 serialisable type(s), 17 exempted with a reason
verify-repositories: ok — 4 concrete implementation(s), named by none of 100 files
verify-toolchain: ok — rust-toolchain.toml pins 1.98.0
verify-migrations: ok — 54 migration file(s) match MANIFEST.sha256
verify-versions: ok — 26 version references all say 0.14.2
verify-privacy-inventory: ok — 325 columns, 26 elements, 37 debug_protections
verify-doc-counts: ok — 13 documented count(s) in 11 of 300 markdown files
verify-security-inventory: ok — 5 services, 4 state store(s), 33 tables,
                            10 secret classes, 15 controls, 7 tracked gaps
verify: ok — the sixteen gates above passed; the verify-docs report is advisory
```

## `just ci`

Re-measured after merging `master` (`0fc1183c`, cratestack 0.16.0 at
`ff0c67be`, and #379 at `a28aea10`) into the review branch:

```text
just fmt-check                      clean
cargo clippy --workspace --all-targets -- -D warnings   clean
just verify                         16 gates green (above)
just test-doc                       143 passed, 0 failed, 1 ignored
just verify-ignored                 0 ignored (expected 0), 55 binaries, 2528 total
just audit-web                      no moderate, high or critical advisory
just deny                           advisories ok, bans ok, licenses ok, sources ok
```

`cargo nextest run --workspace` was not run to completion on this host; the
container-backed suite is slow here. On the pre-merge head it reached
2061/2446 with 0 failures before the session ended. The gate this change adds
is exercised by `cargo test -p xtask test_security_inventory` above and by
`just verify`.

An earlier parallel `just test-web` run timed out three
`payments-filters.test.tsx` cases; the file passes 12/12 alone and the failure
is a 5-second timeout, not an assertion. It was not re-run on the merged tree.

## What the gate does not check

It does not call GitHub, so a tracked issue is checked only by format and by
being cited under `docs/`; `verify-citations` remains the network check. It
does not judge whether a control's evidence is sufficient, and it does not run
the tests it cites. Those stay review questions.
