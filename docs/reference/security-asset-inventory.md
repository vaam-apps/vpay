# Security Asset and Control-Evidence Inventory

_Issue [#150](https://github.com/vaam-apps/vpay/issues/150). The machine-readable
companion is [`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml);
the architectural decision is [ADR-0034](../adr/0034-security-asset-and-control-evidence-inventory.md);
the parent epic is [EPIC #155](https://github.com/vaam-apps/vpay/issues/155)._

## What this is

A code-grounded inventory of the technical assets, trust boundaries, credential
classes, and material security controls in vpay:

- deployable services and the worker mode of the same binary;
- the relational database, its outbox job queue, the idempotency store and the
  client-assertion replay cache;
- secret classes, classified by configuration path or CLI flag, never by value;
- inbound and outbound third-party integrations;
- authentication and authorization boundaries;
- CI/CD workflows, container images and published packages;
- material engineering controls mapped to ISO/IEC 27001:2022 Annex A, with
  repeatable code and test evidence sources.

## What this is not

It is **not** an Information Security Management System (ISMS), and its
existence does not certify vpay under ISO/IEC 27001:2022. Certification applies
to the organisation (VAAM) and its management system, not to an isolated code
repository.

The inventory provides durable engineering facts and repeatable evidence that
the organisation maps to its risk register, risk-treatment plans and Statement
of Applicability (SoA). Policies, risk ownership, HR vetting, business
continuity objectives and external vendor governance remain with the
organisation.

## How it is checked

`cargo xtask verify-security-inventory` (gate 16 of `just verify`) checks the
inventory against the tree:

- `metadata.adr` and `metadata.reference` exist;
- every declared trust tier is unique, and every service names one;
- every service path exists and, when a `crate` is named, that crate's
  `Cargo.toml` declares the package;
- every table named by a state store is created by a migration, **and** every
  table a migration creates is named by a state store;
- every implementation, runbook, ADR, threat-model and privacy-inventory path
  exists;
- every secret `config_keys` entry is one of the closed set of `vpay-config`
  paths the gate knows, so an invented path such as `rails.mtn_momo.api_key` is
  refused;
- every workflow file exists under `.github/workflows/`, every Dockerfile stage
  it names is declared, and every published package's manifest exists, names
  the package, and is not `private: true`;
- every control's implementation paths exist, every evidence path exists, and
  every cited test name is declared by a Rust source under `backends/`;
- every control's status is `implemented`, `partial` or `gap`; a `gap` names a
  `related_gap`;
- every gap names exactly one tracker, its status is known, and a tracked issue
  number is cited by at least one document under `docs/` (an offline liveness
  proxy; there is no GitHub API call in this gate).

It deliberately does **not** verify a GitHub issue's live state, a control's
semantic adequacy, or whether a cited test currently passes. Those are review
questions. `cargo xtask verify-citations` checks issue and pull-request
existence when the network is available.

---

## 1. Deployable Services and Interfaces

| Service           | Path / Crate                | Kind                          | Interfaces                                                                             | Trust tier |
| :---------------- | :-------------------------- | :---------------------------- | :------------------------------------------------------------------------------------- | :--------- |
| **`vpay-server`** | `backends/apps/vpay-server` | HTTP API                      | `/v1`, `/v1/oauth`, `/v1/browser`, `/dash/v1`, `/provider/{code}/callback`, `/healthz` | Tier 2     |
| **`vpay-worker`** | `backends/apps/vpay-server` | Same image, `worker` argument | Outbox consumer; `/livez` and `/metrics` on the observability port                     | Tier 2     |
| **`dashboard`**   | `frontends/apps/dashboard`  | Next.js 15 web app            | Port 3000; upstream `/dash/v1`                                                         | Tier 3     |
| **`checkout`**    | `frontends/apps/checkout`   | Next.js 15 web app            | Port 3000; upstream `/v1/browser`                                                      | Tier 3     |

`/healthz` is on the public server port by design. `/livez` and `/metrics` are
on a separate observability port. See
[docs/security/trust-boundaries.md](../security/trust-boundaries.md) for the
five boundary tiers.

---

## 2. State Stores and Queues

| State Store                 | Backing Technology                  | Authority / Schema          | Security and Invariants                                                                                                                                                                  |
| :-------------------------- | :---------------------------------- | :-------------------------- | :--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Primary Database**        | PostgreSQL 16+                      | `backends/migrations/`      | sqlx runtime queries with bind parameters; `AssertSqlSafe` audited in `vpay-db/src/sql_audit.rs`. Transport TLS and volume encryption are operator-owned assumptions, not enforced here. |
| **Transactional Job Queue** | Table `jobs`                        | `vpay-db::jobs`             | Transactional outbox; `SELECT FOR UPDATE SKIP LOCKED`; worker schedule bound to the database clock ([ADR-0026](../adr/0026-the-database-clock-schedules-jobs.md)).                       |
| **Idempotency Store**       | Table `idempotency_keys`            | `vpay-db::idempotency`      | Prevents duplicate intent creation or mutation on retries.                                                                                                                               |
| **Replay Cache**            | Table `oauth_client_assertion_jtis` | `vpay-db::client_assertion` | Records spent client-assertion `jti` values past validator leeway ([ADR-0028](../adr/0028-a-spent-jti-outlives-the-validators-leeway.md)).                                               |

Backups and restore drills are **not implemented**. The runbook and
[ADR-0013](../adr/0013-database-backups-and-retention.md) are proposed only;
issue [#153](https://github.com/vaam-apps/vpay/issues/153) tracks the gap.

---

## 3. Secret and Credential Inventory

No secret value is stored in source code. Credentials are `${VAR}` placeholders
in configuration ([ADR-0003](../adr/0003-yaml-configuration.md)), a CLI flag, or
an environment variable.

| Credential Class              | Configuration Path / Source                                                            | Rotation                                                                                              |
| :---------------------------- | :------------------------------------------------------------------------------------- | :---------------------------------------------------------------------------------------------------- |
| **MTN MoMo Collections**      | `providers.mtn_momo.credentials.{subscription_key, api_key}`                           | [rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md)                                  |
| **MTN MoMo Disbursements**    | `providers.mtn_momo.credentials.{disbursement_subscription_key, disbursement_api_key}` | [rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md) (no deployment holds a real set) |
| **Orange Money**              | `providers.orange_money.credentials.{client_id, client_secret, merchant_key}`          | [rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md)                                  |
| **Notch Pay**                 | `providers.notchpay.credentials.{public_key, private_key}`                             | [rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md)                                  |
| **Merchant webhook secrets**  | `merchant_clients[].webhooks[].secrets` (one per endpoint; a list allows rotation)     | No runbook yet                                                                                        |
| **Staff password pepper**     | `staff_auth.password_pepper`                                                           | No runbook yet                                                                                        |
| **Staff TOTP encryption key** | `staff_auth.totp_encryption_key`                                                       | No runbook yet                                                                                        |
| **Staff SSO client secret**   | `staff_auth.sso.client_secret`                                                         | No runbook yet                                                                                        |
| **OAuth signing key**         | `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`                             | [rotate-signing-key.md](../runbooks/rotate-signing-key.md)                                            |
| **Database credentials**      | `--database-url` / `DATABASE_URL`                                                      | Operator-owned                                                                                        |

`rotate-signing-key.md` covers the OAuth signing key only. It is not a webhook
secret rotation procedure.

---

## 4. Inbound and Outbound Integrations

| Integration                | Direction     | Transport                                 | Security Controls and Residual Risk                                                                                                                                                                                                                             |
| :------------------------- | :------------ | :---------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **MTN MoMo**               | Bidirectional | HTTPS (rustls, redirects refused)         | The callback route is unauthenticated by design and only advances a poll. MTN does not sign callbacks.                                                                                                                                                          |
| **Orange Money**           | Bidirectional | HTTPS (rustls, redirects refused)         | The callback route is unauthenticated by design and only advances a poll. Orange does not sign callbacks.                                                                                                                                                       |
| **Notch Pay**              | Bidirectional | HTTPS (rustls, redirects refused)         | Public/private key separation and `X-Grant` on outbound calls. `x-notch-signature` is verified as the hex HMAC-SHA256 of the raw body under the endpoint's Hash Key ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md)); a verified callback is still a hint. |
| **OIDC identity provider** | Outbound      | HTTPS (rustls)                            | Authorization-code flow with state, nonce and PKCE generated by vpay; ID-token signature and claim checks.                                                                                                                                                      |
| **Merchant webhooks**      | Outbound      | HTTPS; private targets refused by default | `vpay-signature` over timestamp and body; redirects refused; resolved addresses pass the SSRF guard in `vpay-worker/src/ssrf.rs`.                                                                                                                               |

A rail callback never moves money. The authenticated status read is the only
thing that does, and every rail callback is treated as a hint that pulls a poll
job's `run_at` forward ([docs/flows/crash-safety.md](../flows/crash-safety.md)).

---

## 5. CI/CD, Container Images, and Release Channels

- **Workflows:**
  - `.github/workflows/ci.yml` — `verify` (individual gates), `rust` (`fmt`,
    `clippy -D warnings`, `nextest`, doctests), `supply chain`
    (`cargo deny check`), `web` (prettier, typecheck, lint, vitest,
    `pnpm audit --audit-level=moderate`) and `e2e` (compose and Cypress).
  - `.github/workflows/quality.yml` — CodeQL, Super-linter and Trivy.
  - `.github/workflows/release-please.yml` — release pull requests and tags.
  - `.github/workflows/release.yml` — buildx image index with
    `provenance: mode=max` and `sbom: true`, cosign keyless signatures for the
    images and the Helm chart, and npm publishing for the two SDKs.
- **Container hardening:**
  - `backends/Dockerfile` produces a static musl binary in a `FROM scratch`
    image with no shell, running as `USER 65532:65532`.
  - `frontends/Dockerfile` builds standalone Next.js images for the dashboard
    (`runner`) and checkout (`checkout`) stages, each with numeric
    `USER 1000:1000` and `EXPOSE 3000`.
- **Published packages:**
  - `@vaam-apps/vpay-sdk` (`sdks/nodejs`)
  - `@vaam-apps/vpay-stripe-js` (`sdks/stripe-js`)

The Tauri plugin under `sdks/tauri` is **not published**, and neither is the
Rust SDK or the Flutter plugin.

---

## 6. ISO/IEC 27001:2022 Control Evidence Map

| Annex A Control                              | Theme          | Implementation                                                  | Repeatable Evidence                                                                 | Status                                  |
| :------------------------------------------- | :------------- | :-------------------------------------------------------------- | :---------------------------------------------------------------------------------- | :-------------------------------------- |
| **A.5.15 Access Control**                    | Organizational | `vpay-api` surface and scope checks; tenant-scoped repositories | OAuth2 `private_key_jwt` ([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)) | Implemented                             |
| **A.8.2 Privileged Access Rights**           | Technological  | Staff TOTP and OIDC sign-in; server-side sessions               | ID-token claim checks and replay guard                                              | Implemented                             |
| **A.8.7 Supply Chain Protection**            | Technological  | Lockfiles, `cargo-deny`, `pnpm audit`                           | `deny.toml`; one JS advisory risk-accepted (GAP-103)                                | Implemented                             |
| **A.8.8 Technical Vulnerability Management** | Technological  | Dependency auditing in CI and locally                           | `cargo deny check`; `pnpm audit --audit-level=moderate`                             | Implemented; SLA is GAP-151             |
| **A.8.9 Configuration Management**           | Technological  | Boot-time validation in `vpay-config`                           | Literal livemode secrets and unknown keys refused                                   | Implemented                             |
| **A.8.11 Data Masking**                      | Technological  | Redacting `Debug` types; privacy inventory                      | `cargo xtask verify-privacy-inventory`, both directions                             | Implemented                             |
| **A.8.12 Data Leakage Prevention**           | Technological  | Redirect refusal; webhook SSRF guard                            | Conformance test `redirects_are_refused_and_never_followed`                         | Implemented                             |
| **A.8.13 Information Backup**                | Technological  | Runbook and ADR only                                            | No backup or restore has ever run                                                   | **Gap #153**                            |
| **A.8.15 Logging**                           | Technological  | Application logging only                                        | No centralized security-event log or SIEM schema                                    | **Gap #154**                            |
| **A.8.16 Monitoring**                        | Technological  | `prometheusrule.yaml`, `servicemonitor.yaml`                    | Alert rules unit-tested against synthetic series                                    | Implemented (never fired in production) |
| **A.8.20 Network Security**                  | Technological  | rustls outbound; ingress terminates TLS                         | NetworkPolicy is opt-in, default off, never applied                                 | Partial                                 |
| **A.8.24 Use of Cryptography**               | Technological  | Argon2id, AES-256-GCM, HMAC-SHA256, RSA signing                 | `staff_auth` and `signing` unit tests                                               | Partial; key lifecycle is GAP-152       |
| **A.8.25 Secure Development Life Cycle**     | Technological  | `just verify` and CI gates                                      | Documented gates, no written CVE SLA                                                | **Gap #151**                            |
| **A.8.28 Secure Coding**                     | Technological  | Clippy and rustfmt; bind parameters; `AssertSqlSafe` audit      | `clippy.toml`, `Cargo.toml`, `sql_audit.rs`                                         | Implemented                             |
| **A.8.31 Environment Separation**            | Technological  | Profile-selected config; livemode refusals                      | Test/live key mismatch refused at boot                                              | Implemented                             |

---

## 7. Known Gaps and Residual Risks

1. **GAP-103: Web dependency audit allowlist** ([#103](https://github.com/vaam-apps/vpay/issues/103)) — `pnpm audit` runs at `--audit-level=moderate`; GHSA-vfj7-8cjw-p6xm is ignored by id under the maintainer decision of 2026-10-09, and the dated allowlist-with-reason escape hatch is not built.
2. **GAP-56: Account-holder lookup audit and rate limiting** ([#56](https://github.com/vaam-apps/vpay/issues/56)) — A dedicated scope and durable per-key audit log are not built.
3. **GAP-151: Secure SDLC and vulnerability management evidence** ([#151](https://github.com/vaam-apps/vpay/issues/151)) — No written CVE intake and fix-release SLA.
4. **GAP-152: Production key lifecycle hardening** ([#152](https://github.com/vaam-apps/vpay/issues/152)) — Automated rotation and dual control for rail and signing keys are not built.
5. **GAP-153: Backup and disaster-recovery drills** ([#153](https://github.com/vaam-apps/vpay/issues/153)) — No backup exists and no restore drill has run.
6. **GAP-154: Centralized security logging** ([#154](https://github.com/vaam-apps/vpay/issues/154)) — No standardized audit event schema for SIEM ingestion.
7. **GAP-CALLBACK-VERIFICATION: MTN and Orange callbacks are not verified** — Notch Pay's `x-notch-signature` is verified ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md), PR #357), but MTN does not sign callbacks, Orange's `BodyToken` check is not built, and no per-account callback secret exists (RFC-0009). Callbacks remain hints for every rail; only the authenticated status read moves money.
