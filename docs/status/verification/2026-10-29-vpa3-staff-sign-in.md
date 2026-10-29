# VPA-3 verification: staff sign-in is served again, beside the console keys

_Written 2026-10-29 while the work was done; every number below was measured on
the tree this page is committed with._

What was built is in [docs/status/backend.md](../backend.md) ("Staff sign-in is
served again, beside the console keys") and in
[the flow page](../../flows/dashboard-auth/human-login.md); this page is what was
run, what broke it on purpose, and what could not be run.

## Gates

| Gate                                                         | Result                                                                   |
| ------------------------------------------------------------ | ------------------------------------------------------------------------ |
| `cargo fmt --all --check`                                    | clean                                                                    |
| `cargo clippy --workspace --all-targets -- -D warnings`      | clean                                                                    |
| `cargo deny check`                                           | advisories, bans, licenses and sources ok                                |
| `cargo test -p vpay-config -p vpay-api -p vpay-server` (lib) | 185 + 536 + (9 + 8) passed                                               |
| doctests of `vpay-config`, `vpay-api`, `vpay-db`             | 27, 7 and 10 passed                                                      |
| `cargo test -p vpay-tests-integration --test staff_sign_in`  | 28 passed against a native PostgreSQL 16 (27 restored or adapted, 1 new) |
| `cargo test -p vpay-server --test cli`                       | 51 passed against a native PostgreSQL 16                                 |
| the other `/dash/v1` and envelope integration suites         | see below                                                                |
| `just verify`                                                | the seventeen gates passed (`verify-docs` is advisory)                   |

The other integration suites that build a `Config` or a router, run the same way:
`boot_coherence` 1, `console_pairing` 11, `dashboard_procedure_transport` 6,
`dashboard_read_surface` 20, `mcp_merchant` 14, `merchant_token_flow` 15,
`payment_intents` 26 and `signed_transport` 7, all passed.

**How the Postgres suites ran without Docker.** They start `postgres:16-alpine`
through `vpay_testkit::containers`. There is no Docker daemon where this was
written, so a throwaway, **uncommitted** patch swapped each suite's container for
a fresh native PostgreSQL 16 server per test (`initdb` and `pg_ctl` from
`/usr/lib/postgresql/16/bin`, run as the `postgres` user, data under `/tmp`),
and was reverted before the commits; the data directory was deleted. One run of
`staff_sign_in` lost a test to the patch itself (`pg_ctl start` failed: the free
port it had picked was taken before the server bound it); the same suite, run
again, passed 28 of 28.

`just verify` failed twice on the way, both for the documents and neither for the
code: `verify-links` named the new flow page while it was still untracked, and
`verify-doc-counts` found `docs/flows` holding six directories where its README
said five (the sentence now says why the sixth exists).

## What the tests found

- **The new two-doors test passed for the wrong reason.** `a_session_bearer_and_a_console_key_both_read_dash_v1`
  sends a signature **and** an `Authorization: Bearer` header and expects `400`.
  The sealed test client only forwarded the envelope's own headers, so the bearer
  never left the process and the request answered `200` as a plain signed read. The
  client now forwards the `authorization` header a case sets
  (`tests/support/sealed.rs`), and the case asserts the `400`.
- **A log line broke a VPA-2 guard.** `staff_add_creates_a_staff_member_and_prints_its_id_on_stdout`
  asserts that the word `password` appears nowhere in a passwordless `staff add`.
  The new line said "one-time password (when one was issued)". It now says
  "sign-in credential", and its boolean field is `credential_issued`.
- **A write is the layer's `405`, not the scope step's `403`.** The
  unit test first expected `403` for a `POST` carrying a valid session; the
  envelope layer answers `405` for a method its binding does not list, even
  unsigned. The test now asserts the `405` at the router and the `403` of
  `dash_scope`'s own backstop through a probe router with no layer.

## Guards, broken on purpose

Each mutation was applied to the tree, the named test was seen to fail, and the
file was restored. The diffs and failure output are logged one per file in the
session's scratchpad (`vpa3/sabotage/`).

| #   | Mutation                                                                | Failed                                                                                            |
| --- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| 1   | `human_login_served` ignores the blocks                                 | `human_login_is_served_exactly_when_permitted_and_both_blocks_are_present` (and one more)         |
| 2   | `dashboard.merchant_id` and `dashboard_client.merchant_id` not compared | `the_console_and_the_sign_in_must_name_one_tenant`                                                |
| 3   | a `staff_auth` secret may be absent                                     | `a_staff_auth_block_without_both_secrets_is_refused_in_every_deployment`                          |
| 4   | `StaffAuth` loses `deny_unknown_fields`                                 | `staff_auth_sso_is_refused_until_it_is_restored`                                                  |
| 5   | a `staff_auth` block with the method off is accepted                    | `every_auth_methods_rule_refuses_its_fixture_with_its_own_variant`                                |
| 6   | a `dashboard_client` block with the method off is accepted              | same                                                                                              |
| 7   | a `staff_auth` without `dashboard_client` is quietly not served         | same                                                                                              |
| 8   | explicit `human_login` with neither block is accepted                   | same                                                                                              |
| 9   | `/dash/v1` is `Optional` whether or not the method is served            | `the_dash_mode_is_optional_iff_human_login_is_served` and three more                              |
| 10  | a signature beside a bearer is let through                              | `a_signature_and_a_bearer_together_are_a_400`                                                     |
| 11  | the `vpay_merchant_id` claim is not checked                             | `a_token_that_is_not_this_tenants_staff_token_is_forbidden`                                       |
| 12  | the registration's scope is not checked                                 | same                                                                                              |
| 13  | a session may use any method the surface refuses                        | `a_session_may_not_use_a_method_the_surface_does_not_serve`                                       |
| 14  | `LoginRequest`'s `Debug` prints the password                            | `the_credential_bearing_wire_types_do_not_print_their_secrets`                                    |
| 15  | `TokenResponse`'s `Debug` prints the access token                       | `the_grants_wire_types_do_not_print_their_secrets`                                                |
| 16  | the dashboard registration gains the refresh grant                      | `the_registration_is_a_public_client_on_one_grant_with_one_scope`                                 |
| 17  | `unresolved_mode(Optional)` on the procedure transport                  | `a_session_may_not_use_a_method_the_surface_does_not_serve`                                       |
| 18  | the xtask forgets `staff_auth.totp_encryption_key`                      | `just verify-security-inventory` (the first attempt only broke the array length: a compile error) |
| 19  | a listed type's `Debug` is derived                                      | `just verify-privacy-inventory`                                                                   |
| 20  | a `debug_protections` row names a type that does not exist              | same                                                                                              |
| 21  | the scope step does not ask whether the member is active                | `disabling_a_staff_member_refuses_their_live_session` and the two-doors test                      |
| 22  | the scope step does not compare the member's tenant                     | `moving_a_staff_member_to_another_merchant_refuses_their_existing_token`                          |
| 23  | `staff add` never mints a password                                      | `staff_add_mints_a_one_time_password_only_where_human_login_is_served`                            |

Two mutations **survived**, and say what the gates do not do:

- Deleting `LoginRequest`'s row from `privacy-inventory.yaml` `debug_protections`
  passes `verify-privacy-inventory`: the gate refuses a listed type that is stale
  or derives `Debug`, but it does not find a hand-written `Debug` that nobody
  listed. The canary tests (14, 15) are what hold the redaction; the list is
  the register.
- Deleting the `staff_totp_encryption_key` class from `security-inventory.yaml`
  passes `verify-security-inventory`: the gate refuses a class that names an
  unknown config key (18), not a config key no class names.

Both are the same finding about the same pair of gates, recorded as it was
measured; neither is changed here.

## Not done, and what this does not prove

- **No deployment ran this.** Every check above is a test against a process on a
  loopback socket and a throwaway database. The OP's JWKS over loopback is
  proved by the integration suite, not by a pod.
- **Staff SSO is not restored** (VPA-4). `staff_auth.sso` is refused as an
  unknown field and the SSO secret class is not in the security inventory.
- **`vpay-db`'s own Postgres suites, `postgres_smoke`, and the other integration
  suites** (`invoices`, `customers`, `refunds`, `webhooks`, `checkout_sessions`,
  the worker suites and the rest) were not run: this change adds no migration
  and edits none of the repositories they exercise. The e2e compose was not run.
- The dashboard application (VPA-5, VP-6b) and the SDK token mode (VPA-6) are
  not touched.
