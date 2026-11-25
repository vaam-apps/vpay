# Issue #290 Phase A1: `legal_entity` config and boot rules

_Written 2026-11-20 on `feat/290-a1-legal-entity`, branched from `master`
(`81cf2467`). Implements RFC-0009 §§ 4, 6 (Delivery step A1)._

## What changed

- Added `LegalEntity { id, name }` and `Deployment::legal_entity: Option<LegalEntity>`
  in `backends/crates/vpay-config/src/lib.rs`.
- Added `MerchantClient::legal_entity: Option<String>` in
  `backends/crates/vpay-config/src/oauth.rs` (included in `Debug`).
- Added `ConfigError::MerchantsWithoutLegalEntity` and
  `ConfigError::MerchantOfAnotherLegalEntity` and enforced the three boot rules in
  `Config::validate_all` (`backends/crates/vpay-config/src/config.rs`):
  1. Two or more distinct `merchant_id`s without `deployment.legal_entity` are
     refused, naming the configured `merchant_id`s.
  2. Any merchant whose `legal_entity` is missing (when 2+ merchants) or differs
     from `deployment.legal_entity.id` is refused, naming the merchant.
  3. One-merchant deployments keep both fields optional and boot unchanged
     (explicit mismatches are still refused).
- Updated `justfile` (`gen-demo-keys`) and the four existing two-merchant fixtures
  so each remains a single-defect fixture.
- Documented `deployment.legal_entity` and `merchant_clients[].legal_entity` in
  `docs/reference/vpay-config.md`.

## Verification

- `cargo test -p vpay-config`: **211** unit/fixture tests passed, 7 doctests
  passed (208 at the reviewed commit `03c5a70d`; +3 from the review fixes
  below). Fixtures and the rule each covers:
  - `legal-entity-missing-two-merchants.yml` -> `MerchantsWithoutLegalEntity`
    (rule 1).
  - `legal-entity-two-merchants-one-company-without-deployment.yml` ->
    `MerchantsWithoutLegalEntity`: both merchants claim `acme-sarl` and
    `deployment` declares none, so there is no fallback to a merchant's own
    entity.
  - `legal-entity-merchant-mismatch.yml` -> `MerchantOfAnotherLegalEntity`
    (rule 2).
  - `legal-entity-two-projects-one-company.yml` -> boots.
- Rule 2 is an **exact** match:
  `a_legal_entity_is_matched_exactly_not_case_insensitively_or_by_prefix`
  refuses `ACME-SARL` and `acme` for `acme-sarl`.
- One merchant claiming an entity while `deployment` declares none is refused:
  `one_merchant_claiming_an_entity_without_a_deployment_entity_is_rejected`.
  This is deliberately **stricter** than RFC-0009 § 6 rule 3; the refusal
  renders the absent deployment entity as `<none>` rather than claiming a
  mismatch.
- Mutation check (RFC-0009 § 5): temporarily disabling the rule 2 comparison in
  `Config::validate_all` caused `merchant_of_another_legal_entity_is_rejected`
  to fail (`left: None`, `right: Some(MerchantOfAnotherLegalEntity { .. })`);
  restored immediately after.

## Review findings addressed (athena, 2026-11-21)

| Finding                                                                                           | Fix                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Major — no test for a fallback to a default entity (mutant M12)                                   | new fixture `legal-entity-two-merchants-one-company-without-deployment.yml` + `two_merchants_claiming_one_company_without_deployment_legal_entity_are_rejected` |
| Major (AGENTS rule 2) — stale docs                                                                | `docs/status/backend.md` and `docs/rfc/0009-provider-accounts-per-merchant.md` now say A1 is built and A2 is open                                               |
| Minor — exact match untested (M10, M11)                                                           | `a_legal_entity_is_matched_exactly_not_case_insensitively_or_by_prefix`                                                                                         |
| Minor — single-merchant case stricter than RFC-0009 § 6 rule 3 and message said "another company" | `one_merchant_claiming_an_entity_without_a_deployment_entity_is_rejected`; the message renders `<none>` and no longer claims a mismatch                         |
| Minor — `docs/status/README.md` arithmetic and the missing page                                   | the `2026-11-20-legal-entity-boot` page is listed; the counts are 146 files / 121 named / 25 omitted                                                            |
| Minor — `deployment.legal_entity.name` read by nothing                                            | logged, with the id, on the "configuration loaded and validated" line (`vpay-api::v1::boot`)                                                                    |
| Minor — refusal printed `Some("beta-sarl")`                                                       | `ConfigError::MerchantOfAnotherLegalEntity` renders the plain value or `<none>`                                                                                 |
| Minor — skills `(default)` is A2's `provider_account_id`                                          | [vpay-skills#95](https://github.com/vaam-apps/vpay-skills/pull/95) reworded to mark it A2                                                                       |
| Process — PR body                                                                                 | AI Usage Declaration, Risk and Reviewer focus added; the adversarial review is linked; the companion-skills line is updated                                     |

## Not done here

- Step A2 (`charges.provider_account_id`, `schemas/vpay.cstack`,
  `schemas/privacy-inventory.yaml`, `docs/status/backend.md` row update, and
  Phase A container test) is deferred to the follow-up A2 PR.
- Phase B (`B0`–`B7`) is unscheduled and untouched.
- Companion `vpay-skills` PR: **[#95](https://github.com/vaam-apps/vpay-skills/pull/95)**.
