# VP-6a verification: the console is a signed message under a paired key

_Written 2026-10-27 while the work was done; every number below was measured on
the tree this page is committed with._

What was built is in
[ADR-0036](../../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)
and [dashboard-auth.md](../../flows/dashboard-auth.md); this page is what was
run, what broke it on purpose, and what could not be run.

## Gates

| Gate                                                                                           | Result                                                                                                               |
| ---------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `just verify`                                                                                  | the sixteen gates passed                                                                                             |
| `cargo fmt --all -- --check`                                                                   | clean                                                                                                                |
| `cargo clippy --workspace --all-targets -- -D warnings` on 1.98.0 and on 1.99.0                | clean on both                                                                                                        |
| `cargo deny check`                                                                             | ok                                                                                                                   |
| `vpay-config`, `vpay-api`, `vpay-wire` and `xtask` unit tests                                  | all passed                                                                                                           |
| `vpay-db`, every binary, against `postgres:16-alpine`                                          | 268 passed                                                                                                           |
| `vpay-server`, its CLI suite (staff add, pair-key, trust store) against `postgres:16-alpine`   | 62 passed                                                                                                            |
| `console_pairing` (10), `dashboard_read_surface` (20), `dashboard_procedure_transport` (5)     | 35 passed                                                                                                            |
| `postgres_smoke`, with the drift constants re-measured (208 changes, 27 relations, 20 columns) | 51 passed                                                                                                            |
| `just verify-ignored`                                                                          | ok at 54 suites (the dated note in the justfile)                                                                     |
| `pnpm -r test` (the regenerated `@vpay/dash-client`; dashboard app source untouched)           | 480 passed; the two `payments-filters` calendar tests timed out at 5 s under load and passed on their own (12 of 12) |

The clean full-workspace pass was not one uninterrupted run: the sandbox disk is
shared and filled twice, and Docker's `vfs` storage driver copies the image into
each container, so two runs died with `no space left on device` while creating a
container. Every suite above was rerun to the end after space was freed; none of
those failures was a test failure.

## What broke it on purpose

Each guard was removed in the source, the test that names it was run and seen
to fail, and the source was restored from the unmutated text. The test asserts
the layer's own coarse body, which is why swapping `Required` for `Optional`
is caught and not just a status code.

| Mutation                                                        | Test that failed                                                                    |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `/dash/v1` and `/svc/v1` layers set to `EnvelopeMode::Optional` | `no_dash_route_is_reachable_without_a_signature`, `a_merchant_key_cannot_reach_svc` |
| the procedure layer set to `EnvelopeMode::Optional`             | `an_unsigned_caller_a_merchant_and_a_revoked_key_are_the_coarse_401`                |
| the `expires_at > now` predicate dropped from the resolver      | `a_key_expires_thirty_days_after_it_was_paired`                                     |
| the `revoked_at IS NULL` predicate dropped from the resolver    | `a_revoked_key_is_refused_on_its_very_next_request`                                 |
| merchant keys registered on `/svc` with `console_pairing`       | `a_merchant_key_cannot_reach_svc`                                                   |
| the capability check skipped in `service_scope`                 | `a_service_without_console_pairing_cannot_pair_or_revoke`                           |
| a just-in-time member provisioned with `is_admin: true`         | `vaam_pairs_a_key_and_the_person_reads_as_a_plain_member_for_thirty_days`           |
| the admin refusal removed from the pairing route                | `pairing_never_creates_or_upgrades_an_administrator`                                |
| the `OPTIONS` arm removed from the dashboard scope              | `an_anonymous_options_is_the_routers_405_not_a_500`                                 |
| a non-admin allowed to name the tenant with the query parameter | `a_non_admin_cannot_move_the_tenant_with_the_query_parameter`                       |
| the key lifetime set to 744 hours                               | `vaam_pairs_a_key_and_the_person_reads_as_a_plain_member_for_thirty_days`           |
| the active-member check removed from the dashboard scope        | `a_disabled_member_and_a_foreign_member_are_refused_after_the_signature_verified`   |
| the procedure layer built without its `/dash/v1` mount prefix   | `the_procedure_answers_the_callers_own_merchant_over_http`                          |

In every row exactly the listed test or tests failed, and the tree was clean
again afterwards.

## What could not be run

- The compose stack and the Cypress and e2e suites: no Docker-in-Docker here,
  and the dashboard's own Cypress spec was deleted with the sign-in it drove.
- `helm lint` and `kubeconform`: neither is installed. The chart edits
  (management deployment envelope keys, SSO and egress values removed) are
  read-checked and covered only by the repository's text gates.
- Docker image builds and `hadolint`: no Dockerfile changed.
- The dashboard cannot sign in until VP-6b adds the sealed sign-in to its
  frontend. Nothing here pretends otherwise.
