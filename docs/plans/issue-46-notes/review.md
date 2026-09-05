# Issue #46 — sabotage review of `claude/issue-46-refund-fee`

Reviewed `git diff 65a5952..9228ca4` on 2026-09-05. The implementer's own
account is [impl.md](impl.md); this file records what a second pass
**measured**, which of its claims survived, and what it missed.

Everything below was run, not read off the code.

## 1. The gate, re-run end to end

`just ci` on `9228ca4`, recipe by recipe, Node 22.23.2 (`.nvmrc`) after
`pnpm install --frozen-lockfile`, against real Postgres and WireMock
containers:

| Recipe | Exit | What it printed |
|---|---|---|
| `fmt-check` | 0 | clean |
| `clippy` | 0 | `--workspace --all-targets -- -D warnings`, clean |
| `verify` | 0 | *"ok — the ten gates above passed"*; `verify-status` 1 token, `verify-sdk-parity` 346 tests / 26 dated gaps, `verify-serde` 51 types / 15 exempted, `check-schema` ok |
| `test-rust` | 0 | **1289 tests run, 1289 passed, 0 skipped** in 798 s |
| `test-doc` | 0 | every crate ok |
| `verify-ignored` | 0 | *"0 ignored (expected 0), 41 test binaries (expected 41), 1289 total"* |
| `lint-web` | 0 | `pnpm -r typecheck`, `pnpm -r lint` clean |
| `test-web` | 0 | `sdks/nodejs` **173 passed** |
| `deny` | 0 | advisories/bans/licenses/sources ok |

**Every number in impl.md §7 reproduces exactly.** The self-report is
accurate about the gate.

## 2. Findings

### F1 — money: a renderer may net the fee out of the payer's money and no test fails

`docs/flows/merchant-auth.md`, added by this diff, says of `fee`: *"It is
**not** deducted from `amount`: `amount` is the payer's money and a buyer's
refund never nets a fee."* That is the integrator's invariant 1 in the issue
("The buyer's refund never nets a fee"), and it is the reason `fee` is a
second field rather than an adjustment to the first.

**Measured:** changing the renderer to

```rust
amount: row.amount - row.fee.unwrap_or(0),
```

leaves `cargo nextest run -p vpay-api` at **244 tests run, 244 passed**. Every
refund test uses `fee: None` for its `amount` assertion, and the two cases
that use `Some(0)` / `Some(250)` read only the `fee` key back. The most
load-bearing sentence the diff adds to the wire contract is unchecked.

Fixed by extending `an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero`'s
sibling coverage: see commit "the payer's amount is never net of the fee".

### F2 — correctness of the proof: the TypeScript `| null` can be deleted and `just ci` stays green

`docs/sdks/parity.md` says the Node cell keeps an absent key and an explicit
`null` apart, and that *"each cell's tests assert exactly that"*.

**Measured:** changing `fee?: number | null` to `fee?: number` in
`sdks/nodejs/src/types.ts` leaves `just lint-web` at exit 0 and
`pnpm --filter @vaam-apps/vpay-sdk test` at exit 0 (173/173). The runtime test
still passes because types are erased and `expect(x).toBe(null)` does not
consult them, and `tsc` never complains because `toBe` takes `any`. Only
deleting the whole property is caught (impl.md's mutation 4); narrowing it is
not.

Fixed by a type-level assertion in `sdks/nodejs/src/types.test.ts`, the file
that already exists for exactly this purpose, named in the parity row.

### F3 — misleading claim: `exactOptionalPropertyTypes` is not what preserves absent-vs-null

Three places say the Node column keeps "absent" and `null` apart *because of*
`exactOptionalPropertyTypes`: `sdks/nodejs/src/types.ts`'s doc comment ("this
package's `exactOptionalPropertyTypes` is what makes the choice meaningful …
rather than one that silently reads as `null`"), `docs/sdks/parity.md`, and
impl.md §5.

**Measured** with the repo's own `tsc` 5.9.3, on `interface R { fee?: number | null }`:

| | `--exactOptionalPropertyTypes` | without it |
|---|---|---|
| read type of `r.fee` | `number \| null \| undefined` | `number \| null \| undefined` |
| `const y: R = { fee: undefined }` | `TS2375`, refused | accepted |

So the three read states come from `?` plus `\| null`, not from the flag, in
any configuration; and an absent property never "reads as `null`" in
TypeScript. What the flag actually buys is narrower and worth stating
correctly: it forbids *spelling* "absent" as an explicit `undefined` when
constructing one.

### F4 — misleading claim: `#[serde(default)]` is not what decodes an absent `fee`

`sdks/rust/src/model.rs` heads a section *"`#[serde(default)]`, and what it
does and does not preserve"* and says *"The default also means this SDK still
decodes a `refund` from a vpay older than the field"*;
`sdks/rust/tests/resources.rs` says *"`serde(default)` is what keeps that
decodable"*.

**Measured:** deleting `#[serde(default)]` from `Refund::fee` leaves
`a_refund_fee_decodes_as_unknown_free_or_a_real_cost` — whose fourth case
omits the key entirely — **passing**. `serde` already yields `None` for a
missing `Option<T>` field; the sibling `reason: Option<String>` on the same
struct carries no `default` for that reason.

The attribute stays (the issue asked for it, and being explicit costs
nothing), but the two comments claimed a proof they do not have.

### F5 — misleading claim: `docs/flows/merchant-auth.md` says `GET /v1/events` answers 404

Line 315 of the Resources table, under a header reading *"**Served** marks
what a running `vpay-server` actually answers as of 2026-09-03"*:

    | `GET` | `/v1/events` | … | `list` of `event` | ⛔ 404 |

`/v1/events` and `/v1/events/{id}` are both in `vpay_api::v1::V1_ROUTES` and
have been since `edfc5c9`, **2026-09-03** — and this diff adds, 250 lines
below in the same file, *"(Partly retired 2026-09-03: `/v1/events` is
served…)"*. The document now contradicts itself, on the page that is the
merchant wire contract. Pre-existing, but half of the contradiction is new.

`docs/roadmap.md:664` carries the same stale statement and is **not** fixed:
it sits inside a dated phase addendum that a later addendum already
supersedes, and rewriting closed history is not this PR's business.

### F6 — coverage: `charge.refund.updated` is claimed by three documents and tested by none

`merchant-auth.md` ("The same value appears on `charge.refunded` and
`charge.refund.updated`"), `webhooks.md` and `docs/status.md` all assert the
field on **both** refund event types. Only `charge.refunded` has a test.
Fixed by running the existing case over both types.

### F7 — nit: `docs/status.md`'s note about `verify-status` bullet syntax is inside a content bullet

The port bullet ends with *"— note that a bullet in this section may not
*begin* with a backticked path, because `verify-status` reads exactly those as
declared `NotImplemented` tokens"*. The statement is **true** (checked against
`declared_tokens` in `.xtask/src/main.rs`: it takes `- ` then a leading
backtick), but attaching a note about the file's own syntax to a sentence
about `vpay_provider::Refunded::fee` reads as if it were a fact about the
port.

### F8 — nit: `docs/sdks/parity.md`'s "the first of those two" lost its referent

The new `refund.fee` paragraph was inserted between the "measured on …"
paragraph and *"A note on the first of those two"*, which refers to the two
`checkout.session.expired` rows two paragraphs up.

## 3. Mutations

Every mutation was applied on this branch, the command run, the tree reverted
with `git checkout --`. **Observed** is what the run printed.

| # | Mutation | Command | Observed | Verdict |
|---|---|---|---|---|
| 1 | Delete `pub fee` (and its doc block) from `RefundObject`, and its assignment | `cargo nextest run -p vpay-api -E 'test(refund) + test(fee)'` | **4 failed**, 2 passed — `the_refund_object_is_the_documented_ten_keys`, `an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero`, `a_refund_delivered_as_charge_refunded_carries_fee_present_and_null`, `the_merchant_sdk_deserialises_the_refund_this_renders` | impl.md confirmed |
| 2 | `fee: row.fee` → `fee: Some(row.fee.unwrap_or(0))` | same | **4 failed**, same four | impl.md confirmed |
| 3 | Delete `pub fee` from `vpay_sdk::Refund` | `cargo xtask verify-sdk-parity`; `cargo nextest run -p vpay-sdk` | parity gate **ok (exit 0)**, 346 tests; nextest exits 101 on `E0609: no field 'fee' on type 'Refund'` ×2 | impl.md's correction confirmed — the parity gate does **not** catch a deleted field |
| 4 | `ADD COLUMN fee BIGINT DEFAULT 0` in `0031` | `cargo nextest run -p vpay-tests-integration -E 'test(refund_fee)'` | **1 failed** — `left: Some(0)`, `right: None`, on the fourth insert (the one that omits the column) | impl.md confirmed, including that the fourth insert is what makes it fail |
| 5 | Delete `CONSTRAINT fee_non_negative` from `0031` | same | **1 failed** — `a_negative_refund_fee_is_rejected_by_the_database` panics on `expect_err` | impl.md confirmed |
| 6 | *(baseline)* the two `0031` cases plus `schema_migrates_cleanly_on_an_empty_database`, by name, on a real Postgres | same, `--no-capture` | **3 passed**; the rejection printed is `new row for relation "refunds" violates check constraint "fee_non_negative"` | the CHECK fires on a real database |
| 7 | **`amount: row.amount - row.fee.unwrap_or(0)`** | `cargo nextest run -p vpay-api` | **244 run, 244 passed** | **F1 — new gap** |
| 8 | **`fee?: number \| null` → `fee?: number`** in `sdks/nodejs/src/types.ts` | `just lint-web`; `pnpm --filter @vaam-apps/vpay-sdk test` | both **exit 0**, 173/173 | **F2 — new gap** |
| 9 | **Delete `#[serde(default)]`** from `vpay_sdk::Refund::fee` | `cargo nextest run -p vpay-sdk -E 'test(fee)'` | **1 passed** — the absent-key case still decodes to `None` | **F4 — the comment's claim is wrong** |
| 10 | `interface R { fee?: number \| null }` compiled with and without `--exactOptionalPropertyTypes` | `tsc --noEmit` | read type identical (`number \| null \| undefined`); only `{ fee: undefined }` assignment differs | **F3 — the comment's claim is wrong** |

Invariants probed and found **not** at risk, recorded so the absence is
deliberate rather than unexamined:

* **Tenancy.** `RefundObject` is constructed nowhere outside `#[cfg(test)]`,
  and `/v1/refunds` is not in `V1_ROUTES` (nine entries, checked), so there is
  no foreign-merchant read to break. `refunds` has no `merchant_id` column at
  all — a future repository must scope through `payment_intents`. Recorded
  under Reserved.
* **Privacy.** The two `ApiError::Internal` messages the new conversion can
  emit name a column and a `kind_of` *type* (`"a string"`, `"an array"`),
  never a value; `row.status` is a closed `refund_status` label. Nothing
  merchant-supplied reaches a log line.
* **One renderer.** `EventObject`'s conversion copies `row.data` verbatim, and
  the new test round-trips `RefundObject` → JSON → event row → `EventObject` →
  `vpay_sdk::Event::refund()`, so the webhook body and an API response cannot
  disagree by construction.

## 4. The issue's proposal, mapped

| Proposal step | Verdict |
|---|---|
| 1. `merchant-auth.md` gains a tenth field, nullable, absent-not-zero | **Delivered**, with the three-state table and the "never nets `amount`" sentence (which F1 makes true of the code as well as the page) |
| 2. Server: the object and both event payloads carry `fee`, from a new nullable `refunds.fee`; migration count moves in the same commit; the port gains `fee: Option<Money>`; MTN's `NotImplemented` untouched; no adapter fabricates a fee | **Delivered.** Migration `0031`, count 30 → 31 in commit `6fbfc10`. `charge.refund.updated` was claimed but untested — F6 |
| 3. `docs/status.md` declares it present-and-unpopulated beside `mtn_momo::refund`, naming what must exist | **Delivered** |
| 4. `docs/flows/ledger.md` states reported-only, names the invariant that would change | **Delivered and true of the code**: invariant 2 reads as quoted, `vpay_ledger::AccountKind` has the three variants with no merchant dimension, and nothing in `vpay-ledger` posts a refund fee |
| 5. Both SDKs, `Option<i64>` + `#[serde(default)]` / `fee?: number \| null`, decode tests, a parity row naming both | **Delivered**, with the representation documented — F2, F3, F4 correct what the documentation says *about* it, not the shape |
| 6. An events test asserting `fee` present and `null`, and a key-count tripwire on the refund object | **Delivered** (`the_refund_object_is_the_documented_ten_keys` is the tripwire; there was none for refunds before) |

**The extra server surface impl.md §9 flags** — `RefundObject`, `RefundRow`,
`RefundStatus`, none of which the issue's steps 1–4 asked for — is judged
**correct to keep**. The maintainer asked for the field on the wire object and
on the events; a renderer is what makes either testable, and mutations 1, 2
and 7 all run through it. Nothing implies a refund can be created or read
through `/v1`: `V1_ROUTES` is unchanged at nine entries, the Resources table
still says `POST /v1/refunds ⛔ 404`, `docs/api/README.md` keeps it in the
not-routed table, and `docs/status.md` says so in both directions.

## 5. Reserved for the maintainer

* **How a future refunds repository scopes a read.** `refunds` has no
  `merchant_id`; the only path to a tenant is `payment_intents`. Nothing on
  this branch needs it, and choosing the join is a design decision, not a
  gap to fill quietly.
* **`docs/flows/merchant-auth.md`'s Resources table omits `/v1/checkout/sessions`
  entirely** (three routes, served). Pre-existing, unrelated to `fee`, and
  filling it in means writing request fields for a different feature.
