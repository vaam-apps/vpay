<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. §3 below is written so it can be applied verbatim. -->

# Step 9, lane 2b — digits-only twins of the MTN steering MSISDNs

Branch `claude/step9-lane-2b-msisdn`, on top of `5c1950c` (master + lanes 1–3
merged, including lane 3's coordination note `lane-3.md` §4c, which is the gap
this lane closes).

Owns `backends/tests/conformance/wiremock/mtn/mappings/{requesttopay-scenario.json,demo-outcomes.json}`
and one new test in `backends/tests/conformance/tests/adapter_conformance.rs`.
Touched nothing under `frontends/`, `examples/merchant-demo`, the existing
Cypress spec, `docs/status.md` or `docs/flows/`.

## 1. The gap, restated

`examples/merchant-demo` and `frontends/tests/e2e/cypress/e2e/checkout.cy.ts`
steer MTN's WireMock stub by MSISDN using three documentation numbers that
carry hex letters — `237600000ce0`, `237600000f01`, `237600000f02` — so a
mapping can key on them. `frontends/apps/checkout`'s MSISDN form
(`src/lib/msisdn.ts`) validates Cameroon E.164 — `237` + `6` + eight
**digits** — and correctly refuses all three, as it refuses any other
non-digit input. Lane 3 flagged this as coordination item 4c and named the
fix precisely: "add a digits-only twin to `requesttopay-scenario.json`
(`237600000100`, say, in the same `2376000000xx` documentation block) keyed to
the same scenario," in the mappings, not in the phone-number validator.

## 2. What landed

| # | Thing | Where |
|---|---|---|
| 1 | `237600000100` — joins `mtn-e2e-poll`, same walk as `237600000ce0` (PENDING then SUCCESSFUL) | `backends/tests/conformance/wiremock/mtn/mappings/requesttopay-scenario.json` |
| 2 | `237600000101` — arms `mtn-demo-decline`, same walk as `237600000f01` (FAILED/NOT_ENOUGH_FUNDS → `insufficient_funds`) | `backends/tests/conformance/wiremock/mtn/mappings/demo-outcomes.json` |
| 3 | `237600000102` — arms `mtn-demo-expiry`, same walk as `237600000f02` (FAILED/COULD_NOT_PERFORM_TRANSACTION → `payer_timeout`) | `backends/tests/conformance/wiremock/mtn/mappings/demo-outcomes.json` |
| 4 | `a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin`, an `rstest` with 3 cases (`case_1_settles`, `case_2_insufficient_funds`, `case_3_payer_timeout`), MTN only | `backends/tests/conformance/tests/adapter_conformance.rs` |
| 5 | Corrected the two mappings' own "no conformance case sends that MSISDN, so this mapping is unreachable in a conformance run" claims, which item 4 makes no longer true for the digits-only family | both mapping files, inline |

**How the twin joins the scenario.** Each POST mapping's
`matchesJsonPath` grew from `"equalTo": "237600000f01"` to
`"matches": "237600000(f01|101)"` (regex, one stanza, both PartyIds) rather
than a duplicated stanza — the two answers are identical (`newScenarioState`,
`priority`, `response`), so a second stanza would have been the same rule
copy-pasted with nothing to keep it in sync with the first if the response
ever changed. The paired GET mappings needed no change: they already match
`urlPathPattern: "/collection/v1_0/requesttopay/.*"` — "whatever the
reference" — because MTN's status query carries no body and cannot be
steered by anything but scenario state.

**How the new test proves it, not just documents it.** Every existing
wire-level case in `adapter_conformance.rs` calls `query_status` on a
manufactured `ChargeRef` (`Rail::charge`), which fixes `payer_ref` at the
placeholder `237600000000` and steers by *reference* instead — that is
deliberately not reachable through these three mappings (see the "unreachable
in a conformance run" corrections above). The new test instead calls
`adapter.submit()` for real with `payer_ref: Some(msisdn)` and a fresh random
`reference_id` (mirroring `Uuid::new_v4()` in the confirm handler), then polls
`query_status` on the reference `submit` returned — the same walk a real
confirm takes. It is MTN-only and not parameterised over `RailUnderTest`:
Orange is a redirect rail whose submit body carries no MSISDN, so there is
nothing on that rail to steer by a payer's typed phone number.

## 3. Verbatim rows for `docs/flows/adapter-mtn-momo.md`'s steering table

The steering table does not exist yet in that file; §6 below is the section
to add, applying it as-is closes lane 3 coordination item 4c.

### §6 to add — "Documentation MSISDNs (steering table)"

Every one of these is a WireMock scenario key, not a real subscriber. Each
row's two MSISDNs enter the **same** scenario by the **same** mapping (one
`matches` regex, not two mappings) and therefore the same walk — proven by
`a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin` in
`backends/tests/conformance/tests/adapter_conformance.rs`.

| Outcome | Hex MSISDN (not a valid E.164 number — `examples/merchant-demo`, `checkout.cy.ts`) | Digits-only MSISDN (a real Cameroon E.164 number — `frontends/apps/checkout`) | Scenario | First status query | Second status query |
|---|---|---|---|---|---|
| The payer approves | `237600000ce0` | `237600000100` | `mtn-e2e-poll` (`requesttopay-scenario.json`) | `PENDING` | `SUCCESSFUL` |
| The payer has no balance | `237600000f01` | `237600000101` | `mtn-demo-decline` (`demo-outcomes.json`) | `FAILED` / `NOT_ENOUGH_FUNDS` → `insufficient_funds` | — (terminal on the first query) |
| The prompt expires unanswered | `237600000f02` | `237600000102` | `mtn-demo-expiry` (`demo-outcomes.json`) | `FAILED` / `COULD_NOT_PERFORM_TRANSACTION` → `payer_timeout` | — (terminal on the first query) |

The hex family is what `examples/merchant-demo` (`Steering::Msisdn`) and
`frontends/tests/e2e/cypress/e2e/checkout.cy.ts`
(`MTN_E2E_POLL_MSISDN`) send — both build the confirm request body themselves,
never through a phone-number form, so the letters never have to survive
validation. The digits-only family is what a real payer typing into
`frontends/apps/checkout`'s MSISDN field (`src/lib/msisdn.ts`, Cameroon E.164:
`237` + `6` + eight digits) can actually send — that validator correctly
refuses the hex family, as it refuses any other non-digit input, which is why
this table has two columns and not one.

## 4. What this lane did **not** do

- **Did not touch `examples/merchant-demo` or `checkout.cy.ts`.** Both keep
  using the hex MSISDNs; nothing about their behaviour changed, and this
  lane's own guard-failure proof (§5) shows the hex family still works.
- **Did not write `frontends/tests/e2e/cypress/e2e/checkout-hosted.cy.ts`.**
  That spec does not exist yet — it is lane 6's, over
  `frontends/apps/checkout` — and lane 3 §4c named this lane's mapping fix as
  the precondition for it, not a replacement for it. The digits-only MSISDN
  `237600000100` is now available for that spec to type into
  `frontends/apps/checkout`'s form; nothing here wires it in.
- **Did not add an Orange equivalent.** Orange Money is a redirect rail; its
  submit body carries an `order_id`, not an MSISDN, so there is no payer-typed
  field on that rail for a documentation number to steer by, and no gap to
  close there.
- **Did not edit `docs/flows/adapter-mtn-momo.md`, `docs/status.md` or
  `docs/roadmap.md`.** Lane E's to apply from §3 above.

## 5. Guard-failure proof

Broke the guard, ran the case, watched it fail, restored the file, ran the
case again.

Edited `requesttopay-scenario.json`'s `mtn-e2e-poll` entry-point mapping's
`matches` regex from `"237600000(ce0|100)"` back to `"237600000(ce0)"` —
i.e. removed the digits-only alternative — leaving everything else,
including the paired GET mappings, untouched:

```
$ cargo nextest run -p vpay-tests-conformance --retries 0 -j 1 \
    -E 'test(a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin)'

FAIL a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin::case_1_settles

thread '...case_1_settles' panicked at backends/tests/conformance/tests/adapter_conformance.rs:1361:13:
assertion `left == right` failed: msisdn 237600000100: first query
  left: Succeeded { provider_txn_id: Some("1234567890") }
 right: Pending

Summary [1.107s] 1/3 tests run: 0 passed, 1 failed, 30 skipped
```

With `237600000100` unmatched, its POST falls through to
`requesttopay.json`'s ordinary catch-all 202 (the `mtn-e2e-poll` scenario
never leaves `Started`), so the subsequent GET falls through to
`requesttopay-status.json`'s priority-10 catch-all `SUCCESSFUL` instead of
the scenario's `PENDING` — the failure names exactly the difference the guard
exists to prevent, not a generic connection error. File restored (`git diff`
against the base tree shows only the intended additions); the case passes
again:

```
$ cargo nextest run -p vpay-tests-conformance --retries 2 -j 1 \
    -E 'test(a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin)'

PASS a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin::case_1_settles
PASS a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin::case_2_insufficient_funds
PASS a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin::case_3_payer_timeout

Summary [4.106s] 3 tests run: 3 passed, 30 skipped
```

## 6. Counts, measured

| Gate | Result |
|---|---|
| `cargo nextest run -p vpay-tests-conformance --retries 2 -j 1` | **33 tests run: 33 passed, 0 skipped** (was 30 before this lane) |
| `just verify` | ok — all four gates (`verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity`) pass; `verify-docs` unchanged in shape |
| `just verify-ignored` | `0 ignored (expected 0), 41 test binaries (expected 41), 1082 total (minimum 1000)` — the workspace total grew by 3 with this lane's new test; `expected_ignored`/`expected_suites` in the `justfile` needed no change, and the `min_tests` floor comment says explicitly that three tests is not a reason to move it |
| `cargo fmt --all --check` | ok |
| `cargo clippy -p vpay-tests-conformance --all-targets -- -D warnings` | ok, no warnings |
