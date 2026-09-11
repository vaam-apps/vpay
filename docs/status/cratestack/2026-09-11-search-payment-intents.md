# CrateStack — the first `procedure`: `searchPaymentIntents` (2026-09-11)

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### The first `procedure`: `searchPaymentIntents` (2026-09-11, exp54)

**A tested library call with no caller, and that is the first thing to read
on this row.** `schemas/vpay.cstack` declares
`procedure searchPaymentIntents(page: PageInput, filter:
PaymentIntentListFilter): Page<PaymentIntentSummary>`, and
`backends/crates/vpay-db/src/schema/search_payment_intents.rs` implements the
`ProcedureRegistry` method it generates, against the real `payment_intents`
table. **Nothing serves it.** No CrateStack axum router and no RPC dispatcher
is mounted anywhere in this workspace; `persistence::system_context` is the
only `CratestackContext` vpay mints and it carries no tenant, so the body
would refuse it. `GET /dash/v1/payment_intents` is untouched and remains the
only payments list a dashboard can reach. `Payments` is
`cfg_attr(not(test), allow(dead_code))` in consequence, which is the gap made
visible rather than papered over.

**Why a procedure could read a table the models cannot.** The row above says
`model PaymentIntent` carries no `@@allow` arm, so every generated read on it
renders `FALSE`. A procedure does not go through a generated read at all: the
macro emits the signature, the `Args` struct, the policy check and the
`Authorized` witness, and the body is ours
(`cratestack-macros-0.12.0/src/procedure.rs`,
`generate_procedure_registry_method`). The statement is one hand-written
`&'static str`. `the_three_money_models_answer_no_rows_to_every_action` is
unchanged and still green: no arm was added to any money model.

**Three things this cost that the guides do not mention, each measured on
2026-09-11 against the vendored 0.12.0 sources:**

- **`procedure listPaymentIntents` does not compile, and `cratestack check`
  says so.** Every model generates `handle_{list,create}_<plural>` and
  `handle_{get,update,delete}_<singular>` into the axum module whether or not
  anything routes them, so those five stems are taken for all eighteen models;
  the CLI reports the collision by name (`validate/procedure_handler_
collisions.rs`) rather than leaving rustc to say `error[E0428]`. Hence
  `search`.
- **Enum fields ARE `FindMany`-filterable in 0.12.0.** `model/
find_many_where.rs:35-38`'s `is_filterable_scalar` returns true for any
  declared enum — "Enum fields (cratestack#928) are included" — and only the
  _ordering_ operators exclude them (`find_many_where_push.rs:24-28`). So
  "`status` is an enum, therefore `FindMany` cannot filter it" is false for
  this release. What actually rules `FindMany<PaymentIntent>` out is that the
  only thing to do with one is `build_payment_intent_query_from_find_many`,
  which runs the deny-by-default model read — plus that declaring it would
  publish `eq`/`startsWith`/`lt`/`gt` over `client_secret_suffix`, a prefix
  oracle on a live payer credential this surface refuses to render. The
  filter input is therefore hand-rolled, with exactly `/dash/v1`'s three
  fields.
- **`Value::from_plain_json`'s number demotion does not reach the procedure
  reply path** (`cratestack-core-0.12.0/src/rpc.rs:187` encodes with
  `serde_json::to_value`; the two callers of `from_plain_json` are both in the
  model row decoder). **But a `Json` field declared in a procedure's return
  type maps to `::cratestack::Json<::cratestack::Value>`**
  (`procedure/type_tokens.rs:31`), so constructing one from merchant JSON
  reintroduces the demotion by hand. `PaymentIntentSummary` therefore carries
  no `metadata` and no `payment_method_types` — nor `client_secret_suffix`,
  nor `seq`.

**Offset pagination, and it is additive rather than a replacement.**
`PageInput` is `{ limit, offset }` and `Page<T>` is
`{ items, total_count, page_info }` (`cratestack-core-0.12.0/src/page.rs`);
`total_count` comes from a `count(*) OVER ()` in the same statement, so the
total and the page cannot disagree — except past the end of the set, where
there is no row to carry it and the answer is `null` rather than `0`.
`/dash/v1`'s cursor paging is deliberately untouched: it shares
`crate::v1::paging` with the merchant API so `has_more` means one thing to an
operator and to a merchant, and changing that is a maintainer's decision.
**The ceiling is 100, a copy of `vpay_api::v1::paging::MAX_LIMIT` that nothing
gates into agreement** — `vpay-db` cannot see that constant. If one moves,
move the other.

**Three corrections to the paragraph above, from the exp54 review
(2026-09-11). All three are in the branch as landed; they are recorded rather
than silently folded in, because two of them were claims this page made before
they were true.**

1. ~~`total_count` is `null` past the end of the set~~ — it was also `null`
   for an **empty first page**, which is a different fact and a worse answer.
   At `offset == 0` the statement asked for `limit + 1` rows starting at the
   first one, so an empty result _proves_ the filtered set is empty: the
   answer is `0`, and `page_of` now says so. An operator filtering by a status
   they have none of was previously told the total was unknown.
   `the_envelope_reports_the_page_it_actually_has` pins both arms; the
   container test's `canceled` page asserts `Some(0)` rather than `None`.
2. **The ceiling was a copy of `MAX_LIMIT`; the _default_ was not a copy of
   anything.** `PageInput::resolve(max)` defaults an absent `limit` to `max`,
   so this surface answered 100 rows where `GET /dash/v1/payment_intents`
   answers 10 (`DEFAULT_LIMIT`) — the exact "ten times the page the REST list
   does" the constant's own comment ruled out, arriving through the default
   instead of the ceiling. `DEFAULT_PAGE_LIMIT` (10) is now separate and
   `resolve_page` supplies it before handing `resolve` the clamp untouched.
3. **Offset paging is not stable while payments are being created, and
   nothing here said so.** `payment_intents_seq_key` (migration `0014`) is a
   UNIQUE index, so `ORDER BY seq DESC` is a total order and a single page is
   never ambiguous — that is the whole of what it buys. `OFFSET` is counted
   afresh on every call, so an intent created between page 1 and page 2 shifts
   the window: the last row of a page reappears at the top of the next one,
   and a row is missed at the tail for every insert behind the caller's back.
   New rows land at offset 0, so it is worst on the busiest merchant.
   **Nothing compensates for it and nothing should pretend to** — a
   reconciliation, an export or a sum reads `/v1`'s cursor list, which
   `starting_after` anchors to a `seq` an insert cannot move. This is the
   honest price of the numbered table `Page<T>` exists to give.

**Twelve tests, 0 ignored, and the three mutations were run rather than
described.** Eleven are no-database; one starts a container. (Eleven tests
until the exp54 review added `the_two_failure_code_vocabularies_are_one_
vocabulary`; every count in this section was re-measured on that review's
head rather than carried over.) All three mutations were re-run after the
review's own changes and still redden the same named tests — the tenancy one
with `merchant_a`'s page coming back as
`["pi_b_only", "pi_a_new", "pi_a_old"]`, which is the leak itself rather than
a proxy for it.

| mutation                                                                      | what goes red                                                                                                                   |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `WHERE merchant_id = $1` → `WHERE ($1::TEXT IS NOT NULL)`                     | `the_page_is_the_tenants_own_rows_filtered_and_bounded`: `merchant_a`'s page comes back `["pi_b_only", "pi_a_new", "pi_a_old"]` |
| `resolve_page(args.page)` → `(limit.unwrap_or(DEFAULT), offset.unwrap_or(0))` | the same test, with Postgres' own `OFFSET must not be negative`                                                                 |
| `validated_status`'s vocabulary check → `if true`                             | `an_unknown_status_is_refused_rather_than_answered_with_an_empty_page`                                                          |

The second mutation leaves `a_hostile_limit_is_clamped_before_it_reaches_postgres`
**green** — it pins `resolve_page`'s contract, not the body's use of it
— which is why both tests exist and why the module doc says so.

**One vocabulary had no test and now does (exp54 review).**
`the_two_failure_code_vocabularies_are_one_vocabulary` pins
`schemas/vpay.cstack`'s `enum FailureCode` against `vpay_core::FailureCode`,
as the status pair was already pinned. It was missing and the asymmetry
mattered: an unknown `status` is a caller's typo and answers `400`, but an
unknown `last_payment_error_code` is a **stored** value and answers
`Internal` — so a variant present in one transcription and not the other
turns every payment that failed for that reason into a `500` on an operator's
list. `search_payment_intents.rs` is the only consumer of
`types::FailureCode` in the workspace, so the schema's copy had no other
reader to disagree with.

**What is NOT claimed.** No transport, no dashboard call, no generated client.
No `@@audit`, for the row above's reason — this is a read. The filter set is
the three `/dash/v1` already has and not one more, so there is still no search
by payer phone (the column is never written) and none by id prefix (no index).
`total_count` is `null` for a page past the end of the set. And the procedure
has never been exercised through `cratestack-axum`'s or the RPC dispatcher's
own decode path, because neither is mounted: what is proven is the body, the
policy check and the witness, called the way a transport would call them.
