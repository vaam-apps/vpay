//! The `audit_events` repository
//! (`backends/migrations/0059_audit-events-and-lookup-rate-limits.sql`) — the
//! generic, privacy-minimal, append-only journal for privacy-sensitive and
//! security-sensitive operations (ADR-0020 §3, ISO 27001 A.12.4).
//!
//! # Decisions and Constraints
//!
//! * **Provisional pending RFC-0002 D7**: initiated and terminal audit records,
//!   sharing one `operation_id`. Refuse lookup if initial audit write fails.
//! * **Provisional pending RFC-0002 D3**: `target_repr` holds a keyed HMAC-SHA256
//!   digest of `tenant_id` + normalized MSISDN under a dedicated secret,
//!   never the raw MSISDN or returned account-holder name.
//! * **ADR-0020 §3**: One generic journal, not a second store. Append-only
//!   and bounded tenant-scoped reads. No update or delete methods exist.
//!
//! # Queries
//!
//! * [`AuditEvents::append_audit_event`] appends one record.
//! * [`AuditEvents::list_audit_events_by_tenant`] lists bounded records for a
//!   tenant in reverse chronological order.
//! * [`append_in_tx`] appends one record inside an existing transaction.

use async_trait::async_trait;
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};

/// The input to append an audit event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAuditEvent {
    /// `aud_…`, minted by `vpay_core::ids::audit_event_id`.
    pub id: String,
    /// When the event occurred.
    pub occurred_at: OffsetDateTime,
    /// The tenant (merchant_id) owning this event.
    pub tenant: String,
    /// Actor kind: `signer` or `machine_token`.
    pub actor_kind: String,
    /// Actor identifier: lowercase hex thumbprint for signer, client_id for machine_token.
    pub actor_id: String,
    /// Privacy/security action: `account_holder:lookup`.
    pub action: String,
    /// Outcome: `initiated`, `found`, `not_found`, `invalid`, `unsupported`, `rate_limited`, `rail_error`.
    pub outcome: String,
    /// Target class: `account_holder`.
    pub target_class: String,
    /// Approved target representation: keyed HMAC-SHA256 hex digest (provisional pending RFC-0002 D3).
    pub target_repr: String,
    /// `op_…`, groups initiated and terminal rows of one operation.
    pub operation_id: String,
}

/// A stored audit event row.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AuditEventRow {
    pub id: String,
    pub occurred_at: OffsetDateTime,
    pub tenant: String,
    pub actor_kind: String,
    pub actor_id: String,
    pub action: String,
    pub outcome: String,
    pub target_class: String,
    pub target_repr: String,
    pub operation_id: String,
}

/// The repository for generic audit journal operations.
#[async_trait]
pub trait AuditEvents: Send + Sync {
    /// Appends one audit event row.
    ///
    /// # Errors
    ///
    /// [`DbError`] if the insert fails (e.g. check constraint violation, duplicate id).
    async fn append_audit_event(&self, event: &NewAuditEvent) -> Result<AuditEventRow, DbError>;

    /// Lists up to `limit` audit events for a single tenant, newest first.
    ///
    /// Enforces strict tenant scoping (ADR-0020 §3, ADR-0020 §5).
    ///
    /// # Errors
    ///
    /// [`DbError`] if the query fails.
    async fn list_audit_events_by_tenant(
        &self,
        tenant: &str,
        limit: i64,
    ) -> Result<Vec<AuditEventRow>, DbError>;
}

pub(crate) const INSERT_AUDIT_EVENT: &str = "\
    INSERT INTO audit_events (\
        id, occurred_at, tenant, actor_kind, actor_id, action, outcome, target_class, target_repr, operation_id\
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
    RETURNING id, occurred_at, tenant, actor_kind, actor_id, action, outcome, target_class, target_repr, operation_id";

pub(crate) const SELECT_AUDIT_EVENTS_BY_TENANT: &str = "\
    SELECT id, occurred_at, tenant, actor_kind, actor_id, action, outcome, target_class, target_repr, operation_id \
    FROM audit_events \
    WHERE tenant = $1 \
    ORDER BY occurred_at DESC, id DESC \
    LIMIT $2";

/// Inserts an audit event inside an active transaction.
pub(crate) async fn append_in_tx(
    conn: &mut sqlx::PgConnection,
    event: &NewAuditEvent,
) -> Result<AuditEventRow, DbError> {
    sqlx::query_as::<_, AuditEventRow>(INSERT_AUDIT_EVENT)
        .bind(&event.id)
        .bind(event.occurred_at)
        .bind(&event.tenant)
        .bind(&event.actor_kind)
        .bind(&event.actor_id)
        .bind(&event.action)
        .bind(&event.outcome)
        .bind(&event.target_class)
        .bind(&event.target_repr)
        .bind(&event.operation_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(classify_write)
}

#[async_trait]
impl AuditEvents for crate::repository::PgRepositories {
    async fn append_audit_event(&self, event: &NewAuditEvent) -> Result<AuditEventRow, DbError> {
        sqlx::query_as::<_, AuditEventRow>(INSERT_AUDIT_EVENT)
            .bind(&event.id)
            .bind(event.occurred_at)
            .bind(&event.tenant)
            .bind(&event.actor_kind)
            .bind(&event.actor_id)
            .bind(&event.action)
            .bind(&event.outcome)
            .bind(&event.target_class)
            .bind(&event.target_repr)
            .bind(&event.operation_id)
            .fetch_one(&self.pool)
            .await
            .map_err(classify_write)
    }

    async fn list_audit_events_by_tenant(
        &self,
        tenant: &str,
        limit: i64,
    ) -> Result<Vec<AuditEventRow>, DbError> {
        sqlx::query_as::<_, AuditEventRow>(SELECT_AUDIT_EVENTS_BY_TENANT)
            .bind(tenant)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `model AuditEvent` carries **no `@@allow` arm**, and this pins the absence.
    ///
    /// Pinned in this direction for ADR-0034 rules: nothing queries this table
    /// through the generated CrateStack layer, and queries are hand-written
    /// statements.
    #[test]
    fn the_audit_event_model_answers_no_rows_to_every_action() {
        use crate::schema::cratestack_schema::models::AUDIT_EVENT_MODEL as descriptor;

        for (action, policies) in [
            ("read", descriptor.read_allow_policies),
            ("create", descriptor.create_allow_policies),
            ("update", descriptor.update_allow_policies),
            ("delete", descriptor.delete_allow_policies),
        ] {
            assert!(
                policies.is_empty(),
                "model AuditEvent grew an @@allow(\"{action}\", …) arm. Nothing queries this \
                 model through the generated layer."
            );
        }
    }

    #[test]
    fn the_statements_contain_expected_clauses() {
        assert!(INSERT_AUDIT_EVENT.contains("INSERT INTO audit_events"));
        assert!(INSERT_AUDIT_EVENT.contains("RETURNING"));
        assert!(SELECT_AUDIT_EVENTS_BY_TENANT.contains("WHERE tenant = $1"));
        assert!(SELECT_AUDIT_EVENTS_BY_TENANT.contains("ORDER BY occurred_at DESC, id DESC"));
        assert!(SELECT_AUDIT_EVENTS_BY_TENANT.contains("LIMIT $2"));
    }
}
