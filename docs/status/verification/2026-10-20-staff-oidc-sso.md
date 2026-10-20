# 2026-10-20 — Staff single sign-on through an OpenID Connect provider

Branch `feat/staff-oidc-sso`, from `master` at `388672e`. Implements
[ADR-0030](../../adr/0030-staff-sign-in-federates-to-an-oidc-provider.md): the
four decisions ADR-0019 reserved (R1 for `oidc`, R2, R3, R4), taken by the
maintainer on 2026-10-19. **One migration (`0050`), one CrateStack model, three
unauthenticated routes, two dashboard routes, one config block, one Helm value.
No `NotImplemented` token moved, and password + TOTP sign-in is unchanged.**

This page says what was run, what it printed, what a real Keycloak showed, and
what was **not** run. The decisions are the ADR's; the process is
[dashboard-auth/sso.md](../../flows/dashboard-auth/sso.md).

## What a real Keycloak 26.6.1 showed

`backends/tests/integration/tests/staff_sso_keycloak.rs` boots
`quay.io/keycloak/keycloak:26.6.1` with `tests/fixtures/keycloak-realm.json`,
drives the browser's half by hand (login form, OTP form, `CONFIGURE_TOTP`,
`prompt=login`, SSO cookie) and hands the resulting `code` to vpay's real
callback over a real Postgres. It is **one test, one container, about fifteen
seconds**, and it passed **6 of 6** consecutive runs after the realm was
corrected (see the measurement that corrected it, below).

| Sign-in                                                                                                                                                                     | `acr` | `amr`           | vpay (`accepted_amr: ["otp"]`)                    |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- | --------------- | ------------------------------------------------- |
| Default browser flow, password + OTP, `oidc-amr-mapper` on the client _(measured 2026-10-19 on the same image by the person who commissioned this change, not re-run here)_ | `"1"` | `[]`            | would refuse every sign-in                        |
| Realm flow whose executions carry `default.reference.value` (`pwd`, `otp`), password + OTP                                                                                  | `"1"` | `["pwd","otp"]` | **signed in**, member provisioned                 |
| First sign-in, OTP **enrolled during it** (`CONFIGURE_TOTP`)                                                                                                                | `"1"` | `["pwd"]`       | **`reauthenticate`** (log: `acr="1" amr=["pwd"]`) |
| …the `prompt=login` retry: password-only form (user remembered), then OTP form with the next step's code                                                                    | —     | has `otp`       | **signed in**                                     |
| Repeat sign-in from the SSO cookie, **3 s later**, executions carry `default.reference.maxAge` = 28800                                                                      | `"0"` | `["pwd","otp"]` | **signed in**, link reused                        |
| Same, **2.5 s later, no `maxAge`** (probe against a container, not in the suite)                                                                                            | `"0"` | **`[]`**        | would take the retry every time                   |
| A user outside the group, with a valid OTP                                                                                                                                  | —     | —               | `401`, not provisioned, no retry                  |

**A correction to an earlier measurement.** It was measured on
2026-10-19, before this change was written, that an SSO-cookie token "still carries `amr ["pwd","otp"]`". It does
— **inside about a second**. The first version of the Keycloak test re-entered
immediately, passed on a good run and failed **2 of 8** (`acr="0" amr=[]`, and
once `acr="1" amr=[]`), at which point the cause was found: Keycloak drops `amr`
from a cookie token after a window that `default.reference.maxAge` on the
authenticator config controls, and a person is never that quick. Probed on a
fresh container: re-entry at 2.5 s gave `amr []`; after setting `maxAge` to 28800
through the admin API it gave `["pwd","otp"]`. The realm fixture now sets it, the
test waits three seconds before re-entering so the setting is load-bearing in the
test rather than a coincidence of speed, and ADR-0030 and the flow page list it as
a realm requirement. Without it SSO still works — vpay's retry covers it — but
every repeat sign-in goes round Keycloak's forms.

## Gates

All run on this branch, in the worktree, on macOS (aarch64) with Docker 29.8.

| Command                                                 | Result                                                                                                                                                                                                                                                            |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just fmt`                                              | clean (prettier `--write` over the tree, `cargo fmt --all`)                                                                                                                                                                                                       |
| `just verify`                                           | **ok — the fifteen gates passed**, including `verify-migrations` (50 files match `MANIFEST.sha256`), `verify-privacy-inventory` (314 columns, 26 elements), `verify-serde` (112 types), `verify-doc-counts` (13 counts agree), `check-schema` (cratestack 0.15.1) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (`just clippy`, inside the run below)                                                                                                                                                                                                                       |
| `cargo nextest run --workspace --no-fail-fast`          | **2198 tests, 50 binaries, 0 ignored, 0 skipped: 2192 passed, 6 failed** — see below                                                                                                                                                                              |
| `just test-doc`                                         | **128 doctests passed, 1 ignored** (the one pre-existing `ignore`), counted separately from the above                                                                                                                                                             |
| `just verify-ignored`                                   | `0 ignored (expected 0), 50 test binaries (expected 50), 2198 total` — `expected_suites` was **48** and is **50**, the two new binaries                                                                                                                           |
| `just lint-web`, `just test-web`                        | pass; dashboard **33 files, 406 passed, 1 skipped** (it was 30, 316 and 1)                                                                                                                                                                                        |
| `just helm-check`                                       | **ok** — lint, 4 renders, 24 guards, kubeconform `Valid: 68, Invalid: 0`                                                                                                                                                                                          |
| `node tools/verify-coverage.mjs` (vpay-skills)          | parity: 20 skills, 255 paths, 47 feature pages                                                                                                                                                                                                                    |

**The six failures, accounted for rather than waved away:**

- **Four are host setup** and are named in `CLAUDE.md`:
  `the_sign_in_rate_limit_is_per_source_address`,
  `the_second_factor_is_rate_limited_and_not_only_the_password`,
  `a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget` and
  `two_replicas_share_one_sign_in_budget` bind `127.0.0.x` source addresses and
  this Mac has no `lo0` aliases (`Can't assign requested address (os error 49)`,
  with the message naming `just loopback-aliases`). **`just loopback-aliases` was
  not run**: it needs `sudo ifconfig` and changes the machine's network
  configuration, which is not this change's to do. They are unmodified by this
  change (one fixture literal in that file gained `sso: None`), and they are not
  gated or ignored.
- **Two are Docker load flakes under a 22-minute full run**
  (`worker_recovery::a_young_redirect_charge_is_not_failed_as_a_dead_order_until_it_is_older`,
  `checkout_sessions::every_url_rule_is_enforced_at_create_and_names_its_parameter`).
  Both **pass in isolation** (`2 tests run: 2 passed`). Two earlier fail-fast
  `just ci` runs stopped on other members of the same family
  (`SSLRequest 0x48` on a freshly started Postgres; `PortNotExposed` on the rail
  stub) — the failure mode `vpay-testkit/src/containers.rs` documents.
- **`just ci` itself was not green end to end.** Its `test-rust` step is
  fail-fast and the host cannot pass the four above; `audit-web` could not reach
  `registry.npmjs.org` (`ETIMEDOUT`, three retries — a network failure, not an
  advisory); and `just deny` reports `advisories FAILED` for
  `RUSTSEC-2026-0346`, `-0347` and `-0348` against `rustls 0.23.45`. **That
  version is `master`'s own** — this change's `Cargo.lock` diff adds `p256`,
  `url` and `wiremock` to existing dependency lists and no package — so these are
  advisories published since `master` was cut, not something this change
  introduced. `bans`, `licenses` and `sources` are ok. Every other `just ci`
  step passed individually.

## New tests, and what each is for

| Where                   | Count | Proves                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ----------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `staff_sso.rs`          | 23    | the real router, a real Postgres, a simulated provider over HTTP with tokens signed by a key the test generates: the happy path through `/dash/v1/oauth/token` and a `/dash/v1` read; link reuse; roles stay local; nonce, `aud`, `azp`, `iss`, expiry, future `iat`, unknown `kid`, no `kid`, `alg: none`, HS256; the group gate; `amr` and `acr` rules; the retry in all four shapes the maintainer listed; disabled member; state replay and wrong state; email collision; a row for another issuer; a refused code; PKCE and `client_secret_basic` at the token endpoint; flow expiry and sweep; the rate limit; an unconfigured deployment; a discovery document for another issuer; that nothing the provider sent reaches a response |
| `staff_sso_keycloak.rs` | 1     | the section above                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `vpay-api` unit         | 20    | id_token verification (RS256 and ES256), the allow-list, the second-factor rule, groups, nonce; `KeySetCache` (encryption keys skipped, EC keys kept, one throttled re-fetch, rotation, failure); address normalisation                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `vpay-db`               | 9     | a pending flow is consumed exactly once, **8 racing takes have 1 winner**, expiry and sweep, the table's CHECKs, `find_federated` on the pair, provisioning is atomic on both unique violations, the `@@allow` arms                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `vpay-config`           | 9     | every shape rule, the empty-MFA-lists boot refusal (either list alone is enough), SSO needs a dashboard client and the staff secrets, livemode https and stub-host rules, a literal secret refused in livemode, the YAML shape and its defaults                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| dashboard (vitest)      | +90   | the two routes in every branch, the login page, the API helpers, the middleware pin, an axe case                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |

`postgres_smoke.rs`: `EXPECTED_DRIFT_CHANGES` **201 → 206** and
`EXPECTED_DRIFTED_RELATIONS` **26 → 27**, **measured against a freshly migrated
Postgres** — the report read `drift detected in 27 table(s)/view(s) (206
change(s) total)`, four `[safe] CHECK` lines and one `[safe] index` line, all on
`staff_sso_attempts` — and the migration count `49 → 50`.
`schema::tests::no_generated_model_route_is_mounted_only_the_one_procedure_is`
grew its `MODEL_TABLES` to 21 (a model declared and not listed there is the
defect that test was written for).

## Mutation checks

Each mutation edited one line, ran the named tests, and was reverted from a
backup (`git diff` afterwards shows none). **Killed** means a test went red.

| Mutation                                            | Result                                                                                                                                                                                                         |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| skip the nonce check                                | killed                                                                                                                                                                                                         |
| skip the group gate                                 | killed                                                                                                                                                                                                         |
| let a retry flow retry again                        | killed                                                                                                                                                                                                         |
| skip the disabled-member check                      | killed                                                                                                                                                                                                         |
| skip the `state` comparison                         | killed                                                                                                                                                                                                         |
| skip the second-factor check                        | killed                                                                                                                                                                                                         |
| start the retry without `prompt=login`              | killed                                                                                                                                                                                                         |
| provision `is_admin: true`                          | killed                                                                                                                                                                                                         |
| provision into another `merchant_id`                | killed                                                                                                                                                                                                         |
| `find_federated` ignores the issuer                 | killed                                                                                                                                                                                                         |
| `consume` ignores expiry                            | killed                                                                                                                                                                                                         |
| `start` stops sweeping                              | killed                                                                                                                                                                                                         |
| `start` stops rate limiting                         | killed                                                                                                                                                                                                         |
| discovery stops comparing `issuer`                  | killed                                                                                                                                                                                                         |
| provisioning on the pool instead of the transaction | killed (`provisioning_a_federated_member_is_atomic`)                                                                                                                                                           |
| replace the `EmailTaken` refusal with a log line    | **survived, and is an equivalent mutant**: the next statement still refuses (`LinkOrphaned`), so the wire answer is the same `401`; the test also asserts no member, no credential and no session were created |

**Not mutation-checked:** `consume`'s compare-and-swap row count. A sequential
test cannot reach it; `racing_takes_of_one_sso_flow_yield_exactly_one_winner`
exercises the race but a mutation that removes the CAS was not run. The
session-fixation property (the callback mints a new token) is asserted
(`session != pending`) but its mutation was not run in this repository (the
dashboard agent ran the equivalent one on its side: reusing the pending token as
the session reddened two vitest cases).

## What was NOT run, and what this does not prove

- **Nothing was run in a cluster**, behind a real ingress, or with a real
  NetworkPolicy. `networkPolicy.idpEgress` renders and passes kubeconform; whether
  a CNI enforces it as intended is unobserved. The `-server` branch of its helper
  (management tier off) is not covered by `just helm-check` — every CI values file
  that enables `networkPolicy` also enables `management`; it was rendered with
  ad-hoc `--set` only.
- **No provider but Keycloak 26.6.1**, and no real provider outside a container.
- **No browser run of the dashboard's two routes.** They are covered by vitest
  against a stubbed `fetch` and by the Rust suites on the other side of the wire;
  nothing drives the real pair through a browser, and the Cypress suite has no
  SSO case. `next build` could not run here (the SWC binary download timed out).
- **`just loopback-aliases`** (above), **`audit-web`** (network) and
  **`just ci` end to end** (above).
- **No `docs-check-citations`** (needs a GitHub token); this page cites no pull
  request or run id.
- **No Storybook story** for the login button; an axe case in `a11y.test.tsx`
  stands in, and `just test-storybook` was not run.
- **The schema change is not exercised by the e2e/demo compose stack.** `0050` is
  applied by every container suite and by `just verify`'s `check-schema`; the
  demo stack was not booted.
