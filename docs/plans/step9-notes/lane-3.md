<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. §6 and §7 below are written so they can be applied verbatim. -->

# Step 9, lane 3 — the page (`frontends/apps/checkout`)

Branch `claude/step9-lane-3-page`, on top of `f186d67` (master + the Step 9
plan).

## 1. What landed

`frontends/apps/checkout`, a new Next 15 App Router app — 59 tracked files,
~6 900 lines of TypeScript, of which about half are tests.

| # | Thing | Where |
|---|---|---|
| 1 | The app itself: `output: 'standalone'`, no server actions, no cookies, `outputFileTracingRoot` at the repo root | `frontends/apps/checkout/next.config.ts` |
| 2 | Routes `/c/[id]` (hosted), `/e/[id]` (embedded), `/c/[id]/return` — all `force-dynamic` | `app/c/[id]/page.tsx`, `app/e/[id]/page.tsx`, `app/c/[id]/return/page.tsx` |
| 3 | Every security header, in one place: `Referrer-Policy: no-referrer`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` on every response; `Content-Security-Policy: frame-ancestors …` derived per request | `middleware.ts`, `src/lib/csp.ts` |
| 4 | The origins lookup (`GET {VPAY_API_URL}/v1/browser/checkout/origins?key=…`), server-side, fail-closed four ways | `middleware.ts`, `src/lib/api.ts` (`fetchCheckoutOrigins`) |
| 5 | The browser client for the two checkout reads — never `@vpay/api-client` | `src/lib/api.ts` (`BrowserCheckoutApi`) |
| 6 | The state machine: a pure reducer, 12 states, 10 events | `src/lib/machine.ts` |
| 7 | The controller: confirm/poll through `@vpay/stripe-js`, the redirect decision, `vpay:complete` after a session re-read | `src/lib/controller.ts` |
| 8 | The return page's own machine and controller — no confirm transition, because it holds no intent secret | `src/lib/return.ts` |
| 9 | Child half of the `postMessage` protocol: `vpay:resize` (first paint + ResizeObserver), `vpay:complete`, `vpay:redirect`; explicit target origin, inbound origin filter | `src/lib/frame.ts` |
| 10 | Origin logic: `originOf`, `normalizeOrigins`, `resolveParentOrigin` | `src/lib/origins.ts` |
| 11 | Entry decision (embed check first, then credentials) | `src/lib/entry.ts` |
| 12 | D6 URL reading: secret from the fragment, publishable key from the query, `client_secret` in a query string **ignored** | `src/lib/link.ts` |
| 13 | `{CHECKOUT_SESSION_ID}` substitution and the outcome→URL rule | `src/lib/forward.ts` |
| 14 | Cameroon E.164 MSISDN normalisation | `src/lib/msisdn.ts` |
| 15 | Minor-unit money rendering, no floating point anywhere in it | `src/lib/money.ts` |
| 16 | Rail→page-flow map (D9), with the offered list read off the intent | `src/lib/rails.ts` |
| 17 | `fr`/`en` dictionaries, `Accept-Language` negotiation, `{name}` interpolation | `src/i18n/{en,fr,index}.ts` |
| 18 | Every screen as pure React, plus the two client containers | `src/components/*.tsx` |
| 19 | 22 Storybook stories with the a11y addon, hosted in `@vpay/ui`'s existing Storybook | `src/components/checkout-screens.stories.tsx`, `frontends/packages/ui/.storybook/main.ts` |
| 20 | A `node:http` stub of all five routes, in the wire contract's shapes | `src/testing/browser-stub.ts` |

The only file outside `frontends/apps/checkout` that this lane touched is
`frontends/packages/ui/.storybook/main.ts` (one added glob, so the checkout
stories are built by the Storybook that CI's `web` job already builds). No
`@vpay/ui` primitive was added — see §5.

`pnpm-lock.yaml` changed (+55 lines): the new package's dependencies. Every
version is one the workspace already resolved (`next` 15.5.25, React 19,
vitest 3.2.7, the Testing Library pair, tailwind/daisyUI/postcss), so no new
package entered the tree — the lockfile gained an importer, not resolutions.
It is committed.

## 2. Counts, measured

| Gate | Result |
|---|---|
| `pnpm install --frozen-lockfile` | ok (after the lockfile commit) |
| `pnpm -r typecheck` (`just lint-web`) | ok, 15 projects |
| `just test-web` (`pnpm -r test`) | **271 tests in 16 files for `@vpay/checkout`, all passing, 0 skipped and 0 ignored**; `@vpay/ui` 3, everything else unchanged |
| `pnpm --filter @vpay/checkout build` | ok — `.next/standalone/frontends/apps/checkout/server.js` present |
| `pnpm --filter @vpay/ui build-storybook` | ok — 23 checkout entries in `storybook-static/index.json` (22 stories + the autodocs page) |
| `just audit-web` | see §8 |

There is no `it.skip`, no `describe.skip` and no `--passWithNoTests` in this
app: `vitest run` fails if it finds no tests.

## 3. Decisions taken in this lane, and why

- **The page's own modules are pure; the React layer is wiring.** The state
  machine is a reducer with no `fetch`, no timer and no DOM, and the entry
  decision (`decideEntry`) is a pure function over `location`/`referrer`.
  That is what lets every refusal and every transition be a test rather than
  a branch inside a `useEffect` nobody can reach twice.
- **The embed check runs before the credential is read.** `decideEntry`
  answers `refused` for a framer that is not on the merchant's list even
  when the URL carries no key and no secret at all, so the refusal cannot be
  used to probe which half of a link is wrong. Pinned by
  `entry.test.ts:"refuses before it looks at the credential"`.
- **Two locks on framing, not one.** `frame-ancestors` on the HTML response
  is what a browser enforces before a pixel is painted; `resolveParentOrigin`
  against the same list is what the page enforces on every `postMessage`.
  The hosted page also refuses to render inside a frame at all, which is the
  second lock for a browser that ignored the first.
- **The middleware's origin list is forwarded to the route on a request
  header** (`x-vpay-embed-origins`), overwritten rather than appended to, so
  the page's `postMessage` target is a member of the list the browser was
  actually given — a second lookup could answer differently.
- **The language switch is client-side.** A link to `?lang=fr` has no
  fragment, and resolving a fragment-less relative URL **drops** the current
  one — which on this page is the session's `client_secret`. The server picks
  the initial locale from `Accept-Language` (so the first byte of HTML has a
  correct `lang`), and the switch swaps the dictionary in place.
  `checkout-view.test.tsx` asserts the rendered page contains no `<a href>`
  at all.
- **French is the default** when `Accept-Language` expresses no preference:
  Cameroon-first, and Orange's own hosted page is French by default.
- **The exponent table is the page's own; grouping is `Intl`'s; there is no
  float.** `5000 XAF` is turned into the string `"5000"` by moving a decimal
  point through the integer's digits, and that string is what
  `Intl.NumberFormat` formats. `minor / 100` would be float arithmetic in the
  money path, which the Rust half of this repository denies workspace-wide.
- **`contextOf` strips the session's `client_secret`** on the way from the
  wire into the state every screen renders from. The controller holds the
  credential separately; a second copy on a rendered object is what ends up
  inside a devtools snapshot or an error report.
- **The CSP is `frame-ancestors` and nothing else, and that is a stated
  gap.** A `script-src`/`default-src` policy worth having needs a per-request
  nonce threaded through Next's inline bootstrap scripts. Shipping a
  permissive `default-src` would read like a content policy while forbidding
  nothing. See §5.
- **Environment variables are read with bracket notation.** Next replaces
  `process.env.NEXT_PUBLIC_FOO` (dot access) with a literal at build time,
  which would bake one deployment's API URL into lane 4's image.
  `env.test.ts` sets the variable *after* import and reads it back;
  additionally, `grep -rl "localhost:8080" .next/server .next/static` after a
  build with that value set found nothing.
- **A missing `NEXT_PUBLIC_VPAY_API_URL` throws** rather than defaulting. A
  checkout page pointed at the wrong API is an operator's 500, not a payer's
  "something went wrong".

## 4. Coordination items for other lanes — please read

These are the places where this lane had to decide something the plan did not
pin. Each is a real interface, and each is cheap to change on this side if
the answer differs.

### 4a. The two browser reads render the session with the intent **expanded**

Confirmed by the integrator mid-lane, and this is what is implemented and
stubbed:

```jsonc
// GET /v1/browser/checkout/sessions/{cs_id}?key&client_secret
{
  "id": "cs_…", "object": "checkout.session", "livemode": false,
  "ui_mode": "hosted", "status": "open", "payment_status": "unpaid",
  "success_url": "…", "cancel_url": "…", "return_url": null,
  "url": "…", "expires_at": 1757000000, "created": 1756913600,
  "client_secret": "cs_…_secret_…",
  "payment_intent": { /* the full PaymentIntent, WITH its client_secret */ },
  "merchant": { "name": "Boutique Test" }
}
```

`GET …/return?key&t=…` renders the **same object** with the intent's
`client_secret` absent **and the session's `client_secret` absent too** — the
return page must not be able to confirm anything, and the type here (`Omit<…,
'client_secret'>`) is what makes that a compile-time fact rather than a
convention. The merchant's display name is `merchant: { name }`; if lane 1
renders `merchant_name` instead, `isSessionEnvelope` in `src/lib/api.ts` is
the one place to change.

### 4b. **The hosted `url` and the return URL need the publishable key**

The plan's URLs are `{base}/c/{cs_id}#{client_secret}` and
`{base}/c/{cs_id}/return?t={return_token}`. **Neither carries a publishable
key**, and all three browser routes require one. Requested shapes:

- hosted `url`: `{checkout.public_base_url}/c/{cs_id}?key={pk}#{client_secret}`
- `ChargeRef::return_url` (lane 2, D2): `{checkout.public_base_url}/c/{cs_id}/return?t={return_token}&key={pk}`

The key in a query string is correct — browser-checkout D1: a publishable key
is not a secret and is rendered into a merchant's public page by
construction. The secret stays in the fragment.

Until that lands, the page copes without inventing anything:

- `/c/{id}` also accepts a `key=` inside the fragment (`#client_secret=…&key=…`);
- the return page falls back to a publishable key the checkout page wrote to
  `sessionStorage` for that session id in the same tab (**only** the key,
  never a secret);
- with neither available it renders `error.missing_key`, naming the missing
  parameter, rather than an outcome it did not read.

### 4c. Lane 6: the demo steering MSISDNs are not valid phone numbers

`237600000ce0` (scenario `mtn-e2e-poll`), `237600000f01` and `237600000f02`
carry hex letters. The MSISDN form validates Cameroon E.164 — `237` + `6` +
eight **digits** — and refuses them, as it refuses any other non-number
(`msisdn.test.ts` pins this explicitly). A form that accepted letters as a
phone number would be accepting them for every payer.

`checkout-hosted.cy.ts` therefore cannot type `237600000ce0` into this page.
The fix belongs in the WireMock mappings, which this lane must not touch: add
a digits-only twin to `requesttopay-scenario.json` (`237600000100`, say, in
the same `2376000000xx` documentation block) keyed to the same scenario.
`237600000400` and `237600000000` are already digits-only and already used by
`confirm_rails.rs` / the conformance suite.

### 4d. Lane 4: build and environment

- `pnpm --filter @vpay/checkout build` runs `pnpm --filter @vpay/stripe-js build`
  first (see §5), so the Dockerfile's `checkout` target must copy `sdks/` —
  which the plan already says it will.
- `output: 'standalone'` with `outputFileTracingRoot` at the repo root emits
  `.next/standalone/frontends/apps/checkout/server.js` plus a
  `node_modules/` and a top-level `package.json`. Static assets are **not**
  copied by Next: `.next/static` and any `public/` must be copied in
  alongside, as Next's own docs say.
- Environment: `NEXT_PUBLIC_VPAY_API_URL` (required, the browser's origin;
  read at **runtime**, so it does not need to be a build arg) and
  `VPAY_API_URL` (optional; without it the embedded page's CSP is
  `frame-ancestors 'none'`, which is correct but means no merchant can embed).
- The app listens on `3001` by default (`next start -p 3001`); the standalone
  server honours `PORT`.

### 4e. Lane 5: the protocol as the child implements it

- `vpay:complete` carries `session` as the `cs_…` **id string** plus
  `status` (the session's own, after a re-read that happens once the intent
  is terminal). Never the session object, never a secret —
  `controller.test.ts` and `secrets.test.ts` both assert it.
- `vpay:redirect` is posted for a redirect rail whenever the page is framed;
  the page never attempts a top-level navigation itself in that case. Same
  for the final forward to `return_url`.
- `vpay:resize` is posted **on channel open** (first paint) and on every
  `ResizeObserver` callback, so a parent that creates the iframe at
  `height: 0` grows it.
- Every message names the framer's origin explicitly. `'*'` appears nowhere.
  Inbound messages whose `event.origin` differs are dropped unread.

## 5. What this lane did **not** do

- **No browser has run this page.** Every test here is vitest against a
  `node:http` stub and jsdom. There is no screenshot, no Cypress run, and no
  proof that the CSP is enforced by a real browser rather than merely sent —
  that is lane 6's, and this row should stay honest until lane 6 measures it.
- **No real rail, no real server.** The five routes this app speaks to do not
  exist yet (lane 1 builds three of them). The stub implements the wire
  contract's shapes; if lane 1's shapes differ, these tests will keep passing
  while the page breaks. §4a is the interface to check.
- **The CSP has no `script-src`, `default-src`, `connect-src` or
  `form-action`.** Only `frame-ancestors`. A nonce-based policy over Next's
  inline bootstrap is not built.
- **No `@vpay/ui` primitive was added.** The screens are the checkout app's
  own components, not shared ones; `PayerSheet` was not reused, as the plan
  required. The only `@vpay/ui` change is one Storybook glob.
- **Storybook's a11y addon is configured but nothing gates on it.**
  `build-storybook` proves the stories compile; it does not run axe. No
  automated accessibility gate exists in this repository, and this lane did
  not add one. What *is* asserted, in vitest: every control is a native
  focusable element with an accessible name, the live region is mounted from
  first render, focus moves to the new screen's heading, and the MSISDN error
  is tied to its field with `aria-describedby`/`aria-invalid`.
- **`pnpm -r lint` is still broken repo-wide** (`@vpay/ui` and `@vpay/tokens`
  declare `eslint src` with no eslint dependency; `@vpay/config`'s `./eslint`
  export points at a file that does not exist). The checkout app deliberately
  declares **no** `lint` script rather than adding a fourth broken one.
  Neither `just lint-web` nor CI's `web` job runs `pnpm -r lint`, so nothing
  regressed; it was already this way.
- **`@vpay/checkout`'s `build`/`typecheck`/`test` each run
  `pnpm --filter @vpay/stripe-js build` first.** `@vpay/stripe-js`'s `exports`
  resolve to `dist/`, which is gitignored, so on a clean checkout the
  typecheck would otherwise fail with `TS2307`. `just lint-web` already does
  this for `@vpay/sdk` via `build-sdk-node`; doing it inside the package's own
  scripts avoids touching the justfile or `.github/workflows/ci.yml`, both of
  which other lanes own. It costs three ~2 s `tsc` runs.
- **No justfile recipe was added.** `just lint-web` (`pnpm -r typecheck`) and
  `just test-web` (`pnpm -r test`) already fan out to every workspace project,
  so the new app is covered by both without a change.
- **The auto-forward countdown is 5 s and not configurable.** No merchant
  control over it exists.
- **Nothing here is wired into compose, Helm, the demo or CI's `e2e` job** —
  lanes 4 and 6.

## 6. Guard-failure proofs

Each guard was broken, the suite run, the failure observed, and the file
restored (`pnpm --filter @vpay/checkout test` green again after each).

**A — the framer allow-list check.** In `src/lib/origins.ts`,
`resolveParentOrigin`'s `return allowed.includes(origin) ? origin : null`
replaced by `return origin`:

```
× resolveParentOrigin > refuses a framer that is not on the list
× resolveParentOrigin > refuses when the list is empty, whatever the referrer says
× resolveParentOrigin > refuses a look-alike origin
× an embedded page > refuses a framer that is not on the list
× an embedded page > refuses when the origins lookup produced nothing — fail-closed
× an embedded page > refuses before it looks at the credential, so a hostile framer learns nothing
Tests  6 failed | 20 passed (26)
```

**B — D6, the secret is never read from a query string.** In
`src/lib/link.ts`, one line added:
`clientSecret = clientSecret ?? query.get('client_secret')`:

```
× parsePageCredentials > IGNORES a client_secret in the query string
Tests  1 failed | 10 passed (11)
```

**C — the origins lookup fails closed.** In `src/lib/api.ts`,
`fetchCheckoutOrigins`'s `catch { return []; }` replaced by
`catch { return ['https://shop.example']; }`:

```
× the embedded page > is 'none' when the lookup fails — fail-closed, not fail-open
Tests  1 failed | 11 passed (12)
```

## 7. Status rows for lane E (verbatim)

Add to `docs/status.md`'s **Frontend** table:

| `frontends/apps/checkout` (the hosted/embedded payment page) | 🟡 | **New 2026-09-04 (Step 9, lane 3).** A Next 15 App Router app, `output: 'standalone'`, no server actions and no cookies, serving `/c/{cs_id}` (hosted), `/e/{cs_id}?key=pk` (embedded) and `/c/{cs_id}/return?t=…` (the return trip, top-level in both modes). Every response carries `Referrer-Policy: no-referrer`, `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`; `Content-Security-Policy: frame-ancestors 'none'` on the hosted and return pages, and on the embedded page the merchant's registered origins resolved **server-side** by `middleware.ts` from `GET {VPAY_API_URL}/v1/browser/checkout/origins?key=…` — fail-closed on a missing key, a missing `VPAY_API_URL`, a failed lookup and an empty list alike, all four proven in `src/middleware.test.ts` against the shipping middleware function. The page reads the session, shows the amount (integer minor units, no floating point anywhere in the conversion), the merchant name and a rail selector **only when the intent offers more than one rail this page can drive**; MTN collects a Cameroon E.164 MSISDN and confirms and polls through `@vpay/stripe-js`; Orange confirms with `redirect: 'if_required'` and then either navigates top-level or asks its parent to (`{type:'vpay:redirect'}`) when framed. `fr` and `en`, chosen from `Accept-Language` server-side, switchable in the page **without navigating** — a `?lang=` link would drop the URL fragment the session credential lives in. **271 vitest tests in 16 files, 0 skipped**, including a `node:http` stub of all five browser routes, both guard directions of the origin check, and a test that no `console.*` call, no navigation and no `postMessage` ever carries a secret. **🟡, and for one reason: no browser has ever rendered this page.** Every test is vitest and jsdom; the CSP is proven *sent*, not proven *enforced*; the routes it speaks to do not exist yet (lane 1). See `docs/plans/step9-notes/lane-3.md` §5 |

Add to the same table:

| Checkout Storybook stories (a11y addon) | 🟡 | **New 2026-09-04 (Step 9, lane 3).** 22 stories covering every screen the checkout state machine can be in — loading, rail selector, MSISDN form and its rejection, Orange prompt, confirming, waiting (with and without a failed poll), redirecting, succeeded/failed/canceled, forwarding, expired, embedding refused, no drivable rail, invalid link — in both locales for the two busiest, plus three return-page screens. They are built by `pnpm --filter @vpay/ui build-storybook` (CI's `web` job) from the *same* literal states `checkout-view.test.tsx` asserts against, so a screen cannot have a story without a test or a test without a story. 🟡 because the a11y addon is configured but **nothing runs axe in CI**: `build-storybook` proves the stories compile |

## 8. `just audit-web`

**Green, 2026-09-04, and it did run** — worth stating because it nearly did
not. npm's audit endpoint was answering `503 Service Unavailable` (and, for
a stretch, `ERR_SOCKET_TIMEOUT`) while the plain registry answered a `PING`
in 220 ms, so the first attempt of each label failed the way an unreachable
registry does. The recipe's retry ladder is what turned that into a real
measurement rather than a `REGISTRY UNREACHABLE`:

```
audit-web: production dependency graph only (attempt 3 of 4)
No known vulnerabilities found
audit-web: whole workspace, dev dependencies included (attempt 2 of 4)
No known vulnerabilities found
audit-web: ok — no high or critical advisory in the workspace
```

Both labels answered, so the new package's dependency graph is genuinely
audited and not merely un-refused. Nothing was added to
`pnpm.auditConfig.ignoreCves` (still absent) and no `pnpm.overrides` entry
changed: every version this app pulls in is one the workspace already
resolved.
