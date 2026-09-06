//! The `disabled_clients` kill-switch query layer (`backends/migrations/
//! 0012_create-disabled-clients.sql`).
//!
//! YAML stays authoritative for client *identity* (does this client exist,
//! what is its key — ADR-0003, ADR-0010); this table only ever *subtracts*
//! access, letting an operator revoke a compromised client instantly,
//! without a deploy. A correct "may this client authenticate right now"
//! answer needs both sources, per ADR-0010's Consequences section — this
//! module answers only the database half.
//!
//! All three of this module's statements run through CrateStack since
//! 2026-09-06 — the read since the day before it — and this is the only
//! module in `vpay-db` where that is true. Why this table went first, why the
//! writes waited for the read to be proven, and what a missing `@@allow`
//! costs each of the three, is in
//! [docs/reference/vpay-db.md § CrateStack](../../../../docs/reference/vpay-db.md#cratestack).
//!
//! # Caching — deliberately none, argued
//!
//! [`DisabledClients::is_client_disabled`] is a plain `SELECT` on every call, with no cache in
//! front of it. That is a considered choice, not an oversight:
//!
//! - **A per-request `SELECT` is simplest and always correct.** It reads the
//!   one row that matters, by primary key, every time — there is no window
//!   in which the answer can be stale.
//! - **A cache directly undoes the one thing this table exists to provide.**
//!   The entire point of `disabled_clients` (this module's own header, and
//!   ADR-0010) is that revocation is *instant* — "an operator flips a client
//!   to disabled and it takes effect immediately, no deploy required." Any
//!   cache with a nonzero TTL reintroduces exactly the deploy-speed problem
//!   this table was built to avoid: a compromised merchant key gets disabled
//!   at 2am, and every replica that already cached "not disabled" keeps
//!   accepting it for the rest of the TTL. A revocation mechanism that is
//!   merely "faster than a deploy" is a much weaker claim than "instant,"
//!   and this repository does not get to silently narrow that claim.
//! - **The query this replaces is genuinely cheap.** `client_id` is the
//!   table's primary key, so this is an index-only point lookup against a
//!   table that — per this table's own migration header — holds one row per
//!   *disabled* client only, not one per registered client; in the common
//!   case (an operator has disabled nothing) the table is empty and Postgres
//!   answers from a couple of cached pages. It is called on the same request
//!   path as `private_key_jwt` signature verification (RSA/EC), which costs
//!   far more CPU per call than an indexed point lookup costs in network
//!   round-trip time — the kill-switch check is not the bottleneck a cache
//!   would be solving for.
//!
//! If a future load test shows this lookup actually dominates token-issuance
//! latency, the answer is a *short*, explicitly-bounded cache (seconds, not
//! minutes) built and justified at that point against a measured cost — not
//! a change made speculatively here.

use crate::error::DbError;
use crate::persistence::{classify_cratestack, system_context};

/// The `.cstack` model `is_client_disabled` reads, named once so the error a
/// denial produces and the query that produced it cannot drift apart.
const MODEL: &str = "DisabledClient";

#[async_trait::async_trait]
pub trait DisabledClients: Send + Sync {
    /// Reports whether `client_id` currently has a `disabled_clients` row.
    ///
    /// See the module doc comment for why this is a plain, uncached `SELECT` on
    /// every call rather than a cached lookup.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Persistence`] if the lookup itself fails — **not**
    /// [`DbError::Query`], which is what this said until 2026-09-06 and what
    /// the hand-written `SELECT EXISTS` produced. The read runs through
    /// CrateStack now, and `FindUnique::run` stringifies its `sqlx::Error`
    /// into `CratestackError::Database` rather than carrying a SQLSTATE, so
    /// every failure of this query arrives as
    /// `PersistenceError::Backend`. It still classifies `Category::Storage`
    /// with the code `database_query_failed`, so a caller branching on the
    /// classification rather than on the variant sees no change; a caller
    /// matching the variant would have silently stopped matching.
    async fn is_client_disabled(&self, client_id: &str) -> Result<bool, DbError>;

    /// Disables `client_id`, with an optional free-text operator note.
    ///
    /// Idempotent at this layer via `INSERT ... ON CONFLICT (client_id) DO
    /// UPDATE`: a raw duplicate `INSERT` is rejected by the database's own
    /// primary key (proven by
    /// `backends/tests/integration/tests/postgres_smoke.rs`'s
    /// `a_duplicate_disabled_client_id_is_rejected_by_the_database`), but an
    /// operator re-running "disable this client" — the actual caller of this
    /// function — should not have to first check whether it is already
    /// disabled. A second call updates the recorded `reason` (e.g. a fuller
    /// explanation added after the fact) and leaves the original `disabled_at`
    /// untouched, so "when was this client first disabled" stays accurate across
    /// repeated calls.
    ///
    /// That paragraph is unchanged by the move to CrateStack on 2026-09-06,
    /// and the reason it is unchanged is a measured property rather than an
    /// intention: `.upsert()` renders `ON CONFLICT (<pk>) DO UPDATE SET` over
    /// `descriptor.upsert_update_columns`, which
    /// `cratestack-macros`' `model/descriptor/columns.rs` builds by dropping
    /// the primary key and every `@default(...)` column — so `disabled_at`,
    /// which carries `@default(now())`, is not in the `SET` list and a second
    /// disable cannot move it. `reason` is, so a second disable overwrites it
    /// with whatever was passed, including `None`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Persistence`] if the write fails — **not**
    /// [`DbError::Query`], which is what this said until 2026-09-06 and what
    /// the hand-written statement produced. A caller branching on the
    /// classification sees no change (a `23505` is still `Category::Conflict`
    /// with `resource_conflict`, because `classify_cratestack` and
    /// `classify_write` are asserted against each other in
    /// `persistence.rs`); a caller matching the variant would have silently
    /// stopped matching. One classification is genuinely new here and could
    /// not arise before: a model policy refusing the write reaches this as
    /// [`crate::PersistenceError::Denied`] → `Category::Internal`, because
    /// the only context this crate writes under is a `SystemContext` and a
    /// refusal therefore means the schema and this call site disagree.
    async fn disable_client(&self, client_id: &str, reason: Option<&str>) -> Result<(), DbError>;

    /// Re-enables `client_id` by removing its `disabled_clients` row.
    ///
    /// A no-op, not an error, if `client_id` was not disabled to begin with —
    /// "make sure this client is enabled" is naturally idempotent as a `DELETE`.
    /// That contract is why the implementation is `delete_many().where_(pk)`
    /// and not `delete(pk)`, which is the shape the rest of this crate would
    /// lead you to expect; see the comment on the implementation for the
    /// measurement behind it.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Persistence`] if the write fails — see
    /// [`Self::disable_client`]'s `# Errors` for why this stopped being
    /// [`DbError::Query`] on 2026-09-06 and what a caller can observe.
    async fn enable_client(&self, client_id: &str) -> Result<(), DbError>;
}

#[async_trait::async_trait]
impl DisabledClients for crate::repository::PgRepositories {
    async fn is_client_disabled(&self, client_id: &str) -> Result<bool, DbError> {
        // The one query in this crate that runs through CrateStack today.
        // `find_unique` is a `SELECT … WHERE client_id = $1 LIMIT 1` with the
        // model's `@@allow` clauses compiled into the same `WHERE`, so a row
        // the policy does not admit is indistinguishable from a row that is
        // not there — which is exactly why
        // `a_disabled_client_reads_the_same_through_both_paths` compares this
        // against a direct sqlx read instead of merely asserting it does not
        // error. Deleting the `@@allow("read", …)` line from
        // `schemas/vpay.cstack` turns this function into a permanent "no
        // client is disabled", and that test is the only thing that notices.
        //
        // `.is_some()` rather than reading a column: the question is whether
        // a row exists, and the row's own fields (`disabled_at`, `reason`)
        // are operator context this trait deliberately does not return — see
        // the module doc.
        self.cs
            .disabled_client()
            .find_unique(client_id.to_owned())
            .run(&system_context())
            .await
            .map(|row| row.is_some())
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))
    }

    async fn disable_client(&self, client_id: &str, reason: Option<&str>) -> Result<(), DbError> {
        // The statement this replaces, byte for byte:
        //   INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2)
        //   ON CONFLICT (client_id) DO UPDATE SET reason = EXCLUDED.reason
        // `.upsert()` renders exactly that (plus a `RETURNING` the model
        // projection), because `CreateDisabledClientInput` carries only the
        // columns without a `@default(...)` — `disabled_at` is excluded from
        // both the insert list and `upsert_update_columns`, so the column
        // default fires on insert and the value survives every later call.
        // `.preview_sql()` on this builder prints it if you want to read it
        // back.
        //
        // Two round trips where there was one: `.upsert()` probes the
        // conflict target with `SELECT ... FOR UPDATE` inside its own
        // transaction before running the statement, to tell a create from an
        // update for the event/audit fan-out this model has neither of. That
        // is CrateStack's fixed shape for `upsert` and it is accepted rather
        // than worked around: an operator disabling a client is not a hot
        // path (unlike `is_client_disabled`, which is on the token path and
        // stayed a single `find_unique`).
        //
        // Unlike the read, a policy mistake here is LOUD. `upsert_exec.rs`
        // evaluates `create_allow_policies` before any SQL runs and returns
        // `CratestackError::Forbidden` when the list is empty, and
        // `upsert_resolve.rs` gates the update branch the same way — so
        // deleting `@@allow("create", ...)` or `@@allow("update", ...)` from
        // `model DisabledClient` fails this call rather than silently writing
        // nothing. `"upsert"` is the action name in the error because either
        // slot can be the one that refused; CrateStack's own sentence, which
        // `classify_cratestack` keeps as `detail`, names which.
        let input = crate::schema::cratestack_schema::CreateDisabledClientInput {
            client_id: client_id.to_owned(),
            reason: reason.map(ToOwned::to_owned),
        };

        self.cs
            .disabled_client()
            .upsert(input)
            .run(&system_context())
            .await
            .map(|_row| ())
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "upsert", error)))
    }

    async fn enable_client(&self, client_id: &str) -> Result<(), DbError> {
        // `delete_many().where_(client_id = $1)` and NOT `delete(client_id)`,
        // which is the builder the primary key invites and the one the design
        // note for this change originally named. The reason is this trait's
        // own documented contract — "a no-op, not an error, if `client_id`
        // was not disabled to begin with" — and it is measured, not guessed:
        // `cratestack-sqlx`'s `write/delete_exec.rs` runs
        // `DELETE ... WHERE pk = $1 AND (<policy>) RETURNING ...` and, when
        // nothing comes back, returns `CratestackError::Forbidden("delete
        // policy denied this operation")`. It cannot tell "the policy refused
        // you" from "there was no such row", so `.delete()` would turn every
        // enable of an already-enabled client into a `Category::Internal`
        // error and break `disabled_client_lookup_reflects_disable_and_enable`
        // and `find_client_reflects_the_disabled_clients_kill_switch`.
        // `delete_many` reports `BatchSummary { total: 0, .. }` instead, which
        // is what "no-op" means.
        //
        // The price, stated plainly because it is the opposite of the upsert
        // above: `delete_many`'s policy clause is part of the `WHERE`
        // (`push_action_policy_query`), so deleting `@@allow("delete",
        // auth().isSystem())` makes this remove zero rows and return `Ok` —
        // silently, exactly as a missing `read` policy does. It is caught by
        // the enable half of
        // `a_client_disabled_through_cratestack_is_visible_to_both_paths`,
        // which reads the row back through a raw `SELECT` afterwards, and the
        // mutation was run. The failure direction is at least the safe one: a
        // client that should have been re-admitted stays revoked.
        //
        // The count is deliberately dropped rather than returned or asserted
        // on. It is 0 or 1 — `client_id` is the primary key — and requiring 1
        // is the same thing as making this call fail on an absent row, which
        // is what the paragraph above rejects.
        self.cs
            .disabled_client()
            .delete_many()
            .where_(crate::schema::cratestack_schema::disabled_client::client_id().eq(client_id))
            .run(&system_context())
            .await
            .map(|_summary| ())
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "delete", error)))
    }
}

#[cfg(test)]
mod tests {
    //! What the two writes render to, asserted without a database.
    //!
    //! `UpsertRecord::preview_sql` is a *render* of the conflict-bearing
    //! statement, not a transcript of the executed path — the real call wraps
    //! it in a `SELECT … FOR UPDATE` probe, may run an `ON CONFLICT DO
    //! NOTHING` first (`upsert_resolve.rs`), and evaluates the create/update
    //! policies outside the statement, none of which appears here. What it
    //! *is* exact about is the part this crate had to reason about to keep
    //! [`DisabledClients::disable_client`]'s documented behaviour: the insert
    //! column list and the `DO UPDATE SET` list, both taken from the same
    //! `ModelDescriptor` the executed statement uses.
    //!
    //! `backends/crates/vpay-db/tests/repositories.rs` is where a real
    //! Postgres proves the behaviour these strings only imply.

    use sqlx::postgres::PgPoolOptions;

    /// A pool that has never opened a connection, and cannot: the port is
    /// unroutable. `connect_lazy` does no I/O, and neither does
    /// `preview_sql`, so this whole module is a string comparison wearing a
    /// database's clothes.
    fn lazy_cratestack() -> crate::schema::cratestack_schema::Cratestack {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("a lazy pool parses its URL and connects to nothing");
        crate::schema::cratestack_schema::Cratestack::builder(pool).build()
    }

    /// The upsert must render the statement it replaced, not merely something
    /// that also works.
    ///
    /// The hand-written `INSERT … ON CONFLICT (client_id) DO UPDATE SET
    /// reason = EXCLUDED.reason` was removed on 2026-09-06; this is the
    /// assertion that the generated one is the same statement. Two properties
    /// carry the trait's documented contract and neither is obvious from the
    /// call site:
    ///
    /// - `disabled_at` is **not** in the insert column list, so the column
    ///   default `now()` fires — the same reason the old statement named two
    ///   columns.
    /// - `disabled_at` is **not** in the `DO UPDATE SET` list, which is what
    ///   makes "a second disable leaves the original `disabled_at` untouched"
    ///   true. `upsert_update_columns` drops every `@default(...)` column.
    ///
    /// **What this test does and does not catch, measured rather than
    /// asserted.** Removing `@default(now())` from `disabled_at` in
    /// `schemas/vpay.cstack` does not reach this test at all: it also adds
    /// `disabled_at` to `CreateDisabledClientInput`, and `disable_client`'s
    /// struct literal (no `..Default::default()`, deliberately) then fails to
    /// compile — `error[E0063]: missing field `disabled_at``, run on
    /// 2026-09-06. The two filters in `model/descriptor/columns.rs` and
    /// `model/inputs.rs` are the same predicate, so the insert list and the
    /// `SET` list can never diverge from each other on a schema edit.
    ///
    /// What is left for this test, and it is the reason it exists: the
    /// rendering belongs to a **pinned external crate**. `cratestack-sqlx`
    /// 0.11.1's `upsert.rs` builds `DO UPDATE SET {col} = EXCLUDED.{col}`;
    /// a version that rendered `COALESCE(EXCLUDED.reason, reason)` instead,
    /// or that stopped honouring the column default on insert, would change
    /// `disable_client`'s observable behaviour with no diff in this
    /// repository at all. That is what a version bump can do silently and
    /// what a string comparison is good for.
    #[tokio::test]
    async fn the_upsert_renders_the_statement_the_hand_written_one_was() {
        let cs = lazy_cratestack();
        let input = crate::schema::cratestack_schema::CreateDisabledClientInput {
            client_id: "acme".to_owned(),
            reason: Some("key compromised".to_owned()),
        };

        let sql = cs.disabled_client().upsert(input).preview_sql();

        assert!(
            sql.starts_with(
                "INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2) \
                 ON CONFLICT (client_id) DO UPDATE SET reason = EXCLUDED.reason"
            ),
            "the generated upsert is no longer the statement it replaced: {sql}"
        );
        assert!(
            !sql.contains("disabled_at = "),
            "`disabled_at` must never be assigned by the upsert — a second disable would move \
             the timestamp that records when the client was FIRST disabled: {sql}"
        );
        assert!(
            !sql.contains("(client_id, disabled_at, reason)"),
            "`disabled_at` must be left to the column default on insert: {sql}"
        );
    }

    /// The delete this crate does *not* use, recorded so the reason survives.
    ///
    /// `.delete(pk)` renders a perfectly good statement — and
    /// `cratestack-sqlx`'s `delete_exec.rs` turns "it matched no row" into
    /// `CratestackError::Forbidden`, which is why `enable_client` is
    /// `delete_many` instead (see the implementation's comment). This asserts
    /// the shape that makes the two builders different: the single-row delete
    /// targets the primary key and can return exactly one row, so "zero rows"
    /// is the only signal it has left for a policy refusal.
    ///
    /// If a future CrateStack gives `.delete()` a way to say "no such row"
    /// without saying "forbidden", `enable_client` should move to it — and
    /// this test is the note that explains why it did not already.
    #[tokio::test]
    async fn the_single_row_delete_targets_the_primary_key_and_is_deliberately_unused() {
        let cs = lazy_cratestack();

        let sql = cs.disabled_client().delete("acme".to_owned()).preview_sql();

        assert!(
            sql.starts_with("DELETE FROM disabled_clients WHERE client_id = $1"),
            "{sql}"
        );
        assert!(
            !super::MODEL.is_empty(),
            "the model name the error carries must not be blank"
        );
    }
}
