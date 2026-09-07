//! `schemas/vpay.cstack`, compiled.
//!
//! This module exists to hold one macro invocation and to keep everything it
//! expands to inside this crate. It is `mod schema;` in `lib.rs` — never
//! `pub mod` — and nothing below it is re-exported, ever.
//!
//! Why that is a rule rather than a preference, and what enforces it, is in
//! [docs/reference/vpay-db.md § CrateStack](../../../../docs/reference/vpay-db.md#cratestack).

// `include_server_schema!` expands to `pub mod cratestack_schema { … }`,
// which is why the invocation lives in a private module rather than in
// `lib.rs`: at the crate root the same expansion would publish
// `vpay_db::cratestack_schema::*` — every model struct, every delegate, and
// the generated `pub mod axum` — to every consumer, reversing ADR-0016
// standard 5 in a diff that looks like one line. `cargo xtask
// verify-repositories` fails if this module is ever made `pub` or named in a
// `pub use`, because the generated module does not exist until after macro
// expansion and a text-scanning gate would otherwise see nothing wrong.
//
// The path is resolved against `CARGO_MANIFEST_DIR`
// (`cratestack-macros-0.12.0/src/include/parse.rs:18`), not against this file,
// so it climbs out of `backends/crates/vpay-db`.
::cratestack::include_server_schema!("../../../schemas/vpay.cstack", db = Postgres);

#[cfg(test)]
mod tests {
    //! No database. Every assertion here is either a question about the
    //! **compiled** `ModelDescriptor` — `schemas/vpay.cstack` as rustc saw it
    //! — or a render of a statement (`preview_sql` does no I/O).
    //!
    //! The subject is the three models S5 added that **nothing queries**:
    //! `PaymentIntent`, `Charge` and `Refund`. Every other model in this
    //! crate is guarded by an `every_action_this_module_calls_has_an_allow_arm`
    //! beside the code that calls it. These three have no such code, so the
    //! guard has nowhere else to live — and they are the models where the
    //! guard has to point the *other* way, because what would be wrong is an
    //! arm appearing, not one going missing.

    use sqlx::postgres::PgPoolOptions;

    use super::cratestack_schema::{self, models};

    /// A pool that has never opened a connection, and cannot: the port is
    /// unroutable. `connect_lazy` does no I/O, and neither does
    /// `preview_sql`. [`crate::disabled_clients`]' device.
    fn lazy_cratestack() -> cratestack_schema::Cratestack {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("a lazy pool parses its URL and connects to nothing");
        cratestack_schema::Cratestack::builder(pool).build()
    }

    /// `model PaymentIntent`, `model Charge` and `model Refund` carry **no
    /// `@@allow` arm at all**, and every generated read on them therefore
    /// answers zero rows.
    ///
    /// Both halves are asserted, because they are different failures. The
    /// descriptor half says the schema still declares nothing; the rendered
    /// half says what Postgres would actually be sent — an allow list that is
    /// empty makes `push_allow_policy_query` emit the literal `FALSE`
    /// (`cratestack-sqlx-0.12.0/src/query/support/policy.rs:52-55`), which is
    /// spliced into the statement's own `WHERE`.
    ///
    /// WHY THIS IS PINNED IN THE DIRECTION IT IS. Everywhere else in this
    /// crate the hazard is an `@@allow` going *missing*, silently turning a
    /// working read into `Ok(None)`. Here it is the reverse: these three
    /// models exist so the drift report can compare four money tables column
    /// by column, and **no query on them runs through CrateStack** — the row
    /// structs all carry a `jsonb` column the models must not declare (see
    /// `no_generated_read_on_a_money_table_can_carry_its_jsonb_column`). An
    /// arm added here would be a standing permission over `payment_intents`,
    /// `charges` and `refunds` with no caller asking for it, and nothing else
    /// in `just ci` would say a word. If a query on one of these tables ever
    /// does move, move the assertion with it and say so in
    /// `docs/reference/vpay-db.md` § "The money tables".
    #[tokio::test]
    async fn the_three_money_models_answer_no_rows_to_every_action() {
        let cs = lazy_cratestack();

        for (name, read, create, update, delete) in [
            (
                "PaymentIntent",
                models::PAYMENT_INTENT_MODEL.read_allow_policies,
                models::PAYMENT_INTENT_MODEL.create_allow_policies,
                models::PAYMENT_INTENT_MODEL.update_allow_policies,
                models::PAYMENT_INTENT_MODEL.delete_allow_policies,
            ),
            (
                "Charge",
                models::CHARGE_MODEL.read_allow_policies,
                models::CHARGE_MODEL.create_allow_policies,
                models::CHARGE_MODEL.update_allow_policies,
                models::CHARGE_MODEL.delete_allow_policies,
            ),
            (
                "Refund",
                models::REFUND_MODEL.read_allow_policies,
                models::REFUND_MODEL.create_allow_policies,
                models::REFUND_MODEL.update_allow_policies,
                models::REFUND_MODEL.delete_allow_policies,
            ),
        ] {
            for (action, policies) in [
                ("read", read),
                ("create", create),
                ("update", update),
                ("delete", delete),
            ] {
                assert!(
                    policies.is_empty(),
                    "model {name} grew an @@allow(\"{action}\", …) arm. Nothing in vpay-db \
                     queries this model — every statement on payment_intents, charges and \
                     refunds is raw sqlx, because each row struct carries a jsonb column the \
                     model cannot declare. A permission with no caller is one nobody reviewed \
                     for a caller; if a query moved, move this assertion with it"
                );
            }
        }

        // And the statement that would be sent. `system_context()` is the
        // strongest context vpay can produce, so if even that renders FALSE
        // then nothing reads these tables through CrateStack.
        let ctx = crate::persistence::system_context();
        for (name, sql) in [
            (
                "payment_intents",
                cs.payment_intent().find_many().preview_scoped_sql(&ctx),
            ),
            ("charges", cs.charge().find_many().preview_scoped_sql(&ctx)),
            ("refunds", cs.refund().find_many().preview_scoped_sql(&ctx)),
        ] {
            assert!(
                sql.contains("FALSE"),
                "a generated read on {name} no longer renders the deny-by-default FALSE, so \
                 some @@allow arm is now admitting rows on a money table nothing queries: {sql}"
            );
        }
    }

    /// No generated read on the three money tables can carry the `jsonb`
    /// column its row struct needs — which is the single measured reason
    /// nothing on those tables moved in S5.
    ///
    /// `PaymentIntentRow` carries `payment_method_types` and `metadata`,
    /// `ChargeRow` carries `provider_ref_extra`, `RefundRow` carries
    /// `metadata`. None is declared, so none is in the projection, so a
    /// generated read cannot build any of the three structs.
    ///
    /// **This test is designed to fail when the blocker is lifted**, exactly
    /// as `the_events_insert_cannot_move_until_a_json_column_can_be_modelled`
    /// is. The second half is why declaring the columns is not the answer on
    /// its own: `Value::from_plain_json` routes every JSON number through
    /// `Number::as_i64()` and falls back to `as_f64()`
    /// (`cratestack-core-0.12.0/src/value.rs:95-106`), so a number outside
    /// `i64` comes back as a float. `payment_intents.metadata` and
    /// `refunds.metadata` are **merchant-authored** and echoed back verbatim
    /// on the wire, which makes that demotion a value vpay changed without
    /// being asked.
    ///
    /// If this goes red because `map_scalar` learned `jsonb`, read the GAP
    /// notes in `schemas/vpay.cstack` before deleting it: a `map_scalar` fix
    /// does NOT close the number-precision half, and the second assertion
    /// below is the one that says so.
    #[tokio::test]
    async fn no_generated_read_on_a_money_table_can_carry_its_jsonb_column() {
        let cs = lazy_cratestack();

        for (table, sql, columns) in [
            (
                "payment_intents",
                cs.payment_intent().find_many().preview_sql(),
                ["metadata", "payment_method_types"].as_slice(),
            ),
            (
                "charges",
                cs.charge().find_many().preview_sql(),
                ["provider_ref_extra"].as_slice(),
            ),
            (
                "refunds",
                cs.refund().find_many().preview_sql(),
                ["metadata"].as_slice(),
            ),
        ] {
            assert!(
                sql.contains(&format!("FROM {table}")),
                "this test is no longer looking at a {table} read: {sql}"
            );
            for column in columns {
                assert!(
                    !sql.contains(column),
                    "`{column}` is in the generated {table} projection now, so the row struct \
                     this crate returns could be built from a generated read. Before moving \
                     any query on this table, read the second assertion in this test and the \
                     GAP note in schemas/vpay.cstack: mapping the type is not the same as \
                     round-tripping the values: {sql}"
                );
            }
        }

        // The upstream gap itself, asserted rather than cited, so this test
        // is the thing that notices the day it closes. `i64::MAX as u64 + 1`
        // is the smallest JSON number `as_i64()` refuses; a merchant may put
        // it in `metadata` and expect it back.
        let too_big = serde_json::json!({ "merchant_ref": u64::try_from(i64::MAX).unwrap() + 1 });
        let round_tripped = ::cratestack::Value::from_plain_json(too_big.clone()).to_plain_json();
        assert_ne!(
            round_tripped, too_big,
            "`Value::from_plain_json` round-trips a JSON number outside i64 losslessly now. \
             That is the measured blocker that kept every query on payment_intents, charges \
             and refunds raw — re-read docs/reference/vpay-db.md § \"The money tables\", and \
             if it is genuinely closed, declare the jsonb columns and move the reads"
        );
    }
}
