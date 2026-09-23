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

// The bodies of this schema's `procedure` declarations — one today. It is a
// child of *this* module rather than a sibling of it because the
// `ProcedureRegistry` trait it implements lives inside the expansion above,
// which is private here and stays that way (ADR-0016 standard 5).
//
// **No longer `#[cfg_attr(not(test), allow(dead_code))]`, as of Lane C
// (docs/plans/2026-09-13-dashboard-nav-notes/transport.md).** That attribute
// said, in so many words, "nothing in any shipping binary calls this
// procedure" — and said to delete it "the day something serves the
// procedure". `dashboard_procedure_router` below is that caller:
// `Payments` is constructed by the line building the procedure router
// (`cratestack_schema::axum::procedure_router(..., Payments, ...)`) in every
// build, not only under `cfg(test)`, so the struct is no longer dead outside
// tests and the lint has nothing to silence.
//
// ~~`docs/status.md` § "The first `procedure`" says the same thing in the
// same words, updated in this commit.~~ **Struck by the Lane C review,
// 2026-09-13: both halves of that sentence were false.** `docs/status.md`
// was not touched by the commit that deleted the attribute, and it has had
// no § "The first `procedure`" since it was cut from 6 151 lines to 259 on
// 2026-09-11. The attribute's own instruction — "update `docs/status.md` in
// the same commit" — was written before that split; `docs/status.md`
// § "Where a new row goes" now sends a change of this kind to the area page
// instead, which is `docs/status/cratestack.md`. That page, and the dated
// `docs/status/cratestack/2026-09-13-dashboard-procedure-transport.md` it
// indexes, *were* updated in that commit — so the documentation duty was
// discharged, on the pages that now own it, and only this sentence naming
// the wrong page was wrong.
mod search_payment_intents;
// The body of `procedure searchWebhookDeliveries` (Lane D, slice: webhook
// deliveries). A sibling of `search_payment_intents` for the reason its own
// module doc gives: the `ProcedureRegistry` trait lives inside the private
// expansion above, so every procedure body has to be a child of this module.
// `search_payment_intents::Payments`'s `impl ProcedureRegistry` delegates to
// `search_webhook_deliveries::search` — see that file's own header for the
// join-based tenancy predicate this slice's table needs and
// `search_payment_intents.rs` does not.
mod search_webhook_deliveries;

// The body of `procedure searchCustomers` — `search_payment_intents`'
// sibling for `model Customer`. `pub(super)` on its one free function, not a
// second `ProcedureRegistry` impl: the trait requires exactly one
// implementer per schema (`Payments`, in `search_payment_intents.rs`), so
// this module's contribution is a function that `Payments`' own
// `search_customers` method delegates to, thinly, rather than a struct of
// its own.
mod search_customers;

// The body of `procedure searchCheckoutSessions` (Lane D, slice: checkout
// sessions). A sibling of `search_customers` and declared the same way — a
// plain `mod` here rather than a `#[path]` child of `search_payment_intents`,
// because it imports nothing private from that file. Only `search_refunds`
// needs the `#[path]` form, and its own comment says why.
mod search_checkout_sessions;

/// Mounts `searchPaymentIntents` over HTTP — the read-only CrateStack
/// transport this schema has never had a caller for before Lane C
/// (docs/plans/2026-09-13-dashboard-nav-notes/transport.md).
///
/// **`procedure_router`, never `router()`.** The generated `router()`
/// merges `model_router(...)` — the CRUD CrateStack generates for every one
/// of this schema's twenty models (nineteen until `ManualPayment`, #251),
/// creates/updates/deletes included,
/// whether or not anything routes it — with `procedure_router(...)`
/// (`cratestack-macros-0.12.0/src/include/server/axum_module/router_fn.rs`).
/// `procedure_router` is the same generated function with that merge
/// removed, emitted `pub` with the same arguments minus
/// `body_limit_bytes` (`axum_module.rs:138`), so "reads only" is expressed
/// by calling a different generated function rather than by trusting a
/// route table.
///
/// **`auth_provider` never produces a system context.** This function takes
/// whatever `cratestack::AuthProvider`-implementing value its caller built
/// ([`crate::dashboard_transport::ExtensionAuthProvider`], in practice) and
/// hands it straight to CrateStack; it never calls
/// [`crate::persistence::system_context`], the only place in this crate
/// that can produce a context for which `is_system()` is true, and this
/// file does not call it.
///
/// `resolvers: ()` because `schemas/vpay.cstack` declares no `@computed`
/// field — CrateStack's own macro emits `impl ComputedFieldResolver for
/// ()` exactly when that is true
/// (`cratestack-macros-0.12.0/src/include/server.rs:139`), so there is no
/// hand-written resolver to maintain.
pub(crate) fn dashboard_procedure_router<Auth>(
    db: cratestack_schema::Cratestack,
    auth_provider: Auth,
) -> ::cratestack::axum::Router
where
    Auth: ::cratestack::AuthProvider,
{
    cratestack_schema::axum::procedure_router(
        db,
        search_payment_intents::Payments,
        (),
        ::cratestack_codec_json::JsonCodec,
        auth_provider,
    )
}

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
    ///
    /// `pub(super)` so `search_payment_intents`' own tests can take one:
    /// that module's `the_tenancy_refusal_happens_before_the_statement_does`
    /// depends on the pool being *unreachable*, so a second copy of this
    /// helper would be a second copy of the one property it relies on.
    pub(super) fn lazy_cratestack() -> cratestack_schema::Cratestack {
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

        // The `FROM` clauses are literals rather than `format!("FROM {table}")`
        // — `crate::sql_audit`'s scanner reads every `format!` in this crate
        // as a statement being built and refuses an interpolation that is not
        // a crate constant. It is right to: the cheapest way past it is to
        // widen its allowlist, and docs/status.md records that the answer is
        // to stop using `format!` here instead.
        for (from_clause, sql, columns) in [
            (
                "FROM payment_intents",
                cs.payment_intent().find_many().preview_sql(),
                ["metadata", "payment_method_types"].as_slice(),
            ),
            (
                "FROM charges",
                cs.charge().find_many().preview_sql(),
                ["provider_ref_extra"].as_slice(),
            ),
            (
                "FROM refunds",
                cs.refund().find_many().preview_sql(),
                ["metadata"].as_slice(),
            ),
        ] {
            assert!(
                sql.contains(from_clause),
                "this test is no longer looking at a read with `{from_clause}`: {sql}"
            );
            for column in columns {
                assert!(
                    !sql.contains(column),
                    "`{column}` is in the generated projection of the read with \
                     `{from_clause}` now, so the row struct this crate returns could be built \
                     from a generated read. Before moving any query on this table, read the \
                     second assertion in this test and the GAP note in schemas/vpay.cstack: \
                     mapping the type is not the same as round-tripping the values: {sql}"
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

    /// Walks candidate paths for every one of the twenty models' generated
    /// CRUD, proving none of them is mounted through
    /// [`super::dashboard_procedure_router`] — the mechanical version of
    /// `DASH_ROUTES`' own doc's reason for keeping a route table at all:
    /// axum 0.8 cannot enumerate a built `Router`, so this probes it instead
    /// of trying to inspect it. _(This said "nineteen" until 2026-09-23, and
    /// the list below carried nineteen, while `schemas/vpay.cstack` had
    /// declared a twentieth, `model ManualPayment`, since #251. The list is
    /// now checked against the schema's own `cratestack_schema::MODELS`
    /// first, so that gap fails this test instead of hiding in it.)_
    ///
    /// **Decisive.** Changing `super::dashboard_procedure_router`'s call
    /// from `cratestack_schema::axum::procedure_router(...)` to
    /// `cratestack_schema::axum::router(...)` (the merged form that also
    /// mounts `model_router`) turns every one of the 160 route assertions
    /// below red at once — twenty models, two paths each, four methods per
    /// path. _(This said "seventy-six" until 2026-09-23, which is nineteen
    /// times four and forgets the two paths; it said "seventy-two", eighteen
    /// times four, on the day it was written. Neither count was ever the
    /// loop's.)_ Every model's `list`/`create` path
    /// (`/{plural}`) and `get`/`update`/`delete` path (`/{plural}/{id}`)
    /// would start answering `405` (a route matched, the method did not)
    /// instead of this crate's honest `404` (no route matched at all) —
    /// `cratestack-macros-0.12.0/src/axum/model/routes.rs` is where those
    /// paths and that four-method set come from, and
    /// `docs/reference/vpay-db/cratestack.md`'s "the table name is decided
    /// by the model name" is why the twenty table strings below are the same
    /// ones `backends/migrations/*.sql` names as tables.
    #[tokio::test]
    async fn no_generated_model_route_is_mounted_only_the_one_procedure_is() {
        use tower::ServiceExt as _;

        // A pool that never connects, exactly like every other test in this
        // module. Every probed request below is refused before a statement
        // could run: the twenty models' paths never match any route at
        // all, and the one real procedure is asked with a method it does
        // not serve (`GET`, never `POST`) so this test proves routing
        // without needing the lazy pool to answer anything.
        let cs = lazy_cratestack();
        let auth = crate::dashboard_transport::ExtensionAuthProvider(std::sync::Arc::new(
            |_: &::cratestack::axum::http::Extensions| Some("acme-cameroon-tenant".to_owned()),
        ));

        // `(model, table)`. The model column is checked against
        // `cratestack_schema::MODELS` — the macro's own list of every
        // `model` in `schemas/vpay.cstack`, as rustc compiled it — before a
        // request is sent, in both directions: a model the schema declares
        // and this list lacks is one whose generated CRUD this test would
        // never probe, and a name here the schema no longer declares is a
        // probe of nothing. Until 2026-09-23 this was a hand-kept
        // `[&str; 19]` of table names with no such check.
        const MODEL_TABLES: [(&str, &str); 20] = [
            ("Currency", "currencies"),
            ("Provider", "providers"),
            ("PaymentIntent", "payment_intents"),
            ("Charge", "charges"),
            ("Refund", "refunds"),
            ("CheckoutSession", "checkout_sessions"),
            ("LedgerTransaction", "ledger_transactions"),
            ("LedgerEntry", "ledger_entries"),
            ("DisabledClient", "disabled_clients"),
            ("Event", "events"),
            ("WebhookDelivery", "webhook_deliveries"),
            ("Customer", "customers"),
            ("StaffMember", "staff_members"),
            ("StaffSession", "staff_sessions"),
            ("OauthAuthorizationCode", "oauth_authorization_codes"),
            // Nineteenth, added when this branch's `model Credential`
            // (migration 0044) merged with Lane C's transport: a model
            // declared after this list was written is exactly the one whose
            // generated CRUD nobody has yet proved is unmounted.
            ("Credential", "credentials"),
            ("Invoice", "invoices"),
            ("InvoiceItem", "invoice_items"),
            // Twentieth, 2026-09-23 — the case the comment above warns
            // about, and it happened: `model ManualPayment` (migration 0049,
            // #251) was declared without being added here, and nothing
            // noticed until a skills re-verification did.
            ("ManualPayment", "manual_payments"),
            ("RateLimitWindow", "rate_limit_windows"),
        ];

        let mut listed: Vec<&str> = MODEL_TABLES.iter().map(|(model, _)| *model).collect();
        let mut declared: Vec<&str> = cratestack_schema::MODELS.to_vec();
        listed.sort_unstable();
        declared.sort_unstable();
        assert_eq!(
            listed, declared,
            "MODEL_TABLES and schemas/vpay.cstack's models disagree. Add or remove the \
             `(model, table)` pair; the table is `pluralize(to_snake_case(model))`, the name \
             the migrations create"
        );

        for (_, table) in MODEL_TABLES {
            for path in [format!("/{table}"), format!("/{table}/some_id")] {
                for method in ["GET", "POST", "PATCH", "DELETE"] {
                    let router = super::dashboard_procedure_router(cs.clone(), auth.clone());
                    let request = ::cratestack::axum::http::Request::builder()
                        .method(method)
                        .uri(&path)
                        .body(::cratestack::axum::body::Body::empty())
                        .expect("a well-formed request");
                    let response = router
                        .oneshot(request)
                        .await
                        .expect("axum's own Router::call is infallible");
                    assert_eq!(
                        response.status(),
                        ::cratestack::axum::http::StatusCode::NOT_FOUND,
                        "{method} {path} answered something other than this crate's honest \
                         404 — a generated model CRUD route may be mounted"
                    );
                }
            }
        }

        // The one path this transport *does* mount, asked with a method it
        // does not serve — so this test is not merely proving that
        // everything 404s regardless of what is mounted. CrateStack
        // procedures are `POST`-only; a `GET` on the same path matches the
        // route (`405`) rather than missing it (`404`), which is the
        // discriminator this assertion needs.
        let router = super::dashboard_procedure_router(cs, auth);
        let request = ::cratestack::axum::http::Request::builder()
            .method("GET")
            .uri("/$procs/searchPaymentIntents")
            .body(::cratestack::axum::body::Body::empty())
            .expect("a well-formed request");
        let response = router
            .oneshot(request)
            .await
            .expect("axum's own Router::call is infallible");
        assert_eq!(
            response.status(),
            ::cratestack::axum::http::StatusCode::METHOD_NOT_ALLOWED,
            "the one procedure this transport mounts must itself be reachable (a matched \
             route answering the wrong method), not merely absent"
        );
    }
}
