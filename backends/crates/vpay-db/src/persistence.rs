//! [`PersistenceError`] — the leaf a `CratestackError` becomes — and the one
//! function that produces it.
//!
//! This module is the whole seam between CrateStack's error vocabulary and
//! vpay's. Nothing outside it matches on a `CratestackError`, and no
//! `CratestackError` reaches a caller of this crate: [`crate::DbError`]
//! `#[from]`s [`PersistenceError`] and delegates its classification, exactly
//! as ADR-0011 and ADR-0016 standard 1 ask.
//!
//! Why vpay classifies these itself rather than using CrateStack's own
//! `status_code()`/`public_message()` — and what the read path can and
//! cannot produce — is in
//! [docs/reference/vpay-db.md § CrateStack](../../../../docs/reference/vpay-db.md#cratestack).

use cratestack::{CratestackContext, CratestackError, SystemContext};

/// The name a system read of this crate's tables is attributed to.
///
/// It reaches `cratestack_sqlx::audit::actor_from_context` as
/// `actor.id = "system:vpay-db"`, so a row written under this context is
/// attributable without any audit-path code knowing a system caller is a
/// different kind of caller. Nothing audits today; the name is chosen now so
/// that it does not have to be chosen retroactively for rows already written.
const SYSTEM_SERVICE: &str = "vpay-db";

/// Everything CrateStack's data layer can fail with, in vpay's own terms.
///
/// One variant per decision a caller could make differently, mirroring
/// [`crate::DbError`]'s own split rather than CrateStack's: the three
/// integrity violations are told apart because
/// [`crate::error::classify_write`] already tells them apart and the two
/// paths must not disagree about whether a duplicate charge is a `409` or a
/// `503`.
///
/// Deliberately derives no `Serialize`/`Deserialize`: it never reaches a
/// wire. A merchant sees whatever [`vpay_core::Classify::public_message`]
/// returns for the category, never these strings — `detail` carries driver
/// text that names tables and constraints.
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    /// SQLSTATE `23505`. The twin of [`crate::DbError::UniqueViolation`],
    /// and it must classify identically: see this module's tests, which
    /// assert the two agree rather than trusting that they were written to.
    #[error("write violates the unique constraint {constraint}: {detail}")]
    Unique {
        /// The Postgres constraint or index name. `<unnamed>` if the driver
        /// reported none — never a guess, matching `classify_write`.
        constraint: String,
        /// The driver's own message, kept whole for operator logs.
        detail: String,
    },

    /// SQLSTATE `23503`.
    #[error("write violates the foreign key constraint {constraint}: {detail}")]
    ForeignKey {
        /// The Postgres constraint name.
        constraint: String,
        /// The driver's own message, kept whole for operator logs.
        detail: String,
    },

    /// SQLSTATE `23514`. Its own variant rather than being folded into
    /// [`Self::Backend`], because CrateStack can *generate* a CHECK from a
    /// `@db_enforce` validator, so a violation here may name a constraint
    /// vpay never wrote by hand — and an operator staring at
    /// `charges_amount_range_check` needs the name to find out which.
    /// It classifies `Internal` all the same, for `classify_write`'s stated
    /// reason: a CHECK violation is a vpay bug that reached Postgres.
    #[error("write violates the check constraint {constraint}: {detail}")]
    Check {
        /// The Postgres constraint name.
        constraint: String,
        /// The driver's own message, kept whole for operator logs.
        detail: String,
    },

    /// A model policy refused the operation
    /// (`CratestackError::Forbidden`).
    ///
    /// **`Internal`, not `Forbidden`.** Every CrateStack call in this crate
    /// runs under [`system_context`], and a `SystemContext` cannot be
    /// derived from a request, so a refusal cannot have been caused by
    /// anything a merchant sent: it means the schema's `@@allow` clauses and
    /// this crate's call sites disagree, which is a deploy-time bug that
    /// pages. Classifying it `Forbidden` would put "you are not allowed to
    /// do that" in front of a merchant for a mistake vpay made.
    #[error("{model}: a model policy denied a system {action}: {detail}")]
    Denied {
        /// The `.cstack` model the policy is attached to.
        model: &'static str,
        /// `read`, `create`, … — the action slot the policy refused.
        action: &'static str,
        /// CrateStack's own message, for the operator log.
        detail: String,
    },

    /// `CratestackError::NotFound`, which the data layer raises for
    /// `sqlx::Error::RowNotFound` on the paths that demand a row.
    ///
    /// **Not** what an absent row looks like on the read path this crate
    /// uses: `find_unique(...).run(ctx)` returns `Ok(None)`, and "no such
    /// disabled client" is an answer rather than an error.
    #[error("{model}: no row matched")]
    NotFound {
        /// The `.cstack` model whose read found nothing.
        model: &'static str,
    },

    /// Everything else — a pool timeout, a decode failure, a protocol error,
    /// and (today) *every* failure of a CrateStack read, because
    /// `FindUnique::run` maps its `sqlx::Error` with
    /// `CratestackError::Database(error.to_string())` rather than through
    /// `cratestack_error_from_sqlx`, so a read never carries a SQLSTATE at
    /// all. `Storage`, exactly as [`crate::DbError::Query`] is.
    #[error("database error: {0}")]
    Backend(String),
}

impl vpay_core::Classify for PersistenceError {
    fn category(&self) -> vpay_core::Category {
        use vpay_core::Category;
        match self {
            // Identical to `DbError::UniqueViolation`, and the test below
            // asserts that rather than leaving it to whoever edits one of
            // the two next. Retry advice on a duplicate charge is advice to
            // charge a payer twice (`error.rs`).
            Self::Unique { .. } => Category::Conflict,
            Self::ForeignKey { .. } => Category::InvalidRequest,
            // A CHECK the application should have satisfied before writing,
            // a policy that refused a caller that cannot be a merchant, and
            // a demanded row that was not there: all three are vpay's own
            // mistake, and none is fixed by retrying.
            Self::Check { .. } | Self::Denied { .. } | Self::NotFound { .. } => Category::Internal,
            Self::Backend(_) => Category::Storage,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            // The same two codes `DbError` publishes, because a merchant
            // branching on `error.code` must not be able to tell which of
            // vpay's two persistence paths served the request.
            Self::Unique { .. } => "resource_conflict",
            Self::ForeignKey { .. } => "invalid_reference",
            Self::Check { .. } => "check_violation",
            Self::Denied { .. } => "policy_denied",
            Self::NotFound { .. } => "row_not_found",
            Self::Backend(_) => "database_query_failed",
        }
    }
}

/// The context every CrateStack call in this crate runs under.
///
/// `vpay-db` is trusted in-process code: it is reached only through the
/// repository traits, never on behalf of an authenticated request, and the
/// `/v1` handler that ultimately caused a call has already made whatever
/// authorisation decision belongs to it. `SystemContext` is the only
/// producer of a context satisfying `auth().isSystem()`, and CrateStack
/// gives it no `From<CratestackContext>`, so this cannot be reached from a
/// request-derived context even by mistake.
///
/// Built per call rather than cached on the repository: it is two `BTreeMap`
/// inserts against a `&'static str`, the query it accompanies is a network
/// round trip, and a cached context is a piece of mutable state that would
/// have to be reasoned about the first time a per-caller context is wanted.
pub(crate) fn system_context() -> CratestackContext {
    SystemContext::for_service(SYSTEM_SERVICE).into_context()
}

/// The single place a `CratestackError` becomes a vpay error.
///
/// The mirror image of [`crate::error::classify_write`], and deliberately
/// the same shape: branch on the SQLSTATE, carry the constraint name, never
/// invent one. It reads `db_sqlstate()`/`db_constraint()` rather than
/// matching `DatabaseTyped` directly, because those accessors answer for
/// `ConflictTyped` too and `CratestackError` is `#[non_exhaustive]`.
///
/// `model` and `action` are supplied by the call site because
/// `CratestackError::Forbidden` carries only a sentence: the framework knows
/// which policy refused, the caller knows which of vpay's queries asked, and
/// only the second is useful in a log that has to be read at 3am.
///
/// Not `pub`, for `classify_write`'s stated reason: nothing outside these
/// repositories has a `CratestackError` to classify.
pub(crate) fn classify_cratestack(
    model: &'static str,
    action: &'static str,
    error: CratestackError,
) -> PersistenceError {
    // `<unnamed>` rather than a guess, matching `classify_write`: a plausible
    // constraint name here would make a test asserting the name pass against
    // the wrong rule.
    let constraint = || error.db_constraint().unwrap_or("<unnamed>").to_owned();
    let detail = error.to_string();

    match error.db_sqlstate() {
        Some("23505") => {
            return PersistenceError::Unique {
                constraint: constraint(),
                detail,
            };
        }
        Some("23503") => {
            return PersistenceError::ForeignKey {
                constraint: constraint(),
                detail,
            };
        }
        Some("23514") => {
            return PersistenceError::Check {
                constraint: constraint(),
                detail,
            };
        }
        _ => {}
    }

    match error {
        CratestackError::Forbidden(_) => PersistenceError::Denied {
            model,
            action,
            detail,
        },
        CratestackError::NotFound(_) => PersistenceError::NotFound { model },
        // Everything else, including the typed variants whose SQLSTATE was
        // not one of the three above. `#[non_exhaustive]`, so this arm is a
        // wildcard by force rather than by choice — which is the argument
        // for keeping this function the *only* place it appears.
        _ => PersistenceError::Backend(detail),
    }
}

#[cfg(test)]
mod tests {
    use cratestack::{CratestackError, DbErrorInfo};
    use vpay_core::{Category, Classify as _, Retry};

    use super::{PersistenceError, classify_cratestack, system_context};
    use crate::error::{DbError, classify_write};

    fn typed(sqlstate: &str, constraint: &str) -> CratestackError {
        CratestackError::DatabaseTyped(DbErrorInfo {
            detail: format!("ERROR: violates constraint \"{constraint}\""),
            sqlstate: Some(sqlstate.to_owned()),
            constraint: Some(constraint.to_owned()),
        })
    }

    /// A `23505` must reach a merchant as the same thing whichever of vpay's
    /// two persistence paths produced it.
    ///
    /// Written against the *other* function's answer rather than against a
    /// literal `Category::Conflict`, so that changing one and not the other
    /// fails here — which is the only failure mode that matters. `sqlx` will
    /// not let a `DatabaseError` be constructed in a unit test, so the
    /// `classify_write` side is exercised on the variant it produces rather
    /// than on a synthetic driver error; `tests/repositories.rs` is where a
    /// real Postgres proves that variant is what a real `23505` becomes.
    #[test]
    fn a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx() {
        let via_cratestack =
            classify_cratestack("Charge", "create", typed("23505", "one_charge_per_intent"));
        let via_sqlx = DbError::UniqueViolation {
            constraint: "one_charge_per_intent".to_owned(),
            source: sqlx::Error::RowNotFound,
        };

        assert_eq!(via_cratestack.category(), via_sqlx.category());
        assert_eq!(via_cratestack.code(), via_sqlx.code());
        assert_eq!(via_cratestack.retry(), via_sqlx.retry());
        assert_eq!(via_cratestack.category(), Category::Conflict);
        assert_eq!(via_cratestack.category().http_status(), 409);
        assert_eq!(via_cratestack.retry(), Retry::Never);
        assert!(
            matches!(&via_cratestack, PersistenceError::Unique { constraint, .. }
                if constraint == "one_charge_per_intent"),
            "the operator log has to name the rule that fired: {via_cratestack}"
        );

        // And it must NOT be what CrateStack itself would have said. Its own
        // `status_code()` for a `DatabaseTyped` is 500 with a canned
        // "internal error" — a duplicate charge answered as an outage, which
        // is the reason vpay classifies these itself.
        assert_eq!(
            typed("23505", "one_charge_per_intent")
                .status_code()
                .as_u16(),
            500,
            "if this stops being 500, re-read whether vpay still needs its own mapping"
        );
        assert_ne!(
            via_cratestack.category().http_status(),
            typed("23505", "one_charge_per_intent")
                .status_code()
                .as_u16(),
        );

        // A foreign key must not collapse into the same answer.
        let fk = classify_cratestack(
            "Charge",
            "create",
            typed("23503", "charges_currency_code_fkey"),
        );
        assert_eq!(fk.category(), Category::InvalidRequest);
        assert_eq!(
            fk.category(),
            classify_write_fk().category(),
            "the two paths must agree about a dangling reference too"
        );
    }

    fn classify_write_fk() -> DbError {
        DbError::ForeignKeyViolation {
            constraint: "charges_currency_code_fkey".to_owned(),
            source: sqlx::Error::RowNotFound,
        }
    }

    /// A policy denial is vpay's bug, not the caller's.
    ///
    /// Every CrateStack call in this crate runs as the system principal, and
    /// a `SystemContext` cannot be built from anything a request controls —
    /// so a refusal means the schema and the call site disagree. `Forbidden`
    /// here would tell a merchant they lack permission for a mistake they
    /// could not have made and cannot fix.
    #[test]
    fn a_policy_denial_is_internal_and_never_the_callers_fault() {
        let denied = classify_cratestack(
            "DisabledClient",
            "read",
            CratestackError::Forbidden("policy denied".to_owned()),
        );

        assert_eq!(denied.category(), Category::Internal);
        assert_eq!(denied.retry(), Retry::Never);
        assert_ne!(
            denied.category(),
            Category::Forbidden,
            "an @@allow refusal under a system context is a deploy bug, not a 403"
        );
        assert!(denied.to_string().contains("DisabledClient"), "{denied}");
        assert!(denied.to_string().contains("read"), "{denied}");

        // The public sentence must not carry the framework's message.
        let public = denied.public_message();
        assert!(!public.contains("policy denied"), "{public}");

        // And it must survive the trip through `DbError` unchanged — the
        // composite delegates rather than re-deciding (ADR-0011).
        let wrapped: DbError = denied.into();
        assert_eq!(wrapped.category(), Category::Internal);
        assert_eq!(wrapped.code(), "policy_denied");
    }

    /// A read failure carries no SQLSTATE at all, so it must land on
    /// `Storage` — the same answer `DbError::Query` gives, since that is
    /// what the sqlx read it replaces produced.
    #[test]
    fn an_untyped_read_failure_is_a_storage_outage_like_the_sqlx_read_it_replaced() {
        let backend = classify_cratestack(
            "DisabledClient",
            "read",
            CratestackError::Database("pool timed out".to_owned()),
        );

        assert_eq!(backend.category(), Category::Storage);
        assert_eq!(
            backend.category(),
            DbError::Query(sqlx::Error::PoolTimedOut).category()
        );
        assert_eq!(
            backend.code(),
            DbError::Query(sqlx::Error::PoolTimedOut).code()
        );
    }

    /// The context this crate reads under satisfies the predicate the schema
    /// names, and nothing that arrives from outside can.
    #[test]
    fn the_system_context_is_the_one_the_schema_policy_names() {
        let ctx = system_context();
        assert!(
            ctx.is_system(),
            "@@allow(\"read\", auth().isSystem()) needs this"
        );
        assert!(ctx.is_authenticated());
        assert!(
            !cratestack::CratestackContext::anonymous().is_system(),
            "if this ever passes, the kill-switch read is reachable by an unauthenticated caller"
        );
    }

    /// `classify_write`'s own contract, restated where the mirror lives: a
    /// non-database error is not an integrity violation.
    #[test]
    fn a_non_database_sqlx_error_is_not_classified_as_an_integrity_violation() {
        assert!(matches!(
            classify_write(sqlx::Error::PoolTimedOut),
            DbError::Query(_)
        ));
    }
}
