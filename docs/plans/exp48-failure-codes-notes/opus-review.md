# exp48 — sabotage review of `claude/exp48-failure-codes`

Reviewed 2026-09-10/11 against `origin/master` at `524289b` (the branch was
rebased onto it before anything else; the rebase was clean, and #109's prettier
sweep had not landed). The branch as delivered was three commits, `bdbfd2a..acff17c`,
re-based to `5e87b15`.

What this file is for: the **vendor-document diff** the branch's central claim
rests on, re-retrieved independently rather than taken on trust; the four
findings that came out of it; and what was deliberately left alone.

The implementer's own notes are [opus.md](opus.md). They are unusually
candid — §1.3, §3 and §6 volunteer the weaknesses a review normally has to
find — and one section of them, §1.2, is retracted there and here.

---

## 1. The vendor claim, re-retrieved

The load-bearing claim is unverifiable from inside the repository: a snapshot
of MTN's `ErrorReason.code` enum, hand-transcribed into a Rust `const`. A
sabotage review that does not fetch the document itself is reviewing the
transcription against nothing.

Fetched 2026-09-11, unauthenticated, from the URLs the branch cites:

```text
GET https://momodeveloper.mtn.com/developer/apis?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/schemas/668d4753d54e6119240c675d?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/operations/RequesttoPayTransactionStatus?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Remittance/schemas/671b10e9fb426803749ff26e?api-version=2022-04-01-preview
```

All answered `200`. **The portal was reachable, so nothing below is marked
unverified.**

### 1.1 The diff against `PUBLISHED_ERROR_REASONS`

| Check | Result |
|---|---|
| Codes in MTN's enum | 17 |
| Codes in `PUBLISHED_ERROR_REASONS` | 17 |
| In the vendor list, absent from the adapter | **none** |
| In the adapter, absent from the vendor list | **none** |
| Order identical | **yes** |

The transcription is exact, value for value and in the document's own order.
No finding.

### 1.2 The two `UNPUBLISHED_REASONS`

`COULD_NOT_PERFORM_TRANSACTION` and `SENDER_ACCOUNT_NOT_ACTIVE` appear
**nowhere** in the Collection API's components document — not in `ErrorReason`,
not in any of its ninety-odd other schemas. Re-checked against the Remittance
API's components document (whose own `ErrorReason` has twenty-six values,
including a `TRANSACTION_CANCELED.` with a trailing full stop): absent there
too. The Disbursements API publishes no schema through this endpoint, exactly
as [opus.md](opus.md) §1.3 says.

So §1.3 is confirmed in full, and the caveat is stated where an integrator
reads it — `docs/flows/failures.md` (dagger footnote on the per-rail table),
`docs/flows/adapter-mtn-momo.md` (dagger on the mapping table) and
`docs/status.md`, not only in the `UNPUBLISHED_REASONS` constant. No finding.

### 1.3 What the diff turned up that the branch got *wrong in its own favour's opposite*

See **Finding 1**. `ErrorReason` is not, as the branch says in five places, a
schema whose relationship to `requesttopay` is unpublished.

---

## 2. Findings

### Finding 1 — the "load-bearing caveat" is false, and it understates the branch's own evidence
**Severity: misleading-claim.** Fixed.

Five files carried a variant of:

> `ErrorReason` is the whole Collection API's error schema, not
> `requesttopay`'s. MTN publishes no per-operation subset, so a row here is a
> deliberate assumption in the safe direction.

The document says otherwise, at two levels:

* `RequestToPayResult.reason` is `{"$ref": "#/components/schemas/ErrorReason"}`.
  MTN types this exact field with this exact enum.
* `RequesttoPayTransactionStatus` (`GET /v1_0/requesttopay/{referenceId}`) —
  the call this adapter polls — answers `RequestToPayResult` on its `200`,
  described as *"note that a failed request to pay will be returned with this
  status too … the 'reason' field can be used to retrieve a cause in case of
  failure"*. Two of that response's worked examples carry `PAYER_NOT_FOUND`
  and `PAYEE_NOT_FOUND` in `reason.code`. The `404` and `500` responses are
  `ErrorReason` directly.

This is the unusual case of a branch **under**-claiming. `PAYMENT_NOT_APPROVED`
→ `payer_declined` is a citation about `requesttopay`, not an inference from a
neighbouring operation, and a maintainer reading "this is a deliberate
assumption" could reasonably have reverted the three new rows as guesswork.
It also blunts the caveat that *is* real: the two unpublished strings are
missing not from some large shared error schema but from the enum MTN types
the very field the adapter reads.

What remains true, and is now what the caveat says: **a schema types a field,
it does not promise a value.** Nothing here has called MTN, and "Real sandbox"
stays ⛔.

Corrected in `vpay_adapter_mtn_momo::mapping`'s header,
`docs/flows/adapter-mtn-momo.md` (twice), both WireMock metadata blocks, and
[opus.md](opus.md) §1.2, which is retracted in place because four other files
cited it.

### Finding 2 — the one number that reaches `payer_declined` was proven by nothing
**Severity: gate-hole.** Fixed.

The branch added `237600000103`, its `mtn-demo-declined` mapping, and a promise
that typing it reaches `payer_declined` — in `docs/runbooks/demo.md`,
`docs/flows/adapter-mtn-momo.md`, `docs/status.md`, `examples/shop/README.md`
and the shop's test-number panel. **Nothing executed it.** Not the conformance
suite, not `demo-walk` (which sends the hex family), not `checkout.cy.ts`. The
shop's vitest proves a mapping *mentions* the number; it cannot prove the
mapping *answers*.

So issue #59's own shape survived one layer down: the code that had just
stopped being unreachable was reachable only according to prose.

The justification was a sentence in `docs/flows/adapter-mtn-momo.md`:

> `237600000103` (2026-09-10) is such a row, as `237600000503` is — and
> **neither is in the twin test**, because there is no twin to agree with.

`237600000503` had been a case in
`a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin` since exp22
(`adapter_conformance.rs:1888`), and that test's own doc comment carries a
paragraph explaining why it is there *despite having no twin*. The sentence
asserted the opposite of the code three lines above it.

Fixed by adding the missing case, correcting the sentence, and rewriting the
test's doc comment so the test says what it is for (every MSISDN a payer can
type) rather than only where its numbers came from.

### Finding 3 — "reaches `payer_declined` from a browser" was never true
**Severity: misleading-claim.** Fixed.

`docs/flows/adapter-mtn-momo.md` and `docs/status.md` both said the new MSISDN
"reaches `payer_declined` from a browser". No browser has typed it —
`checkout.cy.ts` drives the hex family — and after Finding 2's fix what is
proven is a socket-level walk through `submit` and `query_status`. Both
sentences now say that, and name what does *not* cover it.

### Finding 4 — the branch left a stub comment asserting the opposite of its own new row
**Severity: correctness (documentation).** Fixed.

`demo-outcomes.json` carried, in capitals, from before this branch:

> IT IS NOT SPELLED `EXPIRED`, AND THAT IS DELIBERATE. … **MTN documents no
> `EXPIRED`** … Inventing an MTN `EXPIRED` to make the two rails look alike
> would be this stub asserting something about a rail nobody has called.

The branch added `EXPIRED` → `payer_timeout` to `FAILURE_REASONS` on the
strength of the published enum — which contains `EXPIRED` — and left this
paragraph untouched, in a file it edited. A future reader would have taken the
capitals as authoritative and removed the new row as an invention.

The old sentence conflated two vocabularies. MTN has no `EXPIRED` **status**
(`RequestToPayResult.status` is `PENDING`/`SUCCESSFUL`/`FAILED`, verified), and
it does publish an `EXPIRED` **reason**. The comment now says exactly that, and
records why the stub still answers `COULD_NOT_PERFORM_TRANSACTION` — including
that this string is itself one of the two MTN does not publish.

### Nit — `mtn-demo-decline` and `mtn-demo-declined`, one letter apart
Fixed. The new scenario was named one character longer than its neighbour, which
means `grep mtn-demo-decline` matches both while the two mean different things
to a payer (no balance vs. refused on the handset). Renamed `mtn-demo-refused`;
the rename touches only content this branch introduced.

---

## 3. Verdict on the two deviations

### Deviation 1 — Orange gets no `payer_declined`: **sound, keep it**

`STATUS_TABLE` is the five statuses Orange documents. Read case by case, no
status the adapter can receive means "the payer refused":

* `INITIATED`, `PENDING` → alive; `SUCCESS` → settled.
* `EXPIRED` → `payer_timeout`, and the conformance case
  `the_payers_exit_from_the_hosted_page_decides_the_charge` asserts that a
  payer clicking **Cancel** arrives this way.
* `FAILED` → `provider_error`, because Orange documents no sub-reason for it —
  so it cannot be read as a refusal either.
* Anything else is `ProviderError::Malformed`, never a `Failed`, so an unknown
  word cannot become a decline.

Giving Orange `payer_declined` would have required inventing a `CANCELLED` the
rail does not document. Refusing was correct, and refusing *and saying so in a
checked `cannotExpress` row* is better than either.

### Deviation 2 — the authoritative table lives in `failures.md`: **sound, keep it**

`docs/flows/payment-lifecycle.md` carries a three-row per-rail summary (what
each rail reaches, where its vocabulary comes from, what a refusing payer
produces) and links to the code-by-code table. `failures.md` carries the
taxonomy and the authoritative table. Nothing duplicates the taxonomy; the
lifecycle page states the lifecycle-relevant fact and defers. Putting a second
code-by-code table in `payment-lifecycle.md`, as the brief asked, would have
created exactly the second copy this branch exists to eliminate.

One residual, not fixed: the lifecycle summary hard-codes "all eleven",
"three" and "twelve of them mapped". Those numbers are pinned in Rust and in
the shop's vitest, but not in the doc, so the doc can go stale silently. That
is the drift exposure every prose count in this repository has; flagged, not
special-cased.

---

## 4. Blast radius

| Surface | Disagrees with `PRODUCED_FAILURE_CODES`? |
|---|---|
| `examples/shop` README + panel | No — checked in both directions by `test-numbers.test.ts`, which reads the adapters' Rust |
| `frontends/apps/checkout/src/lib/failures.ts` | **No, and it structurally cannot.** It is a `Record<FailureCode, MessageKey>` — total over the taxonomy, enforced by the compiler, and rail-agnostic. It makes no per-rail reachability claim, so there is nothing for a per-rail list to contradict. The implementer's "not done" item — no `verify` gate covering it — is accurate, and the risk it leaves is nil |
| Both TS SDKs, Rust SDK | Doc prose only; the exported code unions are unchanged eleven |
| `provider_callback`, `webhooks.rs`, `worker_recovery` | Untouched, green |

**Did a code's meaning change under an existing demo number?** `EXPIRED` on MTN
was previously unmapped (`provider_error` by fallback) and is now
`payer_timeout`. Checked against `origin/master`: before this branch, `EXPIRED`
appeared in MTN's stub tree only inside the prose comment of Finding 4 — no
mapping, no conformance case and no demo number produced it. So no existing
outcome changed. On Orange `EXPIRED` was already `payer_timeout`, untouched.

**`demo-walk`'s six outcomes:** `examples/merchant-demo` steers by the hex
MSISDNs (`…0ce0`, `…0f01`, `…0f02`) and two Orange amounts (5001, 5002), none
of which the new mappings intercept — the new arm requires
`payer.partyId == 237600000103` and the new status mapping requires a scenario
state only that arm sets. Unchanged by construction, and confirmed by the run
in §6.

---

## 5. Mutations

| # | Mutation | Expected | Result |
|---|---|---|---|
| 1 | Swap the codes of `EXPIRED` and `PAYMENT_NOT_APPROVED` in `FAILURE_REASONS` (the implementer's, re-run) | conformance fails | **FAILED as required** — see §6 |
| 2 | Promise `insufficient_funds` for Orange in **both** `README.md` and `test-numbers.ts` (the implementer's, re-run) | shop vitest fails | **FAILED as required** — see §6 |
| 3 | **Mine:** delete `FailureCode::PayerDeclined` from MTN's `PRODUCED_FAILURE_CODES` while `…0f05`/`…0f06` still produce it | something must fail | **FAILED as required** — see §6 |
| 4 | **Mine:** delete the new conformance case's row, keeping every document's promise | the gate hole reopens silently | **PASSED — which is the finding.** conformance 53/53, shop vitest 108/108, MTN units 62/62, all green with five documents promising a number nothing ran |
| 5 | **Mine:** make the demo stub answer `APPROVAL_REJECTED` instead of `PAYMENT_NOT_APPROVED` — a *different reason with the same taxonomy code* | only a reason-level assertion can catch it | **FAILED as required**: `case_4_payer_declined`, "expected \"PAYMENT_NOT_APPROVED\" inside \"APPROVAL_REJECTED: The payment was not approved by the payer\"". Under the old `!raw.is_empty()` this mutation was invisible |

Mutation 4 is recorded because it is the one that says why Finding 2 mattered:
the branch as delivered had a green suite and an unexecuted promise, and no
gate could tell the difference. Mutation 5 is why the new case asserts the
reason and not only the code — `PAYMENT_NOT_APPROVED` and `APPROVAL_REJECTED`
both map to `payer_declined`, so the code alone cannot tell the two stubs
apart, and three documents name a specific one.

All five were reverted and the suites re-run green. Mutations 1 and 3 were run
against an uncommitted tree and `git checkout` on the mutated file silently
took a review fix with it; the fix was re-applied and everything was committed
before mutations 4 and 5. Recorded because it is the kind of scaffolding
mistake that otherwise reaches a report as a claim.

---

## 6. Gates

Exit codes read from a file, not from a harness banner.

**Baseline — the branch as delivered, rebased onto `524289b`, no review
commits.** `just ci` **exit 0**: twelve gates ok, `cargo nextest run
--workspace` 1691 tests / 1691 passed / **0 skipped**, doctests 111 passed /
1 ignored, every web suite green, `cargo deny` ok. So exp48 did not ship a red
gate; the hole Finding 2 names is one no gate could see.

**Final — `0136bba`, both review commits in.** `just ci` **exit 0**:

| Recipe | Result |
|---|---|
| `fmt-check` | ok |
| `clippy` | ok, no warnings |
| `verify` | ok — the twelve gates |
| `test-rust` | **1692 run, 1692 passed, 0 skipped** (1691 baseline + the new case) |
| `test-doc` | 111 passed, 1 ignored |
| `verify-ignored` | ok |
| `lint-web` | ok |
| `test-web` | 63 + 8 + 146 + 208 + 4 + 74 + **108 (shop)** + 182 + 507, all passed |
| `deny` | advisories / bans / licenses / sources ok |

Suite-level, run separately: conformance **54 / 54, 0 skipped** (53 as
delivered); MTN units **62 / 62**; Orange units **57 / 57**.

`just demo-walk` on project `exp48-review` (ports 19400/19402/19403,
14400/14401/14480): **exit 0**, and the six outcomes are unchanged —

| # | Rail | Status | Code |
|---|---|---|---|
| 1 | `mtn_momo` | `succeeded` | — |
| 2 | `mtn_momo` | `requires_payment_method` | `insufficient_funds` |
| 3 | `mtn_momo` | `requires_payment_method` | `payer_timeout` |
| 4 | `orange_money` | `succeeded` | — |
| 5 | `orange_money` | `requires_payment_method` | `payer_timeout` |
| 6 | `orange_money` | `requires_payment_method` | `provider_error` |

— which is the walkthrough's own table, and which contains no
`payer_declined`. That is now stated in the runbook rather than left to be
noticed.

**One extra measurement**, because the runbook makes a claim about a browser
path no automated test drives: on the live demo stack, `POST
/collection/v1_0/requesttopay` with `payer.partyId = 237600000103` answered
`202`, and the following status query answered
`{"status":"FAILED","reason":{"code":"PAYMENT_NOT_APPROVED",…}}`. So the
mapping behaves the same way when mounted by `compose.yml` as it does in the
conformance container.

**Two `just ci` runs before that one failed** and are reported rather than
dropped: both on `postgres:16-alpine` container-start timeouts, in
`staff_sign_in::a_disabled_account_is_refused_at_the_stage_route` and
`vpay-db::repositories::record_success_settles_a_delivery_once_…` — tests with
no relationship to this branch. The host's rootless Docker daemon degrades as
never-started containers accumulate (75 at the worst point). Clearing the
abandoned `Created` containers fixed it both times. The daemon was **not**
restarted: the user's `vpay-demo` stack and another agent's Postgres were live
on it, and that is not this review's call to make.

---

## 7. Left alone, deliberately

* **`TRANSACTION_CANCELED` stays `provider_error`.** The implementer reserved
  it for the maintainer and gave three reasons ([opus.md](opus.md) §2). The
  reasons hold, and Finding 1 does not disturb them: MTN publishes the code
  with no description, and typing a field is not the same as saying who
  cancels. A reviewer picking a defensible default here would be taking a
  decision that was explicitly reserved. **Still the maintainer's.**
* **No seventh demo payment.** Decided against, and the runbook now says so in
  as many words rather than leaving a reader to infer coverage. Adding one
  would invalidate §4's pasted transcript and the "six outcomes for six"
  wording in the measurement records further down that page — those are dated
  records of runs that happened, not prose to be edited. The number is now
  proven over a socket, which is stronger evidence than a transcript and does
  not require rewriting a record. **The trade is flagged for the maintainer.**
* **No `verify` gate for the checkout page's `failures.ts`.** See §4: it is
  total and rail-agnostic, so there is no claim for a gate to check.
* **No prettier sweep.** #109 had not landed; this branch does not reformat.
* **No rail has been called.** Unchanged, and the reason every ⛔ in
  `docs/status.md`'s "Real sandbox" column is still ⛔.
