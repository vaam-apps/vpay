# exp21 — the sabotage review

A review of `claude/exp21-checkout-page`, rebased onto `origin/master` after
PRs #55 #60 #62 #64 landed. The implementer's own account is
[`opus.md`](opus.md); this file records what was _checked_, what was found, and
what was left alone.

The short version: the branch's design held up under attack and its unit suite
is unusually good, and **the branch was broken in a real browser**. The
implementer said plainly that no Cypress spec had been run and that nothing
proved they still passed. They did not. Three of `shop-hosted.cy.ts`'s three
tests failed on the vpay checkout page, and the cause was in this branch.

---

## 0. The rebase

`origin/master` was 45 commits ahead. Two conflicts, both expected:

- **`docs/flows/hosted-checkout.md`** — both sides' Status entries are true and
  both are kept. Master's popup paragraph said "a popup has no framer, so the
  child channel returns `null` and the page says nothing", which this branch
  makes false; that half is marked superseded in place rather than deleted, and
  points at "The popup, and why it is a third peer".
- **`pnpm-lock.yaml`** — master's taken whole (ZenStack 3 and the shop demo's
  tree came with those PRs and this branch has no opinion about them), then one
  `pnpm install` added this branch's four packages.
  `pnpm install --frozen-lockfile` passes on the result.

`just fmt-check` and `just verify` green on the rebased head before anything
else ran.

## 1. Findings

Severity: **gate-hole** (something a gate should have caught and did not),
**correctness**, **rule-break**, **misleading-claim**, **nit**.

### F1 — gate-hole/correctness — the page threw a hydration error in a browser

`app/layout.tsx` emitted the `branding.yaml` colour inside an explicitly
written `<head>` element. React threw **#418**, _hydration failed because the
server rendered HTML did not match the client_, uncaught, on the hosted payment
page. `shop-hosted.cy.ts`: **3 tests, 0 passing, 3 failing**, all three on the
same error, page stuck on "Loading this payment…".

Measured three ways against the shipping standalone build, each a one-second
Cypress run against a local server:

| build               | `branding.yaml`                   | result   |
| ------------------- | --------------------------------- | -------- |
| explicit `<head>`   | mounted, `primary_color` set      | **#418** |
| explicit `<head>`   | absent, so that `<head>` is empty | passes   |
| `href`/`precedence` | mounted, `primary_color` set      | passes   |

An explicitly rendered `<head>` is an ordinary host element whose children
React reconciles exactly, and a Next App Router document's head is full of
nodes React did not put there. React 19's hoisting writes the style in as a
_resource_ instead. Fixed in `664454d`; `src/layout.test.tsx` asserts the
**element tree** (React's own hoisting produces a `<head>` in the _markup_ —
that is what hoisting is), and putting the `<head>` back fails that one case
and nothing else.

Why no gate caught it: `just ci` does not run Cypress, and the defect needs a
`primary_color` to exist at all — so the unit suite could not have seen it, and
the one suite that could was never run.

### F2 — gate-hole — the origins lookup that holds the payment page's first byte had no timeout

`fetchCheckoutOrigins` is awaited by `middleware.ts` before the response
starts. It ran for `/e/{id}` alone until this branch; it now runs for `/c/{id}`
and `/c/{id}/return` too. A refusal or an unreachable host _rejects_, and the
existing `catch` answered `[]` for both — but a peer that accepts the
connection and then says nothing is `await` forever, and that is now the
primary payment surface. The implementer named this gap honestly in
`docs/status.md` rather than hiding it; it is fixed rather than named.
`ORIGINS_TIMEOUT_MS` is two seconds, overridable per call, and the abort lands
in the same fail-closed `[]`. Fixed in `7f159e1`.

### F3 — correctness — a failed payment was grey and a cancelled one was red

`OutcomePanel` took its colour from `@vpay/tokens`' `statusTone` through the
intent status each outcome implies, mapping `failed` onto
`requires_payment_method` — `neutral`, because on a **dashboard** that status
means "awaiting a payment method". Accurate mapping, wrong screen: a payer
whose payment failed read a grey box while a payer who cancelled read a red
one. It is on the committed `outcomes-hosted.png`.

The implementer surfaced this as a maintainer decision. The delegate returned
it as a fix. It is fixed as narrowly as the evidence allows: `statusTone` is
**unchanged**, so the dashboard is untouched, and `checkoutOutcomeTone` is a
new payer-facing table with `failed: 'error'`. `canceled` stays red rather than
becoming a softer amber — that is still a design call and is **not taken**.
Fixed in `5466b06`.

### F4 — misleading-claim ×5 — sentences that said what the code does not do

Fixed in `aed4a66` and `ef90ff1`:

- `compose.demo.yml` said `compose.e2e.yml` mounts neither YAML file, "so the
  Cypress specs exercise the no-files path". They do not: `just test-e2e` and
  CI's `e2e` job both add `-f compose.demo.yml`. That is the _right_
  arrangement — it is the path that found F1 — and the comment now says so.
- `docs/flows/hosted-checkout.md` said "the container was run by hand with
  them" and cited `opus.md`, which says "no container was run with the mounted
  files, and no pod ever". A doc contradicting its own citation.
- The same file's a11y bullet still said "every control is a native focusable
  element", which stopped being true when the opt-in became Base UI's
  `<span role="checkbox">`. The implementer corrected that sentence in
  `screens.tsx` and left the copy in the flow doc standing.
- `shop-hosted.cy.ts` still described "waiting out the five-second countdown".
- `origins.ts` carried `resolveParentOrigin`'s doc comment stacked directly
  above `soleOrigin`, so the wrong function was documented.

### F5 — correctness — the shipped examples broke the demo

`config/checkout/*.example.yaml` are mounted by `compose.demo.yml` into the
demo _and_ into `just test-e2e`'s stack. `logo_url` pointed at
`cdn.vaam.example`, which does not resolve and never will (RFC 2606), so the
demo payment page carried a **broken image** on every screen — visible on the
regenerated `branded.png`, where it also pushed the operator's name onto two
lines. `public_base_url` pointed at `https://checkout.vaam.example` against a
page served on `localhost:3080`, firing the page's own misconfiguration warning
in the browser console on every load of a deployment that is configured
correctly. Both commented out with their documentation intact. Fixed in
`ef90ff1`.

### F6 — rule-break — `docs/flows/configuration.md` said nothing about the two new files

The repository's own configuration page described one document, loaded by
`vpay_config::Config::load`, validated hard enough to refuse a boot. There are
now three, in two processes, with opposite failure rules. A section was added
naming the paths, the override variables, the "a bad key costs that key" rule,
the fact that nothing reconciles the two `public_base_url` values, and the Helm
gap. Fixed in `aed4a66`.

## 2. What was attacked and held

Every item below was probed and needed no change.

**The popup peer.** A referrer from a non-registered origin opens no channel; a
stripped referrer opens no channel and the page falls back to a top-level
navigation (`decideEntry` → `openerOrigin: null` → `channel: null` →
`#navigateTopLevel` → `navigate`); `soleOrigin` with two registered origins
answers `null`; `vpay:complete` is guarded by `#announced` on both controllers.
`'*'` appears in no `postMessage` in this app — there is exactly one call site
(`frame.ts:129`) and it names `parentOrigin`. The middleware's CSP takes
`embedded ? origins : []`, so a popup registration cannot make a hosted page
framable. 25 cases in `entry.test.ts`, 17 in `origins.test.ts`, 13 in
`frame.test.ts`, 18 in `middleware.test.ts`.

**A hostile `branding.yaml`.** Measured, not reasoned about, with
`display_name: "</h1><script>alert(1)</script>"`,
`logo_url: "javascript:alert(1)"`, a `primary_color` that is a CSS-injection
attempt, and a `support_contact` carrying an `onerror=` payload. Result: the
two URL/colour values are **refused** with one problem each; the two free-text
values are rendered **escaped** (`&lt;script&gt;`, `&quot;`) as text nodes;
`themeStyleSheet` answers `null` rather than emitting an unvalidated colour. A
malformed document is defaults plus one problem naming the file. No path from a
mounted file to markup. `javascript:` is now in the case that proves it.

**Page memory.** The stored MSISDN is re-validated on read through
`normalizeCameroonMsisdn` (not a second regex); the record never leaves the
device — one origin, IndexedDB, and `secrets.test.ts`' credential trace covers
the page; the opt-in defaults to unticked on a fresh device and to ticked only
where a record already exists; the clear control works and unticking on a
submit clears too; `page_memory: false` produces `NO_PAGE_MEMORY` **and**
`offered: false`, so it disables read and write and removes the control.

**Accessibility.** Every control on every screen, native or not, has an
accessible name and a tab index; Base UI's `<span role="checkbox">` is asserted
for role, tab index, name and `aria-checked`, and both the label click and the
space key are driven rather than assumed; the reduced-motion block is the only
hand-written CSS in the app.

## 3. Mutations

Each applied to the shipping source, suite run, source restored.

| Mutation                                                          | Result                                                                                                 |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `layout.tsx`: put the explicit `<head>` back                      | **1 failed** (`renders NO explicit <head> element`) — and it is the case that names the browser defect |
| `api.ts`: drop `signal: controller.signal` from the origins fetch | **1 failed** — the never-answering `fetch` hangs to vitest's own 5 s timeout                           |
| `@vpay/tokens`: `checkoutOutcomeTone.failed` back to `neutral`    | **2 token cases + 14 view cases failed**                                                               |

The implementer's own fourteen were not re-run; they are recorded in
[`opus.md`](opus.md) §5 and the three above are the review's.

## 4. The gates

**`just ci`**, recipe by recipe, exit code read from a file rather than from a
banner — on the rebased head before any fix, on the fixed head, and again on
every head that followed a documentation commit, so that no run this file
reports was made on a tree that then changed. **Every recipe exit 0 on every
run**, with the same numbers throughout:

| recipe           | result                                                                                                                                                          |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fmt-check`      | ok (Rust; `just fmt` also runs prettier over ~222 unrelated files and was deliberately not run)                                                                 |
| `clippy`         | ok, `-D warnings`                                                                                                                                               |
| `verify`         | all ten gates — `verify-links` **854 links in 153 tracked files**, `verify-status` 1 declared unimplemented item, `verify-toolchain` 1.98.0                     |
| `test-rust`      | **1401 run, 1401 passed, 0 skipped** — 43 binaries, real Postgres and real WireMock rails, thirteen to seventeen minutes a run                                  |
| `test-doc`       | **96 passed, 1 ignored**                                                                                                                                        |
| `verify-ignored` | **0 ignored (expected 0), 43 binaries (expected 43), 1401 total (floor 1080)**                                                                                  |
| `lint-web`       | ok                                                                                                                                                              |
| `test-web`       | `@vpay/checkout` **448 in 23 files, 0 skipped** (was 442 in 22), `@vpay/tokens` **7 in 1** (was 3), `examples/shop` 96, `sdks/stripe-js` 146, `sdks/nodejs` 180 |
| `deny`           | advisories, bans, licenses, sources ok                                                                                                                          |

Nothing under `backends/` is touched by this branch or by this review, so every
Rust number is master's.

**`just test-e2e` on the rebased head, before any fix** — `checkout.cy.ts` 1/1,
`dashboard.cy.ts` 3/3, **`shop-hosted.cy.ts` 0/3**, and the framed run never
reached (`pnpm run e2e:default && pnpm run e2e:framed`).

**`just test-e2e` on the fixed head** — **11 tests, 11 passing, 0 failing, 0
skipped**: `checkout.cy.ts` 1, `dashboard.cy.ts` 3, `shop-hosted.cy.ts` 3,
`shop-embedded.cy.ts` 4. Own compose project `exp21-review` on its own ports,
torn down with `down -v`.

The final `just ci` is in the Status entry of
[`../../flows/hosted-checkout.md`](../../flows/hosted-checkout.md) and in
[`../../status.md`](../../status.md).

## 5. Left alone, and why

- **The PIN vault refusal stands.** Verified rather than taken on trust:
  `grep -rni '\bpin\b' backends/ --include='*.rs'` finds `Box::pin` and prose;
  `POST /v1/browser/payment_intents/{id}/confirm` accepts
  `payment_method_data[type]` and `[mtn_momo][msisdn]` and nothing else. There
  is no PIN anywhere in this system to recall into a form. Building the vault
  would mean adding a mobile-money PIN field to a payment page that consumes
  it nowhere.
- **`@base-ui-components/react` at `1.0.0-rc.0`.** A pre-1.0 dependency on a
  payment page is the maintainer's call and stays surfaced, not decided.
- **`canceled` stays red.** See F3.
- **The Helm ConfigMap.** Still absent; `just helm-check` needs the network and
  is not in `just ci`, so a template added here is one nothing in this pass
  could gate. Named in three places now instead of one.
- **`examples/shop` and `sdks/stripe-js`** were not touched at all. The only
  edit outside `frontends/apps/checkout`, `frontends/packages/tokens`,
  `config/checkout` and `docs/` is one stale **comment** in
  `shop-hosted.cy.ts`; no spec assertion was changed, weakened or skipped.
- **The bumblebee theme's contrast is checked by nobody.** `@vpay/ui`'s
  Storybook runs axe's contrast rule against `corporate` and `business`, which
  this app does not use, and that Storybook is not in `just ci`. Named in the
  flow doc rather than papered over.
