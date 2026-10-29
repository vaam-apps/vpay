# VPA-2 verification: the human sign-in's persistence, restored

_Written 2026-10-28/29 while the work was done; every number below was measured
on the tree this page is committed with._

What was built is in [docs/status/backend.md](../backend.md) ("The human
sign-in's tables are back, empty, and nothing writes them") and
[docs/status/cratestack.md](../cratestack.md) (the drift record); this page is
what was run, what broke it on purpose, and what could not be run.

## Gates

| Gate                                                         | Result                                                                                                  |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| `cargo fmt --check`                                          | clean                                                                                                   |
| `cargo clippy --workspace --all-targets -- -D warnings`      | clean                                                                                                   |
| `cargo deny check`                                           | advisories, bans, licenses and sources ok                                                               |
| `just verify`                                                | the seventeen gates passed (`verify-docs` is advisory)                                                  |
| `cargo test -p vpay-db --lib`                                | 149 passed (against a native PostgreSQL 16, see below)                                                  |
| `cargo test -p vpay-db --test repositories`                  | 137 passed, 18 of them in the credential, session, code, SSO and migration families this change touches |
| `cargo test -p vpay-tests-integration --test postgres_smoke` | 55 passed, including the drift count (221 / 30) and the multi-column CHECK list (33)                    |

**How the Postgres suites ran without Docker.** They start `postgres:16-alpine`
through `vpay_testkit::containers`. There is no Docker daemon where this was
written, so a throwaway, **uncommitted** patch swapped that helper for a fresh
native PostgreSQL 16.15 server per test (`initdb`, `pg_ctl`, from
`/usr/lib/postgresql/16/bin`), and was reverted before the commits. The suites
are otherwise unchanged. `vpay-db`'s `tests/postgres.rs` and the other
integration suites, `vpay-server`'s CLI suite and the e2e compose were **not
run**; none of them reads a table this change touches.

## The migration is the originals' DDL, measured

`pg_dump -s` of `staff_sessions`, `staff_sso_attempts` and
`oauth_authorization_codes` on a database migrated through 0055, and on one
migrated through 0055, 0056, 0057 and 0058, are byte-identical once pg_dump's
per-run `\restrict` token is removed; so is `credentials` (which shows
`credentials_password_and_totp_are_retired` gone and nothing else moved).

`cratestack migrate baseline --strict` under the schema at `origin/master`
reproduced the pinned 209 / 27 on a database through 0057 (the method is
sound), and reports 221 / 30 under this schema through 0058: the thirteen
`[safe]` lines 0056 removed return and the retiring CHECK's line goes (209 + 13

- 1).

## Guards, broken on purpose

Each mutation was applied to the tree, the named test was seen to fail, and the
file was restored. The diffs and failure output are logged one per file in the
session's scratchpad (`vpa2-sabotage/`).

| #   | Mutation                                                                 | Failed                                                                                   |
| --- | ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| 1   | 0058 keeps `credentials_password_and_totp_are_retired`                   | `migration_0058_restores_the_three_tables_empty_and_keeps_what_0056_kept`                |
| 2   | 0058 renames `staff_sessions_state_is_known`                             | same (the shape list)                                                                    |
| 3   | 0058 forgets `staff_sessions_expiry_idx`                                 | same                                                                                     |
| 4   | `touch` loses its `last_seen_at < now` filter                            | `a_session_has_two_bounds_and_an_expired_one_is_refused_not_deleted`                     |
| 5   | `IDLE_TIMEOUT` 30 -> 60 minutes                                          | same                                                                                     |
| 6   | `ABSOLUTE_LIFETIME` 12 -> 24 hours                                       | same, **after a fix**: the first version of the test read the constant and survived this |
| 7   | `mark_authenticated` loses its `state = 'pending_totp'` guard            | `a_session_is_promoted_once_carries_its_token_and_is_revoked_by_deleting`                |
| 8   | `delete_others` loses its `id != keep_id` filter                         | same                                                                                     |
| 9   | `consume_code` loses `consumed_at IS NULL`                               | `an_authorization_code_is_spent_exactly_once_and_dies_with_its_session`                  |
| 10  | `consume_code` checks expiry before the swap                             | same                                                                                     |
| 11  | `advance_counter` loses its `counter <` compare                          | `the_credential_guards_are_compare_and_swaps_and_only_one_caller_wins`                   |
| 12  | `model StaffMember` loses its `delete` arm                               | `staff::tests::every_action_this_module_calls_has_an_allow_arm`                          |
| 13  | `MODEL_TABLES` lists `Currency` where `StaffSsoAttempt` was              | `schema::tests::no_generated_model_route_is_mounted_only_the_one_procedure_is`           |
| 14  | `CredentialKind::parse` loses `password`                                 | `credentials::tests::the_kind_vocabulary_round_trips_and_admits_nothing_else`            |
| 15  | 0058 swaps `oauth_authorization_codes_method_is_s256` for a weaker CHECK | `an_authorization_code_with_a_plain_pkce_method_is_refused_by_the_database`              |
| 16  | 0058 drops `ON DELETE CASCADE` on `session_id`                           | `signing_out_cascades_onto_a_code_in_flight`                                             |
| 17  | 0058 forgets `staff_sessions_staff_idx`                                  | `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`                      |
| 18  | privacy inventory forgets `staff_sessions.state`                         | `just verify-privacy-inventory`                                                          |
| 19  | a byte of 0058 moves after its manifest line                             | `just verify-migrations`                                                                 |
| 20  | security inventory forgets `staff_sessions`                              | `just verify-security-inventory`                                                         |
| 21  | `StaffSession` loses `access_token_expires_at`                           | `just check-schema` (the generated dash-client is stale)                                 |

Mutation 13 first failed to compile (the array length is a second guard); it was
re-run with the length kept.

## Not done, and what this does not prove

- Nothing calls the restored repositories, so this proves the persistence and
  nothing of a sign-in. VPA-3 owns the routes, `staff_auth`, `staff add`'s
  password, the un-retired `staff_auth` and `dashboard_client` keys and the
  `human_login` method.
- The OP's authorization-code slot is still `RefusingAuthorizationCodeStore`.
- The security inventory's `staff_auth.*` secret classes were not restored: their
  configuration keys do not exist yet.
- The migration was applied to empty and to seeded post-0056 databases, never to
  a copy of a production database.
