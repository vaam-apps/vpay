# STATUS

**What actually works today.** This page is the contract behind the repo's
second rule: _never advertise a feature as done when it clearly is not._

It is short on purpose. Until 2026-09-11 it was **6 151 lines** — a running log
in which every step and every review appended a dated narrative, several of them
over a thousand words — and it is the first page AGENTS.md and CLAUDE.md send
every reader to. Nothing was thrown away to shorten it: every dated measurement,
every struck-through claim and every "this said X until date Y and was wrong"
moved **verbatim** into [`docs/status/`](status/), and each row below links to
the page its history is on. If you are looking for the sentence that used to be
here, it is still in the repository; start from
[the archive index](status/README.md).

## Overall

> **vpay is a scaffold.** It compiles, lints clean, and its tests pass — but it
> cannot take a payment. Do not deploy it.

**The load-bearing sentence, unchanged since 2026-09-03 (Step 3): no HTTP call
to a real rail has ever been made.** Every payment in this repository's history
settled against a `wiremock/wiremock` host that answered the way these documents
say a rail answers. No payer has ever been prompted on a real handset, no money
has moved, and no merchant endpoint outside this repository has ever been POSTed
to.

That banner was narrowed by nine dated addenda rather than replaced — Steps 2,
3, 4, 5c, 7 (twice), 8, 9 and exp31, each retiring a specific claim on a
specific date. The whole chain, unedited, is in
[status/overall-history.md](status/overall-history.md); it is worth reading if
you want to know what was retired and when, because two of those entries retire
sentences this page no longer makes. **Nothing has been added to it since
2026-09-07**, and nothing that has landed since has earned an addendum: customer
erasure and the address object, MTN's failure codes, demo tenancy, `@vpay/ui`,
Gateway API routing, the first CrateStack `procedure`, and the `/dash/v1` read
seam and its BFF each moved a row on a page below without narrowing the sentence
above.

## Where things stand

Each row links to the page that carries the rows, the evidence and the history.
Every 🟡 and ⛔ on those pages is there because a test says so.

| Area                                                     | Today                                                                                              | Detail                                                                                                                                            |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Backend** — `/v1`, `/dash/v1`, `/provider`, `/browser` | 🟡 Real routes, real rows, real adapters; every rail call has gone to a stub                       | [status/backend.md](status/backend.md)                                                                                                            |
| **Adapters** — `mtn_momo`, `orange_money`                | 🟡 Wire calls proven against WireMock, ⛔ never against MTN or Orange                              | [status/backend.md](status/backend.md), and the payer-window note in [status/backend-orange-hosted-page.md](status/backend-orange-hosted-page.md) |
| **Frontend** — checkout page, dashboard, demo shop       | 🟡 Built and walked by a real browser against a stub rail                                          | [status/frontend.md](status/frontend.md)                                                                                                          |
| **Infrastructure** — images, compose, Helm, migrations   | 🟡 Boots in compose and in CI; ⛔ no pod has ever run                                              | [status/infrastructure.md](status/infrastructure.md)                                                                                              |
| **Data layer** — sqlx, CrateStack, the schema            | 🟡 `schemas/vpay.cstack` compiles into `vpay-db`; the migration/model drift is counted, not closed | [status/cratestack.md](status/cratestack.md), [status/sqlx-and-op-stores.md](status/sqlx-and-op-stores.md)                                        |
| **Merchant SDKs** — `sdks/rust`, `sdks/nodejs`           | 🟡 Parity is machine-checked in both directions; the gaps are dated and owned                      | [status/merchant-sdks.md](status/merchant-sdks.md), [sdks/parity.md](sdks/parity.md)                                                              |
| **An MVP**                                               | 🟡 Two of eight conditions met (items 1 and 6); the other six are decided by one sentence          | [status/mvp.md](status/mvp.md)                                                                                                                    |

## How this page is checked

Some documents in this repository are read by a gate rather than by a reviewer,
and this is one of them. `cargo xtask verify-status` scans the workspace for
every `ProviderError::NotImplemented("…")` token and **fails in both
directions**: a token with no bullet under the heading below fails the build,
and — since 2026-09-03 — so does a bullet naming a token no shipping code
carries any more. The scanner reads _code_: since 2026-09-05 it lexes, so a
token written in a comment of any kind, in a `#[doc = "…"]` attribute or inside
any string, raw-string or character literal is prose, and prose declares
nothing.

`just verify` is **twelve gates and one advisory report**. What each one refuses,
and what it printed on `face3da` on 2026-09-11:

| Gate                  | What it refuses                                                            | Last printed                                 |
| --------------------- | -------------------------------------------------------------------------- | -------------------------------------------- |
| `verify-no-mocks`     | a test double reachable from a shipping binary                             | no test double reachable                     |
| `verify-status`       | an undeclared — or a stale — `NotImplemented` token                        | 1 unimplemented item                         |
| `verify-errors`       | an unclassified error type, or `anyhow` in a library crate                 | 19 error types, 16 `#[from]` variants        |
| `verify-sdk-parity`   | an SDK capability with no row, or a row naming no capability               | 463 proving tests, 33 dated gaps, 32 methods |
| `verify-links`        | a repository link that resolves to no tracked path                         | 1 116 links in 222 files                     |
| `verify-npm-scope`    | an unpublishable manifest, or a retired package name outside the record    | 2 publishable packages, 1 private            |
| `check-schema`        | a `schemas/vpay.cstack` that does not type-check                           | ok at cratestack 0.11.1                      |
| `verify-serde`        | a serialisable type that does not spell the wire convention                | 90 types, 16 exemptions                      |
| `verify-repositories` | a repository implementation named outside `vpay-db`, or an exported schema | 4 implementations, 83 source files outside   |
| `verify-toolchain`    | a `backends/Dockerfile` that drifts from `rust-toolchain.toml`             | 1.98.0                                       |
| `verify-ui`           | a UI primitive built outside `@vpay/ui`                                    | nothing: silent on success, exit 0 only      |
| `verify-migrations`   | an applied migration whose bytes changed                                   | 42 files                                     |
| `verify-docs`         | **nothing — it exits 0 whatever it finds**                                 | advisory report                              |

Those numbers are a measurement of one tree on one day, not a promise. What each
gate used to miss, the mutation that proved each hole shut, and the dates every
one of these counts moved on, are in [status/gates.md](status/gates.md) — 509
lines of it, unedited. Read the numbers in it as date-stamps rather than totals:
it calls `verify-sdk-parity` the reader of "the fourth machine-checked
document", which was true on 2026-09-03, when four of these twelve gates
existed.

### Unimplemented items tracked by `verify-status`

Every token below appears verbatim in shipping source. Removing an item here
without removing it from the code fails CI, and — since 2026-09-03 — so does
leaving an item here that no shipping code carries any more; both halves of
that are read off the gate's own output by
`verify_status_reports_both_directions_from_the_gate_itself`. The scanner
counts only occurrences in code, so a token quoted in a comment of any kind,
in a `#[doc = "…"]` attribute or in a string literal neither declares
anything nor counts as one — you never have to strip an honest sentence from
a doc comment to keep this gate green
(`a_token_quoted_in_a_comment_is_not_a_shipping_claim`,
`a_token_outside_code_is_never_a_shipping_claim`,
`a_token_in_a_doc_attribute_is_not_a_shipping_claim` and
`the_lexer_tells_the_four_states_apart` in `xtask`).

**There is exactly one, down from eight on 2026-09-03 (Step 3), and the two
that left did so for opposite reasons — which is the distinction this list
exists to keep visible.** Six went because the code was _written_:
`{mtn_momo,orange_money}::{submit, query_status, parse_callback}` are real
HTTP calls now. `orange_money::refund` went because it was **never unbuilt
work in the first place** — Orange's Web Payment product documents no refund
API, so the adapter stops overriding the port and inherits the trait's
default `ProviderError::Unsupported`: a permanent capability answer the core
can branch on (`supports_refunds: false`), asserted by the conformance case
`a_rail_without_the_refund_capability_answers_unsupported`. A rail that will
never support an operation must not be described with the same token as work
someone still has to do.

- `mtn_momo::refund` — MTN refunds are a different product (Disbursements)
  with its own subscription key and its own token scope; nothing in
  `config/application.yml` or `ProviderHost` carries a disbursement key and
  no deployment has been issued one, so nothing honest can be built yet
  (Step 3 design, decision 3). `supports_refunds` stays **`true`** for
  `mtn_momo` on purpose: the _rail_ refunds, and answering `Unsupported`
  would be a lie about MTN rather than an admission about us. `refund` is
  therefore the one operation on the one rail that still returns a token —
  reachable only through `POST /v1/refunds`, which is not routed, so no
  caller can currently provoke it
  (`refund_is_not_implemented_and_does_not_pretend`,
  `unimplemented_operations_never_fabricate_success` in the conformance
  suite).

**Declared and unpopulated, beside that token: the refund `fee`.** Added
2026-09-05 for [issue #46](https://github.com/vaam-apps/vpay/issues/46), which
an integrator filed because vpay's `refund` object never said what the
movement cost and their own type had no way to spell "unknown", so they were
shipping a hardcoded `0`. It is **not** a `NotImplemented` token and does not
belong in the list above — nothing about it is unbuilt. What exists, and is
asserted:

- the column `refunds.fee` (migration `0031`), nullable, no `DEFAULT`, with a
  `fee_non_negative` CHECK — `an_unreported_refund_fee_stays_null_and_never_becomes_zero`
  and `a_negative_refund_fee_is_rejected_by_the_database` in `postgres_smoke`;
- the wire field `vpay_api::model::RefundObject::fee` and the ten-key
  tripwire on that object — `the_refund_object_is_the_documented_ten_keys`,
  `an_unreported_refund_fee_renders_null_and_a_reported_zero_renders_zero`,
  `a_refund_delivered_as_either_refund_event_carries_fee_present_and_null`
  (which renders through **both** refund event types, since `data.object` is
  this object), and `a_reported_fee_never_moves_the_payers_amount`, which
  holds the one invariant the field exists to protect: `amount` is the
  payer's money and is never net of the fee. That last case was added on
  review — until it existed, `amount: row.amount - row.fee.unwrap_or(0)`
  passed all 244 of `vpay-api`'s tests
  ([plans/issue-46-notes/review.md](plans/issue-46-notes/review.md), F1). That
  244 is a dated measurement of the pre-rebase tree and is deliberately not
  restated: what matters is that the same mutation was **re-run on 2026-09-06
  after the rebase onto issue #45's merge and still fails that case**, on
  `left: 1750, right: 2000`
  ([plans/issue-46-notes/impl.md](plans/issue-46-notes/impl.md) § 11);
- the port's own `vpay_provider::Refunded::fee` (`Option<Money>`), the only
  thing that could ever fill it;
- both merchant SDKs' `Refund.fee`, with the parity row and its five tests.

None of those four bullets _begins_ with a backticked path, and none may:
`verify-status` reads exactly that shape — `- ` then a backtick — as a
declared `NotImplemented` token, and the docs→code half of the gate would
then fail because no shipping code carries one. That is the gate working
rather than a trap, and it is why each bullet above opens with a noun.

**What has to exist before it is ever anything but `null`.** For MTN: a
Disbursements subscription key and token scope in `config/application.yml` /
`ProviderHost` (the same missing credential as the token above), a written
`mtn_momo::refund`, **and** a real Disbursements response that actually
carries a fee — none of the modelled MTN responses in
`vpay-adapter-mtn-momo/src/wire.rs` has a fee field today, and whether that
product reports one has never been verified against MTN's sandbox, which this
repository has never called. For Orange: nothing, ever — the Web Payment
product documents no refund API, the adapter answers `Unsupported`, and there
is no refund to charge a fee for. **An adapter must not invent one**: `None`
is "the rail did not report a fee" and `Some(0)` is "the rail said it was
free", and collapsing them is the exact defect the issue reports one layer up.

**Also missing, and larger:** nothing **writes** a `refunds` row. Reading one
is no longer missing — issue #45 landed `vpay_db::Refunds::get_for_merchant`
and `GET /v1/refunds/{id}` on 2026-09-06, while this branch was open, so
`RefundObject` does cross a wire and `fee` is `null` on every object it can
produce. What is still missing is the whole of the write side: no
`POST /v1/refunds` (it is declared in the wire contract and mounted nowhere),
no `create` in the repository, no adapter that can execute a refund, and no
writer for `charge.refunded` / `charge.refund.updated` — both types are in the
`type_is_a_documented_event` vocabulary and neither has ever been emitted. So
what the tests above prove about the _event_ surface is that the contract
holds, not that a refund event works.

## Legend

| Marker             | Meaning                                                                                |
| ------------------ | -------------------------------------------------------------------------------------- |
| ✅ **Done**        | Implemented, tested, and the tests actually assert behaviour                           |
| 🟡 **Partial**     | Some of it is real. The rest is listed explicitly below                                |
| ⛔ **Not started** | No implementation. Calls return `NotImplemented`; tests are `#[ignore]`d with a reason |

Nothing in this repo is ✅ unless a test would fail if it broke.

## Verification history

Every "Last verified" note this page ever carried is in
[status/verification/](status/verification/), one page per block of dates,
unedited — including the runs that failed, the flakes that were kept rather than
dropped, and the "what this note does _not_ claim" paragraphs, which are the
half worth reading.

**The two most recent entries are both 2026-09-11**, and neither is this branch:
`claude/exp51-demo-tenant`, whose evidence is `just test-e2e` (22 Cypress tests,
22 passing) rather than `just ci`, and `claude/exp46-customer-address`, at `just
ci` exit 0 with **1 698 tests run, 1 698 passed, 0 skipped** across 46 binaries,
`test-doc` 112 passed / 1 ignored, and twelve gates. Both are on
[status/verification/2026-09-11.md](status/verification/2026-09-11.md), together
with the header of the 2026-09-10 `claude/exp45-worker-pool-bound` entry, which
sits between them on this page and whose own `just ci` paragraph this page has
never carried.

**Five of the seven changes that have landed since those entries were written
carry no `just ci` note here at all** — [#119](https://github.com/vaam-apps/vpay/pull/119)
(Gateway API routing), [#117](https://github.com/vaam-apps/vpay/pull/117) (the
payments procedure), [#116](https://github.com/vaam-apps/vpay/pull/116)
(`@vpay/ui`), [#121](https://github.com/vaam-apps/vpay/pull/121) (MTN's failure
codes) and [#123](https://github.com/vaam-apps/vpay/pull/123) (the `/dash/v1`
read seam and its BFF). That gap predates this split and is not closed by it:
each of those changes moved rows on the pages above, and none of them added the
gate run that would say on what tree.

## Where a new row goes

When you land a change, add its evidence to the page it belongs to, not here:

- a new capability, a new route, a new adapter behaviour →
  [status/backend.md](status/backend.md);
- a browser or dashboard change → [status/frontend.md](status/frontend.md);
- an image, a chart, a compose file, a migration →
  [status/infrastructure.md](status/infrastructure.md);
- a schema or CrateStack change → [status/cratestack.md](status/cratestack.md),
  plus a dated page under [status/cratestack/](status/cratestack/) if it
  measured something;
- an SDK change → [status/merchant-sdks.md](status/merchant-sdks.md) **and**
  [sdks/parity.md](sdks/parity.md), which is a gate;
- a new gate, or a gate that grew a direction →
  [status/gates.md](status/gates.md), and update the table above;
- your `just ci` evidence → a dated page under
  [status/verification/](status/verification/), newest date first.

Only four things belong on **this** page: the banner, the table of areas, the
gate table, and the declaration above — which lives here because `cargo xtask
verify-status` reads `docs/status.md` for it by name. Everything else is an
archive page, and [status/README.md](status/README.md) is the index of them.
