# Access Review and Privileged Account Export

_Issue [#152](https://github.com/vaam-apps/vpay/issues/152), [EPIC #155](https://github.com/vaam-apps/vpay/issues/155). ISO/IEC 27001:2022 Control references: Annex A.5.15 (Access Control), A.5.18 (Access Rights), A.8.2 (Privileged Access Rights). Architectural context: [ADR-0017](../adr/0017-staff-authentication.md), [ADR-0018](../adr/0018-cross-tenant-admin-reads.md), [ADR-0036](../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)._

## 1. Scope and Cadence

This procedure defines the repeatable export and review process for all privileged accounts, staff members, console keys, and active sessions within vpay.

- **Cadence:** Performed at least **quarterly**, and immediately upon staff departure or role change.
- **Ownership:**
  - **In-repo database & CLI state:** Owned by the vpay operator.
  - **Identity Provider (IdP) & SSO Federation:** Owned by the organization's IT / Identity team (e.g. Keycloak / Okta / Google Workspace group memberships and MFA enforcement).

---

## 2. Review Checklist

1. **Verify Shared-Account Prohibition:** Confirm that every active account corresponds to an identifiable named individual. No shared administrator or staff accounts are permitted.
2. **Review Administrator Grants:** Confirm that only authorized operators retain `role = 'admin'` (cross-tenant read grant, ADR-0018).
3. **Audit Active Console Keys:** Verify that paired console keys (`console_keys`) belong to active members, have not exceeded their 30-day lifecycle, and revoke any unrecognized or stale keys.
4. **Audit Active Staff Sessions:** Ensure active sessions (`staff_sessions`) match authorized active sign-ins and terminate lingering sessions where appropriate.
5. **Reconcile with IdP:** Cross-check the local member export with the organization's SSO identity directory to ensure deprovisioned staff are marked `active = false`.

---

## 3. Repeatable Access Review Export (SQL)

Connect to the PostgreSQL instance via `psql` (e.g. `kubectl exec -it deployment/vpay-server -- psql "$DATABASE_URL"` or operator bastion):

```sql
-- 1. All staff members, assigned merchant, admin role, and active status
SELECT
    m.id AS staff_id,
    m.merchant_id,
    m.email,
    m.name,
    m.role,
    m.active,
    m.created_at,
    m.updated_at
FROM staff_members m
ORDER BY m.role DESC, m.merchant_id, m.email;

-- 2. Active console keys (paired within 30 days and not revoked)
SELECT
    k.thumbprint,
    k.staff_id,
    m.email,
    m.merchant_id,
    m.role,
    k.label,
    k.paired_at,
    k.expires_at,
    k.revoked_at,
    CASE
        WHEN k.revoked_at IS NOT NULL THEN 'REVOKED'
        WHEN k.expires_at <= now() THEN 'EXPIRED'
        ELSE 'ACTIVE'
    END AS status
FROM console_keys k
JOIN staff_members m ON m.id = k.staff_id
ORDER BY k.expires_at DESC;

-- 3. Active staff web sessions (when human_login is served)
SELECT
    s.id AS session_id,
    s.staff_id,
    m.email,
    m.merchant_id,
    s.created_at,
    s.expires_at,
    s.revoked_at,
    CASE
        WHEN s.revoked_at IS NOT NULL THEN 'REVOKED'
        WHEN s.expires_at <= now() THEN 'EXPIRED'
        ELSE 'ACTIVE'
    END AS status
FROM staff_sessions s
JOIN staff_members m ON m.id = s.staff_id
ORDER BY s.expires_at DESC;
```

---

## 4. Remediation Actions

### Revoking a Console Key Immediately
If an unauthorized or stale console key is discovered, revoke it via the CLI on the pod/host:

```bash
vpay-server staff revoke-key --thumbprint "<64_char_hex_thumbprint>"
```
_Proven by:_ `a_revoked_key_is_refused_on_its_very_next_request` (`backends/tests/integration/tests/dashboard_read_surface.rs`).

### Deactivating a Staff Member
To deactivate a staff member and cut off all session access:

```sql
UPDATE staff_members
SET active = false, updated_at = now()
WHERE email = 'departed.user@example.test';

-- Invalidate all associated console keys and sessions
UPDATE console_keys
SET revoked_at = now()
WHERE staff_id IN (SELECT id FROM staff_members WHERE email = 'departed.user@example.test')
  AND revoked_at IS NULL;

UPDATE staff_sessions
SET revoked_at = now()
WHERE staff_id IN (SELECT id FROM staff_members WHERE email = 'departed.user@example.test')
  AND revoked_at IS NULL;
```
_Proven by:_ Member re-read on every request (`vpay_api::envelope::console::session_scope`).
