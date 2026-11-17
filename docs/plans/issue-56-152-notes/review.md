# Review Record: Issues #56 and #152

> **Branch:** `feat/privileged-access-and-lookup-controls`  
> **Base:** `origin/master` (`ee200d43`)  
> **Implementation Record:** [impl.md](impl.md)  
> **Status:** Cleared (Approved, 0 P0/P1/P2/P3 findings)

---

## Review Protocol

Per `task.md` Section 7:

- **Reviewer Persona:** Senior Staff Software Engineer (`~/.agents/skills/code-reviewer/SKILL.md`) enforcing repository standards (`vpay`, `vpay-conventions`, `vpay-data-layer`, `vpay-merchant-api`, `vpay-docs-status`, and `AGENTS.md`).
- **Model:** Gemini 4 Argon (`Model: inherit`).
- **Execution:** Fresh reviewer context per iteration.
- **Exit Condition:** Zero P0, P1, and P2 findings and all verification gates green.

---

## Iteration 1

- **Commit Reviewed:** `553a9882`
- **Verdict:** Request Changes

### Findings & Resolutions

1. **[P1] Prettier Formatting Failure in `impl.md` Breaks `just fmt-check`**
   - _Issue:_ Markdown table updates in `impl.md` caused `fmt-check-web` to fail.
   - _Fix:_ Ran Prettier and `just fmt` across all docs and files; verified `just fmt-check` exits 0.

2. **[P1] Integration Test 56-1 Does Not Test Second Credential of the Same Merchant**
   - _Issue:_ `account_holders.rs` tested `CLIENT_B` (tenant `MERCHANT_B`), testing tenancy isolation rather than proving that rate limiting is per-credential within the same tenant.
   - _Fix:_ Registered a secondary COSE key for `CLIENT_A` (tenant `MERCHANT_A`). After `CLIENT_A` hits 429 on its primary key, the request signed with the secondary key succeeds with 200 OK, verifying that rate-limiting buckets do not collapse across the merchant.

3. **[P1] Missing Fail-Closed Enforcement and Inconsistent Livemode Validation for `lookup_audit_salt`**
   - _Issue:_ Unconfigured salt produced unkeyed HMAC digests (`b""`), violating RFC-0002 D3 and fail-closed security. Livemode validation allowed missing salt.
   - _Fix:_ Added `ConfigError::MissingPrivacyLookupAuditSalt` in `vpay-config` to enforce non-empty salt in livemode. Added fail-closed check in `ask_rail` returning `ApiError::Internal("privacy controls not configured")` if salt is empty.

4. **[P2] Acceptance Criterion 56-4 Postgres Test Missing on Refund Path**
   - _Issue:_ Criterion 56-4 requires Postgres assertions on both paths; refunds only asserted audit events in an in-memory unit test.
   - _Fix:_ In `backends/tests/integration/tests/refunds.rs`, added queries against `harness.pool` verifying `initiated`, terminal `not_found`, and terminal `rate_limited` records in `audit_events` with the expected target hash and proving zero raw MSISDN leaks.

5. **[P3] Terminal Audit Event Write Error Swallowed Without Logging**
   - _Issue:_ Terminal audit event writes in `ask_rail` ignored errors via `let _ = ...`.
   - _Fix:_ Added `tracing::error!(%error, "failed to append terminal audit event for account-holder lookup")` for terminal and error audit writes.

---

## Iteration 2

- **Commit Reviewed:** `5475479d`
- **Verdict:** Approved (0 P0, P1, P2, P3 findings)

### Verification Summary

| Gate / Suite                                | Result   | Details                                                                                                                              |
| ------------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `just verify`                               | **PASS** | 17/17 gates passed (check-schema, verify-migrations, verify-repositories, verify-privacy-inventory, verify-security-inventory, etc.) |
| `cargo clippy --all-targets -- -D warnings` | **PASS** | 0 warnings                                                                                                                           |
| `just fmt-check`                            | **PASS** | 0 formatting diffs across Rust and Prettier                                                                                          |
| `just test-doc`                             | **PASS** | All documentation tests passed                                                                                                       |
| `just verify-ignored`                       | **PASS** | 0 ignored tests (2743 total across 58 test binaries)                                                                                 |
| `account_holders` integration suite         | **PASS** | 9/9 passed in 50.19s                                                                                                                 |
| `refunds` nominated payee suite             | **PASS** | 2/2 passed in 6.51s                                                                                                                  |
| Invariant: RFC-0002 diff                    | **PASS** | `git diff origin/master -- docs/rfc/0002-gdpr-policy-and-operator-decisions.md` is strictly empty                                    |
| Invariant: `task.md` uncommitted            | **PASS** | `task.md` confirmed absent from repository history                                                                                   |
| Caller Completeness                         | **PASS** | `ask_rail` in `account_holders.rs` and `refunds.rs` confirmed as only production callers; MCP cannot bypass                          |
