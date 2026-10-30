# VPA-4 verification: staff SSO is served again, as a part of `human_login`

_Written 2026-10-30 while the work was done; every number below was measured on
the tree this page was first committed with. The review fixes that followed (the
chart's `idpEgress`, the redacted `subject`) were checked as their own sections
say, and the Rust gates below were **not** re-run on them: see "Not done"._

What was built is in [docs/status/backend.md](../backend.md) ("Staff SSO is
served again, as a part of `human_login`") and in
[the flow page](../../flows/dashboard-auth/sso.md); this page is what was run,
what broke it on purpose, and what could not be run.

## What was compared with `f885170^`

The code is restored from the parent of #383, so the first check is how far it
differs from what stood there (`diff` of `git show f885170^:<path>` against the
tree):

| File                                         | Against `f885170^`                                                                                                                                  |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vpay-api/src/staff/sso/error.rs`            | byte-identical (an empty diff)                                                                                                                      |
| `vpay-api/src/staff/sso/verify.rs`           | identical outside `#[cfg(test)]`, except that `IdentityClaims`'s `Debug` redacts `subject` (and its doc comment says why); two test changes (below) |
| `vpay-api/src/jwks_cache.rs` (`KeySetCache`) | byte-identical (an empty diff), with its four tests                                                                                                 |
| `vpay-api/src/staff/sso/provider.rs`         | identical in code; a `#[cfg(test)]` canary test for `SsoProvider`'s `Debug` added                                                                   |
| `vpay-api/src/staff/sso/mod.rs`              | the three wire types' `Debug` are hand-written instead of derived, with a canary test; the handlers are identical                                   |
| `vpay-db/src/staff_sso_attempts.rs`          | VPA-2's, not restored; `consume` is still the single-use compare-and-swap, `sweep_expired_sso_attempts` still runs on `start`                       |

A review of the restoration found one difference worth stating plainly: the
restored `Debug` of `IdentityClaims` printed the provider's `sub`, which is the
join key of `credentials.subject`, a `staff_credential_secret` the privacy
inventory classes `control: forbid`. `f885170^` printed it, and its canary test
never put a canary in `subject`, so nothing noticed. The `Debug` now prints
`"[redacted]"` for it, the canary test sends a distinctive `sub` and asserts it
is absent (and that the claims still carry it), and the inventory row for
`IdentityClaims` names `staff_credential_secret` among the elements it redacts.
That mutation (printing `subject` again) was not run against the test, because
this change was made where there was no disk for a build: CI is its first run.

So the security properties are the old code's: `state` compared in constant time
against the stored hash and the flow burned on a mismatch; a nonce compared by
hash; an S256 PKCE verifier sealed at rest and sent with the code; the discovery
`issuer` equal to the configured one byte for byte; keys only from that
document's `jwks_uri`; an algorithm allow-list (no `none`, no HS\*); `iss`, `aud`,
`azp`, `exp`, `iat` and `sub` checked; the group gate; the second factor by `acr`
or `amr`; and one pending flow taken exactly once, expiring in ten minutes. None
is weaker. What changed is where the provider is built and when it is served.

## Gates

| Gate                                                                       | Result                                                                                                        |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all --check`                                                  | clean                                                                                                         |
| `cargo clippy --workspace --all-targets -- -D warnings` (pinned 1.98.0)    | clean                                                                                                         |
| `cargo deny check`                                                         | advisories, bans, licenses and sources ok                                                                     |
| `cargo test -p vpay-config -p vpay-api --lib`                              | 195 + 558 passed                                                                                              |
| doctests of `vpay-config`, `vpay-api`, `vpay-db`                           | 27, 7 and 10 passed                                                                                           |
| `cargo test -p vpay-tests-integration --test staff_sso`, native PostgreSQL | 24 passed (23 restored, 1 new: `an_oidc_credential_kept_through_0056_resolves_to_its_member`)                 |
| `cargo test -p vpay-tests-integration --test staff_sign_in`, native PG     | 28 passed                                                                                                     |
| `cargo test -p vpay-server --test cli`, native PostgreSQL                  | 53 passed, one new                                                                                            |
| `just verify`                                                              | the seventeen gates passed (`verify-docs` is advisory)                                                        |
| `just verify-ignored`                                                      | `0 ignored (expected 0), 58 test binaries (expected 58), 2723 total (minimum 1080)`; `expected_suites` was 56 |
| `pnpm exec prettier --check` on the docs and YAML touched                  | clean                                                                                                         |

After the review fixes, on the tree they leave: `cargo +1.98.0 fmt --all --check`
clean, `pnpm exec prettier --check` clean on the touched markdown, YAML and JSON,
`just helm-check` ok, and every `cargo xtask verify-*` gate that `just verify` runs
except `check-schema` and `verify-ui` (shell, and untouched) ok. The Rust test and
clippy runs were skipped, since the disk had under 5 GB free; CI runs them.

The native PostgreSQL patch of VPA-3 (a per-test `initdb` under `/tmp`, in place
of the testcontainers helper) was applied locally, reverted before each commit,
and its data directories deleted. `staff_sso.rs` needed the same two-line change
by hand, being new.

## What the first runs found

- **`human_login_tests::with_the_method_served_the_staff_routes_are_mounted`
  failed** on the first run: it walks `STAFF_ROUTES` and asserts none answers
  `404`, and with `human_login` served and no `staff_auth.sso` the two SSO
  routes that act (`start`, `callback`) answer the SSO module's own `404`, by
  design. The test now tells the two `404`s apart by error code
  (`unknown_route` is the router's), so a route that fell off the router still
  fails it (guard 11 below).
- **`staff_sign_in.rs` and `staff_sso_keycloak.rs` did not compile** against the
  new `StaffAuth.sso` field and, for the Keycloak file, against VPA-3's `Config`
  (`auth`, `dashboard`, `service_clients`). The Keycloak file had not been
  compiled by anything since #383; `cargo clippy --workspace --all-targets` is
  what found it.
- **A mutation survived at once** (guard 2): the first version of the sabotage
  of the nested `client_secret` loader still passed, because a secret the loader
  cannot see reads as the empty string, which the livemode rule also refuses
  with the same error. A document that is _correct_ is what tells the two
  apart, and no test loaded one: `a_livemode_sso_block_written_as_placeholders_loads_and_carries_the_resolved_secret`
  and its fixture are the repair.
- **A mutation survived `f885170^`'s own suite** (guard 15): dropping `exp` from
  the verifier's required claims. A token with no expiry was accepted by the
  restored code exactly as it was by the original. The case was added to the
  expiry test; the code is unchanged.

## Guards, broken on purpose

Each mutation was applied to the tree, the named test was seen to fail, and the
file was restored (logs, one per file, in the session's scratchpad,
`vpa4/sabotage/`). The security checks are the old code's; rows 13 to 25 are the
comparison with `f885170^` made executable.

| #   | Mutation                                                          | Failed                                                                                                                                        |
| --- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | both `accepted_acr` and `accepted_amr` empty is allowed           | `an_sso_block_with_no_second_factor_rule_is_refused_and_either_list_alone_is_enough`                                                          |
| 2   | the loader cannot see the nested `client_secret` as written       | **survived** until the placeholders case was added; then `a_livemode_sso_block_written_as_placeholders_loads_and_carries_the_resolved_secret` |
| 3   | `validate_staff_sso` is never called                              | `every_sso_shape_rule_refuses_its_own_value`, `a_livemode_sso_block_needs_https_and_no_stub_host` and two more                                |
| 4   | `StaffSso`'s `Debug` prints the client secret                     | `the_sso_client_secret_never_reaches_debug`                                                                                                   |
| 5   | `StaffSso` loses `deny_unknown_fields`                            | `a_misspelt_sso_key_is_refused_rather_than_defaulted`                                                                                         |
| 6   | a `staff_auth` block is accepted with `human_login` not permitted | `every_auth_methods_rule_refuses_its_fixture_with_its_own_variant` (the new SSO fixture and VPA-3's)                                          |
| 7   | `SsoStartResponse`'s `Debug` prints the pending-flow token        | `the_sso_wire_types_do_not_print_their_secrets`                                                                                               |
| 8   | `SsoProvider`'s `Debug` prints the client secret                  | `the_provider_prints_no_part_of_its_configuration_but_the_issuer`                                                                             |
| 9   | `IdentityClaims`'s `Debug` prints the email                       | `the_debug_of_the_claims_prints_no_personal_data`                                                                                             |
| 9a  | `IdentityClaims`'s `Debug` prints the subject                     | **not run**, no build here; `the_debug_of_the_claims_prints_no_personal_data` carries the subject canary                                      |
| 10  | `KeySetCache` re-fetches on every unknown `kid`                   | `an_unknown_kid_forces_one_refetch_and_then_is_throttled`                                                                                     |
| 11  | `POST /staff/sso/start` is not mounted                            | `with_the_method_served_the_staff_routes_are_mounted`                                                                                         |
| 12  | `STAFF_ROUTES` forgets the callback                               | `the_route_table_names_eleven_unauthenticated_paths`                                                                                          |
| 13  | the token's `iss` is not checked                                  | `the_wrong_issuer_audience_azp_or_a_missing_azp_with_two_audiences_is_refused`                                                                |
| 14  | the token's `aud` is not checked                                  | the same                                                                                                                                      |
| 15  | `exp` is not a required claim                                     | **survived `f885170^`'s suite**; now `expiry_is_enforced_with_sixty_seconds_of_leeway_and_iat_may_not_be_in_the_future`                       |
| 16  | an `iat` in the future is accepted                                | the same                                                                                                                                      |
| 17  | a mismatched `azp` is accepted                                    | `the_wrong_issuer_audience_azp_or_a_missing_azp_with_two_audiences_is_refused`                                                                |
| 18  | the `state` is not compared                                       | `a_replayed_or_wrong_state_is_refused_and_burns_the_flow`                                                                                     |
| 19  | the nonce is not compared                                         | `token_defects_are_refused_and_provision_nobody`, `no_other_failure_earns_a_retry`                                                            |
| 20  | the required-group gate is skipped                                | `a_person_outside_the_required_group_is_refused_without_a_retry` and one more                                                                 |
| 21  | the second factor is not required                                 | `a_password_only_token_earns_one_prompt_login_retry_and_the_retry_signs_in` and two more                                                      |
| 22  | the PKCE verifier sent with the code is wrong                     | `a_first_sso_sign_in_provisions_a_member_and_reads_their_merchants_payments` and three more                                                   |
| 23  | a disabled member may sign in                                     | `a_disabled_member_is_refused_even_with_a_valid_token`                                                                                        |
| 24  | the start route is not rate limited                               | `start_is_rate_limited`                                                                                                                       |
| 25  | a first sign-in is provisioned as an administrator                | `roles_stay_local_whatever_the_token_claims` and one more                                                                                     |
| 26  | the token's `sub` is changed before the link is looked up         | `an_oidc_credential_kept_through_0056_resolves_to_its_member`                                                                                 |
| 27  | `StaffLogin::from_config` leaves `sso: None`                      | `a_management_pod_offers_staff_sso_exactly_when_the_block_is_written` (the real `serve`)                                                      |
| 28  | an expired pending flow is returned by `consume`                  | `a_pending_flow_expires_and_start_sweeps_expired_rows`                                                                                        |
| 29  | `SsoCallbackRequest` derives `Debug`                              | `just verify-privacy-inventory`                                                                                                               |
| 30  | a `debug_protections` row names a type that does not exist        | the same (the stale row)                                                                                                                      |
| 31  | the xtask does not know `staff_auth.sso.client_secret`            | `just verify-security-inventory` (the inventory names an unrecognised key; the first attempt only broke the array length: a compile error)    |
| 32  | the A.8.2 control cites a test that does not exist                | `just verify-security-inventory`                                                                                                              |

A mutation that proved less than it looked like (tried, not in the table):
looking an existing link up with a wrong subject, in the callback only, breaks
just `a_disabled_member_is_refused_even_with_a_valid_token`, because `provision`
asks again after its insert hits the credential's unique index and re-resolves
the very member that was kept. Resolution is held by two lookups, so row 26
changes the subject both of them see, and is what holds the kept-credential
case.

## Not done, and what this does not prove

- **The review fixes were not compiled or tested.** `IdentityClaims`'s redacted
  `subject` and its extended canary test were written where the disk had room for
  `cargo fmt` and the xtask gates but not for a `vpay-api` build; neither
  `cargo test -p vpay-api --lib sso` nor clippy ran on them. The change is one
  field and one test, and CI is its first compile.
- **`staff_sso_keycloak.rs` was not run.** It needs a Docker daemon (a Keycloak
  container beside a Postgres container) and this session has none. It is gated
  as it was at `f885170^`: by needing Docker, with no `#[ignore]` and no feature
  (`verify-ignored` pins zero). It was compiled and linted
  (`cargo clippy --workspace --all-targets`) and adapted to VPA-3's `Config`;
  what it asserts is what `f885170^` asserted. Run it with `cargo test -p
vpay-tests-integration --test staff_sso_keycloak` where Docker is.
- **The pending flow's compare-and-swap under a race**, `consume`'s `ok != 1`
  branch, is `vpay-db`'s Docker-harness test
  (`racing_takes_of_one_sso_flow_yield_exactly_one_winner`), not run here. The
  sequential replay is proved by row 18's test and by `a_replayed_or_wrong_state_is_refused_and_burns_the_flow`.
- **`vpay-db`'s own suites**, `postgres_smoke` and the integration suites that
  neither build a `StaffAuth` nor serve the management pod were not run: this
  change adds no migration and edits no repository (one doc comment).
- **Nothing ran against a deployment**, a real ingress or any provider but a
  simulated one over HTTP. The dashboard application has no SSO routes (VPA-5),
  so no browser walked the flow.
- **`networkPolicy.idpEgress` is restored** (it was deleted by #383) from
  `f885170^`: `vpay.idpEgressRules` and its two includes, the values, the schema,
  the README section and the `ci/values-full.yaml` shapes. `git diff f885170^`
  on each of those files shows no `idpEgress` line, and the README differs only
  by its section's lead-in, which points at the flow page now that the chart has
  no "Staff single sign-on" section. Helm 3.19.0 and kubeconform 0.6.7
  were fetched for it: `helm lint` is clean with the defaults and with
  `ci/values-full.yaml`; `just helm-check` passed (26 guards, 68 resources
  valid); the default, `ci/values-route.yaml` and
  `ci/values-route-networkpolicy.yaml` renders are byte-identical to the
  renders before the restoration; with `networkPolicy.enabled` and an
  `idpEgress.cidrs` peer the rule renders on `-management` in the split release
  and on `-server` with management off, and on neither the worker nor the split
  `-server`; `0.0.0.0/0`, `::/0` and `169.254.169.254/32` are refused by the
  schema. Not shown: that a CNI enforces it, and no CI values file renders the
  management-off branch (checked by hand, once). Nothing has run in a cluster.
