# Privileged Access Matrix

_Issue [#152](https://github.com/vaam-apps/vpay/issues/152), [EPIC #155](https://github.com/vaam-apps/vpay/issues/155) (ISO/IEC 27001 readiness). Control references: ISO/IEC 27001:2022 Annex A.5.15 (Access Control), A.8.2 (Privileged Access Rights), A.8.20 (Network Security), A.8.24 (Use of Cryptography). Architecture decisions: [ADR-0017](../adr/0017-staff-authentication.md), [ADR-0018](../adr/0018-cross-tenant-admin-reads.md), [ADR-0020](../adr/0020-privacy-controls-and-evidence.md), [ADR-0035](../adr/0035-v1-is-a-signed-message-not-a-token.md), [ADR-0036](../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md), [ADR-0037](../adr/0037-authentication-methods-are-deployment-configuration.md)._

## 1. Principles

1. **Explicit Credential Identity:** Every inbound authenticated request carries an explicit credential identity ([ADR-0020 § 5](../adr/0020-privacy-controls-and-evidence.md)). Authentication proves the possession of a cryptographic key or validated bearer token; authorization verifies the credential's declared scopes and surface permissions.
2. **Surface Isolation:** Distinct API surfaces (`/v1`, `/dash/v1`, `/svc/v1`, `/v1/browser`) mount dedicated cryptographic resolvers. A credential valid on one surface is rejected with a coarse `401 Unauthorized` on other surfaces ([ADR-0022](../adr/0022-surface-isolation-and-independent-scaling.md)).
3. **No Default Privilege:** A registered credential with no scopes holds no permissions; every read and write is refused (`403 Forbidden`).
4. **Prohibition of Shared Administrator Accounts:** Shared administrative credentials are strictly prohibited. Every staff operator must possess an individual account created via CLI (`vpay-server staff add`), authenticated via paired developer console keys (Ed25519/P-256) or individualized SSO/MFA credentials.
5. **Break-Glass Accountability:** Emergency access is constrained to host-level CLI subcommands (`vpay-server staff pair-key`, `vpay-server staff revoke-key`). Pairing never creates or upgrades an administrator; admin privileges require explicit operator intervention.

---

## 2. Role and Surface Authorization Matrix

| Caller Kind | Surface | Transport & Auth Mechanism | Scope / Capability Constraint | Refusal Outcome | Proving Code & Test |
|---|---|---|---|---|---|
| **Merchant Signer** | `/v1/*` | Inbound HTTPS; COSE_Sign1 Ed25519 envelope ([ADR-0035](../adr/0035-v1-is-a-signed-message-not-a-token.md)) | Tenant-scoped; route declared scope policy (`payments:read`, `payments:write`, `account_holders:read`) | 403 Forbidden (`insufficient_scope`) if scope missing; 401 if key unknown | `vpay_api::envelope::scope::merchant_scope`; `a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not` (`payment_intents.rs`), `credential_without_account_holders_read_scope_is_refused_403` (`account_holders.rs`) |
| **Merchant Signer** | `/dash/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `vpay_api::envelope::console::dash_scope`; `a_key_that_is_not_a_paired_console_key_is_the_same_coarse_refusal` (`dashboard_read_surface.rs:1214`) |
| **Merchant Signer** | `/svc/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `vpay_api::envelope::service::service_scope`; `a_merchant_key_cannot_reach_svc` (`console_pairing.rs:382`) |
| **Machine Client (Bearer Token)** | `/v1/*` | Inbound HTTPS; `Authorization: Bearer <JWT>` (OAuth 2.0 / MCP, [ADR-0037](../adr/0037-authentication-methods-are-deployment-configuration.md)) | Bound to client registration scopes in YAML; cannot exceed registered grants | 403 Forbidden (`insufficient_scope`) or 401 if invalid/expired | `vpay_api::envelope::scope::token_scope`; `a_valid_client_token_authenticates_the_request` (`merchant_token_flow.rs`), `a_token_claiming_more_than_its_registration_grants_cannot_write` (`mcp_merchant.rs:614`), `a_read_scoped_token_cannot_call_a_mutation` (`mcp_merchant.rs:535`) |
| **Machine Client (Bearer Token)** | `/dash/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `the_printed_password_cannot_reach_dash_v1` (`staff_sign_in.rs:1421`) |
| **Machine Client (Bearer Token)** | `/svc/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `a_merchant_key_cannot_reach_svc` (`console_pairing.rs:382`) |
| **Console Key** | `/dash/v1/*` | Inbound HTTPS; COSE_Sign1 P-256 envelope ([ADR-0036](../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)) | 30-day bounded lifecycle; revoked keys cut off immediately; tenant bound (admin reads allowed only if `admin: true`) | 401 Unauthorized if expired, revoked, or unpaired | `vpay_api::envelope::console::dash_scope`; `a_session_bearer_and_a_console_key_both_read_dash_v1` (`dashboard_read_surface.rs:1375`), `a_key_expires_thirty_days_after_it_was_paired` (`dashboard_read_surface.rs:1309`), `a_revoked_key_is_refused_on_its_very_next_request` (`dashboard_read_surface.rs:1350`) |
| **Console Key** | `/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `vpay_api::envelope::scope::merchant_scope`; `a_console_or_service_key_cannot_reach_v1` (`signed_transport.rs:509`) |
| **Console Key** | `/svc/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `vpay_api::envelope::service::service_scope`; `a_merchant_key_cannot_reach_svc` (`console_pairing.rs:418`) |
| **Service Client** | `/svc/v1/*` | Inbound HTTPS; COSE_Sign1 service key ([ADR-0036 § 4](../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)) | Capability check per route (`ServiceCapability::ConsolePairing`) | 403 Forbidden (sealed) if capability lacking; 401 if key unknown | `vpay_api::envelope::service::service_scope`; `a_service_client_with_capability_pairs_a_key` (`console_pairing.rs:480`), `a_service_client_without_the_pairing_capability_is_refused_a_pairing` (`console_pairing.rs:511`) |
| **Service Client** | `/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `vpay_api::envelope::scope::merchant_scope`; `a_console_or_service_key_cannot_reach_v1` (`signed_transport.rs:519`) |
| **Service Client** | `/dash/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `vpay_api::envelope::console::dash_scope`; `a_key_that_is_not_a_paired_console_key_is_the_same_coarse_refusal` (`dashboard_read_surface.rs:1220`) |
| **Staff Session Bearer** | `/dash/v1/*` | Inbound HTTPS; `Authorization: Bearer <session-token>` (when `human_login` served, [ADR-0030](../adr/0030-staff-sign-in-federates-to-an-oidc-provider.md), [ADR-0037](../adr/0037-authentication-methods-are-deployment-configuration.md)) | Password + TOTP or federated OIDC second-factor; session re-read per request | 401 Unauthorized; 404 Not Found if `human_login` disabled in `auth.methods` | `vpay_api::envelope::console::session_scope`; `with_the_method_off_the_staff_routes_404_and_dash_refuses_unsigned_requests` (`staff_sign_in.rs:1595`), `the_second_factor_rule_is_acr_or_amr_and_a_password_alone_is_not_one` (`staff_sso.rs:400`) |
| **Staff Session Bearer** | `/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `every_cause_of_a_refusal_is_the_same_unsigned_answer` (`signed_transport.rs:430`) |
| **Staff Session Bearer** | `/svc/v1/*` | Inbound HTTPS | Refused at envelope resolution | 401 Unauthorized (unsealed, coarse) | `a_merchant_key_cannot_reach_svc` (`console_pairing.rs:430`) |
| **Operator CLI** | Local Host / Pod CLI | Direct process invocation (`vpay-server staff <subcommand>`) | Subcommand authorization: `add`, `pair-key`, `revoke-key` | Exit 78 on invalid config/arguments | `vpay_config::cli::StaffCommand`; `pairing_never_creates_or_upgrades_an_administrator` (`console_pairing.rs:320`) |

---

## 3. Privileged Roles and Granularity

1. **Staff Member (`stf_...`):**
   - Scope: Bound to single assigned merchant (`merchant_id`).
   - Permissions: Reads dashboard summaries, charges, intents, customers, refunds, and invoices for their merchant. Cannot access internal system keys or cross-tenant data.
2. **Staff Administrator (`stf_...` with `admin: true`):**
   - Scope: May specify `?merchant_id=` on read requests to inspect any tenant's data ([ADR-0018](../adr/0018-cross-tenant-admin-reads.md)).
   - Constraint: Read-only access across tenants. Mutations are never permitted across tenants.
   - Creation: Administrator accounts can **only** be created via host CLI (`vpay-server staff add --admin`). Web pairing routes (`/svc/v1/console_keys`) refuse to create or upgrade an administrator (`pairing_never_creates_or_upgrades_an_administrator`).
3. **Internal Pairing Service (`vaam`):**
   - Scope: Possesses `ServiceCapability::ConsolePairing` on `/svc/v1`.
   - Permissions: Calls `POST /svc/v1/console_keys` and `DELETE /svc/v1/console_keys/{thumbprint}` after phone-based verification. Cannot pair an administrator key.

---

## 4. Key and Secret Lifecycle Hardening

All secrets and credentials conform to [ADR-0020 § 3](../adr/0020-privacy-controls-and-evidence.md):
- **Redacting `Debug` implementations:** Every secret type in `vpay-config` (`CommonArgs`, `ProviderHost`, `StaffSso`, `StaffAuth`, `PrivacyConfig`, `ServiceClient`, `WebhookEndpoint`, `MerchantClient`, `DashboardClient`) implements custom `Debug` formatting replacing sensitive fields with `[redacted]` or `[N redacted]`.
- **Boot Tracing Protection:** Boot sequences capture configuration state without exposing secrets (`boot_logging_never_leaks_configured_secrets` in `v1::boot`).
- **Static CI Secret Scanning:** Trivy runs on every pull request and push in `.github/workflows/quality.yml` with `scanners: "vuln,secret,misconfig"`, reporting leaks via GitHub Security.
- **Rotation Procedures:** Documented in [docs/runbooks/rotate-signing-key.md](../runbooks/rotate-signing-key.md), [docs/runbooks/rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md), and [docs/runbooks/access-review.md](../runbooks/access-review.md).
