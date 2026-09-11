# `vpay-db` reference

Why the code in `backends/crates/vpay-db` looks the way it does. The crate's
own doc comments say _what_ each item is and link here; this page carries the
reasoning, the invariants and the history that a reader needs once — not on
every `cargo doc` build.

Tier: an [ADR](../adr/) records a decision, a [flow](../flows/) describes a
process, and a reference page like this one explains why a particular piece of
code is shaped the way it is. The migrations under `backends/migrations/` are
the schema's own record and are never edited (ADR-0003); this page is the
application's side of them.

- [The repository seam](#the-repository-seam)
  - [Why a trait object and not a generic parameter](#why-a-trait-object-and-not-a-generic-parameter)
  - [Why the transaction API is a closure](#why-the-transaction-api-is-a-closure)
  - [Why `PendingTransaction` owns its `sqlx` transaction](#why-pendingtransaction-owns-its-sqlx-transaction)
  - [What stays `pub`, and why](#what-stays-pub-and-why)
- [The rest of this reference](#the-rest-of-this-reference) — the per-table
  pages under [`vpay-db/`](vpay-db/)
- [TLS: no `CryptoProvider` is installed here](#tls-no-cryptoprovider-is-installed-here)
- [CrateStack](#cratestack) — a pointer to
  [vpay-db/cratestack.md](vpay-db/cratestack.md) and its four sibling pages
- [Migration 0038: `rate_limit_windows`, and the one model with no `@@allow`](#migration-0038-rate_limit_windows-and-the-one-model-with-no-allow)

_This list was a full table of contents of a 3 330-line page until 2026-09-11.
It had also been wrong since 2026-09-06: it offered "`providers` cannot be
written through CrateStack, and that is a maintainer's decision" as an anchor,
and the heading it pointed at had been renamed to "`providers` is written
through CrateStack (D7, resolved 2026-09-06)" when the decision was taken. An
in-page anchor that stops resolving is a broken link `verify-links` cannot
report, because it strips the fragment before it checks the path — which is the
reason the entries below name pages rather than headings._

---

## The repository seam

Every table family exposes one `#[async_trait]` trait (`Charges`, `Jobs`, …)
whose methods are the queries that family owns. `Repositories` is the umbrella
every consumer holds — `&dyn Repositories` in `vpay-api`'s router state and in
every `vpay-worker` handler — and `PgRepositories` is its only implementation,
built by `connect`. Nothing this crate exports takes a `PgPool`, and `PgPool`
is no longer re-exported at all.

ADR-0006 is untouched by any of it: `PgRepositories` is the sole
implementation, tests construct it against real Postgres, and no fake is
written.

### Why a trait object and not a generic parameter

`<R: Repositories>` would appear on every axum handler, on `AppState`, on
`RouterDeps` and on every job handler's signature. `dyn` costs one boxed future
per query on a path that is already awaiting Postgres — the same trade
`vpay_provider::ProviderAdapter` already documents and accepts (ADR-0002).

### Why the transaction API is a closure

`UnitOfWork::transaction` hands the closure a `&mut dyn TxRepositories` and
decides `COMMIT`/`ROLLBACK` from what the closure returns, so "forgot to
commit" is not expressible and no `sqlx` type leaves this crate.
`vpay-worker`'s `Cargo.toml` records that no-`sqlx` rule as deliberate; before
this seam existed it was a comment, and seven call sites spelled `pool.begin()`
anyway.

`TxOutcome` exists because a successful closure has two endings, not one:
`vpay_worker::webhooks`' fan-out loses a race to another drain and must roll
back _without_ that being an error, and the confirm path's duplicate-charge
recovery abandons its transaction and re-reads outside it. Encoding either as an
`Err` would push "not a failure" through the error channel, which is the shape
ADR-0011 exists to prevent.

`TxOutcome::Abandon` does not surface a rollback failure: it logs at `warn!` and
returns `Ok(Abandon)`. `ROLLBACK` is best-effort by construction, so a failure
changes nothing about the database and only about what the caller may report —
and both abandoning call sites have an answer that must survive (the confirm
path's `409`, and `persist_submitted`'s "a rail may hold a live payment"
alert). It is staged in `tests/postgres.rs` by terminating the backend that
holds the open transaction, with the commit path as the control.

The closure is generic over its error type rather than pinned to `DbError`.
Three call sites raise their _own_ layer's error from inside the unit of work —
the confirm path's "the rail accepted a charge whose intent moved" invariant
(`ApiError`) and two worker sites whose payload will not encode (`JobError`).
Pinning the closure to `DbError` would have forced each of them either to
smuggle the error out through the success channel or to relabel it as storage,
which is the exact shape ADR-0011 exists to stop. The signature is
`transaction<'a, T, E, F>(&self, f: F) -> Result<TxOutcome<T>, E>` with
`E: From<DbError> + Send`, so the common case is still spelled `E = DbError` and
each call site names its error type once.

### Why `PendingTransaction` owns its `sqlx` transaction

Not an aesthetic choice, and it looks like an easy simplification. The closure
signature `for<'t> FnOnce(&'t mut (dyn TxRepositories + 'a)) -> TxFuture<'t, _>`
is only usable because the `'a` on the trait object gives the implied bound
`'a: 't`, and that is what lets a closure borrow the caller's locals
(`&NewCharge`, a `&str` merchant id) across an `.await`. With a borrowing
`PgTransaction<'t>` the same signature forces every capture to be `'static`,
which no call site in this workspace can satisfy.

`PendingTransaction` carries no public method beyond the trait, so a caller
outside this crate can obtain one from `TransactionSource::begin_transaction`
and do nothing with it but hand it back — which is what makes `PgRepositories`
the only usable implementation of `TransactionSource`, and therefore of
`Repositories`, without a sealed-trait dance. Dropping it rolls back.

### What stays `pub`, and why

The table-family modules stay `pub` for the row and seed types, the two
`provider_requests` sentinels a test asserts on, and each family's own trait.
They hold no `pub fn` any more — a query is reached through `Repositories`,
never through a free function that would need a `PgPool` to call.

`lock_keys` is `pub` for one reason of its own: a test that wants to prove a
writer actually takes its lock has to be able to _hold_ that lock from outside
(`reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`), and an
operator reading `pg_locks` needs the values to be findable from a crate doc
rather than by grepping for a hex literal.

`Migrations` is a trait on `Repositories` rather than a free function taking a
pool. `run_migrations` is not a table family and had no home once `PgPool`
stopped leaving the crate; making it a fourteenth trait is what let
`pub use sqlx::PgPool` go, where keeping one `pub fn` taking a pool would have
kept the whole re-export alive for one caller.

`connect_lazy` is `#[doc(hidden)]` and mechanically guarded. `connect` is
deliberately eager, which makes "a handle whose queries fail" unobtainable, and
`vpay-api`'s own unit tests need exactly that to prove an unreachable database
produces a refusal rather than an admission. It is **not** a test double — the
pool is the real `sqlx` one and every query really reaches Postgres — which is
why nothing in ADR-0006's dependency rules would object to a binary using it.
`cargo xtask verify-no-mocks` nonetheless fails the build if it appears in
non-test code anywhere under `backends/apps`.

`client_assertion_store` is the one place a _foreign_ trait
(`authkestra_op::client_assertion::ClientAssertionStore`) is implemented over
the database, and it was the precedent this seam followed.

It returns `impl ClientAssertionStore` rather than a named type, and that is
the seam again rather than a flourish: [ADR-0016](../adr/0016-engineering-standards.md)
standard 5 says an implementation is private to this crate and reached through
its trait, and until 2026-09-05 this store was the exception — it was `pub` and
`vpay-api` wrote `SqlClientAssertionStore::new(pool)` by name. `cargo xtask
verify-repositories` is what keeps it that way now. A free function rather than
a method on `Repositories`, because the trait it implements is authkestra's and
putting an authkestra type on vpay's umbrella trait would make the worker —
which mints no tokens — depend on the OP's vocabulary.

## The rest of this reference

`docs/reference/vpay-db.md` was 3 330 lines until 2026-09-11. It is the overview
now; the per-table reasoning and the CrateStack record are on the pages below,
moved **verbatim**. Nothing was summarised away and no dated measurement or
correction was dropped.

| Table or subject                                                    | Page                                                                                                 |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `payment_intents`, `checkout_sessions`                              | [vpay-db/payment-intents-and-checkout-sessions.md](vpay-db/payment-intents-and-checkout-sessions.md) |
| `customers`, `invoices`                                             | [vpay-db/customers-and-invoices.md](vpay-db/customers-and-invoices.md)                               |
| `charges`, `settlement`                                             | [vpay-db/charges-and-settlement.md](vpay-db/charges-and-settlement.md)                               |
| `events`, `refunds`, `webhook_deliveries`                           | [vpay-db/events-refunds-and-webhooks.md](vpay-db/events-refunds-and-webhooks.md)                     |
| `jobs` — the lease, the dead letter, `pull_forward_in_tx`           | [vpay-db/jobs.md](vpay-db/jobs.md)                                                                   |
| Dynamic SQL strings and sqlx 0.9, and the audit behind the rule     | [vpay-db/dynamic-sql.md](vpay-db/dynamic-sql.md)                                                     |
| CrateStack — the seam, and what compiling the schema does not prove | [vpay-db/cratestack.md](vpay-db/cratestack.md)                                                       |
| CrateStack — what runs through it today                             | [vpay-db/cratestack-what-runs-through-it.md](vpay-db/cratestack-what-runs-through-it.md)             |
| CrateStack — `currencies`, `providers`, decision D7                 | [vpay-db/cratestack-currencies-and-providers.md](vpay-db/cratestack-currencies-and-providers.md)     |
| CrateStack — the money tables                                       | [vpay-db/cratestack-money-tables.md](vpay-db/cratestack-money-tables.md)                             |
| CrateStack — the `procedure` seam                                   | [vpay-db/cratestack-procedure.md](vpay-db/cratestack-procedure.md)                                   |

## TLS: no `CryptoProvider` is installed here

The root `Cargo.toml`'s comment on the `authkestra-*` dependencies documents a
real requirement: those crates build `reqwest` clients with `rustls-no-provider`,
so the _first_ one constructed panics unless a process-wide default
`CryptoProvider` was already installed. `sqlx` looks like the same hazard and is
not.

`sqlx` is configured with `tls-rustls-ring`, which vendors Mozilla's CA bundle
via `webpki-roots` (the runtime image is `FROM scratch` per ADR-0004, so there
is no OS trust store for the `-native-roots` alternative to read). Reading
`sqlx-core`'s own TLS setup (`src/net/tls/tls_rustls.rs`) shows it never
calls `rustls::crypto::CryptoProvider::get_default()` — the call that panics
without an installed default. It builds its own provider inline and passes it
explicitly:

**Re-read at 0.9.0 on 2026-09-05**, because a claim about a dependency's
internals is exactly what a major bump invalidates. Unchanged: `handshake`
still selects `rustls::crypto::ring::default_provider()` under
`_tls-rustls-ring-webpki` (the feature `tls-rustls-ring` expands to) and still
hands it to `builder_with_provider`. The `aws_lc_rs` arm next to it is
`cfg`-ed out by that same feature, and `deny.toml` bans the crate outright, so
both halves of the invariant are checked.

```text
let provider = Arc::new(rustls::crypto::ring::default_provider());
let config = ClientConfig::builder_with_provider(provider.clone())...
```

`builder_with_provider` never consults the process-wide default, so a `sqlx`
Postgres connection negotiating TLS cannot hit the "no default installed" panic
regardless of whether `install_default()` was ever called anywhere in the
process. **So this crate does not call `install_default()`, deliberately.** The
requirement in the root `Cargo.toml` is real but belongs to the dashboard-auth
work, and each binary installs the provider at boot
([vpay-config.md](vpay-config.md#the-boot-sequence), step 2).

---

## CrateStack

The schema in `schemas/vpay.cstack` compiles into this crate's private
`mod schema` (`just check-schema`, and since 2026-09-06 `cargo build`). What
runs through it, what stays raw `sqlx` forever, why the generated module is
private, and what compiling the schema does **not** prove are all on:

- [vpay-db/cratestack.md](vpay-db/cratestack.md) — the section that used to be
  here, including "What stays raw sqlx, and why", "Why the generated module is
  private", "What compiling the schema does not prove" and the error, context
  and dependency-graph subsections
- [vpay-db/cratestack-what-runs-through-it.md](vpay-db/cratestack-what-runs-through-it.md)
  — the transaction seam measured rather than argued, `events.data`, the event
  vocabulary hazard, and the per-action cost of a missing policy
- [vpay-db/cratestack-currencies-and-providers.md](vpay-db/cratestack-currencies-and-providers.md)
  — migrations 0032 and 0033, `reconcile`, and decision D7
- [vpay-db/cratestack-money-tables.md](vpay-db/cratestack-money-tables.md) —
  migration 0037, the `jsonb` blocker, and the ten report lines that are false
- [vpay-db/cratestack-procedure.md](vpay-db/cratestack-procedure.md) — the
  `procedure` seam and `searchPaymentIntents`, the schema's first, 2026-09-11

_This heading is deliberately still called "CrateStack": four doc comments in
`backends/crates/vpay-db/src/` link to `vpay-db.md#cratestack`, and an anchor
that stops resolving is a broken link no gate would report, because
`verify-links` strips fragments before it checks a path._

## Migration 0038: `rate_limit_windows`, and the one model with no `@@allow`

### What runs the statement, added 2026-09-10 by the exp36 review

`count_attempt` is one statement, and three of the things it decides are
decided **inside** it — whether the window has elapsed, what the answer resets
to, and which rows the sweep takes. As delivered, nothing exercised any of
them against a database. `vpay_api::staff::rate_limit`'s unit tests cover
`Verdict::of`'s arithmetic over an integer the statement hands back, the
statement's own test asserts the _text_ of six fragments, and every case over
a booted server runs inside one 300-second window. **A `CASE` that never reset
would have passed all of them** — and its symptom in production is a staff
member locked out of the dashboard for good by ten wrong passwords, which is
the durable lockout ADR-0017 refuses by name.

Three cases in `vpay-db/tests/repositories.rs` now run it, each with a
measured mutation:

| Case                                                                   | Mutation                                          | As mutated                                                                                                     |
| ---------------------------------------------------------------------- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `an_elapsed_rate_limit_window_is_replaced_rather_than_extended`        | `attempts = attempts + 1`, dropping the reset arm | reads `5` where it demands `1`                                                                                 |
| `the_rate_limit_table_grows_by_one_window_and_is_then_swept`           | the sweep takes nothing                           | 1040 rows where it demands ≤ 60                                                                                |
| `the_rate_limit_id_is_the_budget_and_the_scope_column_is_only_a_label` | —                                                 | pins that `scope` separates nothing, so a caller that stopped hashing it into the `id` would merge two budgets |

### The bound, stated as it actually is

Migration 0038 and this module both say a caller spending fresh keys "drains
the table faster than they fill it". That is true **in the limit** and not
true inside one window: nothing has elapsed yet, so there is nothing to sweep.
Measured — 1000 fresh keys inside one window leave **1000 rows**, and 40
attempts past the boundary take the table back down.

So the honest bound is **two rows per attempt for the width of one window,
then flat**: at _r_ attempts per second and a 300-second window, roughly
`2 × r × 300` rows at steady state. It is a bound, which is what the objection
ADR-0017 raised needed answering; it is not "the table never grows".

**172 changes over 24 relations, unmappable 19** (from 167 / 23 / 19). The +5
is four hand-named CHECKs and one index, and **not one column-level line** —
no `column … type differs`, no `column … default value differs`, no undeclared
column. `model RateLimitWindow` declares all five columns and the table was
shaped so it could: `TEXT`, `TIMESTAMPTZ` and `BIGINT` only, no `jsonb`, no
`bytea`, no `int4`, no native enum, no `DEFAULT`.

That is the floor a modelled table can reach in this schema at 0.12.0, and it
is worth stating why the two remaining classes are permanent rather than
outstanding: the grammar has no declaration for a hand-named `CHECK` and none
for an `INDEX`, so each is invisible to the schema in one direction and
visible to the live introspection in the other. `attempts` is `BIGINT` and not
`INT` for exactly the reason `currencies.exponent` had to be widened by
migration 0032 after the fact — `int4` is one of the types `map_scalar`
declines, and a column it declines is excluded from the comparison outright.
`EXPECTED_UNMAPPABLE_COLUMNS` is unmoved at 19, which is the assertion that
makes the +5 mean what it says.

### Why the model carries no `@@allow` arm

Every other modelled table in this file has four arms, or (for the money
tables) none because nothing queries them. This one has none for a third
reason, and it is narrower: **the statement has no builder, not the row.**

```text
INSERT INTO rate_limit_windows (…) VALUES (…)
ON CONFLICT (id) DO UPDATE SET attempts = … + 1 …
RETURNING attempts
```

A CrateStack 0.12.0 `UpdateRateLimitWindowInput` carries **values**;
`attempts = attempts + 1` is an **expression over the row's own column**.
There is no generated builder that renders it, and the alternative —
`find_unique`, decide, `update_many` — is precisely the read-then-write race
this table exists to remove: two replicas reading 9 both write 10 and both
admit.

So `vpay_db::rate_limits` holds one hand-written `sqlx` statement, a
`const &'static str` with no `format!` and therefore nothing for
`crate::sql_audit` to audit, and
`the_rate_limit_window_model_answers_no_rows_to_every_action` pins the arms'
absence in the direction that matters. An arm appearing here would be a
standing permission over a table with **no caller at all** — and this is the
one table in the schema whose keys an unauthenticated caller chooses.

### The three properties of the statement, and what each one stops

They are asserted by name in
`the_statement_keeps_the_three_properties_that_make_it_safe`, because none of
them is obvious from the text and each is a plausible tidy-up:

| Fragment                 | Deleting it                                                                                                                               |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `id <> $1`               | lets the in-statement sweep name the row the `INSERT` is about, through one snapshot                                                      |
| `FOR UPDATE SKIP LOCKED` | lets two concurrent sign-in attempts take the same row locks in different orders — a deadlock, and a `500` for somebody typing a password |
| `LIMIT $5`               | makes the first attempt after an idle period pay for every row that accumulated during it                                                 |
| `attempts + 1`           | makes the limiter count to one forever and admit everything                                                                               |

The sweep removes up to 32 elapsed rows per call against the two one attempt
adds, which is what bounds a table an attacker fills by choosing fresh keys.
The key itself is the SHA-256 of `<action>:<dimension>:<value>`, so the value
— an email address that, on the interesting attempts, has no account — is
never written down.
