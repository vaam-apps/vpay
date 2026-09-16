# 2026-09-16 — stale-claim sweep: eight claims that survived wave 3

Branch `fix/stale-refund-claims`, from `origin/master` at `7a79684e`
(PR #178, "the refund path end to end"). **Documentation and three code
comments only. No behaviour changed, no test was deleted, no gate was
relaxed.** The subject is claims that were true when written and that wave 3
made false, plus two counts that were wrong before it.

## Why these eight were missed

Every sweep on this branch's history grepped for `Unsupported`, "not routed",
"no create" and `NotImplemented`. None of them grepped for

- a **count** ("thirty-one methods", "nine items", "thirteen unit tests"),
- an **SDK-capability claim** ("neither merchant SDK carries the field"),
- or a **comment asserting the state of code further down its own file**.

All eight belong to one of those three classes. Nothing in `just ci` checks
any of them.

## What was found, and what was measured

| #   | Claim                                                                                                               | Verdict                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `vpay-db/src/ledger.rs` header: "nothing reaches `crate::Refunds::create`"                                          | **False, and contradicted inside its own crate.** `vpay_api::v1::refunds` calls `TxRepositories::create_refund_in_tx` (`v1/refunds.rs:810`), which runs `refunds::create_in_tx` — the statements `Refunds::create` wraps — from a shipping binary. The uncalled half is `Settlement::apply_refund_succeeded`, whose only callers are in `vpay-db/tests/repositories.rs`; `vpay-ledger`'s header already said so. Corrected to name the settlement half.           |
| 2   | `vpay-adapter-mtn-momo/src/lib.rs` `capabilities()`: "`refund` below is still an unbuilt `NotImplemented` token"    | **False since 2026-09-15.** `refund` is MTN's Disbursements `transfer`, ~300 lines below the comment in the same file (`POST {base}/disbursement/v1_0/transfer`).                                                                                                                                                                                                                                                                                                 |
| 3   | `ProviderAdapter::refund`: "nothing calls `refund` today, so this is a trap set for the `POST /v1/refunds` handler" | **The handler exists and calls it** (`v1/refunds.rs:888`) — and honours the trap: an `Ok` leaves the row `pending`, pinned by `a_refund_is_created_pending_and_the_rail_is_instructed`. The same shape in both adapters' "nothing calls `parse_destination` outside tests yet" is false too: `resolve_destination` strips the rail code and calls it.                                                                                                             |
| 4   | `docs/api/README.md`: "**Thirty-one methods across twenty paths**, re-counted from `V1_ROUTES` on 2026-09-07"       | **Wrong twice.** Counted from every `methods:` field in `V1_ROUTES` today: **37 methods across 23 paths**. Counted the same way at `c49ca967`, the commit that wrote the sentence: **33 across 21** — Step 9's `GET /v1/checkout/sessions` and `POST /v1/checkout/sessions/{id}/expire` (both 2026-09-04) were already missing from it. `HEAD` is in no entry and is counted by neither figure.                                                                   |
| 5   | `docs/flows/stripe-sdk-compat.md`: "Neither merchant SDK carries the field either"                                  | **False.** `sdks/rust/src/resources.rs` has `pub destination: Option<RefundDestination>` on `CreateRefundParams`; `sdks/nodejs/src/types.ts` has `destination?: RefundDestination` on its own `CreateRefundParams` and `resources/refunds.ts` writes `destination[<rail>][msisdn]`. `docs/sdks/parity.md`'s row is ✅/✅ and its § "Not that the server offers every capability" retracts the gap in its own words — two vpay documents contradicting each other. |
| 6   | `docs/flows/adapter-orange-money.md`: "All **nine** 'to confirm' items above still stand"                           | **The list is ten**, numbered 1–10, since item 10 (`FAILED` sub-reasons, issue #59) was added on 2026-09-10.                                                                                                                                                                                                                                                                                                                                                      |
| 7   | `docs/status/backend.md`: "`POST /v1/refunds` is unrouted until wave 3"                                             | **Three rows say it.** The **Ledger balancing invariant** row uses it as a live reason for its own 🟡, so it is struck through in place and the reason it is still 🟡 — nothing settles a `pending` refund — is stated instead. The other two are inside entries dated 2026-09-05 and 2026-09-15 and keep this page's archive rule; the page's banner now names them rather than saying "several rows".                                                           |
| 8   | `docs/flows/merchant-auth/resource-contract.md`: "All four refund routes are served since 2026-09-16"               | **Ambiguous, not false.** It means the four RFC-0003 § 2 added; the resource is **five methods across three paths** with the 2026-09-05 read. Reworded, and pointed at `the_refund_resource_is_mounted_for_exactly_five_methods`.                                                                                                                                                                                                                                 |

### The extra one: "thirteen unit tests" vs "Twelve new"

`docs/status.md` claimed "seven conformance cases and thirteen unit tests",
both "against a real `wiremock/wiremock` container";
[2026-09-15-mtn-disbursements-refund.md](2026-09-15-mtn-disbursements-refund.md)
says "Twelve new unit tests in `vpay-adapter-mtn-momo`".

They count different things and **neither is reproducible from the tree**:
"new" and "about the transfer" are groupings nothing records. What is
measurable is that `vpay-adapter-mtn-momo` declares **no dev-dependency at
all**, so its unit tests touch no container — the attribution, not the number,
was the provable error — and that #178 added **24 test functions** to that
crate, 7 of them destination parsing. `docs/status.md` now states the figure a
reader can re-run. The verification page's own "Twelve new" is left as the
dated claim it is.

The seven conformance cases were checked by name and all seven exist in
`backends/tests/conformance/tests/adapter_conformance.rs`.

## Not done

- `backends/migrations/0046`'s `COMMENT ON` still carries its false clause.
  Applied migrations are immutable down to the byte (issue #76) and `0048`
  corrects it forward in the live database. Untouched on purpose.
- `backends/tests/integration/tests/refunds.rs`'s `pool` field doc says
  `Refunds::create` "is not reachable from `/v1`". Narrowly true of the pooled
  method and misleading about the path; out of this change's scope and
  reported rather than edited.

## Gate output

Run on `fix/stale-refund-claims`. `just ci` was deliberately **not** run —
concurrent local builds have OOM-killed this host — so what follows is the
narrow set for what this change touched, and CI is the gate.

```
cargo xtask verify-links     GATE_OUTPUT_LINKS
cargo xtask verify-status    GATE_OUTPUT_STATUS
cargo xtask verify-docs      GATE_OUTPUT_DOCS
cargo check --workspace      GATE_OUTPUT_CHECK
cargo nextest run -p vpay-adapter-mtn-momo   88 tests run: 88 passed, 0 skipped
prettier --check .           GATE_OUTPUT_PRETTIER
```
