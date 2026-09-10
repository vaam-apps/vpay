# exp43 — the sabotage review of the Orange stub's payer window (issue #58)

Review notes for `claude/exp43-orange-stub-race`, rebased onto `origin/master`
at `2c5ef8b` (which carries #99 and #101). The implementation notes are
[`opus.md`](opus.md); this file is what a second pass measured, what it found
wrong, and what it deliberately did not change.

**Verdict: safe as delivered — no.** Nothing in the *stub* was wrong; the
mappings do what they say and the four suites are green. What was wrong was
what the repository says about itself: `docs/status.md`'s two rows on this
exact subject still read as they did before the fix, one of the delivered
numbers is off by a factor of six, and the two limits the new shape has were
written only into a `metadata` block that nothing renders. Every finding is
fixed on this branch; none of them required touching the design.

## What was measured, and how

Everything below is a measurement on this branch's tree, not a reading of it.
Own compose project `exp43-review` on ports 13900-13902/18900-18902; the
maintainer's `vpay-demo` was never addressed.

### 1. The window, from the stub's own request journal, in a real browser

A human (this reviewer, in the Browser pane) bought a coffee on `examples/shop`,
chose Orange, landed on the rail's stub page and then **did nothing**:

| event | offset |
|---|---|
| `POST …/v1/webpayment` — the submit | T |
| `GET /stub-hosted-page/{pay_token}` — the browser lands | **T + 53 ms** |
| `POST …/v1/transactionstatus` → `PENDING` | T + 170 ms |
| → `PENDING` | T + 10.192 s |
| → `PENDING` | T + 30.233 s |
| → `PENDING` | T + 60.289 s |
| → **`EXPIRED`** | **T + 105.398 s** |

105.4 s, which is `poll_delay(0..3)` = 10 + 20 + 30 + 45 to the millisecond
budget of a container. `vpay-worker`'s log settles it:
`the rail reported a charge as failed … failure_code: "payer_timeout"`, with
`a checkout session was settled beside the intent it drives … paid: false`.
**A payer who never clicks reaches `payer_timeout`, not `paid`** — the delivered
claim, confirmed on the stack rather than in a suite with no worker in it.

Note which rung answered the first poll: the browser arrived at 53 ms and the
worker asked at 170 ms, so the payer's own chain answered it and the
unconditional rung was never used. That is the right relationship — the
unconditional rung is *insurance*, and on a quiet machine the browser does not
need it.

### 2. The margin, under load ×5

Five CPU hogs, then the same journey, this time typing `237600000400` — the
number the shop's README says is refused:

| event | offset |
|---|---|
| `POST …/v1/webpayment` | T |
| `GET /stub-hosted-page/{pay_token}` | **T + 60 ms** |
| `GET …/pay?…&msisdn=237600000400` — the payer submits the form | T + 12.123 s |
| `POST …/v1/transactionstatus` → **`FAILED`** | T + 60.956 s |

Two things. First, **the documented Orange test number worked from a browser**:
`vpay-worker` settled it `provider_error`, which is what
`examples/shop/README.md` promises and what a run on 2026-09-06 came back
`paid` for. That is issue #58, closed, end to end, outside any test suite.

Second, the margin question is **monotone in load and the brief's framing had
it backwards**. Under load the worker's *first* poll was 61 seconds late — it
had not asked at all while the payer read the page and typed. Load delays the
poller; the payer's browser is one `302` and one `GET` out of the same submit.
So the ~10 s the unconditional rung buys is a **floor** on the margin, not a
ceiling, and the risk it insures against is a *fast* machine, not a slow one.
Measured first polls across the runs on this tree: 170 ms, 591 ms, 60.96 s.
`vpay_worker::run_loop::IDLE_SLEEP` is one second (read, and pinned by
`the_gauge_and_the_idle_poll_are_ordered_against_the_ladder`), so the second
poll lands at first + 10 s + up to one more; measured 10.02 s and 10.05 s.

### 3. `just demo-walk`, and the number that was wrong

`just demo-up` + `just demo-walk` on the review's own project: **exit 0, six
payments on two rails, 58 seconds end to end.** From the Orange stub's journal
for the same run:

| outcome | submit → first status | polls it took |
|---|---|---|
| 4/6, 5000 XAF (the catch-all) | +0.591 s `PENDING`, then +10.680 s `SUCCESS` | **7** |
| 5/6, 5001 XAF | +0.466 s `EXPIRED` | 2 |
| 6/6, 5002 XAF | +0.414 s `FAILED` | 2 |

So the priority argument holds exactly as delivered — 5001 and 5002 are still
terminal on the first rung — and **the delivered `docs/status.md` was wrong
about the cost.** It said the settling Orange outcome "now settles after **41**
of the demo's own polls where it used to settle on the first". It settles after
**7**, which is the same as the settling MTN outcome and is `SETTLE_POLL_INTERVAL`
(2 s) into a ten-second rung. 41 polls would be 82 seconds, which no rung of
`poll_delay` produces; it looks like a number taken while the authoring host
was at load 14, where the worker's claim loop is starved (this review saw the
same starvation in §2). The row is corrected to the measured 7, with the
walkthrough's total wall clock beside it, because "how long does the demo take
now" is the question that number is there to answer.

### 3b. Cypress, and the two ports two agents collide on

`just test-e2e` on the review's own project: **19 tests, 19 passing, 0
failing**, exit 0 — `checkout.cy.ts` 1, `dashboard.cy.ts` 8,
`shop-hosted.cy.ts` **4**, `shop-embedded.cy.ts` 6. Then `shop-hosted.cy.ts`
**five times over against one standing stack: 20 of 20**, with the new cancel
case at 5.42 / 5.35 / 5.47 / 5.38 / 5.34 s — a 130 ms spread. The *pay* case
is bimodal, 3.7 s twice and ~13.9 s three times, and that is the unconditional
rung visible in the wall clock: when Cypress's click beats the worker's first
poll the charge settles at once, and when it loses, the first poll answers
`PENDING` and it settles on the next rung. Both end `paid`, which is exactly
why the *cancel* case is the one that gates the fix.

**Two runs before that failed for a reason that is not this branch's**, and it
is worth writing down because it will bite the next person. The Cypress side of
`just test-e2e` starts two fixture servers on **fixed** ports —
`checkoutBrowserServer.ts` on 4180 and `frameFixtureServer.ts` on 4181 — each
overridable by an environment variable (`CHECKOUT_BROWSER_PORT`,
`VPAY_E2E_FRAME_FIXTURE_PORT`) that `just test-e2e` does **not** set from a
`just` variable, unlike every one of the seven compose ports beside them. A
concurrent `just test-e2e` in another worktree (exp44's, here) takes both, and
the failure reads as `EADDRINUSE` inside a Cypress plugin or, worse, as
`cy.visit()` failing with `ECONNREFUSED` on a port the spec never chose. This
review worked around it with the two environment variables. It is **on
`master`**, not introduced here, so it is not fixed on this branch; it belongs
with `demo_project` and its six siblings.

### 4. WireMock's double stache — the delivered correction is right

The delivered notes retract an old comment claiming `{{…}}` HTML-escapes.
Re-measured independently against `wiremock/wiremock:3.9.2` with
`v=https://m.test/back?q="x"&y=<z>`:

```
DOUBLE=[https://m.test/back?q="x"&y=<z>]
TRIPLE=[https://m.test/back?q="x"&y=<z>]
ENC=[https%3A%2F%2Fm.test%2Fback%3Fq%3D%22x%22%26y%3D%3Cz%3E]
DEC=[https://m.test/back?q="x"&y=<z>]
```

Identical. The retraction is correct, `urlEncode` does protect the two hrefs,
and `decode=true` exists — which matters below.

### 5. Two Orange charges at once, which nothing had measured

The delivered mappings say "two Orange charges in flight against one stub share
it and the wrong one could be answered". That is an argument. Driven against
the committed mappings on a bare container:

| what happens | what the stub answers |
|---|---|
| A submits, B submits, A polls twice | A `PENDING` then `SUCCESS`; **B's first poll is `SUCCESS`** — A consumed B's rung |
| C submits, D submits, C's payer clicks `Cancel`, D polls | **D gets `EXPIRED`** — C's payer decided D |
| G submits at 5001, H submits, H's payer clicks `Pay`, G polls | **G gets `SUCCESS`** — the payer-action mappings are priority 3, `demo-outcomes.json`'s amount-keyed ones are 4 |

The third is new with this branch and is the sharp one: before it, a payer's
click never reached the container at all, so it could not decide anything —
right or wrong. A new accepted submit resets the scenario to `submitted`, so
nothing is inherited across a quiet boundary; two charges *overlapping* is not
fixable in WireMock, whose scenarios cannot be keyed on a path variable.

**Nothing in this repository is exposed**, and that was checked rather than
assumed:

* `just demo-walk` is strictly sequential (`run_outcomes` finishes each row —
  settled and its webhook verified — before the next confirm) and opens no
  page, so it never arms anything; §3's journal is the evidence.
* both Cypress specs that touch Orange pay one at a time, and `cypress run`
  walks spec files in series.
* **every Rust suite gets its own container per test.** The conformance
  harness's `start()` and `confirm_rails`'s `harness()` both call
  `vpay_testkit::containers::start_wiremock`, which starts a fresh
  `wiremock/wiremock:3.9.2` on each call. The brief's suggestion to "make the
  conformance harness serialise" is unnecessary, and saying so is the point of
  having looked.

The limit is now written on the stub's rendered page (`id="stub-limits"`), in
`examples/shop/README.md`, in `docs/runbooks/demo.md`, in `docs/status.md` and
in the mapping's own metadata. Whether the payer-action mappings should sit
*below* the amount-keyed demo mappings instead of above them is a **maintainer
decision** and is not taken here: the argument for 3 is real (a payer standing
on the page is a stronger statement than an amount a merchant chose) and so is
its cost (the third row above).

### 6. The form's hidden inputs are an injection point, and `checked_return_url` would not stop it

The delivered notes record that the form stays vulnerable to a `return_url`
containing a double quote. Two things they do not say, and now do:

* `vpay_api::v1::payment_intents::checked_return_url` validates the **scheme
  and the length and nothing else**, so vpay would accept such a URL. The hole
  is reachable from `/v1`, not only from a hand-edited fixture.
* A fix exists: write the hidden inputs `{{{urlEncode …}}}` and split the
  `/pay` mappings on the **presence of `msisdn`** — only the form sends it —
  so the form's arm can answer `{{{urlEncode … decode=true}}}` while the
  links' arm is untouched. §4 proves both helpers behave.

It is **not** taken here. It is surgery on a stub four suites share, for a
value nothing in this repository sends, and validating it needs a full
`just test-e2e`. Maintainer decision — recorded on the stub's own page, where
a person driving the demo will meet it, rather than only in a `metadata` block.

## The gate, recipe by recipe

`just ci` end to end on `a7e4c35`, rebased onto `origin/master` `2c5ef8b`,
exit code read from a file: **exit 0**.

| recipe | result |
|---|---|
| `fmt-check` | clean |
| `clippy` (`--workspace --all-targets -D warnings`) | clean |
| `verify` | **twelve gates ok**; `verify-docs` is advisory and never fails |
| `test-rust` | **1669 run, 1669 passed, 0 skipped**, 25 m 20 s |
| `test-doc` | **111 passed, 1 ignored** (`sdks/rust`'s README block, pre-existing) |
| `verify-ignored` | 0 ignored (expected 0), 45 binaries (expected 45), **1669** total |
| `test-web` | **1284 passed** — checkout 507, dashboard 172, `sdks/nodejs` 208, `sdks/stripe-js` 146, shop 102, ui 74, config 63, tokens 8, api-client 4 |
| `deny` | `advisories ok, bans ok, licenses ok, sources ok` |

Beyond `just ci`, on the review's own compose project (`exp43-review`, ports
13900-13902/18900-18902, torn down; the maintainer's `vpay-demo` never
addressed): `just demo-up` + `just demo-walk` exit 0, six payments, **58 s**;
`just test-e2e` exit 0, **19 tests, 19 passing**; `shop-hosted.cy.ts` ×5,
**20 of 20**.

**Five runs of `just ci` on this tree died before that green** — three the
implementer's, two the review's — every one on
`postgres:16-alpine container starts … container startup timeout`, a different
test each time, never an assertion, on a host at load average 10-16 from other
agents' work. Both of the review's named tests were re-run in isolation and
passed in 9.2 s and 3.1 s. The green was taken by waiting for the host's load
average to fall below five and then starting; that is written into
`docs/status.md` rather than left as folklore.

## Findings

| # | severity | finding | disposition |
|---|---|---|---|
| 1 | misleading-claim | `docs/status.md`'s "The demo's fake test numbers" row still said **"Orange's numbers do not work from a browser at all"** in bold, as its reason for the amber, and closed with "Left as a maintainer decision … not closed and not hidden". The decision is taken, on this branch | fixed, `342efed` |
| 2 | misleading-claim | the same file's `shop-hosted.cy.ts` row still explained its deviation from the Step 9 plan with "the Orange stub's cancel link is the same return URL", so a payer who cancels is paid — the half this branch removed, and there is a fourth case in that spec now | fixed, `342efed` |
| 3 | correctness | the shared-scenario limit was an argument in a `metadata` block; measured, it has three distinct shapes, one of which lets a click on one charge's page pay a charge the README promises expires | fixed, documented in five places, `69df52b`; the priority question raised as a maintainer decision |
| 4 | rule-break | the injection residue was recorded only in `metadata`; nothing renders it, and the demo serves that page to a person | fixed, `69df52b` — on the page, asserted by `the_hosted_pages_pending_chain_is_bounded_and_ends_in_an_expiry` on the `id` |
| 5 | misleading-claim | "settles after **41** of the demo's own polls" — measured **7** | fixed |
| 6 | nit | the delivered gate block's counts (1663 tests, `verify` "twelve gates") predate the rebase onto #101 | fixed with this review's own run |
| 7 | nit | `docs/runbooks/demo.md` §4's "verbatim and complete" transcript carries outcome 4's old `selected by:` line | left as it is — it is dated and pinned to a commit, and hand-editing a transcript labelled verbatim is worse than the staleness. Noted in the section instead |

Nothing was found wrong with the mappings, the priorities, the arithmetic case,
the Cypress cases or the decision to leave `run_at = now()` alone. No test was
weakened; the two the branch rewrote
(`the_stub_hosted_page_links_to_the_return_url_the_submit_carried` and
`a_test_number_typed_on_the_rails_hosted_page_reaches_the_documented_outcome`)
are strictly stronger, and this review added one assertion and removed none.

## The brief's own mutation expectation, again

The delivered notes correct the task brief's "shorten the chain to zero → the
browser case FAILS", and they are right. This review's §2 is the same fact from
the other side: the *payer's* clock and the *worker's* clock are only loosely
coupled, so a browser spec is a gate on the **click arming** and never on the
**length** of the window. The three properties and their three gates
(conformance for "is there a bounded window", the arithmetic case in
`confirm_rails` for "how many seconds", the Cypress cancel case for "does the
click decide it") are the right split and none is redundant.

## What this review did not do

* **Did not re-litigate priority 3 over 4**, or the choice of `EXPIRED` for a
  cancel, or leaving the confirm handler's `run_at = now()` alone. All three
  are argued in the delivered notes and all three are defensible; the first is
  now a written maintainer decision with its cost measured beside it.
* **Did not fix the form's injection point.** §6.
* **Did not run the mutations again.** They are recorded in `opus.md` with the
  failing assertions quoted, the mapping mutations were applied to a copy and
  the committed file was restored byte for byte, and this review re-derived the
  same properties from the mappings and from the journal instead. The one
  mutation this review *did* run is its own: truncating the stub's page body
  was caught by `a_test_number_typed…` on the hidden inputs, first try.
* **Did not test a real Orange sandbox.** Nothing in this repository ever has.
