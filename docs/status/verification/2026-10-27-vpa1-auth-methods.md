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
| `cargo test -p vpay-config`                                                                                                                                    | 161 passed, and 7 doctests                             |
| `cargo test -p vpay-api --lib`                                                                                                                                 | 451 passed (17 of them are `auth_methods_tests`)       |
| `cargo test -p vpay-api --doc`                                                                                                                                 | 24 passed                                              |
| `cargo test -p vpay-server --lib --bins`                                                                                                                       | 9 and 8 passed                                         |
| `cargo check -p merchant-demo`, via `cargo clippy -p merchant-demo --all-targets -- -D warnings`                                                               | clean                                                  |
| `just gen-demo-keys`, then `Config::load_with_env` over the base file and the generated overlay (a scratch test, not committed)                                | the overlay loads: keypair on, tokens off, 2 merchants |
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

## Review round: the adversarial review of #384

The review found nine things. Each is a commit on top of the PR; the guard each
one added was broken, its test seen to fail, and the source restored (a `diff`
against the saved copy printed nothing).

| #   | Finding                                                                             | What it became                                                                                                                                     | The guard that failed when broken                                                                                                                                                                       |
| --- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | the demo's steps 2 and 3 fail under the default config                              | the demo overlay runs `[keypair, hmac_webhooks]` and drops `jwks`, `grant_types`, `allowed_audiences`; a stale overlay is regenerated              | with the freshness predicate removed `gen-demo-keys` kept a stale overlay; with it, it regenerated and said why                                                                                         |
| 2   | the default still demanded the OAuth half of every merchant                         | required together and only for a merchant with a `jwks`; every merchant needs a credential (`MerchantNoCredential`)                                | credential check off: `a_merchant_needs_a_credential_and_cose_keys_alone_is_enough_under_the_default`; old behaviour back: that test and `grant_types_without_a_jwks_are_still_only_client_credentials` |
| 3   | the docs said a token caller has its own idempotency and rate-limit namespace       | corrected in ADR-0037, signed-transport.md and this page; integration case (m) pins one key for both credentials                                   | not run here (it needs Postgres); it compiles                                                                                                                                                           |
| 4   | `auth: {methods: ~}` read as the absent key (all four); near-miss keys were dropped | a null is `NoAuthMethods`; `auth_methods`, `auth-methods`, `authmethods`, `authentication` are `NearMissKey`                                       | null read as absent: `every_auth_methods_rule_refuses_its_fixture_with_its_own_variant`; near-miss loop off: `a_near_miss_of_the_auth_key_is_refused_not_ignored`                                       |
| 5   | the shipped `/v1` validator was typed in `main.rs` and copied by every test         | `merchant_token_validator` and `mcp_token_validator` in `vpay-api`, called by `main.rs`, the integration support and the unit fixtures             | `/v1` audience changed to the MCP resource: six cases including `a_v1_token_is_refused_by_mcp_and_an_mcp_token_by_v1`; MCP audience changed to `vpay:v1`: that test                                     |
| 6   | a token caller could be sealed an answer (`Cratestack-Nonce` + `Accept`)            | the response seal policy under `Optional` is "never"                                                                                               | policy removed: `both_ways_a_bearer_caller_is_never_handed_a_sealed_answer`                                                                                                                             |
| 7   | `disable_client` did not reach a token caller                                       | the scope step asks `disabled_clients` for the token's `client_id`: `401` `invalid_token`; a switch that cannot answer is an error                 | check of `Ok(true)` removed: `a_token_for_a_disabled_client_is_refused_and_an_unanswerable_switch_is_an_error`; `Err` swallowed: the same test                                                          |
| 8   | a signed request with `Authorization` is `400` even under `[keypair]` (was ignored) | kept; upgrade note in ADR-0037 and the config reference                                                                                            | (existing) `a_signature_and_a_bearer_together_are_a_400`                                                                                                                                                |
| 9   | nits                                                                                | ADR-0035 marked superseded in part by ADR-0037; `AuthMethods::served()` defined as permitted minus `human_login` in its doc and the discovery docs | docs only                                                                                                                                                                                               |

**The seal-policy test needed a second look.** Its first version sent
`Cratestack-Nonce` and `Accept: application/cose` and passed with the policy
removed too: the layer answered `406` first, because an unsigned request's
response type defaults to CBOR and `/v1` declares JSON only. Sealing needs
`Cratestack-Payload-Accept: application/json` as well. With that header the
test fails without the policy, which is the mutation row above.

**One test is flaky and was not written here.** `both_ways_a_valid_token_reaches_the_handler_and_is_answered_in_plain_json`
(from the PR's first commit) failed once in about ten runs, on its
`auth.method="machine_token"` log assertion, in a run that changed nothing it
reads. The log it captures printed no span context in that run, which is what a
`tracing` callsite cached as disabled by a neighbouring test's missing
subscriber looks like. It passed in the three runs made after, and is not fixed
here.

## Review round 2: the second adversarial review of #384

Six findings. Each is a commit on top of the PR; the guard each one added was
broken in the source, its named test seen to fail, and the source restored with
`git checkout`. The name in brackets labels the run (the failing output was kept
outside the repository).

| #   | Finding                                                                                  | What it became                                                                                                                                                           | The guard that failed when broken (sabotage name)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1  | MCP bearer tokens ignored the kill switch, so "instant on both doors" was false for them | `MerchantMcpAuth` takes the `DisabledClients` port: `Ok(true)` is `InvalidToken`, `Err` is `Unavailable`; an integration case flips `disable_client` under a live token  | `an_mcp_token_for_a_disabled_client_is_refused_and_an_unanswerable_switch_is_unavailable`: check removed (`m1-mcp-no-kill-switch`), `Ok(true)` admitted (`m1c-mcp-disabled-admitted`), `Err` swallowed (`m1b-mcp-lookup-error-admits`). The integration case compiles; it was not run here                                                                                                                                                                                                                                                                       |
| L1  | `Auth:` at top level and `auth:` under `deployment:` loaded as all four methods          | top-level keys match `auth` and the near-miss list case-insensitively (the exact `auth` stays valid); `deployment` denies unknown fields; boot logs the resolved methods | `a_case_variant_of_the_auth_key_is_refused_not_ignored`: `auth` match off (`l1-auth-key-case-sensitive`), list match case-sensitive (`l1-near-miss-list-case-sensitive`), exact `auth` refused too (`l1-exact-auth-refused-too`, five other tests fail); `an_auth_block_under_deployment_is_refused_not_ignored`: `deny_unknown_fields` removed (`l1-deployment-allows-unknown-fields`); `boot_logs_the_resolved_auth_methods` (`boot-log-explicit-flag-wrong`) and `load_config_says_which_methods_the_file_resolved_to` (`boot-log-not-called-by-load-config`) |
| L2  | a token was held to the scopes it was issued with, not to the registration's live YAML   | `ResourceConfig::token_holds` intersects the token's claim with the YAML `scopes`, on `/v1` and on MCP                                                                   | `a_token_holds_only_what_its_claim_and_the_registrations_yaml_both_name` (`l2-token-holds-ignores-yaml`), `a_tokens_scopes_are_held_to_the_registrations_current_yaml` (`l2-v1-scopes-from-token-only`), `an_mcp_tokens_scopes_are_held_to_the_registrations_current_yaml` (`l2-mcp-scopes-from-token-only`); integration `a_token_claiming_more_than_its_registration_grants_cannot_write` compiles only                                                                                                                                                        |
| L3  | ADR-0037 Consequences said vaam sets `[keypair]`                                         | `[keypair, hmac_webhooks]`, per decision 1; the chart's overlay example too                                                                                              | docs only                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| N1  | a dormant OAuth half (`grant_types`/`allowed_audiences` with no `jwks`) booted silently  | `MerchantOauthWithoutJwks`; two fixtures; the demo overlay and every shipped config still load                                                                           | `an_oauth_half_without_a_jwks_is_refused` (`nit1-dormant-oauth-half-accepted`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| N2  | the kill switch was asked before the in-memory registration lookup                       | registration first on `/v1` and MCP; the fake switch counts lookups                                                                                                      | `an_unregistered_clients_token_is_forbidden_without_a_kill_switch_query` (`nit2-v1-query-before-registration`), `an_unregistered_mcp_client_is_refused_without_asking_the_kill_switch` (`nit2-mcp-query-before-registration`)                                                                                                                                                                                                                                                                                                                                    |

**An existing test depended on the old behaviour and was changed, not the
guard.** `both_ways_a_read_token_may_read` registered its merchant with
`payments:write` only and minted a read token, which passed because the token
was never held to the YAML. The router fixture for token cases now registers
both scopes, and `only_a_token_addressed_to_v1_is_accepted_on_v1` (integration)
mints the registration's own `payments:write` for its audience-only case.

## What this does not prove

- That a signed and a token request of one merchant share an idempotency key
  against the real table: the claim is merchant-scoped in the code
  (`Idempotency::claim(merchant_id, key, ...)`), and integration case (m) says
  it from outside, but it compiles here and was not run.
- That `disable_client` reaches a token minted before it, against the real
  `disabled_clients` table: case (b) says it for `/v1` and
  `a_token_minted_before_disable_client_is_refused_after_it` for the MCP
  endpoint, and both compile. The unit tests use an in-memory kill switch.
- That the demo walkthrough runs end to end: the overlay it needs is generated
  and loads, and the example builds, but the stack needs Docker.
- That the SDKs speak the token half: they do not (VPA-6).
- That a legacy-only (`keypair` off) deployment boots end to end: the config and
  the layer mode are tested; the server's key loading is unchanged and still
  requires the response keys.
