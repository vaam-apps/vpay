# exp43 — the Orange stub's test numbers work from a browser (issue #58)

Working notes for `claude/exp43-orange-stub-race`, base `970bfe0`. What is a
_claim about what runs_ is in [`../../status.md`](../../status.md) and in the
flow docs; this file is the reasoning, the measurements and the things that
were considered and not done.

## The problem, restated from the measurement

`vpay_api::v1::payment_intents`' confirm handler enqueues the `poll_charge`
job with `run_at = now()` — `vpay_worker::poll_delay(0)` is the delay before
the **second** attempt, not the first — and `vpay_worker::run_loop`'s idle
sleep is one second. So the first `transactionstatus` lands about half a
second after an Orange submit commits, `transactionstatus.json`'s priority-10
catch-all answers `SUCCESS`, and the charge settles before a payer has read
anything.

exp22 measured it on the demo stack on 2026-09-06, from `wiremock-orange`'s
own request journal:

| Event                                               | Offset         |
| --------------------------------------------------- | -------------- |
| `POST …/v1/webpayment` (the submit)                 | T              |
| `GET /stub-hosted-page/{token}` (the payer lands)   | T + 44 ms      |
| `POST …/v1/transactionstatus` (the first poll)      | **T + 449 ms** |
| `GET …/pay?msisdn=237600000400` (the payer chooses) | T + 11.96 s    |

The order came back **paid** for a number the shop's own panel promises will
fail. That is a false green on a payments demo, which is the specific thing
[`../../../CLAUDE.md`](../../../CLAUDE.md) says not to ship.

## What was built: click-armed, with a bounded chain, and one unconditional rung

The brief offered two shapes and said which was better if WireMock allowed it.
Both were built, because **neither is sufficient alone**, and the reason is
the 400 ms in the table above.

### Why the click-armed design alone does not work

Arming the `PENDING` chain on the hosted page's `GET` is a **race**. The two
events that matter — the browser's `GET` and the worker's first poll — are
44 ms and 449 ms out of the same submit, and the 405 ms between them is not a
property of anything: it is wherever in its one-second idle sleep the worker
happened to be. The browser usually wins. "Usually" is a flaky gate, and the
brief asked for determinism.

### Why the bounded chain alone is expensive

A chain that every Orange charge walks — the brief's option A read literally —
is deterministic, and costs every Orange charge in the repository the whole
chain. Two rungs of `poll_delay` is the minimum that meets "≥ 30 s for the
payer" (10 + 20), so every `worker_e2e`-shaped case, every demo outcome and
every Cypress leg would wait 30 s where it used to wait half a second.

### The shape that is both

One WireMock scenario, `orange-hosted-page-payer`, with three layers:

```
submit           -> `submitted`               (webpayment.json arms it)
1st status       -> PENDING                   -> `submitted-polled`
2nd status       -> SUCCESS (the old answer)  -> `Started`

  ... unless a payer's browser loads the page, which at any point arms:

GET the page     -> `payer-on-page-1`
status x4        -> PENDING                   -> `payer-never-acted`
status           -> EXPIRED                   -> `Started`

  ... and each of the page's exits arms a terminal answer instead:

#pay / the form  -> `payer-paid`      -> SUCCESS, then `Started`
#cancel          -> `payer-gave-up`   -> EXPIRED, then `Started`
the form with a documented test number -> demo-outcomes.json's two arms
```

- **The unconditional rung** turns the 405 ms margin into ~10 s (the first
  rung of the ladder). It is armed by the _submit_, not by the page, so it
  depends on nothing a browser does. It costs exactly one rung, and only to
  charges that were being answered by the catch-all.
- **The bounded chain** is what a payer actually stands in. Four `PENDING`
  answers is `poll_delay(0..3)` = 10 + 20 + 30 + 45 = **105 s**, against the
  30 s the issue asks for and the 120 s outcome timeout the Cypress specs
  give a charge.
- **The click arming** is what makes a payer's action matter. It needed one
  change beyond the scenario: the page's `#pay` and `#cancel` links used to
  point _straight_ at the merchant's URL, so a payer who used one never
  touched the container again — which is precisely why exp22 concluded the
  obvious shape "has no way to disarm". They now point at
  `/stub-hosted-page/{token}/pay` and `…/cancel` on the same container, which
  arm and then `302` to the same two URLs.

### What it cost, measured rather than assumed

Priorities are the whole of the blast-radius argument (lower wins):

| priority | mappings                                                  | effect of the change                                                                                        |
| -------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| 1        | `transactionstatus.json`'s `order_id`-keyed cases         | **none** — they still win, so every conformance reference case is untouched                                 |
| 3        | everything answering from a payer's presence or action    | new                                                                                                         |
| 4        | `demo-outcomes.json`'s amount-keyed mappings (5001, 5002) | **none** — still terminal on the first rung, so `just demo-walk`'s two failing Orange outcomes did not move |
| 6        | the two unconditional rungs                               | new                                                                                                         |
| 10       | the catch-all `SUCCESS`                                   | deferred by one rung                                                                                        |

So the entire cost is: _an Orange charge answered by the catch-all settles one
rung (≈10 s) later than it did._ In the Rust suites that is
`a_charge_no_payer_has_looked_at_is_pending_once_and_then_settles` and nothing
else — no other case in `backends/tests` submits an Orange charge and then
relies on the catch-all's first answer (checked by reading every
`REDIRECT_RAIL` use in `backends/tests/integration`: `provider_callback`
re-points its charges at `REF_SCENARIO`, `confirm_rails` never polls,
`worker_recovery`'s one Orange case never asks the rail, and `worker_e2e`,
`browser_checkout` and `payment_intents` do not drive an Orange settlement at
all). In the demo it is outcome 4, which now settles about ten seconds in.

## The mutations, and the one that did not behave as the brief expected

Each was applied, run, and reverted. The mapping mutations were applied to the
`.e2e/<project>/wiremock-orange` **copy** for the browser runs (reloaded with
`POST /__admin/mappings/reset`, no container restart) and to the committed
tree for the Rust runs, and the committed file was restored byte for byte
afterwards (`git diff` empty).

| mutation                                                                                                  | what fails                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| --------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| the four `payer-on-page-*` rungs and the expiry deleted (**the chain to zero**)                           | conformance: **6 of the 7** window cases — all three `a_test_number_typed…` cases on the new `Pending` assertion (`left: Succeeded { provider_txn_id: Some("stub-txn") }, right: Pending`), `the_hosted_pages_pending_chain_is_bounded_and_ends_in_an_expiry`, and both `the_payers_exit…`. Integration: `the_pending_chain_gives_a_payer_at_least_thirty_seconds`, with "no PENDING chain in …". **The Cypress specs stay green** — see below |
| the `payer-paid` / `payer-gave-up` answers deleted (**the click arming**)                                 | Cypress `shop-hosted.cy.ts`: the Orange **cancel** case fails — `Timed out retrying after 120000ms: Expected to find element: [data-outcome="failed"], but never found it`, because the charge settled `paid` instead. 1 failing, 3 passing                                                                                                                                                                                                    |
| the page's `#pay`/`#cancel` `href`s pointed straight at the merchant again (the shape before this change) | conformance: both `the_payers_exit…` cases, on "the page's control must go through the rail's own container — a link straight to the merchant is a payer the rail never hears from again"                                                                                                                                                                                                                                                      |

### The brief expected the first one to fail the browser case. It does not, and this is why.

The brief's decisive mutation was "shorten the chain to zero → the browser
case FAILS (paid before the click)". It was run, twice — once with the chain
deleted and the unconditional rung kept, once with the **whole** window
deleted so the stub answered exactly as it did before 2026-09-10 — and
`shop-hosted.cy.ts` was **green both times**.

The reason is in the stub's own request journal, and it is a fact about
Cypress rather than about the fix. With the chain deleted:

| event                                       | offset     |
| ------------------------------------------- | ---------- |
| `POST …/v1/webpayment`                      | T          |
| `GET /stub-hosted-page/{token}`             | T + 0.05 s |
| `POST …/v1/transactionstatus` → **SUCCESS** | T + 0.29 s |
| `GET …/pay` (the payer's click)             | T + 0.53 s |

That is **the original bug, reproduced** — the charge was settled 240 ms
before the payer acted — and the "Orange redirect: the payer pays" case
passed anyway, because the outcome it asserts is `paid` and `paid` is what a
race lost still produces. In the _cancel_ case of the same run the click
landed at T + 0.11 s, 170 ms _ahead_ of the first poll, so it won the race and
that case passed too.

A human payer takes about twelve seconds to reach that form (exp22 measured
it). Cypress clicks a link in about sixty milliseconds. So the browser specs
cannot be a gate on **how long** the window is — they are always inside it,
window or no window. What they gate is the **click arming**, and there the
cancel case is sharp: it is the one assertion in the browser suite that a
lost race turns red rather than green.

So the properties and their gates are, deliberately, not the same tests:

- _is there a window at all, and is it bounded_ → `backends/tests/conformance`,
  where there is no worker and the five polls are five function calls;
- _is the window long enough for a human_ → `the_pending_chain_gives_a_payer_
at_least_thirty_seconds`, arithmetic over the mapping file and
  `vpay_worker::poll_delay`;
- _does the payer's click decide the charge_ → the Orange cancel case in
  `shop-hosted.cy.ts`, in a real browser.

None of the three is redundant, and the brief's expectation that one test
would catch all three was the thing this pass measured and had to correct.

## A wrong comment found and corrected

`stub-hosted-page.json` said the double-stache `{{…}}` HTML-escapes, and that
the form's hidden inputs relied on that. **It does not.** Driven against
`wiremock/wiremock:3.9.2` on 2026-09-10 with a return URL of
`https://m.test/back?q="x"&y=<z>`, the rendered page carries

```html
<input type="hidden" name="return" value="https://m.test/back?q="x"&y=<z>">
```

— unescaped, quotes and all, which closes the attribute early. The two hrefs
this change added are safe because they do not rely on the stache at all: they
wrap the URL in WireMock's `urlEncode` helper first. The form is still
vulnerable to a return URL containing a double quote, that is written down in
the mapping's own metadata, and it is **not fixed here**: a GET form's hidden
input has to carry the decoded value, and WireMock cannot HTML-escape an
attribute. No test in this repository sends such a URL.

## What was deliberately not done

- **Not touched: the confirm handler's `run_at = now()`.** Option 2 of
  exp22's three. It is one line and it is the wrong line: an immediate first
  poll is a deliberate property (`docs/flows/crash-safety.md` — a charge is
  asked about as soon as it exists) and slowing it to make a demo comfortable
  is the class of change ADR-0003 exists to refuse.
- **Not invented: a `CANCELLED` status.** Orange documents five and that is
  not one of them, so the Cancel link arms `EXPIRED` → `payer_timeout`. A
  stub that answered `CANCELLED` would be this repository adding a word to a
  rail's vocabulary, which `examples/shop/README.md` refuses to do elsewhere
  on the same rail.
- **Not made concurrent-safe.** One scenario per container means two Orange
  charges in flight against one stub still share it, and the wrong one can be
  answered. Every mapping either returns to `Started` or is re-armed by the
  next submit, so nothing _inherits_ a state, but overlap is not fixable in
  WireMock and the demo and both Cypress specs drive one payment at a time.
  The mapping files said this before this change and still say it.
- **No Cypress case waits for the expiry.** The 105 s chain running out is
  proven in `backends/tests/conformance`, where there is no worker and the
  five polls are five function calls. A browser case for it would be a
  two-minute spec whose only assertion is a timeout.
