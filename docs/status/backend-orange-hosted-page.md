# The Orange stub's hosted page grew a payer's window (2026-09-10, issue #58)

_Archived from [docs/status.md](../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](README.md) is the index of them._

#### The Orange stub's hosted page grew a payer's window (2026-09-10, issue #58)

**This is a change to a stub, and to nothing else.** No adapter, no handler,
no ladder and no configuration moved; `vpay-adapter-orange-money`'s status
table, `vpay_api`'s confirm handler and `vpay_worker::poll_delay` are byte for
byte what they were. What changed is
`backends/tests/conformance/wiremock/orange/mappings/` — the directory
`compose.yml`, `compose.e2e.yml`, `compose.demo.yml` and both Rust suites all
bind-mount, which is why one PR had to make four suites green.

**What was wrong.** The demo's three documented Orange test numbers
(`237600000000`, `237600000102`, `237600000400`) were advertised on the shop's
own checkout panel and in `examples/shop/README.md`, and **could not be
reached from a browser**. The confirm handler enqueues the first status query
at `now()`, the worker's idle sleep is one second, and the stub's priority-10
catch-all answered `SUCCESS` — measured on the demo stack on 2026-09-06 from
the stub's own request journal: submit at T, first `transactionstatus` at
T+449 ms, the payer's form at T+11.96 s, and the order came back **paid** for
`237600000400`. That is a false green on a payment demo, which is the failure
mode `CLAUDE.md` names first. exp22 found it, wrote it down in five places and
left the fix as a maintainer decision.

**What the stub does now.** One WireMock scenario,
`orange-hosted-page-payer`: an accepted `webpayment` arms it, the first
`transactionstatus` is answered `PENDING` **unconditionally**, and the second
falls back to the answer the catch-all always gave. If a payer's browser loads
the hosted page — at any point — it arms a **bounded chain of four more
`PENDING` answers**, which on `poll_delay`'s rungs is 10 + 20 + 30 + 45 =
**105 seconds** of thinking time, after which the page **expires**
(`EXPIRED` → `payer_timeout`). The page's `#pay` and `#cancel` links, which
used to point straight at the merchant's URL and so were invisible to the
stub, now go through the container and `302` on: `#pay` arms `SUCCESS`,
`#cancel` arms `EXPIRED`, and the test-number form arms what it always did.

**Why the unconditional rung, and what it costs.** Arming only on the page's
`GET` is a race — 44 ms against 449 ms out of the same submit, decided by
where in its one-second sleep the worker was — and a browser that usually wins
is a flaky gate. One `PENDING` that depends on nothing but the submit turns
that 400 ms margin into ten seconds. It is the **whole** cost of this change:
an Orange charge answered by the catch-all settles one rung later than it did.
Nothing else moved, and that is a property of the priority ladder rather than
a hope — the two new unconditional mappings sit at priority 6, _below_
`demo-outcomes.json`'s amount-keyed mappings (4), so `just demo-walk`'s 5001
and 5002 outcomes are still terminal on the first rung; and the payer-driven
mappings sit at 3, _below_ `transactionstatus.json`'s `order_id`-keyed cases
(1), so every conformance reference case is untouched.

**Deliberately not done:** the one-line change to the confirm handler's
`run_at = now()`. An immediate first poll is a property
([flows/crash-safety.md](../flows/crash-safety.md) — a charge is asked about as
soon as it exists), and slowing it to make a demo comfortable is what ADR-0003
exists to refuse. Also not done: a `CANCELLED` rail status. Orange documents
five and that is not one of them, so a payer who cancels gets `EXPIRED` →
`payer_timeout`, and the shop's README and the demo runbook now say so where
they used to say the order stayed open.

**The limit this shape has, measured rather than reasoned about (added by the
sabotage review, 2026-09-10).** The payer's whole state machine is **one
WireMock scenario per container**, `orange-hosted-page-payer`, and WireMock
scenarios cannot be keyed on a path variable — so it is keyed on nothing per
charge. That was true of the two demo arms before this change as well; what is
new is that _every_ accepted Orange submit now moves it and _every_ payer
action now arms a terminal answer through it. Driven against
`wiremock/wiremock:3.9.2` with the committed mappings and nothing else:

| two charges in flight                                                  | what the stub answers                                                                                                                                                                    |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A submits, B submits, A polls twice                                    | A `PENDING` then `SUCCESS`; **B's first poll is `SUCCESS`** — A consumed B's rung                                                                                                        |
| C submits, D submits, C's payer clicks `Cancel`, D polls               | **D gets `EXPIRED`** — C's payer decided D                                                                                                                                               |
| G submits at amount `5001`, H submits, H's payer clicks `Pay`, G polls | **G gets `SUCCESS`** — the payer-action mappings are priority 3 and `demo-outcomes.json`'s amount-keyed ones are 4, so a click overrides a number the shop's README promises will expire |

A new accepted submit resets the scenario to `submitted`, so nothing is
inherited across a _quiet_ boundary; what is not fixable in WireMock is two
charges overlapping. **Nothing in this repository is exposed:** `demo-walk` is
strictly sequential and opens no page at all (measured: `5001` → `EXPIRED`,
`5002` → `FAILED`, `5000` → `PENDING` then `SUCCESS`, each on its own submit),
both Cypress specs pay one at a time, and every Rust suite — conformance and
integration alike — calls `vpay_testkit::containers::start_wiremock` per test
and so gets its own container and its own scenario. No harness needed
serialising. The limit is now stated in four places a reader will actually
reach: the mapping's metadata, `examples/shop/README.md`, `docs/runbooks/demo.md`
and **the stub's rendered page itself**. Whether the payer-action mappings
should sit _below_ `demo-outcomes.json`'s amount-keyed ones instead of above
them is left as a maintainer decision; the argument for 3 is in the mapping's
own metadata and the cost of 3 is the third row above.

**The stub page's hidden inputs are an HTML-injection point, and this change
does not close it.** `checked_return_url` validates scheme and length only, so
a `return_url` containing `"` is accepted by `/v1`, reaches the stub's form as
`<input type="hidden" name="return" value="…">`, and closes the attribute
early — on the WireMock origin, in the demo. The two `href`s added by this
change are immune because they percent-encode with `urlEncode` first; the form
is not, because a GET form's hidden input must carry the value the browser will
re-encode. The fix that exists is a decoding hop — hidden inputs written
`{{{urlEncode …}}}`, and the `/pay` mappings split on the presence of `msisdn`
(only the form sends it) so the form's arm can answer
`{{{urlEncode … decode=true}}}` while the links' arm stays as it is. It was
**not** taken here: it is surgery on a stub four suites share, for a value no
test in this repository sends, and it needs a full `test-e2e` to validate. It
is a maintainer decision, recorded on the stub page rather than only in the
mapping's metadata.

**A wrong comment corrected while doing it.** `stub-hosted-page.json` claimed
WireMock's double stache HTML-escapes and that the form's hidden inputs relied
on it. Driven against `wiremock/wiremock:3.9.2` with a return URL of
`https://m.test/back?q="x"&y=<z>`, it does not: the value reaches the
attribute unescaped, quotes and all. The two new hrefs are safe because they
percent-encode with WireMock's `urlEncode` helper instead of relying on the
stache; the _form_ is still vulnerable to a return URL containing a double
quote, which is now written down in the mapping rather than mis-described, and
is not fixed here (a GET form's hidden input must carry the decoded value, and
WireMock cannot escape an attribute).

**Proven by**, all against real containers:

- `backends/tests/conformance` — **51 tests, 51 passed, 0 skipped** (47 before
  this change). The four new ones:
  `a_charge_no_payer_has_looked_at_is_pending_once_and_then_settles` (the
  unconditional rung is exactly one, so a stub that answered `PENDING` for
  ever would fail here rather than hang a suite at a distance),
  `the_hosted_pages_pending_chain_is_bounded_and_ends_in_an_expiry`, and
  `the_payers_exit_from_the_hosted_page_decides_the_charge` × 2 — which
  follows the href **the page rendered** rather than one the test built, so
  pointing the links back at the merchant fails it.
  `a_test_number_typed_on_the_rails_hosted_page_reaches_the_documented_outcome`
  gained the assertion that was impossible before: that the charge is still
  `Pending` while the payer is typing.
- `backends/tests/integration` — `confirm_rails`
  `the_stub_hosted_page_links_to_the_return_url_the_submit_carried` now
  asserts the **`302` `Location`** of each control rather than its `href`,
  which is the stronger of the two; and
  `the_pending_chain_gives_a_payer_at_least_thirty_seconds` reads the chain
  out of the mapping _file_ and multiplies it against
  `vpay_worker::poll_delay`, so shortening either alone fails. `webhooks` and
  `provider_callback` re-run and green.
- `frontends/tests/e2e` — `shop-hosted.cy.ts` gained a fourth case, "the payer
  cancels on the rail's own page and the order reaches `failed`", which
  `waitForOrderStatus` fails the moment it ever reads `paid` — which is what
  it would have read before this change. The Orange pay case now asserts both
  controls point back through the rail's container.

**The gate, at the reviewed head** (`just ci`, end to end, containers, exit
code read from a file, 2026-09-10 on `a7e4c35` rebased onto `origin/master`
`2c5ef8b`, which carries #99 and #101): **exit 0**. `verify` **twelve gates
ok** (the `verify-docs` report is advisory and never fails); `test-rust`
**1669 tests run, 1669 passed, 0 skipped**, 25 m 20 s; `test-doc` **111
passed, 1 ignored** (the ignored one is `sdks/rust`'s README block and is
pre-existing); `verify-ignored` **0 ignored (expected 0), 45 test binaries
(expected 45), 1669 total**; `test-web` **1284 passed** across the nine web
suites (checkout 507, dashboard 172, `sdks/nodejs` 208, `sdks/stripe-js` 146,
shop 102, ui 74, config 63, tokens 8, api-client 4); `deny` `advisories ok,
bans ok, licenses ok, sources ok`; `fmt-check` and `clippy` clean.

_(The delivered commit's block said 1663 tests and is superseded rather than
wrong: it predates the rebase onto #101. **Five runs of this gate on this tree
died before that green**, three the implementer's and two the review's, every
one of them on `postgres:16-alpine container starts … container startup
timeout` — a different test each time, never an assertion, on a host carrying
a load average between 10 and 16 from unrelated work. Each was checked rather
than retried blind: `the_printed_password_cannot_reach_dash_v1` and
`every_credential_failure_on_the_checkout_surface_is_the_identical_404` both
pass in isolation in 9.2 s and 3.1 s. The green above was taken by waiting for
the host's load average to fall below five and then starting.)_

**Measured beyond `just ci`, on a throwaway compose project
(`exp43-orange-stub-race`, ports 13700-13702/18700-18702, torn down; the
maintainer's `vpay-demo` stack was not touched):**

- `just demo-up` + `just demo-walk` — **exit 0, six payments on two rails**,
  every one settled by the worker asking the rail and evidenced by a signed
  webhook. The one number in it that moved is the point: the settling Orange
  outcome settles after **7** of the demo's own polls where it used to settle
  on the first, and the two amount-keyed Orange outcomes (5001, 5002) still
  settle after **2**, unchanged — which is the priority argument above,
  measured rather than asserted. **The whole walkthrough takes 58 seconds**,
  and it is worth saying so where a reader will look for it: the settling
  Orange outcome is now the same 7 polls the settling MTN outcome has always
  been, because `SETTLE_POLL_INTERVAL` is two seconds and the rung it waits
  for is ten. _(This bullet said **41** polls until the sabotage review of
  2026-09-10 re-ran it. 41 polls is 82 seconds, which no rung of `poll_delay`
  produces; the Orange stub's own journal for the corrected run reads
  `webpayment` at T, `transactionstatus` → `PENDING` at T+0.591 s and
  → `SUCCESS` at T+10.680 s, with the 5001 and 5002 outcomes answered
  terminally at T+0.466 s and T+0.414 s. The likeliest cause of 41 is the
  authoring host: the same review measured a first poll arriving **60.96 s**
  late under five CPU hogs, because load starves the worker's claim loop.)_
- `just test-e2e` — **19 Cypress tests, 19 passing, 0 failing**, exit 0,
  re-run by the sabotage review on 2026-09-10: `checkout.cy.ts` (1),
  `dashboard.cy.ts` (8), `shop-hosted.cy.ts` (**4**, one of them new),
  `shop-embedded.cy.ts` (6). The Orange legs are the ones this change is
  about: "the payer pays on the rail's own page" 5.8 s and "the payer cancels
  on the rail's own page and the order reaches `failed`" 15.5 s.
- **`shop-hosted.cy.ts` five times over, against one standing stack: 5 runs,
  4 passing each, 20 of 20** (added by the review, because a new browser case
  that is _usually_ green is the thing this change was supposed to stop
  shipping). The cancel case took 5.42, 5.35, 5.47, 5.38 and 5.34 s — a 130 ms
  spread, which is what determinism looks like. The _pay_ case is bimodal —
  3.7 s twice, ~13.9 s three times — and that is the unconditional rung
  visible in the wall clock: when Cypress's click beats the worker's first
  poll the charge settles at once, and when it loses, the first poll answers
  `PENDING` and the charge settles on the next rung ten seconds later. Both
  end `paid`. Before this change the losing half of that coin ended `paid`
  too, for the wrong reason; the cancel case is where the difference shows.

- **The window itself, in a real browser, read out of the stub's own request
  journal** (added by the sabotage review, 2026-09-10). A person bought a
  coffee on `examples/shop`, chose Orange, landed on the rail's page and did
  nothing: submit at T, the browser's `GET` of the page at **T + 53 ms**,
  `PENDING` at T+0.170 s, T+10.192 s, T+30.233 s, T+60.289 s, and **`EXPIRED`
  at T + 105.398 s** — `poll_delay(0..3)` to the second. `vpay-worker` then
  logged `the rail reported a charge as failed … failure_code: "payer_timeout"`
  beside `a checkout session was settled … paid: false`. **A payer who never
  clicks reaches `payer_timeout`, not `paid`**, on the stack and not only in a
  suite with no worker in it.
- **And the documented test number worked from a browser, under load ×5.** The
  same journey with five CPU hogs running, typing `237600000400`: page at
  T + 60 ms, the form submitted at T + 12.123 s (about the 11.96 s a human took
  on 2026-09-06), the worker's **first** poll at T + 60.956 s → `FAILED`, and
  the charge settled `provider_error` — which is what
  `examples/shop/README.md` promises and what a run on 2026-09-06 came back
  `paid` for. That is also the answer to "is ten seconds enough under load":
  the margin is **monotone in load and in the payer's favour**, because load
  delays the _poller_ while the payer's browser is one `302` and one `GET` out
  of the same submit. The unconditional rung insures against a fast machine,
  not a slow one.

**Three mutations, run and reverted.**

- **The chain deleted (four rungs and the expiry):** conformance **6 of 7**
  window cases fail — the three `a_test_number_typed…` cases on `left:
Succeeded { provider_txn_id: Some("stub-txn") }, right: Pending`, the
  boundedness case, and both `the_payers_exit…`; and
  `the_pending_chain_gives_a_payer_at_least_thirty_seconds` fails with "no
  PENDING chain in …".
- **The click arming deleted (`payer-paid` / `payer-gave-up`):**
  `shop-hosted.cy.ts`'s Orange **cancel** case fails — `Expected to find
element: [data-outcome="failed"], but never found it` — because the charge
  settles `paid`. 1 failing, 3 passing. **It also fails in
  `backends/tests/conformance`**, which the sabotage review re-ran on
  2026-09-10 and the delivered note did not claim: with those two mappings
  removed (12 → 10), `the_payers_exit_from_the_hosted_page_decides_the_charge::case_2_cancel`
  fails in 1.5 s on `cancel: expected Some(PayerTimeout), got Succeeded {
provider_txn_id: Some("stub-txn") }` — and `case_1_pay` **still passes**,
  because a click nobody noticed still falls through to the catch-all
  `SUCCESS` and `paid` is what that case asserts. So the click arming has a
  cheap gate as well as an expensive one, and the pay/cancel asymmetry the
  notes found in the browser is a property of the assertion rather than of
  the browser.
- **The stub's `id="stub-limits"` section deleted** (the review's own
  mutation, on its own assertion, because an assertion never seen red is not
  a gate): `the_hosted_pages_pending_chain_is_bounded_and_ends_in_an_expiry`
  fails on "the stub's limits must be visible ON the stub's page".
- **The page's links pointed straight at the merchant again** (the shape
  before this change): both `the_payers_exit…` cases fail on "the page's
  control must go through the rail's own container".

**And one measured negative result, which corrects the task brief.** The brief
expected "shorten the chain to zero → the browser case FAILS (paid before the
click)". It does not. Run twice — chain deleted, and the whole window deleted
so the stub answered exactly as it did before — `shop-hosted.cy.ts` was
**green both times**, and the stub's own journal says why: submit at T, the
page at T+0.05 s, `transactionstatus` → SUCCESS at T+0.29 s, the payer's click
at T+0.53 s. That is the original bug reproduced _under a green spec_, because
the pay case asserts `paid` and a lost race still produces `paid`. A human
takes ~12 s to reach that form; Cypress clicks in ~60 ms, so a browser spec is
always inside the window whether or not there is one. The browser therefore
gates the **click arming** (the cancel case, where a lost race is red), the
conformance suite gates **that there is a bounded window**, and the
integration case gates **how many seconds it is worth**. Three properties,
three gates, none of them redundant.
