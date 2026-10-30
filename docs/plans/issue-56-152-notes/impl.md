# Implementation Record: Issues #56 and #152

> **Branch:** `feat/privileged-access-and-lookup-controls`  
> **Base:** `origin/master` (`ee200d43`)  
> **Status:** Work in Progress  
> **Review record:** [review.md](review.md)

---

## 1. Provisional Decisions (RFC-0002)

Per `task.md` Section 3, implementation proceeds without waiting for pending maintainer decisions. Every item below is marked with `Provisional pending RFC-0002 D<n>` in code and comments. RFC-0002 remains untouched (`git diff origin/master -- docs/rfc/0002-gdpr-policy-and-operator-decisions.md` is empty).

| Decision                     | Provisional Answer Implemented                                                                                                                                                                                                                                             | Change Point                                                                                                 |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| **D3 Target Representation** | HMAC-SHA256 over `tenant_id` + normalized MSISDN using a dedicated deployment secret (`LookupAuditSalt` / secret class `lookup_audit_salt`). Raw MSISDN and returned names are never stored.                                                                               | `vpay_api::v1::account_holders::audit_target_hash`, `vpay_config::Config::lookup_audit_salt`                 |
| **D5 Scope & Rollout**       | Dedicated scope `account_holders:read` required for `GET /v1/account_holders` and for `POST /v1/refunds` with a nominated payee. Implied by no payment scope. Enforced on release.                                                                                         | `vpay_api::v1::SCOPE_ACCOUNT_HOLDERS_READ`, `vpay_api::envelope::scope`, `vpay_api::v1::refunds`             |
| **D6 Per-Key Limiter**       | Pre-image: `SHA-256("lookup:merchant_key:<merchant_id>:<key_id>")` where key id is key thumbprint (signed COSE) or `client_id` (bearer token). Counted before validation/rail query. 429 response with `Retry-After: <seconds_remaining>`. Fail-closed on counter failure. | `vpay_api::envelope::CallerCredential`, `vpay_api::v1::account_holders::ask_rail`, `vpay_config::RateLimits` |
| **D7 Audit Semantics**       | One `initiated` row before rail call, one terminal row after rail call (`found`, `not_found`, `invalid`, `unsupported`, `rate_limited`, `rail_error`), linked by `operation_id`. Refuse lookup if initial audit write fails.                                               | `vpay_db::audit_events`, migration `0059_audit-events-and-lookup-rate-limits.sql`                            |
| **D8 Operator Surface**      | Bounded tenant-scoped read queries implemented on private `vpay-db` repository trait for audit queries. Dedicated HTTP/CLI operator surface deferred pending maintainer D8 decision.                                                                                       | `vpay_db::AuditEventRepository::list_by_tenant`                                                              |
| **D9 Retention Class**       | Classified under existing `operational` retention class in `schemas/privacy-inventory.yaml` with explicit comment indicating provisional status pending D9.                                                                                                                | `schemas/privacy-inventory.yaml`                                                                             |

---

## 2. Baseline Verification (`origin/master` at `ee200d43`)

- **Branch creation:** `git switch -c feat/privileged-access-and-lookup-controls origin/master`
- **fmt-check:** `just fmt-check` -> `cargo fmt --all -- --check` & `pnpm exec prettier --check .` (passed, exit 0)
- **verify:** 17 gates (`verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity`, `verify-links`, `verify-npm-scope`, `check-schema` via cratestack 0.16.0, `verify-serde`, `verify-repositories`, `verify-toolchain`, `verify-migrations`, `verify-versions`, `verify-privacy-inventory`, `verify-doc-counts`, `verify-security-inventory`, `verify-mcp-parity`, `verify-docs`) passed, exit 0.
- **clippy:** `just clippy` passed, exit 0 (0 warnings across workspace).
- **cargo test:** `cargo test -p vpay-api --lib v1::tests` passed (12 passed, 0 failed).

---

## 3. Acceptance Criteria Traceability

### Ticket #56

| ID       | Criterion                                     | Evidence Path / Proving Test                                                                                     | Status  |
| -------- | --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------- |
| **56-1** | Per-key fixed-window rate limit on lookups    | `backends/tests/integration/tests/account_holders.rs::per_key_rate_limit_blocks_exceeded_attempts`               | Planned |
| **56-2** | Refusal is `429` with `Retry-After` header    | `backends/tests/integration/tests/account_holders.rs::rate_limited_response_carries_retry_after`                 | Planned |
| **56-3** | Limit covers every lookup (including refunds) | `backends/tests/integration/tests/refunds.rs::nominated_payee_spends_lookup_rate_limit`                          | Planned |
| **56-4** | Append-only audit row per lookup              | `backends/crates/vpay-db/tests/repositories.rs::audit_events_append_and_query_by_tenant`                         | Planned |
| **56-5** | Audit holds no raw MSISDN / no returned name  | `backends/tests/conformance/tests/privacy.rs::audit_log_contains_no_raw_phone_numbers_or_names`                  | Planned |
| **56-6** | Audit queryable by operator (repo layer)      | `vpay_db::AuditEventRepository::list_by_tenant`                                                                  | Planned |
| **56-7** | Dedicated scope for lookups                   | `backends/tests/integration/tests/account_holders.rs::lookup_refuses_payments_read_without_account_holders_read` | Planned |
| **56-8** | Scope is route-declared, not method-inferred  | `backends/crates/vpay-api/src/v1/mod.rs::tests::every_mounted_v1_route_has_a_declared_scope_policy`              | Done    |

### Ticket #152

| ID        | Criterion                                      | Evidence Path / Proving Test                                                    | Status  |
| --------- | ---------------------------------------------- | ------------------------------------------------------------------------------- | ------- |
| **152-1** | Role/scope model across privileged interfaces  | `docs/security/privileged-access-matrix.md`, integration tests                  | Planned |
| **152-2** | Prohibition of shared admin credentials        | `docs/runbooks/deploy-and-rollback.md`, `console_keys.rs`                       | Planned |
| **152-3** | Secrets & private keys never logged/in source  | `backends/crates/vpay-config/tests/debug_redaction.rs`                          | Planned |
| **152-4** | Rotation/revocation runbooks & tests           | `docs/runbooks/rotate-signing-key.md`, `rotate-rail-credentials.md`             | Planned |
| **152-5** | Access review evidence repeatable export       | `docs/runbooks/access-review.md`                                                | Planned |
| **152-6** | Time-bounded, auditable break-glass access     | `docs/runbooks/deploy-and-rollback.md`                                          | Planned |
| **152-7** | #56 linked / implemented                       | Section 1 & 3 of this document                                                  | Planned |
| **152-8** | Negative security tests (privilege escalation) | `backends/tests/conformance/tests/access_control.rs`                            | Planned |
| **152-9** | Inventory reconciliation                       | `schemas/security-inventory.yaml`, `docs/reference/security-asset-inventory.md` | Planned |

---

## 4. Mutation Testing Record

| Mutation                                                  | Expected Failing Test | Result |
| --------------------------------------------------------- | --------------------- | ------ |
| One `/v1` route's declared scope policy removed (set to ` | _                     | &[]`)  | `v1::tests::every_mounted_v1_route_has_a_declared_scope_policy` | PASS (panicked at `route /payment_intents POST has an empty declared scope policy`) |
