# Verification log — 2026-09-12, the erasure's idempotency window

Last verified: 2026-09-12, on `claude/issue-111`, based on `7914fc2a`
(`origin/master`). [Issue #111](https://github.com/vaam-apps/vpay/issues/111).

The issue has two halves and only one of them is code. This page is about the
first; the second — whether the merchant agreement obliges a merchant to erase
their own copy on `customer.deleted`, and within what window — is a
maintainer's decision, is **not taken**, and is written out with its options
and their costs in
[flows/customers/privacy-and-erasure.md](../../flows/customers/privacy-and-erasure.md)
§ "The decision, written out — NOT TAKEN".

## What this claims

That the window existed, that it was **demonstrated before it was closed**,
that it is now closed, and that the case proving it fails when the fix is
removed. Nothing here claims the merchant's own copy is reachable; it is not,
and no change in this commit pretends otherwise.

## The window, reproduced

`PostRequest::finish` stores a response **after** the handler's transaction has
committed, so the two writes are not one atom. A `POST /v1/customers/{id}` that
commits, is overtaken by a `DELETE` that erases the same payer — sweeping every
stored copy that exists at that moment — and only then stores its own response
writes the pre-erasure object into `idempotency_keys.response_body`, which
nothing moves again until `expires_at`. Replaying that request's own
`Idempotency-Key` re-reads it: up to **24 hours** after the erasure.

The reproduction is
`an_update_that_loses_the_race_to_an_erasure_stores_no_payer_identifier` in
`backends/tests/integration/tests/customers.rs` — the real router, the real
handlers, a real Postgres. The interleaving is pinned with **row locks the test
itself takes**, so nothing in the shipping code is aware of it and no seam was
added:

1. the test locks the customer's row, so the update's transaction cannot start;
2. the update is sent, claims its key and blocks there;
3. the test locks the update's own `idempotency_keys` row — the row its
   `finish` will write. In production the delay between committing and storing
   is scheduling, a saturated pool or a slow round trip; here it is a lock;
4. the `DELETE` is sent and queues **behind** the update on the customer's row,
   so releasing the first lets the update commit and the erasure run second;
5. the erasure redacts every stored copy it can see — and cannot see the
   update's row, whose `response_body` is still NULL;
6. the test releases the claimed row and the update stores its response.

Both erasure branches run: one customer with a payment intent (**anonymised**,
the row survives with the marker on it) and one without (**hard-deleted**,
there is no row at all).

### Before the fix

```
$ cargo nextest run -p vpay-tests-integration --test customers \
    an_update_that_loses_the_race_to_an_erasure_stores_no_payer_identifier --no-capture

thread '…' panicked at backends/tests/integration/tests/customers.rs:2325:
round anonymised: `Zeruiah Vexlingham anonymised` is back in
idempotency_keys.response_body after the erasure, where it can be replayed for
24 hours: {"address":null,"created":1789243322,
"email":"zeruiah.anonymised@example.invalid","id":"cus_ndxzqvvzy51rk335tzz1spr8",
"livemode":false,"metadata":{"race":"lost"},
"name":"Zeruiah Vexlingham anonymised","object":"customer","phone":"237600000774"}

test result: FAILED. 0 passed; 1 failed; 0 ignored; 23 filtered out
```

Those are the bytes a replay re-emits verbatim (`vpay_api::v1::payment_intents`'
`replay` writes `record.response_body` as it stands), so the wire half follows
from the stored half rather than being a separate claim.

## What closes it

`vpay_db::Idempotency::store` takes a `StoredResponse`, whose `subject` is one
of two:

- `ResponseSubject::Verbatim` — every route but `/v1/customers`. One statement
  on the pool, byte for byte the statement that was there before;
- `ResponseSubject::Customer { id }` — the three customer routes, which pass
  the `cus_…` they already hold. The write runs in a transaction that first
  reads the payer's row `FOR SHARE`, and if that payer is gone runs
  `vpay_db::customers::redact_stored_responses_in_tx` over the body it just
  stored.

The share lock is the half that matters. Every erasure takes `FOR UPDATE` on
that row first (`lock_for_update`, and the sweep's `erase_idle`), so the two
serialise in whichever order they arrive: either the erasure goes first and
this write sees it, or this write goes first and the erasure's own sweep finds
the row. Without the lock the read could be true and the write could land on
the far side of an erasure committing in between — the same race, one statement
smaller. **No row means erased**, which is the hard-delete branch.

Three things it deliberately is not:

- it does not inspect a body. The id comes from the route — a path segment, or
  the one a create minted — so a response that is not a customer object is
  carried through and left alone by the redaction's own `object`/`id` match;
- it does not define what a redacted body is. It runs `vpay_db::customers`'
  statement, the same one the erasure runs — one site, not two, which is why
  `cargo xtask`'s SQL-interpolation audit still expects **61**
  `AssertSqlSafe` sites and not 62;
- it does not change what the racing request is **answered**. That request
  committed before the erasure; answering it with a redacted object would
  report a state its own transaction did not commit. The test asserts the live
  response still names the payer, so a "fix" that redacted the wrong copy fails
  it.

### After the fix

```
$ cargo nextest run -p vpay-tests-integration --test customers \
    an_update_that_loses_the_race_to_an_erasure_stores_no_payer_identifier --no-capture

test an_update_that_loses_the_race_to_an_erasure_stores_no_payer_identifier ... ok
Summary [85.588s] 1 test run: 1 passed, 23 skipped
```

(85 s on a host running four other agents' builds; 1.76 s in the suite run
below, where the container was not contended.)

### The mutation

`store` made to treat `ResponseSubject::Customer` exactly as `Verbatim` — no
row lock, no redaction — and reverted afterwards:

```
thread '…' panicked at backends/tests/integration/tests/customers.rs:2330:
round anonymised: `Zeruiah Vexlingham anonymised` is back in
idempotency_keys.response_body after the erasure, where it can be replayed for
24 hours: {…,"name":"Zeruiah Vexlingham anonymised",…,"phone":"237600000774"}

Summary [80.639s] 1 test run: 0 passed, 1 failed, 23 skipped
```

The mutation is named in the test's own doc comment, so the next reader does
not have to work out what would prove it.

## Measured

Every command below was run on this branch, scoped per crate rather than as
`just ci`: four other agents were building on the same host and a workspace
build has OOM-killed it before. **`just ci` has therefore NOT been run on this
commit**, and that is a gap in this page rather than a claim about one — the
gates below are the ones a change to these files can move.

| Command                                                                                       | Result                                |
| --------------------------------------------------------------------------------------------- | ------------------------------------- |
| `cargo nextest run -p vpay-tests-integration --test customers`                                | **24 passed, 0 skipped** (66.997 s)   |
| `cargo nextest run -p vpay-tests-integration --test payment_intents --test checkout_sessions` | **56 passed, 0 skipped** (302.072 s)  |
| `cargo nextest run -p vpay-tests-integration --test invoices --test postgres_smoke`           | **55 passed, 0 skipped** (133.287 s)  |
| `cargo nextest run -p vpay-db`                                                                | **187 passed, 0 skipped** (287.942 s) |
| `cargo nextest run -p vpay-api`                                                               | **362 passed, 0 skipped** (2.364 s)   |
| `cargo clippy -p vpay-db -p vpay-api --all-targets -- -D warnings`                            | exit 0                                |
| `cargo test --doc -p vpay-db`                                                                 | **8 passed, 0 ignored**               |
| `cargo test --doc -p vpay-api`                                                                | **18 passed, 0 ignored**              |
| `just verify`                                                                                 | **twelve gates, all ok**              |

The three integration suites other than `customers` are there because the
`Verbatim` arm is every route they exercise: if the refactor had changed what a
payment intent, a checkout session, an invoice or an invoice item stores, they
are where it would show.

`--test-threads 2` was needed for the `customers` suite: on a host this busy,
twenty-four simultaneous `testcontainers` starts time out at 120 s, which is a
property of the host and not of any test. Both failures of that kind are
recorded here rather than quietly retried away.

`just verify`, gate by gate on this commit. The only number that moved is
`verify-links`: 1 483 → 1 487 links in 315 → 316 tracked files — this page (the
new file and its three relative links) plus the index entry for it in
[README.md](../README.md). The links added to `flows/customers.md` and to this
directory's sibling `status/backend.md` were already in the earlier count,
because those files were tracked and modified rather than new.

```
verify-no-mocks     ok — no test double reachable from a shipping binary
verify-status       ok — 1 unimplemented item, declared and still in shipping code
verify-errors       ok — 19 error types, all classified; 16 `#[from]` variants delegate
verify-sdk-parity   ok — 469 proving tests, 31 dated gaps, 32 methods across 35 rows
verify-links        ok — 1487 links in 316 tracked markdown files
verify-npm-scope    ok — 2 publishable packages, 1 private
check-schema        ok — 26 declarations (WARNING: cratestack 0.11.1 on PATH, repo pins 0.12.0)
verify-serde        ok — 90 serialisable types, 16 exemptions
verify-repositories ok — 4 implementations, named by none of the 83 files outside vpay-db
verify-toolchain    ok — 1.98.0
verify-migrations   ok — 42 migration files match MANIFEST.sha256
verify-docs         advisory report
```

`check-schema`'s warning is the authoring machine's, unchanged by this branch
and already recorded in [docs/status.md](../../status.md).

**One thing in the advisory report moved and it is worth naming**:
`vpay_api::v1::payment_intents`' `finish` is now **81 lines** and so joins the
"production functions of 80 lines or more" list, which has 24 entries. The
fifth parameter and the `StoredResponse` it builds are what pushed it over. It
is a report and not a gate, and the alternative — a helper taking the same six
values under another name — would have moved lines rather than reduced them.
Moving in the other direction, `vpay_db::customers::redact_stored_copies` is
**99 lines**, down by the statement that became
`redact_stored_responses_in_tx`.

## What did not change

No migration, no schema, no wire contract. `idempotency_keys` is the table it
was; `ResponseSubject` and `StoredResponse` are new Rust types with no
serialised form. The one thing a reader of the old code will notice is that
`PostRequest::finish` takes a fifth argument and every one of its seventeen
call sites names it — fourteen `Verbatim`, three `Customer`. That verbosity is
the point: a fourth customer route added next year has to say which it is.

## The adversarial review of the same day, and the one thing it changed

Everything above was written by the agent that made the change. This section is
the review of it, on the same branch, and it is here rather than in a second
file because a review that lands in its own page is a page nobody reads beside
the claim it is about.

### What was attacked and held

Four mutations, each built and run against a real Postgres on this branch.

| Mutation                                                                                | Result                                                                               |
| --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Drop ` FOR SHARE` from `customers::erased_under_share_lock`, leaving a plain read       | `an_update_that_loses_the_race…` **FAILS**, with the payer's name in `response_body` |
| The same, with the loop's rounds reversed so the hard-delete branch runs first          | **FAILS** on the hard-delete round too                                               |
| Treat `ResponseSubject::Customer` exactly as `Verbatim` (the pre-fix behaviour)         | **FAILS**, byte-for-byte the failure recorded above                                  |
| The same, rounds reversed                                                               | **FAILS** on the hard-delete round too                                               |
| Redact the response the racing request is **answered** with, as well as the stored copy | **FAILS** at the live-response assertion, `customers.rs:2290`                        |

The first of these is the one worth naming. The share lock is the half of the
argument a reader is most likely to take on trust, and it is **not** taken on
trust: the store's read races the erasure's `FOR UPDATE` for the row the
update's own transaction has just released, and a plain read wins that race and
answers "not erased". The lock is what loses it. The recorded mutation ("treat
`Customer` as `Verbatim`") removes the lock and the redaction together; this
one removes only the lock, and the case still goes red.

Both rounds were checked separately because the case `panic!`s on the first
round that fails, so a single mutated run can only ever demonstrate one of
them — the claim "both rounds fail" above is true, and was not observable in
the run that was quoted for it.

Also checked, and found to be as described: every erasure path takes
`FOR UPDATE` on the payer's row first and there are exactly two
(`vpay_api::v1::customers::delete` through `lock_customer_for_update`, and the
retention sweep's `Customers::erase_idle`); no `DELETE FROM customers` exists
outside CrateStack's `hard_delete`, which runs inside the locked transaction;
the pool sets no isolation level, so `FOR SHARE` behaves as the argument
assumes under READ COMMITTED; three customer routes write a customer body and
`/v1`'s other objects render a `cus_…` and no payer identifier; the
interpolation audit really does still count **61** sites (66 `AssertSqlSafe(`
occurrences in `vpay-db/src`, five of them inside `sql_audit.rs`'s own needles
and messages) with `ALLOWED_NON_CONSTANTS` untouched at two entries; and
nothing in the change implements any part of the merchant-copy decision — no
`customer.redacted` type exists anywhere in the tree, and the diff adds no
migration, column, endpoint, dashboard surface or window constant.

`PostRequest::finish` at **81 lines** was left at 81. It is a linear sequence
with no nesting and it grew by one parameter and a four-field struct literal;
splitting it to get under an advisory threshold would move lines rather than
remove them.

### What did not hold: the store redacted more than the row it wrote

Four places — the commit message, `idempotency`'s module comment,
`flows/customers/privacy-and-erasure.md` and this page — say the store redacts
**the body it just stored**. It did not. It ran the erasure's own statement,
whose `WHERE` is `merchant_id` plus the body's `object`/`id`, so it rewrote
every stored response naming that payer across the whole merchant.

That is not a leak, and the race is closed either way. It is three other
things:

- **an unbounded cost on a path any caller can reach.** The customer arm runs
  whenever the payer is gone, and "gone" includes _never existed_: a
  `POST /v1/customers/{cus_that_is_not_real}` answers `404`, and a `404` is a
  `4xx`, so it is stored. Each one scanned every key that merchant had used in
  the last 24 hours, so the cost of one request grew with the number of
  requests before it, and the caller chooses how many;
- **a deadlock that did not exist before.** The statement takes a row lock on
  every match. Two stores for the same erased payer each hold the row they
  wrote and scan towards the other's, in whatever order the plan reads them.
  Postgres would abort one; the transaction rolls back, so the payer stays
  redacted and what is lost is the response, not the guarantee. Not staged —
  the mechanism is the finding;
- **a wider claim than four documents make.**

`redact_stored_responses_in_tx` now takes `only_key: Option<&str>`, in the same
statement (`AND ($5::TEXT IS NULL OR idempotency_key = $5)`, so the audit still
counts 61 and the two sides of the race still cannot spell the redaction
differently). The erasure passes `None` and means every copy. `store` passes
`Some(key)` and fixes the row it wrote, which is the only row it can have put
the payer back into: everything older was either swept by the erasure under its
`FOR UPDATE`, or belongs to another `store` answering for itself under the same
share lock.

Both directions are pinned by a mutation:

| Mutation                                                                 | Result                                                                                                                                           |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `store` passes `None` — equivalently, the narrowing is reverted          | nothing fails; this is a cost and a lock-footprint change, not a behavioural one                                                                 |
| The **erasure** passes `Some("arm-i111r-no-such-key")` instead of `None` | `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table` **FAILS**, naming five literals surviving in `idempotency_keys.response_body` |
| ` FOR SHARE` dropped, on top of the narrowing                            | `an_update_that_loses_the_race…` still **FAILS**                                                                                                 |

The first row is stated rather than hidden: **no test distinguishes the narrow
scope from the wide one**, because the difference is which rows are locked and
scanned and not which rows end up redacted. The argument for it is the one
above, and the evidence that it did not break anything is the third row plus
the suites below.

### Also fixed: two references to a heading that no longer exists

`flows/customers.md` named the erasure page's "Two windows the erasure does not
close" twice — in the directory entry and in the sentence that tells a reader
which section bounds every privacy claim on the page. The commit renamed that
heading to "One window the erasure does not close, and one that used to be two"
and left both references behind. `verify-links` checks a destination path and
never a heading ([AGENTS.md](../../../AGENTS.md) says so), so this was not a
gate failure and would not have become one.

### Measured, after the review's changes

| Command                                                            | Result                                |
| ------------------------------------------------------------------ | ------------------------------------- |
| `cargo fmt --all --check`                                          | exit 0                                |
| `cargo clippy -p vpay-db -p vpay-api --all-targets -- -D warnings` | exit 0                                |
| `cargo test -p vpay-db --lib`                                      | **77 passed, 0 ignored**              |
| `cargo test -p vpay-api --lib`                                     | **362 passed, 0 ignored**             |
| `cargo nextest run -p vpay-tests-integration --test customers`     | **24 passed, 0 skipped** (64.073 s)   |
| `cargo nextest run -p vpay-db --test repositories`                 | **106 passed, 0 skipped** (232.151 s) |
| `cargo nextest run -p vpay-tests-integration --test invoices`      | **16 passed, 0 skipped** (37.119 s)   |
| `cargo xtask verify-links`                                         | ok — 1489 links in 316 files          |
| `cargo xtask verify-status`                                        | ok — 1 unimplemented item, declared   |
| `just fmt-check-web`                                               | ok — on node 22.23.2                  |

`verify-links` counts 1 489 and not the 1 487 recorded further up: the
difference is the two relative links this section adds, both to
[AGENTS.md](../../../AGENTS.md).

`just ci` was **not** run by the review either, for the reason the section
above gives, and the same caveat applies: the gates here are the ones a change
to these files can move, and that is a narrower claim than a green CI run. The
`customers` suite was run with `--test-threads 2`, as the earlier run was; no
`testcontainers` start-up timed out during the review's runs, so there is
nothing of that kind to record here.
