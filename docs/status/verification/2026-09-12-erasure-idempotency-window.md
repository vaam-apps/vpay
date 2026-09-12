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
