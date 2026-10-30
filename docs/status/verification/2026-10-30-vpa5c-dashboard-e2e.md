# VPA-5c verification: the dashboard's sign-in in front of a running stack

_Written 2026-10-30 while the work was done, on `vpa5c-dashboard-e2e` over
`master` at `ee200d4` (VPA-5b, #393, on it). Every number below was measured on
the tree this page was committed with. **The end-to-end spec this ticket adds has
not run**: there is no Docker daemon where this was written, and the Cypress
binary was not downloaded. See "Not done" before reading anything here as a
claim that the sign-in works against a stack._

What was built is in [docs/status/frontend.md](../frontend.md) ("The dashboard's
sign-in is exercised end to end (VPA-5c)"), in the app's
[README](../../../frontends/apps/dashboard/README.md), in
[dashboard-auth/human-login.md](../../flows/dashboard-auth/human-login.md)
("Tests") and in the chart's
[README](../../../deploy/helm/vpay/README.md) ("The dashboard's sign-in: four
values, and the overlay text they must equal"). This page is what was read, what
was run, what was broken on purpose, and what could not be run.

## Facts read in the tree before relying on them

| Fact                                                                                                                                                                                                 | Where it was read                                                                         |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `human_login` is served only where `staff_auth` and `dashboard_client` are both written and `auth.methods` permits it; one without the other, or a block with the method left out, is a boot refusal | `docs/reference/vpay-config.md`, "`auth:`"; `AuthMethods::human_login_served`             |
| `dashboard_client.merchant_id` must equal `dashboard.merchant_id` when both are written                                                                                                              | the same page, `DashboardTenantMismatch`                                                  |
| the two `staff_auth` secrets are `${VAR}` placeholders resolved from the environment of every process that loads the config: server, worker and `staff add` alike                                    | `Config::load_with_env`, `resolve_placeholders`                                           |
| `vpay-server staff add` prints the generated one-time password, alone, where `human_login` is served, and the member's `stf_…` id otherwise                                                          | `staff_add` in `backends/apps/vpay-server/src/main.rs`                                    |
| the profile overlay is one file (`application-{profile}.yml`), and the CI e2e job and `just demo` both load the same generated `application-demo.yml`                                                | `Config::load_with_env`; `ci.yml`'s comment on the `demo` profile; `compose.demo.yml`     |
| `/v1/oauth/token` is mounted only with `machine_tokens`, so serving `human_login` adds no bearer door to `/v1`                                                                                       | `vpay-api/src/lib.rs`, `mints_tokens = surfaces.business && machine_tokens`               |
| the dashboard's inputs are `#dashboard-step-<name>`, the QR is `data-testid="totp-qr"`, the typed key is behind a bottom sheet (`data-testid="totp-secret"`), and refusals are `role="alert"`        | `one-question-steps.tsx`, `enrolment-panel.tsx`, `manual-key-sheet.tsx`, `form-alert.tsx` |
| the session cookie is `__Host-vpay_dash_session`, and a wrong current password or a wrong code is not read as the end of the session                                                                 | `cookies.ts`; `actions.ts`, `changePassword` and `submitTotp`                             |
| `compose.e2e.yml` is always used after `gen-demo-keys` (every recipe and CI step that brings it up depends on it), so a required `env_file:` under `.e2e/` adds no new precondition                  | `justfile`: `test-e2e`, `demo-up`, `stripe-compat`, `sdk-live`; `ci.yml`'s `e2e` job      |

The fifth row decides the shape of the ticket's first item. The brief asked for
the e2e stack and not the demo, but they share one generated overlay, so
`human_login` is served by `just demo` too. That is not a second switch hidden
behind the first; it is one file, and the overlay's own header says so.

## What was restored from `f885170^`

| File                                             | Against `f885170^`                                                                                                                                                                                                          |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cypress/tasks/dashboardTasks.ts`                | 250 to 199 lines: `staffTokenExpiry` is gone (no caller), the rest is the same, with the comments rewritten for this spec                                                                                                   |
| `cypress/support/dashboard.ts`                   | already in the tree and importing a spec that did not exist; its header now names this one, and `expectNoWcagViolations` is new                                                                                             |
| `cypress/e2e/dashboard-human-login.cy.ts`        | not a port of the 1288-line `dashboard.cy.ts` and its nineteen tests: seven, because the rest read payments, refunds and deliveries pages that do not exist until VP-6b. The sign-in's legs are the same, on the new inputs |
| `justfile` `demo-staff`, `test-e2e`, `e2e-specs` | the password file and the call to `demo-staff` come back; the file of ids goes                                                                                                                                              |

## Gates

Run on the tree this page is committed with, in a cloud session with no Docker:

| Gate                                                                                               | Result                                                                                                                                                                                                    |
| -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just helm-check` (helm 3.16.1, kubeconform 0.6.7)                                                 | ok: lint, 4 renders, 28 fixtures and 26 guards fired by name, the new dashboard-env block, kubeconform **68 resources valid in 4 files**. Both tools were installed into `/tmp/tools`, not the repository |
| `just verify`                                                                                      | **the seventeen gates passed**, including `verify-links`, `verify-doc-counts` and `verify-ui`. `verify-links` fails until the new page is staged, because it resolves to git-tracked paths                |
| `just lint-web`                                                                                    | exit 0                                                                                                                                                                                                    |
| `just fmt-check-web`                                                                               | exit 0                                                                                                                                                                                                    |
| `pnpm --filter @vpay/e2e typecheck`                                                                | clean                                                                                                                                                                                                     |
| `docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml config`                      | merges; the dashboard carries the three client variables, and `vpay-server` and `vpay-worker` carry the two staff secrets from the `env_file`. `config` needs no daemon                                   |
| `markdownlint` 0.45.0 on the twelve changed pages (MD013 and MD024 off; that version has no MD060) | one finding, `README.md:266` (MD018), on a line this diff does not touch                                                                                                                                  |

No dashboard source changed (`git diff --stat -- frontends/apps/dashboard` is the
README alone), so `pnpm --filter @vpay/dashboard test` was not re-run. No Rust
file changed.

## Sabotage

Each guard was broken, seen to fail, and put back.

| #   | What was broken                                                       | What failed                                                                                                                                                                                    |
| --- | --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| H1  | the scope variable rendered unconditionally (`value: ""` when unset)  | `helm-check: FAIL — the dashboard Deployment carries VPAY_DASHBOARD_SCOPE although dashboard.clientId, redirectUri, scope and ssoIssuer are all unset`                                         |
| H2  | the issuer variable renamed in the template                           | `helm-check: FAIL — ci/values-full.yaml sets the value behind VPAY_DASHBOARD_SSO_ISSUER but the dashboard Deployment does not carry VPAY_DASHBOARD_SSO_ISSUER=https://idp.example/realms/vpay` |
| G1  | the generated overlay's `auth.methods` put back without `human_login` | `gen-demo-keys` named `auth.methods [keypair, hmac_webhooks, human_login]` and regenerated                                                                                                     |
| G2  | the overlay's redirect URI on another port                            | named `no dashboard_client for shop-merchant-tenant on port 3000` and regenerated                                                                                                              |
| G3  | `.e2e/staff-auth.env` deleted                                         | `are out of sync — regenerating all of them`                                                                                                                                                   |
| G4  | the `staff_auth` block cut out of the overlay                         | `does not carry the staff_auth secrets — regenerating the pair`                                                                                                                                |
| G5  | `demo_dashboard_port=3555`                                            | the stale overlay was regenerated with `http://localhost:3555/dash/v1/callback`; a plain run afterwards restored port 3000                                                                     |
| T1  | the TOTP dynamic truncation's offset mask `0x0f` to `0x07`            | 4 of the 6 RFC 6238 Appendix B vectors mismatched (all six pass unmutated: 287082, 081804, 050471, 005924, 279037, 353130)                                                                     |
| C1  | a misspelled `waitForNextTotpStep` call in the spec                   | `tsc` reported one TS error                                                                                                                                                                    |
| V1  | `verify-doc-counts`: the count sentence set back to 133               | `this line says 133, but … measures 134`                                                                                                                                                       |

**Found by running the gates, not by thinking.** `eslint` refused the first draft
of the spec for `Unsafe member access .methods on an any value` (the discovery
body is now cast to `{ methods?: string[] }`). A first draft of
`expectNoWcagViolations` issued `cy` commands inside a callback that also returned
axe's promise, which Cypress refuses at run time and no static gate sees; it was
restructured before it was ever run. And the first run of the patch that adds the
overlay's blocks to `gen-demo-keys` wrote nothing, because the script had no
write; the freshness checks were what showed the overlay unchanged.

**The real configuration loader** accepted the generated overlay. A temporary
test (deleted) called `Config::load_with_env` on `config/application.yml` plus
the overlay `just gen-demo-keys` wrote, with the secrets read from the generated
`.e2e/staff-auth.env`, and `auth_methods().human_login_served()` was `true` with
`redirect_uris = ["http://localhost:3000/dash/v1/callback"]`. The same load with
the two variables absent was refused: `unresolved ${ENV} placeholder: environment
variable VPAY_STAFF_PEPPER is not set`. That also shows the
`rate_limits: sign_in: { attempts: 100, window_seconds: 300 }` shape is one the
loader accepts, which a reading of the YAML would not have.

**`docker compose config`** with `.e2e/staff-auth.env` moved aside stops at
`env file … not found`, which names what is missing.

## `NOTES.txt`

Rendered by copying the chart to a scratch directory with the file renamed (Helm's
`template` does not print `NOTES.txt`, and `install --dry-run` wants a cluster),
for three shapes. All four values set: the sign-in client line, the "configured
in two places" bullet, and the single sign-on bullet naming
`https://dash.example/api/auth/sso/callback`. Only `clientId` set under an
`http://localhost` origin: the INCOMPLETE line, and the note that only `localhost`
keeps a `__Host-` cookie over `http`. `dashboard.enabled` alone: the none-set line
and "single sign-on is not offered". The text is not asserted by `helm-check`.

## Not done

- **The Cypress spec has not run.** It is typechecked and linted, and its
  selectors, labels and the sequence were read from the components and the
  server actions; no browser has executed it. CI's `e2e (compose)` job is where it
  runs for the first time, and that run is the evidence this page does not have.
  Premises written down before they were observed: that a Cypress-driven Chromium
  stores a `Secure` `__Host-` cookie from `http://localhost:3000` (the spec's
  cookie assertions test it first); that the controlled inputs hydrate before
  Cypress types into them (an input typed before hydration is cleared, and the
  `retries` in `cypress.config.ts` are the only cover); that axe finds no WCAG A
  or AA violation on four screens in a real browser (jsdom's structural pass is
  the only prior evidence); that the bottom sheet's "Done" is clickable as soon as
  the secret is read.
- **No stack was brought up.** `docker compose up`, `just demo-staff` against a
  running stack, `just test-e2e`, and the effect of serving `human_login` on
  `just demo-walk` (read as no change, from where `/v1/oauth/token` is mounted)
  are all unrun. `demo-staff`'s refusal of an id where a password was expected is
  a `case` on the prefix `stf_` that nothing exercised.
- **The runbook's `psql` line** that fetches the member's id for `staff pair-key`
  (`docs/runbooks/demo/dashboard-sign-in.md`) was written from the schema's names
  and not run.
- **Single sign-on has no case.** The stack has no OpenID Connect provider and
  building one was out of scope, so the overlay writes no `staff_auth.sso` and the
  dashboard container has no `VPAY_DASHBOARD_SSO_ISSUER`. Named in `sso.md`.
- **The token-expiry case** the deleted spec had (a thirty-second TTL so a token
  is replaced before it expires) is not restored: it needs a `demo_staff_token_ttl`
  override that was deleted with the sign-in, and the replacement logic is covered
  by the dashboard's unit tests (`gate.test.ts`).
- **The `NOTES.txt` text and the chart's sign-in values are unapplied**: no
  cluster, like every other value in the chart.
- **A partial set of the three client values is not a chart error**, by decision:
  the app already names the variables it is missing, and a second copy of that
  rule in `_validate.tpl` would be a rule that exists twice.
- **`dashboard.extraEnv` has no reserved-name guard**, so an operator who moved
  these variables into the values and left them in `extraEnv` too gets the later
  entry. The chart README says so.
