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

> _2026-09-15 (the first real rail call): the sentence above is **retired in
> its turn.** On 2026-09-15 a EUR `mtn_momo` PaymentIntent
> (`pi_xxd2xj1e914e16c6m63gezag`) was created, confirmed and settled against
> **MTN's real sandbox** (`https://sandbox.momodeveloper.mtn.com`): the
> worker's authenticated status query reported the charge paid and the intent
> reached `succeeded`. **Its replacement is narrower and still load-bearing:
> no real payer, no production rail, and no rail other than MTN's sandbox have
> ever been touched.** The payer number was an MTN-sandbox test MSISDN the
> sandbox settles automatically — no handset was prompted and no real money
> moved; Orange's redirect rail has still never been called; no webhook has
> ever reached a merchant endpoint outside this repository; **no rail has ever
> refunded anything** — `orange_money::refund` is `NotImplemented`, and MTN's
> Disbursements product, which is what a refund on that rail is, **has never
> been called at all**, in sandbox or anywhere else, so `mtn_momo::refund` is
> written and rail-unproven and no deployment even holds the credential it
> needs; and no cluster has ever run vpay. Do not deploy it._
>
> _(That clause said "`mtn_momo::refund` and `orange_money::refund` are both
> `NotImplemented`" until 2026-09-15, when the two halves moved in opposite
> directions on the same day: MTN's token was retired because the Disbursements
> `transfer` call was written, and Orange's `Unsupported` became a token
> because RFC-0003 § 5 decided an Orange refund is an outbound transfer.
> Retiring MTN's token did not narrow the banner — the sentence above says the
> same thing about the rail, which is what the banner is for, and says it about
> a call that now exists and could therefore be mistaken for one that works.)_
>
> _(**Narrowed on one clause, 2026-09-16, and on no other.** "No deployment
> even holds the credential it needs" was true of every deployment and is now
> imprecise about one: the **e2e/demo compose stack** holds a
> `disbursement_subscription_key`, which is a stub string addressed at a
> `wiremock/wiremock` container, put there so the SDKs' live refund suites
> could drive the path end to end. It had no value at all until then, and the
> consequence was that a refund on that stack was a `500` before it reached
> even the stub. **Nothing else in the sentence moved**, and nothing in it is
> weaker: no real MTN Disbursements credential exists anywhere in this
> project, the product has still never been called, and **no rail has ever
> refunded anything**. The four refund routes and both SDKs' refund surfaces
> exist as of 2026-09-16 and **nothing settles a `pending` refund** — there is
> no refund poll ladder — so every refund they can create stays `pending`
> forever. See
> [status/verification/2026-09-16-w3-sdk-refunds.md](status/verification/2026-09-16-w3-sdk-refunds.md).)_

That banner was narrowed by ten dated addenda rather than replaced — Steps 2,
3, 4, 5c, 7 (twice), 8, 9 and exp31, each retiring a specific claim on a
specific date, and the live-sandbox test of 2026-09-15 retiring the "no real
rail" sentence itself. The whole chain, unedited, is in
[status/overall-history.md](status/overall-history.md); it is worth reading if
you want to know what was retired and when, because two of those entries retire
sentences this page no longer makes. **Nothing had been added to it between
2026-09-07 and 2026-09-15** — nothing that landed in that window earned an
addendum: customer erasure and the address object, MTN's failure codes, demo
tenancy, `@vpay/ui`, Gateway API routing, the first CrateStack `procedure`, and
the `/dash/v1` read seam and its BFF each moved a row on a page below without
narrowing the sentence above.

## Where things stand

Each row links to the page that carries the rows, the evidence and the history.
Every 🟡 and ⛔ on those pages is there because a test says so.

| Area                                                     | Today                                                                                                                                                                                                      | Detail                                                                                                                                            |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Backend** — `/v1`, `/dash/v1`, `/provider`, `/browser` | 🟡 Real routes, real rows, real adapters; every rail call went to a stub until 2026-09-15, when the MTN push first went to MTN's real sandbox                                                              | [status/backend.md](status/backend.md)                                                                                                            |
| **Adapters** — `mtn_momo`, `orange_money`                | 🟡 Wire calls proven against WireMock; since 2026-09-15 `mtn_momo`'s **charge path** proven against MTN's real sandbox, ⛔ `mtn_momo::refund` (Disbursements) never called, ⛔ `orange_money` never called | [status/backend.md](status/backend.md), and the payer-window note in [status/backend-orange-hosted-page.md](status/backend-orange-hosted-page.md) |
| **Frontend** — checkout page, dashboard, demo shop       | 🟡 Built and walked by a real browser against a stub rail                                                                                                                                                  | [status/frontend.md](status/frontend.md)                                                                                                          |
| **Infrastructure** — images, compose, Helm, migrations   | 🟡 Boots in compose and in CI; ⛔ no pod has ever run                                                                                                                                                      | [status/infrastructure.md](status/infrastructure.md)                                                                                              |
| **Data layer** — sqlx, CrateStack, the schema            | 🟡 `schemas/vpay.cstack` compiles into `vpay-db`; the migration/model drift is counted, not closed                                                                                                         | [status/cratestack.md](status/cratestack.md), [status/sqlx-and-op-stores.md](status/sqlx-and-op-stores.md)                                        |
| **Merchant SDKs** — `sdks/rust`, `sdks/nodejs`           | 🟡 Parity is machine-checked in both directions; the gaps are dated and owned                                                                                                                              | [status/merchant-sdks.md](status/merchant-sdks.md), [sdks/parity.md](sdks/parity.md)                                                              |
| **An MVP**                                               | 🟡 Two of eight conditions met (items 1 and 6); the other six are decided by one sentence                                                                                                                  | [status/mvp.md](status/mvp.md)                                                                                                                    |

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

`just verify` is **thirteen gates and one advisory report**. What each one
refuses, and what each printed when the first twelve were re-run, one
invocation each, on **2026-09-16** on `888b00c3` — this branch's last commit
before this table was filled in, exactly as the 2026-09-11 column was — with
`DOCKER_HOST=unix:///run/user/1000/docker.sock`. The thirteenth,
`verify-versions`, did not exist then; its column is its own run on
`fix/release-please-yaml-extra-files`, 2026-09-18:

| Gate                  | What it refuses                                                                                                                                                              | Last printed                                          |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `verify-no-mocks`     | a test double reachable from a shipping binary                                                                                                                               | no test double reachable                              |
| `verify-status`       | an undeclared — or a stale — `NotImplemented` token                                                                                                                          | 1 unimplemented item                                  |
| `verify-errors`       | an unclassified error type, or `anyhow` in a library crate                                                                                                                   | 20 error types, 17 `#[from]` variants                 |
| `verify-sdk-parity`   | an SDK capability with no row, or a row naming no capability                                                                                                                 | 603 proving tests, 36 dated gaps, 35 methods, 39 rows |
| `verify-links`        | a repository link that resolves to no tracked path                                                                                                                           | 1 731 links in 375 files                              |
| `verify-npm-scope`    | an unpublishable manifest, or a retired package name outside the record                                                                                                      | 2 publishable packages, 1 private                     |
| `check-schema`        | a `schemas/vpay.cstack` that does not type-check                                                                                                                             | 27 declarations; see the note below                   |
| `verify-serde`        | a serialisable type that does not spell the wire convention                                                                                                                  | 96 types, 17 exemptions                               |
| `verify-repositories` | a repository implementation named outside `vpay-db`, or an exported schema                                                                                                   | 4 implementations, 83 source files outside            |
| `verify-toolchain`    | a `backends/Dockerfile` that drifts from `rust-toolchain.toml`                                                                                                               | 1.98.0                                                |
| `verify-ui`           | a computed class string, a raw status-colour token, a >60-char class, a daisyUI-4 or unrouted daisyUI class, or an import of the deleted `@vpay/ui` (2026-09-12)             | nothing: silent on success, exit 0 only               |
| `verify-migrations`   | an applied migration whose bytes changed                                                                                                                                     | 48 files                                              |
| `verify-versions`     | a release version reference that disagrees, a line release-please must rewrite with no `x-release-please-version` comment, or a bare-string `extra-files` entry (2026-09-18) | 21 version references, all 0.1.1                      |
| `verify-docs`         | **nothing — it exits 0 whatever it finds**                                                                                                                                   | advisory report                                       |

_(`verify-versions` had no row here at all from 2026-09-17, when
[#201](https://github.com/vaam-apps/vpay/pull/201) added it as the recipe's
thirteenth gate, until 2026-09-18 — so this table said twelve while `just
verify` printed thirteen, and it survived the release that gate caught
(#203) and the pull request that repaired it (#204).
[status/gates.md](status/gates.md) § 2026-09-18 is the record.)_

**One of those numbers moved twice on the same day and came back.**
`verify-status` printed **2** unimplemented items partway through 2026-09-15,
not 1: RFC-0003 § 5 gave `orange_money` a `refund` token. It prints **1**
again now, because the same day's MTN work retired
`NotImplemented("mtn_momo::refund")` by writing the Disbursements `transfer`
call — a different token, from a different branch, for a different reason.
The column above is the 2026-09-16 re-run, so it reads **1**. _(It stood at
what the gates printed on 2026-09-11 on `722e579` until 2026-09-16, by which
time six of the thirteen numbers had moved — `verify-errors` 19→20,
`verify-sdk-parity` 550/35/32→603/36/35, `verify-links` 1 600/352→1 731/375,
`check-schema` 26→27, `verify-serde` 90→96, `verify-migrations` 42→48 — and
the column was a measurement of a tree three merges old. Re-running it was cheaper than
carrying the caveat.)_

**And `verify-status` gained a third direction the same day, on review.** It
compared _sets of token strings_ and knew nothing about where a token was
written, so an adapter answering another rail's token was invisible to it.
Measured before the rule existed: with `NotImplemented("orange_money::refund")`
in the Orange adapter replaced by `NotImplemented("mtn_momo::refund")`, the
gate first failed the docs→code way — inviting the wrong repair, "delete the
bullet" — and with the bullet then deleted it printed **"ok — 1 unimplemented
item(s)"**, with a whole rail's gap gone from this page and an adapter blaming
MTN for it. A token whose prefix names a rail this workspace ships an adapter
for must now be carried by _that_ adapter's crate; prefixes that name no rail
are unconstrained, because this repository has no convention saying where such
a token may live. `a_token_naming_another_rail_is_refused_however_well_the_page_matches`
in `.xtask` pins it, in both directions.

**`check-schema` did not run under the version this repository pins**, and the
recipe says so out loud rather than passing quietly: `justfile`'s
`cratestack_version` is `0.12.0`, the CrateStack CLI on the authoring machine's
`PATH` is **0.11.1**, and the check ran in full against the 0.11.1 grammar — 27
model/enum declarations, datasource present. That is a property of one machine,
not of this tree, and it is recorded because a gate that ran against a
different grammar than CI will is a gate whose green means less than it looks.

Those numbers are a measurement of one tree on one day — `888b00c3`, 2026-09-16
— not a promise. **They are `just verify`'s gates invoked one at a time, not
`just verify` itself and not `just ci`**, which this branch's agents were
instructed not to run; that distinction is the whole of what the
thirteen-in-a-row recipe adds. What each
gate used to miss, the mutation that proved each hole shut, and the dates every
one of these counts moved on, are in [status/gates.md](status/gates.md) — 509
lines of it, unedited, plus two dated sections appended below that text. Read
the numbers in it as date-stamps rather than totals: it calls
`verify-sdk-parity` the reader of "the fourth machine-checked document", which
was true on 2026-09-03, when four of these thirteen gates existed.

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

**There is exactly one, down from eight on 2026-09-03 (Step 3) and from two
partway through 2026-09-15 — and the list has moved in both directions, which
is the distinction it exists to keep visible.** All eight of the original
tokens have left it at some point, for two different reasons, and one of the
eight came back.

Seven left because the code was _written_:
`{mtn_momo,orange_money}::{submit, query_status, parse_callback}` are real
HTTP calls now, and `mtn_momo::refund` joined them on 2026-09-15 when the
Disbursements `transfer` call landed. The paragraph after the bullet below,
and the section after that, are the warning that goes with that seventh.

The eighth, `orange_money::refund`, left and came back, and it is the same
token meaning two different things. It left on 2026-09-03 because it was
**not unbuilt work in the first place**: Orange's Web Payment product
documents no refund API, so
the adapter inherited the port's `ProviderError::Unsupported` — a permanent
capability answer the core branches on. It returned on **2026-09-15**, when
the maintainer decided what an Orange refund _is_ (RFC-0003 § 5): an outbound
**transfer** back to a payee. Orange makes transfers, so `supports_refunds` is
`true` and the refusal stopped being a fact about the rail. What is missing is
vpay's call, and that is a token. A rail that will never support an operation
must not be described with the same token as work someone still owes — which
is also why re-listing it took a decision about Orange rather than a decision
about this list.

- `orange_money::refund` — added 2026-09-15 by RFC-0003 § 5. An Orange refund
  is an outbound **transfer** back to the payee: there is no "back the way it
  came" on a redirect rail, where `payer_ref` is `None` and vpay never learns
  who paid, which is why this rail declares
  `RefundDestination::Required`. **No Orange transfer API is documented in
  this repository — not even reconstructed.** The three calls the adapter does
  make were built from Orange Developer's public overview plus community SDKs
  that agree with each other; for transfers no such source exists here, so an
  endpoint path and a request body would be _invented_, in the money path, on
  a rail nobody has ever called. It stays a token until item 5 of
  [flows/adapter-orange-money.md](flows/adapter-orange-money.md)'s "To confirm
  with Orange Cameroun" list has an answer.
  `supports_refunds` is **`true`** — the rail refunds; answering `Unsupported`
  would now be a lie about Orange rather than an admission about us — while
  `supports_partial_refunds` stays **`false`**, decided and not merely left
  over: nothing here knows an Orange transfer's amount semantics, and
  withdrawing a partial-refund capability a merchant had already integrated
  against is a breaking change, where adding one later is not.
  (`refund_is_a_token_about_vpay_not_an_answer_about_orange` in the adapter,
  `a_rail_without_the_refund_capability_answers_unsupported` in the
  conformance suite.)

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

**What has to exist before it is ever anything but `null`.** For MTN, two of
the three landed on 2026-09-15 and the one that matters did not. The
Disbursements keys are in `config/application.yml` (unpopulated in every
deployment but the e2e/demo stack, whose three values are stubs aimed at a
`wiremock/wiremock` container) and `mtn_momo::refund` is written. What is
still missing is **a real Disbursements response that actually carries a
fee**: MTN's documented
transfer response has no fee field, `vpay_adapter_mtn_momo::wire::Transfer`'s
202 carries an empty body, the adapter therefore answers `fee: None`
(asserted by `an_accepted_transfer_reports_no_fee_and_no_key_material` and by
the conformance case), and whether that product reports a fee at all has
never been verified against MTN — **the Disbursements API has never been
called from this repository.** For Orange, since 2026-09-15: an Orange
transfer specification of any kind — this repository has none, so whether that
product reports a fee is not merely unverified, it is unasked. (This paragraph
read "For Orange: nothing, ever — the Web Payment product documents no refund
API … and there is no refund to charge a fee for" until RFC-0003 § 5 decided
that an Orange refund is a transfer back.) **An adapter must not invent one**: `None`
is "the rail did not report a fee" and `Some(0)` is "the rail said it was
free", and collapsing them is the exact defect the issue reports one layer up.

`mtn_momo::refund` left this list on 2026-09-15 for the first reason — the
Disbursements `transfer` call is written (RFC-0003 § 5) — and **a short list
here is the weakest claim this page makes, not the strongest.** A
`NotImplemented` token is one narrow kind of gap: "this function has no body".
A token's absence says nothing about whether a written call has ever been
made, and in the case that just left, it has not. Read the section below and
the **Adapters** row in "Where things stand" before reading this list as good
news.

### `mtn_momo::refund` is written, WireMock-proven and **rail-unproven**

MTN's Disbursements `transfer` call landed on 2026-09-15 (RFC-0003 § 5). It is
real code — a Disbursements subscription key, a separately scoped token from
`POST /disbursement/token/`, and `POST /disbursement/v1_0/transfer` addressed
to the payee the merchant nominated — and **nothing in this repository has
ever called MTN's Disbursements product.** Not in production, not against
MTN's sandbox, not once.

**No deployment of this system holds a REAL Disbursements subscription key.**
`config/application.yml` now carries `disbursement_subscription_key`,
`disbursement_api_key` and `disbursement_api_user`; every deployment but one
leaves all three empty, and `mtn_momo::refund` answers `ProviderError::Config`
naming the first one that is missing. The exception, since 2026-09-16, is the
e2e/demo compose stack, whose three values are stub strings addressed at a
`wiremock/wiremock` container. **Empty, not absent** — corrected on
review 2026-09-15: those three lines added three `${VAR}` names that every
environment loading that file must now define, because an unresolved
placeholder is exit 78 on both binaries before any key check runs. The list
went seven to ten; `.env.example`, `compose.e2e.yml`, `deploy/helm/vpay` and
`docs/runbooks/rotate-rail-credentials.md` carry it, and
`the_repositorys_own_configuration_passes_the_adapter_join` was red on the
arm's branch until the three names were set. So the honest summary is: the call
exists, ~~no caller can reach it (`POST /v1/refunds` is still unrouted)~~
**— corrected 2026-09-16: `POST /v1/refunds` is mounted (RFC-0003 § 2), so a
caller can reach it —** and what it answers on every deployment holding no
credential is "this deployment has no Disbursements credential". The one
exception is the e2e/demo stack, whose stub key reaches a WireMock `202`: that
is a proof about the wire, and MTN's Disbursements product has still never been
called.

What _is_ proven is **seven conformance cases** against a real
`wiremock/wiremock` container — `a_refund_on_a_rail_that_refunds_reaches_the_rail_and_is_accepted`,
`the_refund_is_addressed_to_the_payee_the_merchant_nominated`,
`a_refund_to_a_payee_the_rail_rejects_is_a_decline_and_never_an_accepted_transfer`,
`a_duplicate_refund_reference_is_accepted_and_never_paid_twice`,
`a_refund_never_puts_the_payees_number_in_a_log_line`,
`a_rail_without_the_refund_capability_answers_unsupported` and
`a_required_rail_parses_its_own_destination` — beside the unit tests in
`vpay-adapter-mtn-momo`, which touch **no container at all**: that crate
declares no dev-dependency and runs in process (88 tests, 88 passed, 0
ignored, `cargo nextest run -p vpay-adapter-mtn-momo` on 2026-09-16).
Together they hold that the transfer is
addressed to the nominated payee and not to the charge's payer, that it
carries a bearer minted from the Disbursements token endpoint and the
per-product subscription key (the stub answers 202 for nothing else), that a
refused payee is a decline and not a transport failure, that a duplicate
reference is reported as accepted rather than paid twice, that an unreported
fee stays `None`, and that no refusal and no log line ever carries the
payee's number.

_(This read "seven conformance cases and thirteen unit tests", both "against a
real `wiremock/wiremock` container", until 2026-09-16. Two things were wrong.
The unit tests run against no container, so the sentence credited the stub
with evidence it never produced. And "thirteen" disagreed with the arm's own
dated page, which says "Twelve new unit tests in `vpay-adapter-mtn-momo`"
([status/verification/2026-09-15-mtn-disbursements-refund.md](status/verification/2026-09-15-mtn-disbursements-refund.md)).
Neither figure is reproducible from the tree: "new" and "about the transfer"
are groupings nothing records, and what #178 actually added to that crate is
**24 test functions**, of which 7 are destination parsing rather than the
transfer. The count here is now one a reader can re-run, which is the only
kind worth stating.)_

**What the container suite does _not_ prove, stated because the arm's first
write-up claimed it did**: that a deployment which pasted identical
credentials into both product configurations still gets two separately-scoped
bearers. That suite gives each product different credentials, so the case is
never constructed — remove `Product` from the token fingerprint, collapse the
adapter's two cache slots, or do both, and every case stays green. Two unit
tests in `vpay-adapter-mtn-momo` are the whole of that evidence, one per
mechanism, and the second of them was added on review because collapsing the
slots was caught by nothing at all. **A stub faithful to MTN's published `Transfer` operation but
not to MTN would pass every one of them**, and MTN's portal serves no OpenAPI
schema document for the Disbursement API at all, so the request shape is a
transcription and not a comparison. `docs/flows/adapter-mtn-momo.md` § "Not
proven" carries the list of what a first real call has to check.

**Three things were unsettled and were recorded rather than decided quietly**,
because the `POST /v1/refunds` handler that would settle them did not exist —
`vpay_db::Refunds::create` wrote the row, and nothing routed to it. **That
handler landed on 2026-09-16 and answered the first two; the third is
unchanged.** Item 1 is decided: the handler mints the refund's own
`provider_reference_id`, persists it before the call and passes that, which
`two_partial_refunds_of_one_charge_carry_two_references` proves against the
stub's own request journal. Item 2 is obeyed: an `Ok` leaves the refund
`pending`, and the consequence — **nothing in this repository settles a
`pending` refund** — is now a live gap rather than a warning about a future
one. Item 3 is untouched, because nobody has called that mint. The three as
they were written:

1. **Which reference the transfer carries.** The port hands `refund` one
   `ChargeRef` and a refund needs its own rail reference (migration `0017`'s
   `refunds.provider_reference_id`, which `NewRefund` now requires). The
   adapter uses the reference it is given; if a future handler passes the
   _charge's_, every partial refund after the first gets a `409` and is
   reported **accepted with no money moved**. RFC-0003 carries it as an open
   question.
2. **A 202 is _accepted_, not _settled_.** MTN's `transfer` is asynchronous
   like `requesttopay`, and the port has no refund status read, so
   `Ok(Refunded)` means the rail took the instruction. A write path that
   marked a refund `succeeded` on that basis would be asserting something no
   MTN response has said — and `vpay_db::settlement::apply_refund_succeeded`
   is the statement that would record it.
3. **Whether MTN's Disbursements token endpoint wants the same JSON grant as
   Collections.** Assumed, because PR #177 measured it on Collections against
   the real sandbox. Nobody has called the Disbursements mint.

_(This section replaced the `mtn_momo::refund` bullet that stood here from
Step 3 to 2026-09-15, which said "nothing honest can be built yet". It also
does not begin with a backticked path, and may not: `verify-status` reads
`- ` followed by a backtick as a declared token, and the docs→code half of
the gate would then fail because no shipping code carries a `mtn_momo::refund`
token any more.)_
**Also missing, and larger:** no **shipping** path writes a `refunds` row.
Reading one stopped being missing on 2026-09-06 — issue #45 landed
`vpay_db::Refunds::get_for_merchant` and `GET /v1/refunds/{id}` while this
branch was open, so `RefundObject` does cross a wire and `fee` is `null` on
every object it can produce. _(This paragraph read "nothing **writes** a
`refunds` row … no `create` in the repository" until 2026-09-15, when
RFC-0003 § 3 landed `vpay_db::Refunds::create` and `cancel` — the first
`INSERT INTO refunds` this repository has ever issued, paired with the
reservation on `payment_intents.amount_refund_pending` — plus
`Settlement::apply_refund_succeeded`/`apply_refund_failed` and the first
ledger postings for a refund. See the
"Refunds write path" row in [status/backend.md](status/backend.md).)_

_(Wave 3, 2026-09-16: the four `/v1` refund routes and the first refund events
— see the "four `/v1` refund routes" row on the same page. Two of the three
things named below as missing are no longer missing: `POST /v1/refunds` is
mounted, and `charge.refunded` / `charge.refund.updated` have writers. The
third is unchanged, and a fourth has been added to it.)_

What was still missing after RFC-0003 § 3 was everything between that writer
and a merchant: ~~no `POST /v1/refunds` (it is declared in the wire contract,
mounted nowhere, and is wave 3's)~~, no adapter that has ever executed a
refund — `mtn_momo::refund` is written but its Disbursements product has never
been called and no REAL credential for it exists in this project, `orange_money::refund` is a
`NotImplemented` token, neither answers `Unsupported`, and **no rail call has
ever been made for a refund against a real rail** — and ~~no writer for
`charge.refunded` / `charge.refund.updated`~~, both of which are in the
`type_is_a_documented_event` vocabulary.

**What is still missing, as of 2026-09-16, and it is the whole of what stands
between these routes and a working refund:** no REAL MTN Disbursements
credential exists in this project, so on MTN a refund reaches
`ProviderError::Config` on every stack but the e2e/demo one, whose stub key is
aimed at a `wiremock/wiremock` container and whose `202` proves the wire and
nothing about money; Orange's transfer is unbuilt, so on Orange a refund
is created and immediately `failed` with its reservation released; and
**nothing settles a `pending` refund** — the port has no refund status read
and there is no refund poll ladder (RFC-0003 open question 8, open). A refund
created through `/v1` is therefore _instructed and not paid_, and its
`status` says so.

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

**The most recent entry is 2026-09-16, and NO `just ci` has ever been run on
this refunds branch.** Twenty-seven dated pages under
[status/verification/](status/verification/) carry 2026-09-13 to 2026-09-16,
and **sixteen** of them are RFC-0003's — its five waves, their adversarial
reviews, two merges and this seam pass. Every one of the refunds
pages records the **individual** gates and the **narrow** `cargo nextest`
invocations for the crates its change touched, because every agent on this
branch was instructed not to run `just ci` locally (five concurrent local
builds once OOM-killed the host). **Read that as what it is:** the twelve
gates have been run on this head, and the full workspace test run, the web
job and the doctest sweep that `just ci` adds on top of them have been run on
**no** commit of this branch by anything but GitHub Actions. The gate table
above says which commit its numbers are from.

_(This paragraph said "The most recent entry is 2026-09-11" until 2026-09-16.
It had been wrong since 2026-09-13 and was wrong on `master` as well as here
— `2026-09-13-flutter-lane-b-gate.md` and four more landed under it without
this sentence moving. It is the kind of claim only a reader who opens the
directory can falsify, which is why it survived four sweeps.)_

**The last `just ci` note on this page is 2026-09-11, on
`claude/exp57-docs-split`** — the documentation split this page is the product
of. `just ci` exit 0, exit code
read from a file: **1 748 tests run, 1 748 passed, 0 skipped** across 46
binaries, `test-doc` 113 passed / 1 ignored, twelve gates, `test-web` 1 386.
It moved no capability; the one non-documentation change is `verify-npm-scope`'s
retired-name allowlist, which the split itself broke and which the gate caught.

**The two before it are also 2026-09-11:**
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
