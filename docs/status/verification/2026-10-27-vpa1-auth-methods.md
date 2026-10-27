# VPA-1 verification: authentication methods are deployment configuration

_Written 2026-10-27 while the work was done; every number below was measured on
the tree this page is committed with._

What was built is in
[ADR-0037](../../adr/0037-authentication-methods-are-deployment-configuration.md),
[the config reference](../../reference/vpay-config.md#auth--which-ways-of-authenticating-a-deployment-permits-adr-0037)
and [signed-transport.md](../../flows/signed-transport.md); this page is what
was run, what broke it on purpose, and what could not be run.

## Gates

| Gate                                                                                                                                                           | Result                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `cargo fmt --check`                                                                                                                                            | clean                                                  |
| `cargo clippy -p vpay-config -p vpay-api -p vpay-server -p vpay-db --all-targets -- -D warnings`, and the same over `vpay-tests-integration` and `vpay-worker` | clean                                                  |
| `cargo test -p vpay-config`                                                                                                                                    | 158 passed, and 7 in its second target                 |
| `cargo test -p vpay-api --lib`                                                                                                                                 | 448 passed (15 of them are `auth_methods_tests`)       |
| `cargo test -p vpay-server`, the unit targets                                                                                                                  | 9 and 8 passed                                         |
| `cargo check --tests -p vpay-tests-integration`                                                                                                                | compiles                                               |
| `cargo deny check`                                                                                                                                             | advisories, bans, licenses and sources ok              |
| `just verify`                                                                                                                                                  | the seventeen gates passed (`verify-docs` is advisory) |

**Not run where this was written (no Docker daemon):** `vpay-server`'s CLI suite
(it starts `postgres:16-alpine`; it fails here at `/var/run/docker.sock`,
before reaching any code this change touches), every Postgres-backed suite, and so the integration
cases edited with this change (`merchant_token_flow.rs`, `signed_transport.rs`,
`payment_intents.rs`, and the helper in `support/mod.rs`). They compile; CI runs
them. The sealed round trip against the real replay cache and kill switch is
theirs. The unit test that opens a sealed request through the real layer uses an
in-memory replay cache and kill switch, which is the one thing it replaces.

## What broke it on purpose

Each guard was removed in the source, the tests that name it were run and seen
to fail, and the source was restored from the unmutated text (a `diff` against
the saved copy printed nothing). The logs are not kept; the table is the record.

### `vpay-config`

| Mutation                                         | Failed                                                                                                                                                       |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `NoWayIn` check disabled                         | `every_auth_methods_rule_refuses_its_fixture_with_its_own_variant`                                                                                           |
| `McpNeedsMachineTokens` check disabled           | the same                                                                                                                                                     |
| the `hmac_webhooks` boot gate disabled           | the same                                                                                                                                                     |
| the dormant-`jwks` refusal disabled              | the same                                                                                                                                                     |
| the dormant-`cose_keys` refusal disabled         | the same                                                                                                                                                     |
| the dormant-`dashboard` refusal disabled         | the same                                                                                                                                                     |
| `AuthMethodNotConfigured` disabled               | the same                                                                                                                                                     |
| `human_login` allowed through                    | the same                                                                                                                                                     |
| an empty list allowed through                    | the same                                                                                                                                                     |
| a duplicate allowed through                      | that test and `the_supported_auth_shapes_load_and_resolve_to_what_they_name`                                                                                 |
| an absent `auth.methods` permits `keypair` alone | `an_absent_auth_methods_permits_every_method_and_the_example_still_loads` and `a_merchant_client_with_no_grant_types_is_rejected_while_tokens_are_permitted` |

### `vpay-api`

| Mutation                                                                         | Failed                                                                            |
| -------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `keypair` + `machine_tokens` mapped to `Required`                                | six cases (every token case)                                                      |
| `Optional` as a bare mode instead of the closure with the pinned unresolved mode | `optional_keeps_unserved_methods_a_405_from_the_layer`                            |
| `machine_tokens` alone mapped to `Optional`                                      | `the_envelope_mode_follows_auth_methods`                                          |
| the `400` for a signature and a bearer removed                                   | `a_signature_and_a_bearer_together_are_a_400`                                     |
| `/token` mounted whenever the business surface is                                | `keypair_alone_refuses_a_bearer_with_the_layers_401_and_mounts_no_token_endpoint` |
| the token path's scope check removed                                             | `both_ways_a_token_that_does_not_earn_the_request_is_refused_plainly`             |
| the `keypair` span value removed                                                 | `the_same_router_seals_for_a_signer_and_answers_a_token_in_plain_json`            |
| the `machine_token` span value removed                                           | `both_ways_a_valid_token_reaches_the_handler_and_is_answered_in_plain_json`       |
| discovery lists permitted instead of served methods                              | `discovery_lists_the_methods_a_deployment_serves`                                 |

**One mutation survived the first time**, and is why the second-to-last row
exists: removing the `machine_token` span value passed, because the assertion
that should have read it had not been added (a text replacement that matched
nothing, after a reformat). The assertion was added and the mutation run again;
it failed.

## What this does not prove

- That a token caller is rate-limited or deduplicated as a signer is: the keys
  differ by credential (ADR-0037, Consequences) and no test pins either.
- That the SDKs speak the token half: they do not (VPA-6).
- That a legacy-only (`keypair` off) deployment boots end to end: the config and
  the layer mode are tested; the server's key loading is unchanged and still
  requires the response keys.
