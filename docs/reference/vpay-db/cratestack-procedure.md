# `vpay-db` — CrateStack: the `procedure` seam, and the one procedure on it

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

### The `procedure` seam, and the one procedure on it (2026-09-11)

`schemas/vpay.cstack` declares one `procedure` since exp54, and
`src/schema/search_payment_intents.rs` is its body. Read this section before
adding a second.

**A procedure is the opposite of a delegate.** Everything above this line is
about what the generated query builders can and cannot render. A procedure
renders nothing: `include_server_schema!` emits `pub type Output`, an `Args`
struct, `ALLOW_POLICIES`/`DENY_POLICIES`, the four lifecycle helpers, an
`Authorized` witness and one method signature on a `ProcedureRegistry` trait
(`cratestack-macros-0.12.0/src/procedure.rs`). The statement is the
implementor's. That is precisely why `searchPaymentIntents` can read
`payment_intents` while "The money tables" above says no generated read can:
it does not use one. **It does not weaken that section** — no `@@allow` arm
was added to any money model, and
`schema::tests::the_three_money_models_answer_no_rows_to_every_action` still
pins their absence.

**Four constraints this seam has that the query seam does not:**

1. **The trait lives inside the private module, so the body has to as well.**
   `ProcedureRegistry` is `cratestack_schema::procedures::ProcedureRegistry`,
   and `mod schema` is private for the reason the next section gives. The
   implementation is therefore `src/schema/<procedure>.rs`, a child of
   `schema.rs` — not a table-family module beside `payment_intents.rs`, where
   it could not name the trait.
2. **A procedure name may not collide with any model's generated CRUD handler
   stem.** Every model emits `handle_{list,create}_<plural>` and
   `handle_{get,update,delete}_<singular>` into the generated axum module
   whether or not anything routes them, so `listPaymentIntents`,
   `getCustomer`, `createCharge` and their siblings are all unavailable.
   `cratestack check` reports it by name; without that check it is a raw
   `error[E0428]` from rustc. Hence `searchPaymentIntents`.
3. **A `Json` field in the return type is the demotion again.** It maps to
   `::cratestack::Json<::cratestack::Value>`
   (`procedure/type_tokens.rs:31`), so building one from merchant JSON runs
   `Value::from_plain_json` by hand. The reply encoder itself does not
   (`cratestack-core-0.12.0/src/rpc.rs:187` uses `serde_json::to_value`) —
   the hazard is entirely in constructing the value, and the answer is the
   same as everywhere else in this document: do not carry the column.
4. **The tenancy predicate is the body's, not the policy's.** A procedure
   `@allow` can say `auth() != null`, `auth().isSystem()`, `hasRole("x")` or
   `inTenant("<literal>")` — and that literal is a literal, so no policy can
   express "the caller's own tenant". `searchPaymentIntents` allows any
   authenticated caller and reads `ctx.tenant_id()` in the body, refusing
   with the byte-identical `Forbidden` message the policy itself produces so
   that "not signed in" and "no tenant" are indistinguishable.

**`Page<T>` and `PageInput` are offset pagination and nothing else** — no
cursor, no keyset (`cratestack-core-0.12.0/src/page.rs`). `PageInput::resolve`
is the clamp and the default in one call. vpay passes `MAX_PAGE_LIMIT` (100),
a copy of `vpay_api::v1::paging::MAX_LIMIT` that no gate keeps in step,
rather than CrateStack's own `MAX_LIST_LIMIT` (1000). `total_count` comes from
a `count(*) OVER ()` in the same statement, which makes it exact and
consistent with the page — and absent when the page is past the end of the
set, because the count travels on a row.

**`resolve` is the clamp _and the default_, and that second half is a trap
worth naming (2026-09-11, exp54 review).** `resolve(max)` defaults an absent
`limit` to `max`, so passing `MAX_PAGE_LIMIT` alone answers 100 rows to a
caller who asked for no page size, where `GET /dash/v1/payment_intents`
answers 10 (`vpay_api::v1::paging::DEFAULT_LIMIT`). The two surfaces are read
by the same operator; a 10× difference arriving through the default is the
same defect as one arriving through the ceiling. `search_payment_intents`
therefore supplies `DEFAULT_PAGE_LIMIT` (10) before calling `resolve`, and
hands `resolve` the clamp untouched. A second procedure using `PageInput`
must do the same or say why not.

**What offset pagination cannot promise, and what `seq` does and does not
buy.** `payment_intents.seq` carries a UNIQUE index
(`payment_intents_seq_key`, migration `0014`), so `ORDER BY seq DESC` is a
_total_ order and one statement never has a tie to break. That is the whole
of it. **`OFFSET` is counted afresh on every call**, so a payment created
between one page and the next shifts the window: a caller walking
`offset = 0, 10, 20` over a list that is being written to sees the last row
of a page again at the top of the next one, and misses a row at the tail for
every row inserted behind its back. `seq DESC` puts new rows at offset 0,
which is the direction that makes this happen on a busy merchant rather than
a quiet one. It is a property of offset pagination, not a defect in the body,
and it is the reason `/v1` and `/dash/v1` are cursor-paged: `starting_after`
resolves an id to a `seq` and asks for rows _below_ it, which an insert
cannot move. `total_count` is exact for the statement that returned it and
says nothing about the next call. Anything that must not double-count a
payment — a reconciliation, an export, a sum — reads the cursor list, not
this one.

**The statement is a `&'static str`, so no `AssertSqlSafe` site was added** and
`sql_audit`'s `EXPECTED_ASSERT_SITES` did not move. A procedure has no
`format!` to make: its filters are `$N IS NULL OR …` bind parameters, exactly
as `list_page_filtered`'s are.
