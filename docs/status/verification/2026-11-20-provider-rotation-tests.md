# Issue #152 item 152-4: provider-credential rotation tests, and the gap split

_Written 2026-11-20 on `chore/152-4-provider-rotation-tests`, branched from
`origin/master` (`81cf2467`, post-`v0.18.0`). What changed is in the A.8.24 row
of [security-asset-inventory.md](../../reference/security-asset-inventory.md)
and `schemas/security-inventory.yaml`; this page is what was run._

## What changed

`152-4` ("Rotation/revocation procedures exist for merchant, service and
provider credentials and are testable") was met for merchant, service and
console keys, but the **provider** half was only partially tested: MTN's rotation
case was proven at the fingerprint level and Orange's at the module level, but
neither proved, **through the adapter**, that a rolled secret stops the cached
token being reused. `GAP-152` named it.

- **MTN** — `rotating_only_the_api_key_mints_a_new_token`
  (`vpay-adapter-mtn-momo/src/lib.rs`): a token cached under the current
  `api_key` is not served after only `api_key` changes; the call is forced to
  mint, and the closed-port `base_url` makes "did not reuse" observable as the
  `Transport` error from that mint.
- **Orange** — `rotating_only_the_client_secret_mints_a_new_token`
  (`vpay-adapter-orange-money/src/lib.rs`): the same, for a rolled
  `client_secret`.

Both join the existing rotation evidence: MTN's
`different_credentials_fingerprint_differently`, Orange's
`rotating_only_the_secret_evicts_the_cached_bearer`, Notch Pay's
`the_previous_secret_verifies_during_a_rotation_and_a_retired_one_does_not`, and
the worker's `each_of_two_secrets_is_independently_accepted`.

## The gap split

`#152` could not close while `schemas/security-inventory.yaml` tracked two open
gaps with `tracking_issue` pointing at it. Both now have their own issue, and
their `tracking_issue` is re-pointed:

- **`GAP-152-DUAL-CONTROL`** → issue [#418](https://github.com/vaam-apps/vpay/issues/418)
  (authenticated operator identity, dual control, refused-attempt journalling,
  a database-side guard on `console_keys` inserts).
- **`GAP-152`** → issue [#419](https://github.com/vaam-apps/vpay/issues/419)
  (automated rotation for production rail credentials and signing keys).

## Evidence

| Check                                                                                    | Result                     |
| ---------------------------------------------------------------------------------------- | -------------------------- |
| `cargo test -p vpay-adapter-mtn-momo --lib rotating_only_the_api_key`                    | **PASS**                   |
| `cargo test -p vpay-adapter-orange-money --lib rotating_only_the_client_secret`          | **PASS**                   |
| `just clippy` (`-D warnings`)                                                            | **PASS**                   |
| `just fmt-check`                                                                         | **PASS**                   |
| `cargo xtask verify-security-inventory`                                                  | **PASS** (gaps re-pointed) |
| `cargo xtask verify-links` / `verify-doc-counts` / `verify-migrations` / `verify-status` | **PASS**                   |

No migration, no schema, no route: this is two tests and the record around them.

## What is not done

- **Automated rotation** of production rail credentials and signing keys is
  still unbuilt — it is issue #419, not this change. Rotation remains a runbook
  procedure (a Secret edit plus a restart).
- **No deployment rotation was exercised.** No cluster has run vpay, so the
  runbook end to end (Secret edit, restart, live rail call) is still unproven;
  `docs/runbooks/rotate-rail-credentials.md` §6 says so.
- **Dual control and an authenticated operator identity** for the break-glass
  command are issue #418.
