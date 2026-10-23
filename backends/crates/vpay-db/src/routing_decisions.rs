//! The `routing_decisions` repository (`backends/migrations/0053_method-types-and-routing-decisions.sql`).
//!
//! ADR-0029 D10: the router's decision for one routed charge, written by
//! [`crate::TxRepositories::insert_routing_decision`] **inside the transaction
//! that opens the charge**, so a charge the router chose a rail for never
//! commits without the row that says why. A charge confirmed by a rail's own
//! code has none: the router never saw it.
//!
//! The row is an explanation, never an input: nothing reads it to decide
//! anything about a payment. [`crate::Charges::routing_decision_for`] is for
//! an operator, a dashboard and the tests.

use sqlx::{FromRow, PgConnection};
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};

/// One decision, as `vpay_api` writes it.
///
/// `considered` is the router's whole answer, already serialised: every rail
/// in the order the router returned them, each a candidate with its score and
/// breakdown or an excluded rail with its reason. Serialised by the caller
/// because its shape is `vpay_core::routing`'s, which this crate does not
/// depend on for anything else.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRoutingDecision {
    /// The charge it explains, `charges.id`.
    pub charge_id: String,
    /// `mtn_wallet` or `orange_wallet`.
    pub method_type: String,
    /// The rail the router chose: `charges.provider_code`.
    pub chosen_rail: String,
    /// The rail the merchant pinned on the confirm, if any.
    pub pinned_rail: Option<String>,
    /// Every rail considered, a JSON array.
    pub considered: serde_json::Value,
    /// `vpay_core::routing::SCORE_FORMULA` at the time.
    pub formula: String,
    /// The router's configuration version, 16 hex digits.
    pub config_version: String,
}

/// One `routing_decisions` row, exactly as stored.
#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct RoutingDecisionRow {
    /// See [`NewRoutingDecision`].
    pub charge_id: String,
    /// See [`NewRoutingDecision`].
    pub method_type: String,
    /// See [`NewRoutingDecision`].
    pub chosen_rail: String,
    /// See [`NewRoutingDecision`].
    pub pinned_rail: Option<String>,
    /// See [`NewRoutingDecision`].
    pub considered: serde_json::Value,
    /// See [`NewRoutingDecision`].
    pub formula: String,
    /// See [`NewRoutingDecision`].
    pub config_version: String,
    /// The database's clock at the insert.
    pub created_at: OffsetDateTime,
}

/// Inside the caller's transaction, after the charge it explains.
///
/// # Errors
///
/// [`DbError::UniqueViolation`] on `routing_decisions_pkey` for a second
/// decision for one charge; [`DbError::ForeignKeyViolation`] for an unknown
/// charge or rail; [`DbError::Query`] otherwise, a CHECK's refusal
/// included.
pub(crate) async fn insert(tx: &mut PgConnection, new: &NewRoutingDecision) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO routing_decisions (charge_id, method_type, chosen_rail, pinned_rail, \
         considered, formula, config_version) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&new.charge_id)
    .bind(&new.method_type)
    .bind(&new.chosen_rail)
    .bind(new.pinned_rail.as_deref())
    .bind(&new.considered)
    .bind(&new.formula)
    .bind(&new.config_version)
    .execute(&mut *tx)
    .await
    .map_err(classify_write)?;
    Ok(())
}

/// The decision for one charge, if the router made one.
///
/// # Errors
///
/// [`DbError::Query`] if the read fails.
pub(crate) async fn for_charge(
    pool: &sqlx::PgPool,
    charge_id: &str,
) -> Result<Option<RoutingDecisionRow>, DbError> {
    sqlx::query_as::<_, RoutingDecisionRow>(
        "SELECT charge_id, method_type, chosen_rail, pinned_rail, considered, formula, \
         config_version, created_at FROM routing_decisions WHERE charge_id = $1",
    )
    .bind(charge_id)
    .fetch_optional(pool)
    .await
    .map_err(DbError::Query)
}
