//! The `refunds` repository (`backends/migrations/0017_create-refunds.sql`) —
//! one read, and nothing else.
//!
//! **This module does not create refunds, and there is no write here at all.**
//! Creating one needs a rail refund, and neither rail has one:
//! `mtn_momo::refund` is `ProviderError::NotImplemented` and Orange Money
//! answers `Unsupported` because its Web Payment product documents no refund
//! API (`docs/status.md`). `POST /v1/refunds` stays unrouted. What this module
//! adds is the **authoritative read** a refund has to have once it exists at
//! all: `docs/flows/provider-port.md` calls `query_status` "the authoritative
//! read", `docs/flows/webhooks.md` says delivery is at-least-once and
//! unordered, and a merchant holding a `re_…` with no way to ask what happened
//! to it has neither (issue #45).
//!
//! # Why the scope is a join and not a column
//!
//! `refunds` carries no `merchant_id`. Migration `0017` gives it a `NOT NULL`
//! foreign key onto `payment_intents (id)` instead, and the intent is where
//! the tenant lives — so [`Refunds::get_for_merchant`] joins rather than
//! filtering a column of its own. That is a deliberate choice not to migrate
//! the table: a denormalised `merchant_id` would be a second answer to "whose
//! refund is this?", and two answers to a tenancy question is how one of them
//! ends up stale. The cost is one index lookup on the primary key of
//! `payment_intents` per read.
//!
//! # The rule this module keeps
//!
//! The same one [`crate::payment_intents`] and [`crate::checkout_sessions`]
//! keep: **the merchant-facing query is merchant-scoped in SQL**, so a `/v1`
//! handler cannot forget to filter, and another tenant's refund is
//! indistinguishable from a missing one. There is no unscoped variant here —
//! unlike those two modules, nothing in this repository has a caller that
//! needs one.

// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::AssertSqlSafe;
use time::OffsetDateTime;

use crate::error::DbError;

/// The columns [`RefundRow`] decodes, table-qualified because the one query
/// below joins.
///
/// `status::TEXT` for the reason `payment_intents`' own column list gives: `status`
/// is a Postgres `ENUM` (`refund_status`), and without the cast `sqlx`
/// refuses to decode a user-defined type into `String` at runtime — a failure
/// this crate would only discover against a real database.
///
/// **Not every column of the table**, and that is the choice
/// [`crate::events::EventRow`] makes for `fanout_attempts`: `charge_id`,
/// `failure_code`, `failure_raw`, `provider_reference_id` and `updated_at`
/// are on the table and no reader of a refund branches on them, because none
/// of them is on the wire object `docs/flows/merchant-auth.md` documents.
/// Selecting a column nothing uses is how a row struct starts mirroring the
/// table instead of its callers — and the writer that would fill those five
/// does not exist yet, so guessing at its shape now would be a claim about
/// code nobody has written.
const COLUMNS: &str = "r.id, r.payment_intent_id, r.amount, r.currency_code, \
                       r.status::TEXT AS status, r.reason, r.metadata, r.created_at";

/// One `refunds` row, as the merchant read needs it.
///
/// Not the wire object: `vpay-api` owns that shape (lowercase currency,
/// unix-seconds `created`, `payment_intent` rather than `payment_intent_id`).
/// See [`COLUMNS`] for why this is a projection of the table rather than the
/// whole of it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct RefundRow {
    /// Public `re_…` id, supplied by whoever writes the row — never generated
    /// by Postgres, like every other object id in this schema.
    pub id: String,
    /// The intent this refunds. A real foreign key onto `payment_intents
    /// (id)`, and the only path from a refund to its tenant — see the module
    /// docs.
    pub payment_intent_id: String,
    /// Minor units, strictly positive (`amount_positive`, migration `0017`).
    pub amount: i64,
    /// Carried verbatim from the intent, never converted
    /// (`docs/flows/money.md`).
    pub currency_code: String,
    /// `pending`, `succeeded`, `failed` or `canceled` — the `refund_status`
    /// enum, decoded as text.
    ///
    /// A `String` and not a typed enum for [`crate::events::EventRow`]'s
    /// reason: the vocabulary is closed by Postgres *where it is written*, and
    /// this crate carries a vocabulary as text so the parse belongs to the
    /// layer that renders it.
    pub status: String,
    /// The merchant's own free text (`"duplicate"`,
    /// `"requested_by_customer"`), or `NULL`. Deliberately not an enum in the
    /// schema either: the vocabulary is the merchant's, not vpay's.
    pub reason: Option<String>,
    /// The merchant's key/value pairs. The `metadata_is_object` CHECK
    /// guarantees it is a JSON object.
    pub metadata: serde_json::Value,
    /// When the refund was requested.
    pub created_at: OffsetDateTime,
}

/// The `refunds` reads a consumer of this crate may perform.
///
/// One method, and no write. A `create` here would be a write path no
/// shipping code calls — the refund a merchant would create needs
/// `ProviderAdapter::refund`, which is `NotImplemented` on MTN and
/// `Unsupported` on Orange — and this repository's rule is that an unbuilt
/// feature stays visibly unbuilt (`AGENTS.md` rule 2).
#[async_trait::async_trait]
pub trait Refunds: Send + Sync {
    /// Reads one refund *for this merchant*.
    ///
    /// `None` means "no such refund for you", which covers both a missing id
    /// and another merchant's id. Those two must be indistinguishable, and
    /// folding them together **here** — rather than reading the row and
    /// comparing the tenant in the handler — is what makes them so: a caller
    /// that never learns the row exists cannot leak that it does. It is the
    /// same split `PaymentIntents::get_for_merchant` and
    /// `Events::get_by_id` draw, and the reason `GET /v1/refunds/{id}`
    /// answers `404` rather than `403`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the read fails.
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<RefundRow>, DbError>;
}

#[async_trait::async_trait]
impl Refunds for crate::repository::PgRepositories {
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<RefundRow>, DbError> {
        // The tenant predicate is on the *joined* intent, which is the only
        // place it exists. `JOIN`, not `LEFT JOIN`: `payment_intent_id` is
        // `NOT NULL` and a foreign key, so a refund with no intent cannot
        // exist — and if one somehow did, answering `None` for it is the
        // right answer anyway, because there would be no tenant to attribute
        // it to.
        let sql = format!(
            "SELECT {COLUMNS} FROM refunds r \
             JOIN payment_intents p ON p.id = r.payment_intent_id \
             WHERE p.merchant_id = $1 AND r.id = $2"
        );

        sqlx::query_as::<_, RefundRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }
}
