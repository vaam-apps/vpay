# exp33 — sabotage review of the invoice SDK surface

Reviewer's notes on `claude/exp33-sdk-invoices`, rebased onto `d5a93df`
(master with S5, PR #93). The implementer's own notes are
[`opus.md`](opus.md); this file records what was measured against them.

The review ran twice. The first pass was cut off by an API limit mid-Phase 2,
having reached "both SDKs work live" and having left **three uncommitted
files** in the worktree with nothing written down. This file is the second
pass's, and it starts by saying what happened to those three.

## The three uncommitted files

| File                                    | What the change was                                                                                                                     | Verdict                                                                          |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `.xtask/src/main.rs`                    | a per-column rule for `✅` cells, a `PARITY_COLUMN_SPELLINGS` table, four tests, and thirteen entries added to the vacuity guard's list | **kept**, split into three commits, each with its own measurement                |
| `sdks/nodejs/src/resources/invoices.ts` | `mark_uncollectible` → `markUncollectible`                                                                                              | **kept** — but the change was incomplete: four documents still said the opposite |
| `sdks/nodejs/src/client.test.ts`        | the same rename in the proving case, plus an assertion that the _path_ is still snake_case                                              | **kept**                                                                         |

None was committed as it stood. Each claim in them was re-measured rather
than taken on trust, and the diff carried one `cargo fmt --check` failure
(`.xtask/src/main.rs:9693`), which would have failed CI's `rust` job.

## Findings

Severity in this repository's vocabulary: **gate-hole** (a check that cannot
see a defect), **correctness**, **rule-break**, **misleading-claim**, **nit**.

### 1. The delivered head failed `just ci` — correctness

[`opus.md`](opus.md) says "`just verify` (twelve gates), `just lint-web`,
`just test-web` (1133 across the workspace) all pass on the final head". They
do. `just ci` does not, and it is not in that list:

```text
cargo nextest run -p xtask -E 'test(the_repositorys_own_sdks_enumerate)'
FAILED — sdk_parity_tests::the_repositorys_own_sdks_enumerate_exactly_the_capabilities_the_matrix_records
  left:  32 capabilities   right: 19
```

`the_repositorys_own_sdks_enumerate_exactly_the_capabilities_the_matrix_records`
_names_, rather than counts, every capability the two SDKs declare — and it
is the only thing that does, because `verify-sdk-parity` counts. Thirteen
invoice methods landed in each SDK and the list was not updated, so a head
whose `just verify` was green failed `cargo nextest run --workspace`.

Fixed in `17a49b1`.

### 2. The parity gate could not see a method deleted from one SDK — gate-hole

This is the one [`opus.md`](opus.md) measured (its mutation 1a) and left
standing, correctly noting that closing it is a change to `.xtask` rather
than to the branch. The review made that change, after reproducing the escape
on the committed head:

```text
# delete InvoicesResource::void from sdks/rust ONLY, leave sdks/nodejs alone
cargo xtask verify-sdk-parity
verify-sdk-parity: ok — 443 proving test(s) … 32 SDK method(s) …
exit 0
```

Both directional rules are satisfied while _either_ column declares the
capability, and the ✅ cell's named test is source text that goes on existing
because it belongs to the other SDK. The one thing the row's two cells exist
to tell apart was the one thing unchecked.

The gate has a sixth rule now, and the same mutation exits 1 naming
`sdks/rust`. [`../../status.md`](../../status.md)'s "still not checked" list
had carried this hole in as many words since 2026-09-06; it is struck through
with the measurement.

Fixed in `12f4230`.

### 3. `currency` is required, and both SDKs said it was optional — misleading-claim, in the money path

**The finding the live run paid for.** Both SDKs shipped
`CreateInvoiceParams.currency` optional, with the same sentence in each:
_"omitted from the body entirely when `None`, which is how the server gets to
apply this deployment's own default rather than this SDK guessing it."_

There is no such default. `vpay_api::v1::invoices::create` calls
`payment_intents::parse_currency(params.currency.as_deref(), config)`, whose
first statement is an unconditional
`raw.ok_or_else(|| ApiError::invalid_param("currency", …))`. Against a running
vpay, through the Node SDK:

```text
NO-CURRENCY: refused -> A three-letter `currency` code is required.
```

Two proving tests asserted the opposite **in their own names** —
`an_invoice_needs_only_a_customer_and_the_rest_is_omitted` and "an invoice
needs only a customer, and every other field is omitted" — and both passed,
because a `wiremock` answering `201` to anything cannot disagree. This is
exactly the hazard the ⛔ row said it was there to prevent, arriving in the
shape the row predicted.

`currency` is a plain `String` / `string` now, as it already is on
`CreatePaymentIntentParams`, and both tests are renamed for what they really
pin.

Fixed in `f09309b`.

### 4. Neither SDK had ever run against a vpay — gate-hole, now closed

[`opus.md`](opus.md) left this open and said so plainly, which was the right
call for that pass. The review closed it by running them, and by landing the
runs as suites that cannot report a green without a stack:

- `sdks/rust/tests/live_invoices.rs`, a cargo target behind a `live-stack`
  feature — so `cargo nextest run --workspace` neither builds nor lists it,
  and `expected_suites` stays 45 (measured before and after);
- `sdks/nodejs/src/invoices.live.test.ts`, its own vitest project excluded
  from `pnpm test`.

Both are `sdks/stripe-compat`'s shape, for its reason. Neither uses
`#[ignore]` or `.skip`; with no stack, both **fail**, naming what is missing.

Fixed in `a555843`.

### 5. `markUncollectible`, and a gate that could read two spellings — rule-break

[`opus.md`](opus.md)'s decision 1 chose `mark_uncollectible` in the Node SDK
because `verify-sdk-parity` keyed a row on one spelling verbatim. That is a
check deciding a public API's spelling because it could not read two, and
[ADR-0015](../../adr/0015-sdk-parity.md) decision 1 says the opposite in as
many words: parity is _"per capability, with the same wire semantics — not per
method name"_, and its alternatives reject method-name parity outright.

The interrupted reviewer had renamed it and taught the gate a table. That
work is kept: `PARITY_COLUMN_SPELLINGS` carries the one entry and is read in
**both** directions, so an entry that exempts nothing fails the build, the way
[ADR-0016](../../adr/0016-engineering-standards.md)'s serde exemption table
does.

Four documents still carried the old decision in prose and would have shipped
contradicting the code — [`../../sdks/parity.md`](../../sdks/parity.md),
[`../../status.md`](../../status.md), `sdks/nodejs/README.md` and
[`opus.md`](opus.md) itself. All four now say what is true.

**This remains a maintainer-visible API decision**, and it is one the
implementer explicitly flagged for the maintainer. It is recorded here rather
than buried: the public method in `@vaam-apps/vpay-sdk` is
`invoices.markUncollectible`, the Rust one is
`invoices().mark_uncollectible()`, and the route is unchanged and cannot
change — `POST /v1/invoices/{id}/mark_uncollectible`, asserted by name in both
SDKs' proving tests.

Landed in `e02b406`.

### 6. Nits, recorded and not fixed

- `.xtask/src/main.rs:9693` in the interrupted reviewer's diff failed
  `cargo fmt --check`. Reformatted by hand rather than by `just fmt`.
- `sdks/stripe-compat` still has **no invoice cases**, as
  [`opus.md`](opus.md) says. The review did not add any: that package drives
  the real `stripe` package, gets no parity rows of its own (ADR-0015
  decision 4), and the claim the ⛔ row made was about the two merchant SDKs.
  It is recorded in [`../../flows/invoices.md`](../../flows/invoices.md) as
  its own line rather than folded into the closed one.

## What was checked and found correct

Re-measured against `backends/crates/vpay-api/src/v1/invoices.rs` and
`v1/mod.rs`'s route table, and then against a running server:

- **Paths and methods.** All thirteen. `POST` (not `PATCH`) for both updates,
  matching what a merchant's existing Stripe client sends, on a handler the
  server mounts under both verbs. No `invoice_items.list` in either SDK,
  because the server mounts no collection `GET`.
- **`Idempotency-Key` on every write.** Handled at the client layer in both
  SDKs, for `POST` and `DELETE` alike; the three parameterless transitions
  carry one with an empty body.
- **Encoding, including the `+`-is-literal rule.** Both SDKs percent-encode
  path segments and form values with `encodeURIComponent`'s exact byte set
  (`sdks/rust/src/form.rs`'s `is_safe_byte` is that set), so `+` is `%2B` in
  both, in a path segment and in a query string.
- **The eighteen keys**, read off a live response: `amount_due`,
  `amount_paid`, `amount_remaining`, `created`, `currency`, `customer`,
  `description`, `due_date`, `hosted_invoice_url`, `id`, `lines`, `livemode`,
  `metadata`, `number`, `object`, `payment_intent`, `status`,
  `status_transitions`. A line is eight.
- **Every documented refusal, live.** `del` and `update` on a finalized
  invoice are `409 invalid_state`; a line write against a non-draft parent is
  `400` naming `invoice`; `invoice_items.update` with `description=""` is
  `400` naming the parameter (which is why that patch type is two-state);
  `finalize` with no lines is `400`; a second `pay` and a `void` while an
  intent is live are both `409` naming the intent; an unknown `in_…` is
  `404 resource_missing`. An omitted `quantity` really does default to 1
  server-side, and a metadata key set to `""` really is removed.
- **The update three-state**, live: `due_date: null` clears the due date and
  leaves `description` alone.
- **`sdks/stripe-js`** needed nothing, as [`opus.md`](opus.md) says.

## Mutations

Each applied to the tree, run, and reverted.

| #   | Mutation                                                                                             | Gate                                  | Exit  | Caught                                                       |
| --- | ---------------------------------------------------------------------------------------------------- | ------------------------------------- | ----- | ------------------------------------------------------------ |
| 1   | delete `invoices.void` from `sdks/rust` only — **on the delivered head**                             | `verify-sdk-parity`                   | **0** | **no** — the escape, finding 2                               |
| 2   | the same, after `12f4230`                                                                            | `verify-sdk-parity`                   | 1     | yes, naming `sdks/rust` and not `sdks/nodejs`                |
| 3   | the thirteen invoice capabilities missing from the vacuity list — **the delivered head as it stood** | `cargo nextest -p xtask`              | 100   | yes (finding 1: nothing ran it)                              |
| 4   | delete the one `PARITY_COLUMN_SPELLINGS` entry                                                       | `verify-sdk-parity`                   | 1     | yes, in both directions at once                              |
| 5   | the same                                                                                             | `cargo nextest -p xtask`              | 100   | yes — the canonical list                                     |
| 6   | spell the Node method `mark_uncollectible` again                                                     | `verify-sdk-parity`                   | 1     | yes — `PARITY_COLUMN_SPELLINGS[0] … exempts nothing`         |
| 7   | unset `VPAY_BASE_URL` for the Rust live suite                                                        | `cargo nextest --features live-stack` | 100   | yes — `2 tests run: 0 passed, 2 failed`, never `0 tests run` |
| 8   | point the Node live suite at a dead port                                                             | `pnpm test:live`                      | 1     | yes — the preflight, not a skip                              |

Mutations 1, 3 and 6 are the three that matter: the first two are defects that
were on the branch, and the third is what keeps the spelling table from
rotting.

## Gates on the final head

| Gate                            | Result                                                                                                                                                        |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just verify`                   | twelve gates, exit 0                                                                                                                                          |
| `cargo nextest run -p vpay-sdk` | 164 run, 164 passed, **0 skipped**                                                                                                                            |
| `cargo test --doc -p vpay-sdk`  | 8 passed, 1 ignored                                                                                                                                           |
| `cargo nextest run -p xtask`    | 231 run, 231 passed, 0 skipped                                                                                                                                |
| `just verify-ignored`           | 0 ignored, **45** test binaries (expected 45), 1624 total                                                                                                     |
| `verify-sdk-parity`             | **448** proving tests, **35** dated gaps, 32 methods, 36 rows                                                                                                 |
| `just ci`                       | **exit 0** — 1624 Rust tests run, 1624 passed, **0 skipped** (1278 s); doctests 8 passed 1 ignored; `pnpm -r test` 1261 cases across nine packages, 0 skipped |

Live, against compose project `exp33-review2` on `:18080`, images built from
this head:

| Suite                                                                      | Result                     |
| -------------------------------------------------------------------------- | -------------------------- |
| `cargo nextest run -p vpay-sdk --features live-stack --test live_invoices` | 2 run, 2 passed, 0 skipped |
| `pnpm --filter @vaam-apps/vpay-sdk test:live`                              | 1 file, 3 passed           |

## What this review did NOT do

- **`just sdk-live` was not run end to end in one go.** Its two suite
  invocations were, against the stack the recipe builds, from that recipe's
  own environment. The recipe's `docker compose … up -d --wait` line — the
  same line `just stripe-compat` uses — failed on this host with
  `container exp33-review2-wiremock-orange-1 is unhealthy`, whose health log
  is 157 consecutive `OCI runtime exec failed: … current working directory is
outside of container mount namespace root`. That is a rootless-Docker shim
  fault on the host, not a defect in the recipe or the stack: the same daemon
  had an unrelated stack in a restart loop at the time, and `/healthz` on the
  server this recipe brought up answered `200` throughout.
- **The two new CI steps have not run in CI.** They are copies of the adjacent
  `stripe-node conformance` step, on the stack that job already builds, with
  the same env vars; `actionlint` reports the file clean. Nobody has seen them
  green.
- **No Stripe-compat invoice cases** — see the nit above.
- **Nothing was checked about `sdks/stripe-js`** beyond confirming
  [`opus.md`](opus.md)'s reasoning: it holds no merchant credential and calls
  only `/v1/browser`.
- **No settlement was driven through an invoice.** `pay` was proven to mint an
  intent and a hosted URL; nothing paid that intent, so `invoice.paid` and the
  `paid` status are still proven only by
  `backends/tests/integration/tests/invoices.rs`, server-side, and by neither
  SDK.
- **The dashboard, Cypress and `examples/` were not touched**, as in the
  implementer's pass.
