# Security Asset and Control-Evidence Inventory

_Issue [#150](https://github.com/vaam-apps/vpay/issues/150). The machine-readable
companion is [`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml);
the architectural decision is [ADR-0033](../adr/0033-security-asset-and-control-evidence-inventory.md);
the parent epic is [EPIC #155](https://github.com/vaam-apps/vpay/issues/155)._

## What this is

A code-grounded inventory of every technical asset, trust boundary, credential
class, and material security control in vpay:
- deployable services and background daemons;
- relational databases, persistent job queues, and state stores;
- secrets, signing keys, and external credentials (classified by role and location,
  never recording secret values);
- inbound and outbound third-party integrations;
- authentication and authorization boundaries;
- CI/CD workflows, artifact registries, and release paths;
- material engineering controls mapped to ISO/IEC 27001:2022 Annex A themes with
  repeatable code and test evidence sources.

## What this is not

It is **not** an Information Security Management System (ISMS), nor does its
existence certify vpay under ISO/IEC 27001:2022. ISO/IEC 27001 certification
applies to the organisation (VAAM) and its management system, not to an isolated
code repository.

This inventory provides durable engineering facts and repeatable evidence that
an external ISMS maps to its risk register, risk-treatment plans, and Statement of
Applicability (SoA). Policies, risk ownership, HR vetting, business continuity
objectives, and external vendor governance remain with the organisation.

## How it is checked

`cargo xtask verify-security-inventory` verifies the integrity of
[`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml) against
the real tree:
- every deployable service and crate exists in the workspace;
- every registered database table exists in the migrations set;
- every cited configuration secret maps to a declared field in `vpay-config`;
- every cited test, runbook, workflow, or fixture exists on disk;
- every declared gap links to a live GitHub issue rather than masking unbuilt work.

---

## 1. Deployable Services and Interfaces

| Service | Path / Crate | Kind | Interfaces / Ports | Auth Mechanism |
| :--- | :--- | :--- | :--- | :--- |
| **`vpay-server`** | `backends/apps/vpay-server` | HTTP API service | `/v1` (Merchant)<br>`/dash/v1` (Staff)<br>`/provider/*` (Callbacks)<br>`/healthz`, `/metrics` | `private_key_jwt` / Bearer<br>Staff session cookie<br>HMAC / Callback verification<br>Cluster-internal |
| **`vpay-worker`** | `backends/apps/vpay-server` | Daemon (`vpay-server worker`) | None (Internal outbox consumer) | Database connection auth |
| **`dashboard`** | `frontends/apps/dashboard` | Next.js 15 web app | Client-facing HTTP port 3000 | Staff credentials + TOTP / OIDC SSO |
| **`checkout`** | `frontends/apps/checkout` | Next.js 15 web app | Payer-facing HTTP port 3001 | Ephemeral `client_secret` token |

See [docs/security/trust-boundaries.md](../security/trust-boundaries.md) for network
topology and boundary tier definitions.

---

## 2. State Stores and Queues

| State Store | Backing Technology | Authority / Schema | Security & Invariants |
| :--- | :--- | :--- | :--- |
| **Primary Database** | PostgreSQL 16+ | `backends/migrations/` | Volume encryption at rest; TLS in transit; sqlx compile-time query verification; strict tenant isolation. |
| **Transactional Job Queue** | Table `jobs` | `vpay-db::jobs` | Transactional outbox pattern; `SELECT FOR UPDATE SKIP LOCKED`; worker schedule bound to DB clock ([ADR-0026](../adr/0026-the-database-clock-schedules-jobs.md)). |
| **Idempotency Store** | Table `idempotency_keys` | `vpay-db::idempotency` | Prevents duplicate charge submission or intent mutation on retries. |
| **Replay Cache** | Table `oauth_client_assertion_jtis` | `vpay-db::client_assertion` | Records spent client assertion `jti` identifiers past validator leeway ([ADR-0028](../adr/0028-a-spent-jti-outlives-the-validators-leeway.md)). |

Database backup procedures and restore requirements are documented in
[docs/runbooks/restore-from-backup.md](../runbooks/restore-from-backup.md) and
[docs/adr/0013-database-backups-and-retention.md](../adr/0013-database-backups-and-retention.md).

---

## 3. Secret and Credential Inventory

No secret values are stored in source code. Credentials are fed via configuration
management ([ADR-0003](../adr/0003-yaml-configuration.md)) or environment variables.

| Credential Class | Configuration Property | Storage & Scope | Rotation Runbook |
| :--- | :--- | :--- | :--- |
| **MTN MoMo Credentials** | `rails.mtn_momo.*` | Environment / secret volume | [docs/runbooks/rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md) |
| **Orange Money Credentials** | `rails.orange_money.*` | Environment / secret volume | [docs/runbooks/rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md) |
| **Notch Pay Keys** | `rails.notchpay.*` | Environment / secret volume | [docs/runbooks/rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md) |
| **Webhook Signing Key** | `webhooks.signing_secret` | HMAC-SHA256 secret per deployment | [docs/runbooks/rotate-signing-key.md](../runbooks/rotate-signing-key.md) |
| **Staff Password Pepper** | `staff.password_pepper` | Server-side secret pepper for Argon2id | Environment / deployment secret |
| **Staff TOTP Key** | `staff.totp_encryption_key` | AES-256-GCM symmetric key | Environment / deployment secret |
| **Staff SSO Client Secret** | `staff_sso.client_secret` | OIDC client credential for IdP | [ADR-0030](../adr/0030-staff-sign-in-federates-to-an-oidc-provider.md) |
| **Database Credentials** | `database.url` | Connection URI | Secret volume |

---

## 4. Inbound and Outbound Integrations

| Integration | Direction | Protocol | Security Controls | Threat Model / Flow Reference |
| :--- | :--- | :--- | :--- | :--- |
| **MTN MoMo Gateway** | Bidirectional | HTTPS (rustls) | Inbound IP/callback verification; outbound token refresh | [docs/security/data-flows.md](../security/data-flows.md) |
| **Orange Money Gateway** | Bidirectional | HTTPS (rustls) | Inbound transaction notification verification | [docs/security/data-flows.md](../security/data-flows.md) |
| **Notch Pay Gateway** | Bidirectional | HTTPS (rustls) | Public/Private key separation; callback confirmation poll | [docs/security/threat-models/notchpay.md](../security/threat-models/notchpay.md) |
| **OIDC IdP (Keycloak)** | Outbound | HTTPS (rustls) | PKCE authorization code exchange; JWKS validation | [ADR-0030](../adr/0030-staff-sign-in-federates-to-an-oidc-provider.md) |
| **Merchant Webhooks** | Outbound | HTTPS (rustls) | `vpay-signature` header with HMAC-SHA256 and timestamp | [docs/security/data-flows.md](../security/data-flows.md) |

---

## 5. CI/CD, Container Images, and Release Channels

- **Workflows:**
  - `.github/workflows/ci.yml` — Runs compilation, unit tests, integration suites, and `cargo xtask verify-all`.
  - `.github/workflows/quality.yml` — Runs dependency review, license compliance, and `cargo-deny` security audit.
  - `.github/workflows/release-please.yml` — Automated semantic versioning and release notes.
- **Container Hardening:**
  - `backends/Dockerfile` produces a static musl binary on an unprivileged alpine image (`USER 10001:10001`, `runAsNonRoot: true`), with no shell in production.
  - `frontends/Dockerfile` builds standalone Next.js images for dashboard (`runner` stage) and checkout (`checkout` stage) declaring numeric unprivileged `USER 1000:1000` directives.
- **SDK Packaging:**
  - `@vaam-apps/vpay-sdk` (Node.js)
  - `@vaam-apps/vpay-stripe-js` (Browser)
  - `@vaam-apps/vpay-tauri-checkout` (Tauri v2 desktop plugin)

---

## 6. ISO/IEC 27001:2022 Control Evidence Map

| ISO 27001:2022 Control | Theme | Implemented In | Repeatable Evidence Source | Status |
| :--- | :--- | :--- | :--- | :--- |
| **A.5.15 Access Control** | Organizational | `vpay-api::v1` | OAuth2 `private_key_jwt` token checks; repository tenant scoping | **Implemented** |
| **A.8.2 Privileged Access Rights** | Technological | `vpay-api::staff` | TOTP MFA; OIDC PKCE federated sign-in; ADR-0030 | **Implemented** |
| **A.8.7 Protection Against Malware** | Technological | `Cargo.lock`, `pnpm-lock.yaml` | `cargo deny check`; pnpm isolated nodeLinker | **Implemented** |
| **A.8.9 Configuration Management** | Technological | `vpay-config` | Boot-time schema validation; rejection of test keys in livemode | **Implemented** |
| **A.8.11 Data Masking** | Technological | `vpay-core::privacy` | `cargo xtask verify-privacy-inventory` (36 redacting `Debug` types) | **Implemented** |
| **A.8.12 Data Leakage Prevention** | Technological | `vpay-provider` | Conformance suite: redirects refused and never followed | **Implemented** |
| **A.8.13 Information Backup** | Technological | `backends/migrations` | Migration checksum gate; `docs/runbooks/restore-from-backup.md` | **Implemented** |
| **A.8.20 Network Security** | Technological | `backends/crates/vpay-api` | `docs/adr/0005-rustls-only.md` (rustls with vendored webpki roots) | **Implemented** |
| **A.8.24 Use of Cryptography** | Technological | `vpay-core` | Argon2id for passwords; AES-GCM for tokens; HMAC-SHA256 for webhooks | **Implemented** |
| **A.8.25 Secure SDLC** | Technological | `justfile` | 15 automated pre-commit gates in `just verify` | **Implemented** |
| **A.8.28 Secure Coding** | Technological | `clippy.toml`, `Cargo.toml` | Clippy indexing deny; sqlx compile-time query verification | **Implemented** |
| **A.8.31 Env Separation** | Technological | `vpay-config` | Strict separation of testmode and livemode configurations | **Implemented** |

---

## 7. Known Gaps and Residual Risks

These items represent planned security engineering improvements tracked in open issues:

1. **GAP-103: Web dependency audit ceiling** ([#103](https://github.com/vaam-apps/vpay/issues/103)) — Resolving or formally risk-accepting transitive fast-glob/brace-expansion advisory in web workspace.
2. **GAP-56: Account-holder lookup audit and rate limiting** ([#56](https://github.com/vaam-apps/vpay/issues/56)) — Adding dedicated scope and durable audit log for account-holder lookups.
3. **GAP-151: Secure SDLC and vulnerability management evidence** ([#151](https://github.com/vaam-apps/vpay/issues/151)) — Formalising documented vulnerability response SLAs and dependency update policies.
4. **GAP-152: Production secrets and key lifecycle hardening** ([#152](https://github.com/vaam-apps/vpay/issues/152)) — Automated rotation and dual-control procedures for production rail credentials.
5. **GAP-153: Backup, restore, and disaster recovery drills** ([#153](https://github.com/vaam-apps/vpay/issues/153)) — Establishing repeatable automated recovery test drills against defined RTO/RPO objectives.
6. **GAP-154: Centralized security logging and incident evidence** ([#154](https://github.com/vaam-apps/vpay/issues/154)) — Standardizing audit event schemas for SIEM ingestion.
