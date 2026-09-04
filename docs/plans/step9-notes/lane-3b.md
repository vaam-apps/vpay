<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md. This file is lane 3b's own record and
edits none of those. -->

# Step 9, lane 3b — the three page fixes from the correctness review

Branch `claude/step9-lane-3b-page`, on top of `e49e503` (the gate).

Three fixes, one commit each, every guard broken and observed failing before
it was restored. `frontends/apps/checkout` is the only application this lane
touched; no backend, no SDK, no other app, and neither `docs/status.md` nor
`docs/flows/*` (lane E owns those, and §5 below is written for it).

## 0. A gate blocker found on the way in: the lockfile does not install

`pnpm install --frozen-lockfile` at `e49e503` **fails**, before any of this
lane's work:

```
 ERR_PNPM_LOCKFILE_MISSING_DEPENDENCY  Broken lockfile: no entry for
 'vitest@3.2.7(@types/node@22.20.1)(jiti@1.21.7)(jsdom@25.0.1)' in pnpm-lock.yaml
```

The `frontends/apps/checkout` importer (lane 3) pins its `vitest` peer set
against `jiti@1.21.7`, and no snapshot with that key exists in the file —
the only `vitest@3.2.7` snapshot is the `jiti@2.7.0` one. It is the shape a
merge of two branches that each regenerated the lockfile leaves behind:
lane 3 added the importer, lane 7 (`examples/shop`, Next 16) moved the
`jiti` resolution, and the merge kept lane 3's importer line beside lane 7's
snapshots.

Fixed here as a one-line change to this lane's own importer entry, because
nothing in this branch — or in CI's `web` job — can install without it:

```diff
       vitest:
         specifier: ^3.2.7
-        version: 3.2.7(@types/node@22.20.1)(jiti@1.21.7)(jsdom@25.0.1)
+        version: 3.2.7(@types/node@22.20.1)(jiti@2.7.0)(jsdom@25.0.1)
```

`pnpm install --frozen-lockfile` passes after it. **Lane E: if another lane
fixes the same line, take either — they are the same edit.** This is the
only file outside `frontends/apps/checkout` and `docs/plans/step9-notes/`
that this lane changed.

## 1. `merchant` is a nicety, not a precondition

**The bug.** `isSessionEnvelope` (`src/lib/api.ts`) required
`merchant: { name: string }` on both browser session reads. A server that
omitted the member — a merchant with no display name, a deployment where
lane 1b's member has not landed, a `merchant_name` rename — turned a
perfectly payable session into `error.unexpected`: a dead end for the payer
and a support ticket for the merchant, over a *label*.

**What landed.**

| Where | Change |
|---|---|
| `src/lib/types.ts` | `merchant?: CheckoutMerchant \| undefined` on `CheckoutSessionView` and `CheckoutReturnView`. Where it is present it is still `{ name: string }` — the type for the present case is unchanged |
| `src/lib/api.ts` | the envelope check names only what the page cannot proceed without: `object`, `id`, and an **expanded** `payment_intent`. Not `merchant` |
| `src/lib/machine.ts` | new `merchantOf(value: unknown): CheckoutMerchant \| null`, and `CheckoutContext.merchant` is `CheckoutMerchant \| null`. `unknown` on purpose: the value has been through `JSON.parse` and nothing else |
| `src/lib/return.ts` | the same on `ReturnContext`, read through the same `merchantOf` |
| `src/i18n/{en,fr}.ts` | four `_unnamed` twins — `page.pay_to_unnamed`, `expired.body_unnamed`, `outcome.succeeded_body_unnamed`, `outcome.auto_forward_unnamed` — each written as its own sentence |
| `src/components/screens.tsx` | `merchantLine(t, merchant, named, unnamed, values)`, the one place a name becomes a sentence; `PaymentSummary` and `OutcomePanel` take `merchant: string \| null` |
| `src/testing/browser-stub.ts` | `merchant?: StubMerchant` — `{kind:'named'}`, `{kind:'absent'}` (no key in the body at all) or `{kind:'malformed', value}` |

**Why a second dictionary key rather than a stand-in for `{merchant}`.** A
substituted placeholder — `—`, "the merchant", the session id — is rendered
*inside a sentence written for a real name*, and reads like data the page
has. It does not have it. `Pay —` and `Pay cs_test_…` are both worse than
`Payment`.

**Tests, one per shape.** `controller.test.ts` drives the stub in all three:
`{name}` (context carries the name), absent (context carries `null`, and the
payment goes on to succeed), malformed (the same). `machine.test.ts` pins
`merchantOf` against eleven values. `checkout-view.test.tsx` renders the
unnamed screens in both locales, on the checkout page and the return page.

### Guard-failure proof

**D — the envelope check requires `merchant` again.** `isSessionEnvelope`'s
last clause restored to `isObject(body['merchant']) && typeof
(body['merchant'] as Record<string, unknown>)['name'] === 'string'`:

```
× the merchant name is a nicety, not a precondition > pays a session whose read carried no `merchant` member at all
  → expected { name: 'error', …(1) } to match object { name: 'collect_msisdn', …(1) }
    -   "context": { "merchant": null },  -   "name": "collect_msisdn",
    +   "name": "error",
× the merchant name is a nicety, not a precondition > pays a session whose `merchant` is not the documented shape either
Tests  2 failed | 281 passed (283)
```

**E — an identifier stands in for the missing name.** In
`checkout-view.tsx`, `context?.merchant?.name ?? null` replaced by
`context?.merchant?.name ?? context?.session.id ?? null`:

```
× shows the neutral heading rather than a hole in a sentence (fr)
  → expected 'Payer cs_test_fixture000000000001' to be 'Paiement à régler'
× shows the neutral heading rather than a hole in a sentence (en)
  → expected 'Pay cs_test_fixture000000000001' to be 'Payment'
× puts no identifier, and no stand-in that reads like data, where the name would be
  → pay-to: expected 'Pay cs_test_fixture000000000001' not to contain 'cs_test'
× says the neutral sentence on every screen the name appears in
  → expected 'Go back to cs_test_fixture00000000000…' to be 'Go back to the shop you came from and…'
Tests  8 failed | 275 passed (283)
```

Four of those eight are the four above; the other four are this file's
existing tests tripping over a component left mounted by an assertion that
threw before its `unmount()` — an artefact of the failure, not a second
finding.

Both files restored; **283 passed (283)**, 0 skipped, after each.

## 2. The Orange confirm's `return_url` — the stub stops lying

**The finding.** `startRedirect` (`src/lib/controller.ts`) confirms an Orange
intent with no `return_url`, and the stub accepted it. If the API requires
one on a redirect confirm, every test of the Orange path was green against a
request the server would answer `400`.

**The ruling** (the integrator, with the server side changing in parallel):
a confirm on an intent that belongs to an **open checkout session** does not
require `return_url` — the server substitutes the session's own return page,
which is the only URL that carries the `t=` token the return trip needs. A
`return_url` the page invented would send the payer to a return page it
cannot authenticate.

**So the page is unchanged**: `startRedirect` still sends
`payment_method_data[type]` and nothing else. What changed is the stub, which
now applies the rule in both directions
(`src/testing/browser-stub.ts`):

- a redirect-rail confirm with no `return_url` on an intent whose session is
  open → accepted, as today;
- the same call for an intent with **no open session** → `400` with the
  server's real envelope,
  `{"error":{"type":"invalid_request_error","code":"invalid_request","param":"return_url","message":…}}`
  — `ApiError::invalid_param("return_url", …)`, whose `code` is
  `Category::InvalidRequest`'s `invalid_request` and whose `param` is the
  field name;
- a push rail is untouched by the rule: it redirects nowhere.

New stub option `standaloneIntent: true` models the intent a merchant created
directly through `/v1/payment_intents`: the two checkout-session routes then
answer `404`, because there is no session. The redirect-rail list is the
stub's own (`REDIRECT_RAILS`), deliberately not `RAIL_PAGE_FLOWS` from
`src/lib/rails.ts` — reading the page's map here would make stub and page
agree by construction instead of by contract.

**Lane 1b / the server, please read.** This stub now asserts the exemption
exists. If the server ends up requiring `return_url` on *every* redirect
confirm, this page breaks in production and the four tests below stay green,
because they would then be pinning the wrong ruling. The decisive check is
`§2`'s guard H: with the exemption removed, the page's whole Orange path
fails.

### Guard-failure proof

**F — the stub accepts everything again** (the refusal branch replaced by
`if (false)`):

```
× the confirm’s return_url … > is refused 400 invalid_param by the stub when the intent has no open session
  → expected { …(13) } to be undefined
Tests  1 failed | 286 passed (287)
```

**G — the page invents a `return_url`** (`confirmParams` in `startRedirect`
given `return_url: \`https://checkout.example/c/${sessionId}/return\``):

```
× the confirm’s return_url … > sends no return_url — the session’s return page is the server’s to choose
  → expected 'key=pk_test_0123456789abcdefghij&clie…' not to contain 'return_url'
Tests  1 failed | 286 passed (287)
```

**H — the open-session exemption dropped** (`!(hasSession && sessionStatus
=== 'open')` replaced by `true`, i.e. every redirect confirm needs a
`return_url`). This is the one that shows what the ruling buys:

```
× the Orange redirect > confirms, records the redirect, and navigates top-level when not framed
  → expected { name: 'ready_redirect', …(4) } to match object { name: 'redirecting', …(1) }
× the Orange redirect > asks the parent to navigate when framed, and never navigates itself
× the confirm’s return_url … > sends no return_url — …
× the return controller against the stub > polls the return route until the rail settles, then shows the outcome
× the return controller against the stub > never receives the intent’s client_secret on the return route
× the return controller against the stub > forwards with {CHECKOUT_SESSION_ID} substituted
Tests  6 failed | 281 passed (287)
```

All three reverted; **287 passed (287)**, 0 skipped, after each.

## 3. The secret spy stopped after one path; now it covers three

**The finding.** `src/lib/secrets.test.ts` traced `submitMsisdn` and nothing
else. `startRedirect` — which posts a URL to the parent and navigates the top
level — and the whole `ReturnController`, where the `t=` token lives, were
untraced. And the console/navigate/`postMessage` spies could not see a
credential *retained on rendered state* at all: the review's mutation F
(`contextOf` keeping the session's `client_secret`) was caught only by
`machine.test.ts`.

**What landed** (one file, `src/lib/secrets.test.ts`, 8 tests → 20):

- `traceARedirect(framed)` — the Orange path, both framed and not. Asserts
  the `vpay:redirect` payload and the top-level `assign` URL carry neither
  secret nor `_secret_` nor the return token, that the framed page does not
  *also* navigate itself, and that the confirm's credential stays in the
  body.
- `traceAReturn(framed)` — the return page as its own document. The rail is
  driven to its answer **before** the spies are installed, so the trace holds
  the return page's own traffic; one read then settles it (the return route
  is also the status query), which is what makes "exactly one" assertable.
  Asserts the `t=` token appears in exactly one `fetch` URL, as that read's
  `t` parameter and never in its path, and nowhere else: not in a `console`
  call, not in the `vpay:complete` payload, not in the forward URL, not on
  the state.
- `secretPaths(value)` — walks a state and returns the **paths** whose string
  holds `_secret_`. Paths, not a boolean, because the intent's own
  `client_secret` is supposed to be there (the controller confirms and polls
  with it) and the session's is not. The assertion is
  `['$.context.intent.client_secret']` on both payment paths and `[]` on the
  return page.

### Guard-failure proof

**F (the review's) — `contextOf` retains the session's `client_secret`**
(`return { session: { ...session, client_secret: _sessionSecret }, … }`).
This file now fails too, which is the whole point of the change:

```
× contextOf > drops the session’s client_secret, so no rendered state carries one            (machine.test.ts, as before)
× the state the screens render from … > holds a secret at exactly one path: the intent’s own client_secret
  → expected [ …(2) ] to deeply equal [ '$.context.intent.client_secret' ]
× the state the screens render from … > does the same on the Orange path, where the state also holds a rail URL
Tests  3 failed | 296 passed (299)
```

**I — the redirect URL carries the intent's secret.** In `startRedirect`,
the rail URL replaced by `` `${url}#cs=${clientSecret}` `` before it is
dispatched and navigated:

```
× the state the screens render from … > does the same on the Orange path, …
× the Orange redirect leaks nothing > sends the payer to the rail’s URL and nothing else — no credential appended
× the Orange redirect leaks nothing > asks the parent to navigate with a payload carrying no secret
× the Orange redirect > confirms, records the redirect, and navigates top-level when not framed   (controller.test.ts)
× the Orange redirect > asks the parent to navigate when framed, and never navigates itself       (controller.test.ts)
Tests  5 failed | 294 passed (299)
```

Three of those five are in `secrets.test.ts`, which before this change
traced no redirect at all.

**J — the forward URL keeps the return token** (`ReturnController.forward`
navigating to `` `${url}&t=${returnToken}` ``):

```
× the return page’s token > is not in the URL the payer is forwarded to
× the return controller against the stub > forwards with {CHECKOUT_SESSION_ID} substituted   (return.test.ts)
Tests  2 failed | 297 passed (299)
```

**K — the return read is logged** (`console.debug('vpay: reading', url)` in
`BrowserCheckoutApi.readReturn`):

```
× the return page’s token > is never written to a console method
  → expected 'vpay: reading\nhttp://127.0.0.1:39157…' not to contain 'rrrrrrrrrrrrrrrr…'
Tests  1 failed | 298 passed (299)
```

Every mutation reverted; **299 passed (299)**, 0 skipped, after each.

## 4. Counts, measured after all three fixes

| Gate | Result |
|---|---|
| `pnpm --filter @vpay/checkout test` | **299 passed (299)** in 16 files, **0 skipped, 0 ignored** — was 271 |
| `just lint-web` (`build-sdk-node` then `pnpm -r typecheck`) | exit 0, 16 projects |
| `pnpm -r typecheck` (bare) | exit 0 **once `@vpay/sdk` has been built**. On a clean tree it fails first with `cypress/tasks/checkoutTasks.ts(18,28): error TS2307: Cannot find module '@vpay/sdk'` — that package's `exports` resolve to a gitignored `dist/`, which is exactly why `just lint-web` depends on `build-sdk-node`. Pre-existing, untouched by this lane; **`just lint-web` is the command that matches CI** |
| `pnpm --filter @vpay/checkout build` | exit 0 — `.next/standalone/frontends/apps/checkout/server.js` present |
| `pnpm --filter @vpay/ui build-storybook` | exit 0, still 23 checkout entries in `storybook-static/index.json` (not a required gate here; run because §1 changed two component signatures) |
| `pnpm install --frozen-lockfile` | exit 0 — after §0. It failed before it |

Per-file counts: `secrets.test.ts` 8 → 20, `controller.test.ts` 17 → 24,
`checkout-view.test.tsx` 90 → 96, `machine.test.ts` 33 → 36. No test was
deleted, weakened or skipped; there is still no `it.skip`, no `describe.skip`
and no `--passWithNoTests` in this app.

## 5. For lane E — the numbers in lane 3's proposed status row are now stale

`docs/plans/step9-notes/lane-3.md` §7 offers a `docs/status.md` row for
`frontends/apps/checkout` containing:

> **271 vitest tests in 16 files, 0 skipped**, including a `node:http` stub
> of all five browser routes, both guard directions of the origin check, and
> a test that no `console.*` call, no navigation and no `postMessage` ever
> carries a secret.

Replace that sentence with:

> **299 vitest tests in 16 files, 0 skipped**, including a `node:http` stub
> of all five browser routes, both guard directions of the origin check, and
> a credential trace of all three payer paths — the MTN push, the Orange
> redirect and the return page — asserting that no `console.*` call, no
> navigation, no `postMessage` and no rendered state ever carries a secret,
> and that the return trip's `t=` token appears in exactly one request.

The rest of the row still holds, including the 🟡 and its reason: **no
browser has rendered this page.** This lane did not change that.

## 6. What this lane did **not** do

- **No browser ran anything.** Every proof above is vitest and jsdom against
  a `node:http` stub, exactly as in lane 3. Nothing here is evidence that the
  CSP is enforced, that the Orange redirect works against a real rail, or
  that the return page renders.
- **The server side of §2 is not verified.** This branch changed no Rust. The
  stub now *asserts* the open-session exemption; whether `/v1` implements it
  is lane 1b's, and if it does not, these tests pin the wrong ruling — see
  the note in §2.
- **`merchant` is still not sent by any real server.** §1 makes the page
  tolerate its absence; it does not make lane 1b's member exist.
- **No new Storybook story.** The unnamed-merchant screens are covered by
  vitest, not by a story, so lane 3's "every screen state has a story"
  invariant now has a rendering variant outside it. `CHECKOUT_SCREENS` was
  deliberately left alone rather than grown a parallel `_unnamed` entry for
  every screen.
- **`page.pay_to_unnamed` and its three siblings are this lane's wording.**
  No translator or designer has seen them.
- **Nothing outside `frontends/apps/checkout`** except `pnpm-lock.yaml`
  (§0) and this file. No backend, no SDK, no other app, no `docs/status.md`,
  no `docs/flows/*`, no justfile, no CI workflow.
