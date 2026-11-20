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

- `cargo test -p vpay-config`: 208 unit/fixture tests passed, 7 doctests passed.
  New fixtures:
  - `legal-entity-missing-two-merchants.yml` -> `MerchantsWithoutLegalEntity`
  - `legal-entity-merchant-mismatch.yml` -> `MerchantOfAnotherLegalEntity`
  - `legal-entity-two-projects-one-company.yml` -> boots
- Mutation check (RFC-0009 § 5): temporarily disabling the rule 2 comparison in
  `Config::validate_all` caused `merchant_of_another_legal_entity_is_rejected`
  to fail (`left: None`, `right: Some(MerchantOfAnotherLegalEntity { .. })`);
  restored immediately after.

## Not done here

- Step A2 (`charges.provider_account_id`, `schemas/vpay.cstack`,
  `schemas/privacy-inventory.yaml`, `docs/status/backend.md` row update, and
  Phase A container test) is deferred to the follow-up A2 PR.
- Phase B (`B0`–`B7`) is unscheduled and untouched.
- Companion `vpay-skills` PR (`vpay-ops/references/configuration.md` and
  `vpay-troubleshooting/references/config-boot.md`) to be opened alongside this PR.
