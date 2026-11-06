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

| ID       | Criterion                                     | Evidence Path / Proving Test                                                                                                          | Status |
| -------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| **56-1** | Per-key fixed-window rate limit on lookups    | `backends/tests/integration/tests/account_holders.rs::rate_limit_blocks_and_retry_after_is_provided_and_second_credential_unaffected` | Done   |
| **56-2** | Refusal is `429` with `Retry-After` header    | `backends/tests/integration/tests/account_holders.rs::rate_limit_blocks_and_retry_after_is_provided_and_second_credential_unaffected` | Done   |
| **56-3** | Limit covers every lookup (including refunds) | `backends/tests/integration/tests/refunds.rs::a_nominated_payee_refund_spends_the_lookup_rate_limit_budget`                           | Done   |
| **56-4** | Append-only audit row per lookup              | `backends/crates/vpay-db/tests/repositories.rs::audit_events_append_and_query_by_tenant`                                              | Done   |
| **56-5** | Audit holds no raw MSISDN / no returned name  | `backends/tests/integration/tests/account_holders.rs::audit_table_holds_hmac_target_representation_and_no_raw_pii`                    | Done   |
| **56-6** | Audit queryable by operator (repo layer)      | `vpay_db::AuditEvents::list_audit_events_by_tenant`                                                                                   | Done   |
| **56-7** | Dedicated scope for lookups                   | `backends/tests/integration/tests/account_holders.rs::credential_without_account_holders_read_scope_is_refused_403`                   | Done   |
| **56-8** | Scope is route-declared, not method-inferred  | `backends/crates/vpay-api/src/v1/mod.rs::tests::every_mounted_v1_route_has_a_declared_scope_policy`                                   | Done   |

### Ticket #152

| ID        | Criterion                                      | Evidence Path / Proving Test                                                                                                                    | Status |
| --------- | ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| **152-1** | Role/scope model across privileged interfaces  | `docs/security/privileged-access-matrix.md`, `backends/tests/integration/tests/signed_transport.rs::a_console_or_service_key_cannot_reach_v1`   | Done   |
| **152-2** | Prohibition of shared admin credentials        | `docs/runbooks/deploy-and-rollback.md`, `backends/crates/vpay-api/src/console_keys.rs::pairing_never_creates_or_upgrades_an_administrator`      | Done   |
| **152-3** | Secrets & private keys never logged/in source  | `backends/crates/vpay-api/src/v1/boot.rs::boot_logging_never_leaks_configured_secrets`, CI Trivy scan                                           | Done   |
| **152-4** | Rotation/revocation runbooks & tests           | `docs/runbooks/rotate-signing-key.md`, `backends/tests/integration/tests/signed_transport.rs::a_console_key_revocation_denies_subsequent_calls` | Done   |
| **152-5** | Access review evidence repeatable export       | `docs/runbooks/access-review.md` (SQL exports for staff, paired console keys, web sessions)                                                     | Done   |
| **152-6** | Time-bounded, auditable break-glass access     | `docs/runbooks/deploy-and-rollback.md` (break-glass statement & auditability)                                                                   | Done   |
| **152-7** | #56 linked / implemented                       | Implemented in PR; GAP-56 closed in `schemas/security-inventory.yaml`; operator surface noted pending D8                                        | Done   |
| **152-8** | Negative security tests (privilege escalation) | `backends/tests/integration/tests/signed_transport.rs::a_console_or_service_key_cannot_reach_v1`                                                | Done   |
| **152-9** | Inventory reconciliation                       | `schemas/security-inventory.yaml` (A.5.15, A.8.2, A.8.24 updated), `docs/reference/security-asset-inventory.md`                                 | Done   |

---

## 4. Mutation Testing Record

| Mutation                                                             | Expected Failing Test                                                               | Result                                                                                                    |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| One `/v1` route's declared scope policy removed (set to `\|_\| &[]`) | `v1::tests::every_mounted_v1_route_has_a_declared_scope_policy`                     | PASS (panicked at `route /payment_intents POST has an empty declared scope policy`)                       |
| `CallerCredential` default-constructed when missing                  | `envelope::scope::tests::caller_credential_extraction_refuses_when_absent_with_500` | PASS (panicked at `missing credential must fail: CallerCredential { kind: MachineToken, id: "default" }`) |
| Unclassified outcome written to `audit_events`                       | `vpay-db/tests/repositories.rs::audit_events_append_and_query_by_tenant`            | PASS (refused with `DbError::Query` pointing to constraint `audit_events_outcome_is_known`)               |
| Rate limit counter budget condition changed to strict inequality     | `vpay-api::v1::account_holders::tests::over_budget_refuses_with_429`                | PASS (failed on boundary count equal to limit)                                                            |
| Lookup route policy reverted to `payments:read`                      | `account_holders::credential_without_account_holders_read_scope_is_refused_403`     | PASS (received 200 instead of 403 Forbidden)                                                              |
| Refund nominated payee path skips scope requirement                  | `refunds::a_nominated_payee_refund_without_account_holders_read_is_refused_403`     | PASS (received 200 instead of 403 Forbidden)                                                              |
| Redacting `Debug` removed from `LookupAuditSalt`                     | `vpay-api::v1::boot::tests::boot_logging_never_leaks_configured_secrets`            | PASS (panicked asserting raw salt was found in captured tracing logs)                                     |
| Limiter called only in route instead of `ask_rail`                   | `refunds::a_nominated_payee_refund_spends_the_lookup_rate_limit_budget`             | PASS (refund lookup did not increment rate limit counter)                                                 |
