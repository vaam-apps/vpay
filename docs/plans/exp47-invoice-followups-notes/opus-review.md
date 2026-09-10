# exp47 review — the deviation upheld, and four things nothing held

Sabotage review of `claude/exp47-invoice-followups` (issue #91 items 1 and 2
delivered; 3 and 4 not attempted and still open). Rebased onto `origin/master`
at `524289b` (#105, #107, #108 had landed). Date 2026-09-11.

The branch's design survived. Its **proofs** did not, in four places, and one
number was falsified by the rebase itself. Nothing here weakens a test.

---

## The deviation: upheld, and for a stronger reason than the branch gave

The brief pre-decided that migration `0042` would amend
`paid_means_nothing_remaining` so a refunded paid invoice is storable, with the
decisive mutation "drop the CHECK amendment → the storable test FAILS". The
branch did not amend it and recorded why: under a **gross** `amount_refunded`
the amendment is vacuous and its mutation cannot fire.

That was reproduced against a real Postgres (`postgres:16-alpine`, every
migration applied, a table populated with rows in every status). The shipped
constraint is `CHECK (status <> 'paid' OR amount_remaining = 0)`.

| #   | Row                                                         | Constraint                                                       | Result                           |
| --- | ----------------------------------------------------------- | ---------------------------------------------------------------- | -------------------------------- |
| A1  | `paid`, paid 5000, remaining 0, **refunded 2000**           | as shipped                                                       | **stored**                       |
| A2  | the same, **refunded 5000** (fully refunded)                | as shipped                                                       | **stored**                       |
| B1  | the same as A2                                              | amended to `… OR amount_refunded > 0`                            | **stored** — _identical outcome_ |
| B3a | `paid`, due 5000, paid 4000, remaining **1000**, refunded 1 | as shipped                                                       | **refused**                      |
| B3b | the same row                                                | amended                                                          | **stored**                       |
| C1  | A1's row                                                    | amended to `amount_remaining = amount_refunded` (the net design) | **refused**                      |

A1/A2 versus B1 is the vacuity: the row the amendment was supposed to make
storable is storable without it, so reverting the amendment fails nothing and
the brief's named mutation could not have fired. **B3a versus B3b is more than
that** — the amendment does not merely add nothing, it _widens_ what is
storable: a `paid` invoice with 1,000 still outstanding becomes legal the
moment any refund is recorded against it, which is the row
`paid_means_nothing_remaining` exists to refuse. C1 confirms the third
possibility: spelled as the net design D5 rejected, the amendment refuses the
very row D5 decided to store.

**Verdict: uphold.** Not amending was right, and the branch understated its own
case — the amendment was not neutral, it was a weakening.

### `refunded_at_most_paid` as the replacement, attacked

| Case                                                                         | Result                                       |
| ---------------------------------------------------------------------------- | -------------------------------------------- |
| Partial refund (2,500 of 5,000 paid)                                         | stored                                       |
| Exactly the whole bill (5,000)                                               | stored                                       |
| One minor unit over (5,001)                                                  | **refused** — `refunded_at_most_paid`        |
| Negative (−1)                                                                | **refused** — `amount_refunded_non_negative` |
| Any refund against `void` (paid 0)                                           | **refused**                                  |
| Any refund against `uncollectible`                                           | **refused**                                  |
| Any refund against `open` or `draft`                                         | **refused**                                  |
| Shrink the bill under an existing refund (due/paid 5000→1000, refunded 5000) | **refused**                                  |
| Move a refunded invoice back to `open` (paid → 0)                            | **refused**                                  |

"An invoice whose `amount_paid` is later changed by anything" cannot strand a
refund above it, and not only because the CHECK re-evaluates: on a `paid`
invoice `amount_paid` is _pinned_ to `amount_due` by `amounts_add_up` and
`paid_means_nothing_remaining` together, `void` and `uncollectible` are
reachable only from `open` (`vpay_db::invoices`' two transitions are
`WHERE … status = 'open'`), and `resum_draft`/`finalize_in_tx` — the only
statements that lower `amount_paid` — are `draft`-only and write
`amount_refunded = 0` in the same statement.

### Migration `0042` on a populated table

Applied to a database holding rows in every status, two of them paid: all six
backfilled to `0`; `pg_attrdef` carries **no** default for the column
afterwards (the ADD's backfill default really is dropped); the column is
`NOT NULL`; an `INSERT` that omits it is `23502`, which is the tripwire the
header claims.

---

## Findings

### 1. The concurrency claim was prose — gate-hole

`add_refund_for_intent_in_tx`, migration `0042`'s header,
`docs/reference/vpay-db.md` and `docs/flows/invoices.md` each state that two
refunds settling _concurrently_ add up and that the second re-evaluates the
CHECK against the first's committed value. Every delivered case settles refunds
sequentially. Measured: replacing the increment with a total read first _inside
the same transaction_ leaves
`two_refunds_against_one_invoice_add_up_and_an_over_refund_is_refused` **green**
and loses one of two concurrent refunds.

Fixed by `two_refunds_settling_concurrently_add_up_and_the_over_refund_still_loses`
(`tokio::join!`, two pairs: 3,000 + 1,500 must both commit and total 4,500;
3,000 + 3,000 must produce exactly one winner, with the loser refused by the
CHECK and left `pending`). The mutation now fails it: `left: 3000, right: 4500`.

### 2. `amount_refunded` was only ever asserted at zero — gate-hole, money path

Zero is also what a hard-coded literal produces, and what
`#[serde(default)]` produces for a key that never arrived. Two mutations, both
green as delivered:

| Mutation                                                                                 | As delivered            | After |
| ---------------------------------------------------------------------------------------- | ----------------------- | ----- |
| `amount_refunded: row.amount_refunded` → `amount_refunded: 0` in `InvoiceObject::render` | 342/342 `vpay-api` pass | FAILS |
| `#[serde(rename = "amount_refunded_MUTANT")]` on `vpay_sdk::Invoice`                     | 165/165 `vpay-sdk` pass | FAILS |

No case in the workspace read the key off a wire response, so `just ci` could
not have caught either. Fixed by one case per side, each asserting a **non-zero**
value; the SDK case also decodes a body with the key removed, which is the
pre-`0042` tolerance the parity row claimed and nothing exercised.

### 3. The Rust live case did not prove what two documents said it proved — misleading-claim

`live_invoices.rs` said it was "the only place in this crate that proves the
server actually SENDS it" and `docs/sdks/parity.md` said the live case "proves
the server sends it at all". `#[serde(default)]` plus `assert_eq!(…, 0)` passes
whether the key is present or absent. Both corrected to name what actually holds
each half. The Node live case does observe presence, because a missing key reads
`undefined` there and fails its `toBe(0)`.

### 4. A named fixture did not exist, and the resolve had gaps — rule-break plus coverage

`test_fixtures.rs` pointed at `config_with_invoice_defaults` "below"; no such
function existed anywhere in the tree. Written, and used by four cases covering
what the container suite could not reach:

| Attack                                                  | Answer                                                                                                                         |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `success_url=javascript:alert(1)` sent, both configured | `400` naming `success_url` — **not** silently replaced by the configured page                                                  |
| `success_url=` (blank) sent, both configured            | configured value wins (blank is absent, deliberately)                                                                          |
| One configured, the other sent                          | both resolve, independently                                                                                                    |
| One configured, the other nowhere                       | `400` naming only the absent one                                                                                               |
| Neither anywhere                                        | one `400` naming both, 192 characters on the wire, under the 200 ceiling, with the `merchant_clients[].invoices` clause intact |
| Configured `http://` under livemode                     | `400` at request time as well as at boot                                                                                       |

Mutations: validate only the request's value → the livemode case FAILS;
resolve configuration-then-request → the malformed-URL case FAILS.

The truncation assertion reads the **JSON envelope**, not `Display`: `Display`
is 233 characters because of a 41-character prefix no caller ever sees, and
asserting on it would have been asserting on the wrong string.

### 5. The rebase falsified three sentences — misleading-claim

`0040` (issue #88) landed on master in #108 while the branch was open. "Forty
migration files" and "`0040` and `0041` are not in this tree" were true when
written and false after the rebase, in `docs/status.md`, in
`postgres_smoke.rs`'s assertion message (fixed while resolving the conflict,
along with the count itself: 40 → 41) and in the implementer's own notes.
All three now say forty-one files, highest `0042`, one-wide gap at `0041`.

---

## What was checked and found sound

- **The transaction.** The refund flip and the invoice increment are one
  transaction, both statement functions `pub(crate)`, and the abandon property
  holds: an over-refund rolls the `succeeded` flip back with it.
- **The compare-and-swaps.** `status = 'pending'` on the refund and
  `status = 'paid'` on the invoice are in the `WHERE` clause, not beside it.
  `invoices_payment_intent_key` is a unique partial index, so the
  `fetch_optional` on `payment_intent_id` cannot see two rows.
- **A refund whose intent pays no invoice** settles and touches nothing —
  `Ok(Some((refund, None)))`, not an error.
- **`invoice.paid` is not re-emitted**, and the event count is asserted.
- **The unreachability caveat** is on every doc that mentions the column
  (`docs/api/README.md`, `docs/flows/invoices.md`, `docs/reference/vpay-db.md`,
  `docs/sdks/parity.md`, `docs/status.md`) and in the migration's own
  `COMMENT ON COLUMN`.
- **Items 3 and 4 stay open.** Both gap rows in `docs/flows/invoices.md` are
  unchanged and dated (2026-09-07), and `docs/status.md` claims neither.
- **Drift.** 172 → 173 with the single-column CHECK as the whole of the +1;
  relations 24 and unmappable columns 19 unchanged, both pinned.

## Gates

`just ci`, exit code read from a file rather than from a harness banner.

| Run                              | Head      | Result                                                                                                                                                                                                                                                                                                                                                               |
| -------------------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 — as delivered, rebased        | `bf1faed` | **0**. `verify` twelve gates ok; nextest **1689/1689, 0 skipped**; doctests 120 passed, 1 ignored (`vpay_sdk`); `verify-ignored: 0 ignored, 46 binaries`; web all green; `deny` ok                                                                                                                                                                                   |
| 2 — review's head, first attempt | `5766cbf` | **100**, and _not_ this change: `a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` timed out at 120 s in `failed to create a container: Timeout error`. Host load 19, `fs.inotify.max_user_instances` 128, 24 `created`-state `postgres:16-alpine` containers left by earlier runs — the flake the project memory records. Debris removed, re-run |
| 3 — review's head, re-run        | `5766cbf` | **0**. `verify` twelve gates ok; nextest **1696/1696, 0 skipped** (1545 s, 2 slow); doctests **120 passed, 1 ignored**; `verify-ignored: 0 ignored (expected 0), 46 binaries, 1696 total`; web 1294 vitest cases across nine projects; `deny`: advisories, bans, licenses, sources ok                                                                                |

The seven cases this review added all ran in run 3 (`vpay-api` 5, `vpay-sdk` 1,
`vpay-db` 1) — 1689 → 1696.

`just test-e2e` was **not** run: Cypress fixture ports 4180/4181 are fixed on
master (#106) and nothing on this host was holding them, but item 4 — the
browser case through `hosted_invoice_url` — was not attempted by the branch, so
an e2e run would have proved only that master's suite still passes.

## Left to the maintainer

1. **Gross versus net `amount_refunded`.** Upheld here on the evidence above,
   and still reversible in two places (migration `0042` and one statement). The
   measurement is B3a/B3b: the amendment the brief asked for would have removed
   a live refusal.
2. **`payment_intents.amount_refunded` stays unmaintained.** Deliberate, and
   the reasoning is sound — incrementing one half of a paired total is worse
   than incrementing neither — but it means two columns named the same thing
   now disagree by construction until `POST /v1/refunds` exists.
