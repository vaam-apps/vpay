# The dashboard on Refine, and how much of it CrateStack should generate

**Status: a plan. Nothing in it is built.** Written 2026-09-11 against
`origin/master` at `6b1b7d8` in a read-only worktree
(`.claude/worktrees/exp52-refine-plan`). No source file was changed, nothing was
compiled and nothing was run — every claim below is read from a file, from a
published crate or npm tarball, or from a docs page, and each is cited. §11 is
the list of things I could not determine.

---

## 0. The question I was given, and the half of it I am answering

The maintainer's message, verbatim and complete as received:

> "the dashboard looks like shit actually. Revamp it to use the refine
> structure. We're using cratestack. Why should we bother with raw
> implementation ? Let"

It stops mid-sentence at "Let". I am not guessing what followed. What the
message does say is three things, and this document answers exactly those:

1. the dashboard's appearance is not good enough;
2. it should be restructured onto Refine;
3. since CrateStack is already in the build, raw implementation needs a
   justification.

**The two questions are separable, and separating them is the single most
useful thing in this document.** "Use Refine" is a decision about the browser:
which React data/routing/auth abstraction the dashboard app is built on. "Use
CrateStack's generated transport and client" is a decision about the server:
whether `/dash/v1` stops being a hand-written axum surface and becomes a
generated one. Each can be taken without the other, and they have different
answers.

The short form of both answers, before the detail:

- **Refine headless over `@vpay/ui` is a decision already taken, so §2 costs and
  sequences it rather than arguing it.** The cost that dominates is not the
  dependency: `<Refine>` is a client component, and the dashboard is
  deliberately a server-rendered app whose `/dash/v1` bearer token never reaches
  a browser. Adopting Refine's hooks therefore means adding a same-origin BFF,
  which is a new browser-reachable surface and the one part of this work that
  needs its own security review. §2.3 maps every hook onto the hand-written
  state it replaces.
- **CrateStack's generated transport and client cannot serve `/dash/v1` today,
  and `payment_intents` is precisely the resource that forecloses it.** That is
  not a preference, it is a measured blocker already recorded in
  `docs/reference/vpay-db.md`, and §3.3 states it plainly rather than planning
  around it.

**Nothing here addresses "looks like shit" directly, and that is deliberate.**
Refine is headless; it renders nothing. Appearance lives in `@vpay/ui`, which is
being restructured on **a separate track** — per-component folders, a hard
200-line-per-file limit, shared by checkout and dashboard. This plan **assumes
that package exists in that shape and composes from it**; it does not design it,
and nothing below should be read as a proposal about it. If the appearance is
the actual priority, that track is where it lands and it is independent of both
questions here.

---

## 1. What exists today, stated precisely

### 1.1 The app

`frontends/apps/dashboard` is a Next.js 15 App Router app. Every page file
(`app/page.tsx`, `app/login/page.tsx`, `app/login/totp/page.tsx`,
`app/login/password/page.tsx`, `app/payments/page.tsx`,
`app/payments/[id]/page.tsx`) is a **Server Component** carrying
`export const dynamic = "force-dynamic"`, and none carries `"use client"`. Four
files in the whole app are client components:
`src/components/payments-filters.tsx`, `sign-in-form.tsx`, `totp-form.tsx`,
`password-form.tsx` — and the first of those has no `onSubmit`; it is a plain
`<form method="get">` so that filters live in the URL.

**There is no client-side data fetching anywhere in this app.** Every read is a
server `fetch` inside the render; every mutation is a Server Action passed down
as a prop.

### 1.2 The session, which must survive

The app is the OAuth client itself (ADR-0017 decision 4). The pieces that matter
to any migration:

- `SESSION_COOKIE = "vpay_dash_session"` with
  `{ httpOnly: true, secure: true, sameSite: "lax", path: "/" }` and
  `SESSION_MAX_AGE_SECONDS = 12 * 60 * 60`
  (`src/server/cookies.ts`). `secure: true` unconditionally, no environment
  branch.
- The app **never holds the `/dash/v1` access token**. It reads it back out of
  the `staff_sessions` row on every render via
  `GET /dash/v1/staff/session` → `SessionResponse.access_token`. That is what
  makes sign-out a revocation.
- `REMINT_AFTER_FRACTION = 0.8` (`src/server/gate.ts:59`), with
  `marginMs = ttlSeconds * (1 - REMINT_AFTER_FRACTION) * 1000` and
  `now >= expiresAt - marginMs` (`gate.ts:123-124`). Fail-closed on an absent,
  unparseable or non-positive TTL.
- `refusalFor(failure)` is `failure.status === 401 ? "sign-out" : "outage"` —
  **403 is an outage, not a sign-out** (issue #88 item 2).
- `readDash` (`src/server/dash-read.ts`) retries **exactly one** `401` after a
  re-mint, never a `403`, and returns the original refusal on a second `401`.
- `originRefusal()` (`src/server/csrf.ts`) fronts every one of the four
  `"use server"` exports, comparing `Origin` against the configured public
  origin, falling back to `Host`, and **never reading `X-Forwarded-Host`**
  (issue #88 item 4).

All of this is security-reviewed and is a constraint on, not an input to, any
migration.

### 1.3 The read surface

`/dash/v1` is two `GET` routes and nothing else
(`backends/crates/vpay-api/src/dash/mod.rs:124-135`):

| Method       | Path                            | Handler                           |
| ------------ | ------------------------------- | --------------------------------- |
| `GET`/`HEAD` | `/dash/v1/payment_intents`      | `dash::payment_intents::list`     |
| `GET`/`HEAD` | `/dash/v1/payment_intents/{id}` | `dash::payment_intents::retrieve` |

Five properties of it are load-bearing, and every one of them is the thing a
generator would have to reproduce:

1. **Merchant scope on every query.** `MerchantScope::for_dashboard` is inserted
   into request extensions by `require_dashboard_token`
   (`vpay-api/src/lib.rs:1132-1134`) from the **YAML dashboard binding**, not
   from the token; the extractor fails closed with a 500 if it is absent. Every
   repository call takes `scope.merchant_id()`.
2. **A uniform cross-tenant 404.** Because the tenant predicate is in the SQL,
   "another merchant's row" and "no such row" both produce `Ok(None)` and
   therefore the identical `ApiError::NotFound`. Pinned by
   `another_merchants_intent_is_indistinguishable_from_one_that_never_existed`
   (`backends/tests/integration/tests/dashboard_read_surface.rs:531`), which
   compares the two response **bodies** byte-for-byte with the ids normalised
   out — not merely "both are 404".
3. **A cursor whose tenancy predicate two reviews had to fix.** The cursor _is_
   the `pi_…` id, shape-checked only, resolved to a `seq` by a correlated
   subquery (`vpay-db/src/payment_intents.rs:871-883`):
   `seq < (SELECT seq FROM payment_intents WHERE id = $2 AND merchant_id = $1)`.
   Removing `AND merchant_id = $1` from either subquery left all 35 tests green
   in a 2026-09-06 mutation run; test 12
   (`dashboard_read_surface.rs:1171`) is what makes it red now.
4. **An audience-bound token.** `aud` is the registered
   `dashboard_client.client_id` (ADR-0017 decision 3), with
   `set_required_spec_claims(&["exp", "aud", "iss"])`
   (`vpay-api/src/resource_auth.rs:538-563`), plus a `vpay_merchant_id` claim
   check, a scope check, and a **live staff-row re-read** per request
   (`lib.rs:1105-1126`).
5. **Writes are refused at the boundary, before the router matches.**
   `dash::required_scope` returns `None` for any non-`GET`, and
   `require_dashboard_token` turns that into a 403 — so the surface is
   structurally read-only, not read-only by omission
   (`dash/mod.rs:176-186`, `lib.rs:1022-1028`).

Slices 2–6 (webhooks, checkout sessions, balances, settings, rail health) **do
not exist in any form** — no routes, no handlers, no stubs. A repo-wide grep for
`dash/v1/webhooks`, `dash/v1/balances`, `dash/v1/settings`, `dash/v1/checkout`,
`dash/v1/rail` returns zero hits. Their absence is deliberate
(`dash/mod.rs:118-123`).

### 1.4 The list and detail shapes

List: `ListObject<PaymentIntentObject>` — exactly four keys, `object`, `data`,
`has_more`, `url` (`vpay-api/src/model.rs:1090-1103`). **No `total_count`, no
`next_page`.** `PaymentIntentObject` is the merchant surface's object unchanged
(`model.rs:332-391`) and carries `payment_method_types: Vec<String>` and
`metadata: Map<String, Value>`.

Detail: `PaymentDetail` (`dash/payment_intents.rs:197-213`) — a **four-way
composite** over four tables: the intent, an optional `ChargeSummary`, a
`Vec<RefundObject>`, and a `Vec<TimelineEvent>`, assembled from four separate
repository reads.

### 1.5 The app's query contract

`src/payments-query.ts`: `PAGE_SIZE = 25`; URL params `status`, `created_from`,
`created_to` (both `YYYY-MM-DD`), `after`, `before`; API params `limit`,
`status`, `created_gte`, `created_lte` (RFC 3339), `starting_after`,
`ending_before`. A repeated param takes the **first** value. An unparseable date
is **dropped**, not forwarded. `after` wins if a hand-edited URL carries both
cursors.

`pagerHrefs` encodes the non-obvious rule: `has_more` means "a row exists past
the limit **in the direction just walked**", so on a backward page it reports
_newer_ rows rather than older ones. Any replacement must preserve that
inversion.

---

## 2. Question 1 — Refine for the UI

### 2.1 The shape, which is already decided

Refine is headless. It ships hooks, a router binding, a `dataProvider` contract
and an `authProvider` contract, and renders nothing. The decision is taken:
**`@refinedev/core` + `@refinedev/nextjs-router` over vpay's own `@vpay/ui`**,
which stays Base UI + daisyUI + `class-variance-authority` +
`tailwind-merge`/`clsx` on theme bumblebee. There is no UI-kit question to
evaluate and this plan does not evaluate one.

Two consequences worth stating so they are not rediscovered:

- `just verify-ui` keeps its meaning unchanged. It is a `git grep` gate over
  `frontends/apps` (and `frontends/packages/ui/src`) that fails on any palette
  colour or arbitrary colour value outside a daisyUI theme token. Refine
  contributes no markup and therefore no classes, so nothing in this migration
  puts pressure on that gate. If a diff in this work needs a `className`, it
  belongs in `@vpay/ui`, not in the app.
- **`@vpay/ui`'s own restructuring is a separate track and is not designed
  here.** This plan assumes that package already exists in the agreed shape —
  per-component folders, a hard 200-line-per-file limit, shared by checkout and
  dashboard — and composes from it. Every component named below
  (`Table`, `Select`, `Input`, `Field`, `Alert`, `Spinner`, `StatusBadge`,
  `Stack`, `PageShell`, …) is assumed to arrive from that package in that shape.
  If a screen needs a primitive that does not exist there yet, that is a request
  against that track, not a component written in `frontends/apps/dashboard`.

### 2.2 What Refine actually buys here

Honestly, and in order of how much each is worth **today**:

| Refine capability                                     | Worth today              | Why                                                                                                                                                                                                                                                                          |
| ----------------------------------------------------- | ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dataProvider` as a single seam                       | Moderate                 | `src/payments-query.ts` + `src/server/dash-read.ts` already _are_ that seam, informally. Refine makes it a named contract with a typed shape.                                                                                                                                |
| List state: filters, sorters, pagination (`useTable`) | **Low**                  | The app deliberately keeps filters in the URL so the page bookmarks, reloads, and works with JS off. Refine's `useTable` moves that into client state — a regression against an explicit design decision unless `syncWithLocation` is used, which re-derives the URL anyway. |
| CRUD scaffolding + `useForm`                          | **Zero today**           | `/dash/v1` refuses every non-`GET` at the boundary. There is nothing to create, edit or delete. This is the largest single part of Refine's value proposition and it is currently inert.                                                                                     |
| `authProvider` plumbing                               | Low                      | The app already has a complete, reviewed auth stack. Refine's `authProvider` would wrap it, not replace it (§2.5).                                                                                                                                                           |
| Resource/route registry                               | **Moderate and growing** | `NAV_LINKS` has one entry and `src/layout.test.tsx` gates that every entry resolves to a real page. Refine's `resources` array is the same idea with more structure, and it is what would pay off across slices 2–6.                                                         |
| `useInfiniteList` cursor support                      | Moderate                 | Refine _does_ support cursor pagination — `getList` returns `{ data, total: 0, cursor: { next, prev } }`. See §2.4.                                                                                                                                                          |

**The blunt summary: Refine's value here is a bet on slices 2–6, not a return on
what is built.** Two reads and zero writes do not need a CRUD meta-framework.
That is a legitimate bet — five more slices at one data provider each is exactly
what Refine is for — but it should be taken as a bet, with the cost in §2.6
priced in, rather than as an efficiency.

### 2.3 Which Refine hook replaces which hand-written thing

This is the substance of the migration. Every row names the file that holds the
state today, the hook that would hold it instead, and — where it matters — the
reason the swap is not clean. Hook names and shapes are Refine **v5**
(`@refinedev/core` 5.0.12); v5 renamed `current` → `currentPage` and added a
`result` property to the data hooks
(context7 `/refinedev/refine`, `documentation/docs/migration-guide/4x-to-5x.md`).

#### The payments list

| What                           | Today                                                                                                  | Refine v5                                                                                                 |
| ------------------------------ | ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- |
| Fetch the page                 | `app/payments/page.tsx` calls `readDash<PaymentIntentList>(…)` inside the Server Component             | `useList` (or `useInfiniteList`, §2.4) via the `dataProvider`                                             |
| Filter state                   | `queryFrom(searchParams)` → `PaymentsQuery` (`src/payments-query.ts`), read from the URL on the server | `useTable`'s `filters` / `setFilters`, with `syncWithLocation: true` so the URL stays the source of truth |
| The query string sent upstream | `apiQueryString(query)`                                                                                | the `dataProvider.getList` mapper in §4                                                                   |
| Page size                      | `PAGE_SIZE = 25`                                                                                       | `useTable({ pagination: { pageSize: 25 } })`                                                              |
| Paging controls                | `pagerHrefs(query, rows, hasMore)` → `<Pager previousHref nextHref />`                                 | **does not map.** See below.                                                                              |
| Loading / error                | not modelled — the server either rendered rows or rendered `<ReadFailure />`                           | `tableQuery.isLoading` / `isError`; `<Spinner />` and `<ReadFailure />` from `@vpay/ui`                   |
| Rows                           | `<PaymentsTable rows={…} />`, a Server Component                                                       | the same component, fed from `result.data`                                                                |

**`useTable` is the wrong hook for this list and it is worth being explicit
about why.** `useTable` v5 returns `{ result: { data, total }, tableQuery,
currentPage, setCurrentPage, pageSize, pageCount, filters, setFilters, sorters,
setSorters, createLinkForSyncWithLocation }`. `pageCount` is derived from
`total`, and `/dash/v1` returns **no total** — `ListObject` is
`{ object, data, has_more, url }` and nothing else. A `useTable` wired to it
reports `pageCount` as `0` or `NaN` and its First/Last/page-number controls
become decorative, which is precisely the "looks finished, is not" failure mode
this repo gates against.

So: **`useTable` for `filters` and `syncWithLocation` only, or not at all.** The
list itself is `useList` with `pagination: { mode: "off" }` and the cursor passed
through `meta`, with the existing `Pager` kept as a `@vpay/ui`-composed component
driven by `pagerHrefs`. `pagerHrefs`' direction-sensitive `has_more` inversion
(`has_more` on a backward page means _newer_ rows exist, not older) has no Refine
equivalent and must survive verbatim — Lane 1's decisive check is exactly that.

#### The payment detail

| What            | Today                                                       | Refine v5                                                                                                                                 |
| --------------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Fetch           | `app/payments/[id]/page.tsx` → `readDash<PaymentDetail>(…)` | `useShow` → `{ query, result }`, or `useOne` directly                                                                                     |
| 404 → not-found | `notFound()` on a `404` from vpay                           | the error carries `statusCode: 404`; the route maps it to `notFound()`. **The two 404 bodies must stay indistinguishable** (§1.3 point 2) |
| Render          | `<PaymentDetailView detail={…} />`                          | unchanged, fed from `result`                                                                                                              |

`PaymentDetail` is a four-way composite (§1.4). Refine sees it as one record on
one resource, which is fine — the composition happens server-side and Refine
never learns there were four reads. Do **not** decompose it into four resources
and four hooks: that is four round trips and a consistency window on a page that
is one read today.

#### Filters, as a component

`src/components/payments-filters.tsx` is `"use client"` today but has no
`onSubmit` — it is a `<form method="get">`, deliberately, so the page bookmarks,
reloads and works with JavaScript off. Under Refine it becomes
`setFilters(...)` calls against controlled `@vpay/ui` `Select` / `Input`
primitives.

**That is a regression unless `syncWithLocation: true` is on**, and even then the
JS-off property is gone. If the JS-off property is to be kept, the honest shape
is to leave the GET form exactly as it is and initialise Refine's filter state
from `searchParams` — Refine does not object to being fed. R4 in §7 carries the
mitigation and the test.

#### Forms

There are four forms in the app and **none of them is a data mutation**:
`sign-in-form.tsx`, `totp-form.tsx`, `password-form.tsx` (all three auth), and
`payments-filters.tsx` (a GET). Every one is a Server Action wired through
`FormState` / `FormAction` (`src/form-state.ts`), with `originRefusal()` in front
of it.

**Therefore `useForm` has nothing to do here and should not be introduced.**
`@refinedev/core`'s `useForm` and `@refinedev/react-hook-form`'s wrapper exist to
drive `dataProvider.create` / `.update`, and both of those throw by design (§4).
The three auth forms belong to `authProvider`, not to `useForm`:

| Form                   | Today                          | Refine v5                                                                                                                                                                                                                                                    |
| ---------------------- | ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `sign-in-form.tsx`     | `signIn` Server Action         | `useLogin()` → `authProvider.login`, whose body calls the same Server Action                                                                                                                                                                                 |
| `totp-form.tsx`        | `submitTotp` Server Action     | `useLogin()` with the second-factor payload, same wrapping                                                                                                                                                                                                   |
| `password-form.tsx`    | `changePassword` Server Action | stays a Server Action. Refine's `updatePassword` is for a _forgot-password_ flow; this is a forced-change step inside an authenticated session, and mapping it onto the optional `authProvider.updatePassword` would gain nothing and lose `originRefusal()` |
| `payments-filters.tsx` | `<form method="get">`          | `setFilters` (above), not `useForm`                                                                                                                                                                                                                          |

The rule for all four: **the Server Action keeps its `originRefusal()` guard and
keeps being the thing that touches cookies.** A Refine hook may call it; it may
not replace it. `clearSessionCookie()` throws during a page render and is
callable only from a server action — that is a Next constraint, not a style
choice.

#### Auth plumbing

`authProvider` is a **wrapper over the existing, security-reviewed stack**, and
§2.5 gives its four methods. The hooks that consume it:

| Hook                 | Calls                      | Wraps                                                                                             |
| -------------------- | -------------------------- | ------------------------------------------------------------------------------------------------- |
| `useLogin`           | `authProvider.login`       | `signIn` / `submitTotp` Server Actions                                                            |
| `useLogout`          | `authProvider.logout`      | `signOut`, which posts the revocation **before** clearing the cookie — that order is load-bearing |
| `useIsAuthenticated` | `authProvider.check`       | `requireStaff()`'s gate, surfaced through a server call                                           |
| `useOnError`         | `authProvider.onError`     | `refusalFor` verbatim: `401` → sign out, everything else including `403` → outage                 |
| `useGetIdentity`     | `authProvider.getIdentity` | `SessionResponse`'s `display_name` / `email` / `merchant_id`, for `<SignedInBar />`               |

**What no hook may touch**, because none of it can run in a browser: PKCE
(`src/server/pkce.ts`), the authorization-code exchange (`src/server/oauth.ts`),
the token read out of the `staff_sessions` row, the 80 %-of-TTL re-mint
(`REMINT_AFTER_FRACTION = 0.8`, `gate.ts:59`), the `X-Vpay-Staff-Session` header,
and `readDash`'s single-`401` retry. All of it stays where it is, called from the
BFF (§2.6 cost 1) and from the Server Actions.

The one genuinely new question the wrapper raises: `authProvider.check` runs on
the client on every navigation, and the gate it wraps is the same one that
_performs the re-mint_. A `check` that triggers a re-mint on every route change
is a token mint per navigation. The mitigation is that `check` asks only
"is there a live session" and leaves the re-mint on the read path where it is
today — which is why Lane 3's decisive check 2 is that the re-mint still fires
from a render, not from `check`.

#### Routing

`@refinedev/nextjs-router` supplies `routerProvider`; `useNavigation` /
`useGo` / `useLink` replace hand-written `<Link href>` targets where a resource
name is more honest than a path. `src/nav.tsx`'s `NAV_LINKS` becomes Refine's
`resources` array (Lane 4), and `src/layout.test.tsx`'s gate — every declared
link resolves to an `app/**/page.tsx` on disk — moves with it. Two lists that can
disagree is the failure that lane exists to prevent.

### 2.4 The pagination fit, which is better than it first looks

Refine's default `getList` contract is offset-shaped:
`{ pagination: { currentPage, pageSize }, sorters, filters }` → `{ data, total }`.
`/dash/v1` has no total and no offset, so that shape does not fit.

Refine's documented cursor path does fit. `getList` may return
`{ data, total: 0, cursor: { next, prev } }`, and `useInfiniteList` consumes it,
with `queryOptions.getNextPageParam` available to derive the cursor from the last
row (Refine docs, `data/hooks/use-infinite-list`). That maps onto `has_more` and
the last row's `pi_…` id directly.

**One mismatch survives.** `useInfiniteList` is append-style ("Load more"); the
dashboard's pager is bidirectional prev/next _page_ navigation with a
direction-sensitive `has_more`. Refine does not model "walk backwards from a
cursor" natively. Two honest options, and this is an implementer's choice rather
than a reserved one:

- keep the existing URL-driven pager as a plain component and give the data
  provider `meta.after` / `meta.before` (Refine passes `meta` through
  untouched), or
- switch the payments list to infinite scroll, which is a **product** change and
  therefore is a reserved decision (§8, RD3).

### 2.5 `authProvider` as a wrapper, never a replacement

Refine's `AuthProvider` requires `login`, `check`, `logout`, `onError`. The
correct shape here is that every one of those calls the code that already exists:

| Refine method | Wraps                                                                                                                                                   |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `login`       | the existing `signIn` / `submitTotp` Server Actions, unchanged                                                                                          |
| `logout`      | the existing `signOut` Server Action, which posts the revocation _before_ clearing the cookie                                                           |
| `check`       | a Server-Action or route-handler call whose body is the existing `requireStaff()` gate, returning `{ authenticated }`                                   |
| `onError`     | `refusalFor`'s exact rule: `401` → sign out, everything else → outage. **Widening this to `>= 400` is the named decisive mutation** (issue #88 item 2). |

`getIdentity` maps to `SessionResponse`'s `display_name` / `email` /
`merchant_id`. There is no `register`, no `forgotPassword`, and no
`updatePassword` beyond the existing forced-change step — do not implement the
optional methods just because the interface lists them.

**What must not move into `authProvider`:** PKCE, the code exchange, the
80 %-of-TTL re-mint, the `X-Vpay-Staff-Session` header, and the single-`401`
retry. Those run server-side because the token must not reach a browser. An
`authProvider` that "handles the token" is the failure mode this whole section
exists to prevent.

### 2.6 What Refine costs, stated without softening

1. **`<Refine>` is a client component.** Refine's own Next.js App Router
   documentation puts `"use client"` at the top of the layout that renders
   `<Refine routerProvider={routerProvider}>`. Everything inside it is a client
   component. The dashboard today is server-rendered end to end with a token
   that is _deliberately_ never sent to a browser.

   **Therefore Refine's data layer needs a same-origin BFF.** A set of route
   handlers under `app/api/dash/…` that authenticate with the httpOnly session
   cookie, run the existing `requireStaff()` + `readDash()` path server-side, and
   proxy to `/dash/v1`. The data provider then talks to `/api/dash/…`, not to
   vpay.

   That is a **new browser-reachable surface on the dashboard's own origin** and
   it is the single largest cost in this document. It needs the `csrf.ts` origin
   check applied to it (route handlers are not Server Actions and get nothing for
   free), it needs the same 401/403 discipline, and it needs its own security
   review. The dashboard README currently states there is no such surface; that
   sentence would stop being true.

   The alternative is Refine's SSR path — fetch server-side and hand results in
   as `initialData`/`dehydratedState`. It keeps the token server-side, but it
   also gives up most of what Refine's hooks are for, because the first render is
   still a Server Component doing the fetch.

2. **Dependency surface.** `@refinedev/core` 5.x, `@refinedev/nextjs-router` 7.x,
   and TanStack Query (Refine's core data layer). The dashboard today has React,
   Next, `@vpay/ui`, `@vpay/tokens` and `qrcode`, and no client-side data
   library at all.

3. **The four existing client components and 178 tests.** `payments-filters.tsx`
   is a GET form by design; under Refine it becomes controlled state. Nine
   component-test files and `a11y.test.tsx` render those components directly —
   every one of them would need a Refine provider wrapper in the test harness.

4. **`a11y.test.tsx` and `layout.test.tsx`.** The nav gate asserts that every
   `NAV_LINKS` href resolves to an `app/**/page.tsx` on disk. Refine's
   `resources` array becomes a second declaration of the same thing; the gate has
   to move to it, or there are two lists that can disagree.

### 2.7 Verdict on question 1

The decision to adopt headless Refine over `@vpay/ui` is taken, so this is not a
recommendation about _whether_. It is the two things a planner owes a taken
decision — what it costs, and in what order it should land.

**The cost that dominates is the BFF (§2.6 cost 1), not the dependency.** Refine's
hooks run in the browser; `/dash/v1`'s bearer token must not. Every other line
item — the four client components, the test-harness wrapper, the bundle — is
ordinary migration work. That one is a new browser-reachable surface on the
dashboard's own origin and it is the reason Lane 2 is its own lane with its own
security review rather than a step inside Lane 3.

**The order that follows:** the seam first with no framework (Lane 1), the BFF
reviewed on its own (Lane 2), then Refine behind the existing pages (Lane 3),
then the resource registry (Lane 4). That sequence means every decisive check in
§1.2 — the 80 %-of-TTL re-mint, `403`-is-an-outage, the byte-identical
cross-tenant 404, the origin check — has a lane that owns it and a mutation that
makes it red, instead of all four moving in one diff.

**One expectation worth setting now rather than after:** the part of Refine that
justifies a CRUD meta-framework — `useForm`, create/edit routes, `useTable`'s
pagination — is inert on this surface today and stays inert until `/dash/v1`
grows a write, which ADR-0008 gates behind an `audit_log` that does not exist.
What lands in Lane 3 is list/detail/auth plumbing over two reads. That is a real
simplification of the app's own state, and it is a smaller return than the
framework's full value; both are true and neither should be oversold in the PR
that lands it.

---

## 3. Question 2 — how much should CrateStack generate

### 3.1 The distinction that governs the answer

`/v1` is a **public product contract**, shaped like Stripe's, carried by two
SDKs and by `docs/sdks/parity.md`. Its shape is not CrateStack's to choose, and
nothing in this document proposes changing it.

`/dash/v1` is **internal**. It is consumed by exactly one client — this
dashboard — it is explicitly not an SDK surface (`dash/mod.rs:73-80`), and it is
CRUD-shaped over models `schemas/vpay.cstack` already declares. On the face of
it, it is the one surface in vpay where "why bother with raw implementation" has
real force.

### 3.2 What CrateStack 0.12.0 actually generates

Read from the vendored crates rather than from release notes:

**Server (`cratestack-macros` 0.12.0, emitted into `pub mod axum` by
`include_server_schema!`).** Per model, `GET/POST /{plural}` and
`GET/PATCH/DELETE /{plural}/{id}`
(`src/axum/model/routes.rs::generate_model_axum_routes`). Handlers are generic
over `Auth: ::cratestack::AuthProvider`, whose single method is
`authenticate(&RequestContext) -> Result<CratestackContext, _>`
(`cratestack-core-0.12.0/src/context.rs:92-99`).

**Route suppression exists at 0.12.0.** `@@internal("action")` (cratestack#743)
omits a verb's `MethodRouter` from the merge, and when every verb on a path is
suppressed the `.route(...)` call is not emitted at all — the path is never
registered, so axum's own 404 applies. The TypeScript generator honours it too:
`tests/internal_suppression.rs` asserts a suppressed verb's client method is
**absent**, not present-and-403. (The `@cratestack/refine` README still says
route suppression "isn't implemented, cratestack#514"; at 0.12.0 that sentence
is stale.)

**Client (`cratestack-client-typescript` 0.12.0,
`cratestack generate-typescript`).** Default output is
`package.json`, `tsconfig.json`, `README.md`,
`src/{index,runtime,models,client,queries}.ts` — model/input/enum types, a
framework-neutral fetch client, projection helpers. Four additive opt-in flags,
each byte-identical-when-off:

- `--tanstack` → `src/react-query.ts`, `@tanstack/react-query ^5.0.0` peer;
- `--swr` → a `src/swr/` subtree;
- `--rtk` → `src/rtk-api.ts`, an RTK Query `createApi` endpoint set with
  `providesTags`/`invalidatesTags` derived from the schema (this is the thing the
  release notes mention). For a **REST** schema it uses `fakeBaseQuery()` and
  every endpoint's `queryFn` calls the same generated REST client methods — no
  second transport implementation (`src/rtk/mod.rs` module doc);
- `--refine` → `src/refine.ts`, exporting
  `cratestackRefineResources(client): ResourceMap` — per model the generated API
  accessor, the `@id` field name, the `@@paged` flag and the `@version` field.
  Peer/dev deps `@cratestack/refine` and `@refinedev/core ^5.0.0`
  (`src/package_deps.rs::peer_dependencies_for`).

**`@cratestack/refine` 0.12.0 is a real, published Refine `DataProvider`** over
that generated client (`createCratestackDataProvider(resources, options)`).
`authProvider` is explicitly out of its scope, which is the right split.

So the maintainer's premise is correct in general: CrateStack 0.12.0 has a
first-class Refine story, and `--refine` + `@cratestack/refine` is a supported,
tested path from a `.cstack` schema to a Refine data provider.

### 3.3 The crux: it cannot serve `payment_intents`, and that is the whole of `/dash/v1`

**This is the finding. Stated plainly, as asked.**

`model PaymentIntent` in `schemas/vpay.cstack` carries **no `@@allow` arm at
all**, and deliberately so. A model with no `@@allow` is deny-by-default: every
generated read renders `FALSE` into its `WHERE` and returns zero rows
(`cratestack-sqlx`'s `push_allow_policy_query`). The schema says why, in the
model itself (`schemas/vpay.cstack:484-506`):

> "NO @@allow ARM, AND THAT IS THE POINT. … **no query on `payment_intents` runs
> through CrateStack** … every read and every write in
> `vpay_db::payment_intents` returns a `PaymentIntentRow`, and `PaymentIntentRow`
> carries `payment_method_types` and `metadata` — two `JSONB` columns this model
> does not declare and must not."

And the reason it must not, from `docs/reference/vpay-db.md` §"One blocker
decides three tables, and it is `jsonb`":

> `Value::from_plain_json` routes every JSON number through `Number::as_i64()`
> with an `as_f64().unwrap_or_default()` fallback (`cratestack-core-0.12.0`
> `value.rs:95-106`), so a `u64` above `i64::MAX` anywhere inside a merchant's
> `metadata` comes back as a float.

`metadata` **is** the merchant's payload. Demoting an integer inside it is
silent data corruption on a surface a merchant reads back. Both escape hatches
were considered and rejected upstream of this plan: a second statement for the
`jsonb` columns is two round trips and a consistency window on the money path;
declaring `metadata Json` is strictly worse for drift.

**Now join that to the wire.** `PaymentIntentObject` — the object `/dash/v1`
serialises, unchanged from `/v1` — has `payment_method_types: Vec<String>` and
`metadata: Map<String, Value>` among its thirteen fields
(`vpay-api/src/model.rs:332-391`). The dashboard **renders both**: the list
table's "Methods" column is `payment_method_types`, and the detail page renders
`metadata`.

So a generated `GET /payment_intents` would emit a model shaped by
`model PaymentIntent`, which by construction omits exactly the two fields the
dashboard needs, and the generated read would in any case return zero rows
until an `@@allow` arm is added that the schema argues must not be added while
the columns cannot be decoded.

**Verdict: generation is foreclosed for this resource today.** Not awkward, not
expensive — foreclosed. It becomes possible only when `cratestack-migrate` /
`cratestack-core` map `jsonb` in both directions with integer fidelity, which is
reserved decision **D3** in the data-layer plan and is the maintainer's own
upstream project, not a vpay change.

Since `payment_intents` is the only resource `/dash/v1` serves, the blocker
forecloses the whole surface, not a corner of it. The same blocker holds
`charges` (`provider_ref_extra`) and `refunds` (`metadata`), which are two of
the four tables the detail page composes.

### 3.4 Four further obstacles, which matter after the blocker lifts

Even granting the `jsonb` fix, these do not go away, and each is worth knowing
before anyone re-opens the question:

1. **The cursor is not expressible.** `/dash/v1`'s cursor is a correlated
   sub-select (`seq < (SELECT seq FROM payment_intents WHERE id = $2 AND
merchant_id = $1)`). No CrateStack delegate expresses it — the same reason
   `CheckoutSessions::list_page` and `Events::list_page` stayed raw SQL
   (`docs/reference/vpay-db.md`). The generated REST list is **offset**-paged:
   `CratestackFetchQuery` is `{ filters, sort, limit, offset }`
   (`@cratestack/refine` 0.12.0 `dist/types.d.ts`), and `@cratestack/refine`
   maps Refine's `{ currentPage, pageSize }` onto `limit`/`offset` directly.
   Adopting the generated list means **changing `/dash/v1` from cursor to offset
   paging** — which is both a shape change (reserved, §8 RD2) and a correctness
   regression on a money list, where rows are inserted at the head continuously
   and offset paging shows duplicates and skips.

2. **`total_count` requires `@@paged`, and nothing in `vpay.cstack` declares
   it.** `@cratestack/refine` is explicit that a non-`@@paged` resource's
   `total` "degrades to how many rows this one response returned". `ListObject`
   has no count today and would gain one only by declaring `@@paged` — another
   shape change, and a `COUNT(*)` per list on a money table.

3. **`PaymentDetail` is four tables.** A generated model route filters columns of
   one. The detail page's intent + charge + refunds + events composite has no
   generated equivalent; it would remain hand-written, or become four client
   round trips.

4. **Merchant scope and the uniform 404 _are_ expressible — this is the
   encouraging half.** `ReadPredicate::FieldEqAuth { column, auth_field }` exists
   (`cratestack-policy-0.12.0/src/read_types.rs`) and is lowered from
   `field == auth().field`
   (`cratestack-macros-0.12.0/src/policy/model/comparison.rs:45-67`), so
   `@@allow("read", merchantId == auth().merchantId)` compiles the tenancy filter
   into the `WHERE` of every generated read. A row excluded by policy is
   indistinguishable from an absent one, and the generated detail handler turns
   `Ok(None)` into `CratestackError::NotFound`
   (`src/axum/model/handlers_crud/get.rs:112`) — which is exactly vpay's uniform
   cross-tenant 404, obtained structurally rather than by discipline.

   Two things are needed to use it and neither exists today:
   `schemas/vpay.cstack` declares **no `auth` block** (`ensure_auth_field`
   requires one), and vpay would have to implement `AuthProvider` to turn the
   dashboard bearer token into a `CratestackContext` carrying `merchantId`. Note
   that today the merchant comes from the **YAML binding**, and the token's
   `vpay_merchant_id` claim is cross-checked against it (`lib.rs:1045-1053`); an
   `AuthProvider` that took the claim as authoritative would quietly drop that
   cross-check.

### 3.5 What I would do, and what it would take

**Do not adopt the generated transport for `/dash/v1` now.** D2 in the
data-layer plan ("Is the transport layer ever adopted? Everything above assumes
never") stands unchanged by this analysis, for `/dash/v1` as much as for `/v1`,
and for a reason that was already measured rather than a preference.

If the maintainer decides otherwise later, the honest cost, in order:

1. upstream: `jsonb` mapped in both directions with integer fidelity (D3);
2. `schemas/vpay.cstack` gains an `auth` block and `PaymentIntent` gains
   `@@allow("read", merchantId == auth().merchantId)` — which moves
   `EXPECTED_DRIFT_CHANGES` and needs the
   `every_action_..._has_an_allow_arm` unit-test pattern copied to the model;
3. `@@internal("create")`, `@@internal("update")`, `@@internal("delete")` on
   every model exposed, so the generated router emits reads only and the boundary
   stays structurally read-only;
4. a vpay `AuthProvider` that preserves the binding cross-check, the scope check
   and the live staff-row re-read — none of which CrateStack does for you;
5. a decision on paging: cursor stays and the generated list is not used, or the
   surface moves to offset and accepts the money-list instability;
6. `PaymentDetail` stays hand-written regardless.

That is not "stop bothering with raw implementation". It is a different, larger
piece of raw implementation with a code generator in the middle of it. Saying so
is the point of this section.

### 3.6 The part of CrateStack that _is_ worth taking now

One narrow win, independent of everything above: **`cratestack
generate-typescript` for the types only.**

`PaymentIntentObject`, `ChargeSummary`, `RefundObject`, `TimelineEvent` and
`IntentStatus` are hand-written twice today — once in Rust
(`vpay-api/src/model.rs`, `dash/payment_intents.rs`) and once in TypeScript
(`frontends/apps/dashboard/src/server/api.ts`). The TypeScript copy can drift
silently. A generated `models.ts` would not fix that by itself — the generated
model follows `.cstack`, not the wire object, and the two differ by exactly the
`jsonb` fields — so this is smaller than it sounds. It is nonetheless the only
piece of the generator that does not require the blocker to lift first, and it
is worth scoping separately rather than bundling into a framework migration.

---

## 4. The data-provider design, against `/dash/v1` as it stands today

This is a design for a **hand-written** Refine data provider over the existing
surface. It is deliberately not `@cratestack/refine`, for the reasons in §3.

```ts
// frontends/apps/dashboard/src/dash/provider.ts
export function dashDataProvider(bffBase: string): DataProvider
```

| Refine method                     | Behaviour                                                                                                                                                                                                                                                                                                                                                                                          |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `getList`                         | `resource === "payment_intents"` only. Maps `filters` → `status` / `created_gte` / `created_lte`, `meta.after`/`meta.before` → `starting_after`/`ending_before`, `pagination.pageSize` → `limit` (default 25). Returns `{ data, total: 0, cursor: { next, prev } }` where `next`/`prev` are derived by the same rule `pagerHrefs` uses today, including the backward-page inversion of `has_more`. |
| `getOne`                          | `GET …/payment_intents/{id}`; a 404 surfaces as a Refine error carrying `statusCode: 404`, and the route maps it to `notFound()`.                                                                                                                                                                                                                                                                  |
| `getMany`                         | **Not implemented.** There is no batch read, and faking one as N round trips on a money surface buys nothing.                                                                                                                                                                                                                                                                                      |
| `create` / `update` / `deleteOne` | **Throw.** `/dash/v1` refuses every non-`GET` with a 403 at the boundary. A provider that "works" here would be a lie that only shows up as a 403 at submit time. Throwing at the provider is the honest shape, and `resources` declares `create: false, edit: false, canDelete: false` so no button ever renders.                                                                                 |
| `getApiUrl`                       | the BFF base path.                                                                                                                                                                                                                                                                                                                                                                                 |
| `custom`                          | not implemented.                                                                                                                                                                                                                                                                                                                                                                                   |

Three rules the implementation must keep, each carried over from code that is
already reviewed:

- **A repeated query param takes the first value**, not the last, not joined
  (`one()` in `payments-query.ts`). A joined `succeeded,canceled` is a 400
  naming a parameter the operator did not type.
- **An unparseable date is dropped, not forwarded.**
- **Both cursors present → `after` wins.** The server would 400; the client
  should not send it.

Error mapping is `refusalFor`'s rule, unchanged: `401` → `authProvider.onError`
returns `{ logout: true }`; everything else, **including 403**, is an outage and
renders `ReadFailure`. This is the one place where a generic "≥400 means signed
out" default would silently reintroduce issue #88 item 2.

---

## 5. Pinned versions, with sources

Every version below was read on **2026-09-11** from the source named, not from
memory.

| Thing                          | Version                                                                                                                                                                                                                                                                     | Source                                                                                                                                                  |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `@refinedev/core`              | **5.0.12** (`dist-tags.latest`, published 2026-04-02)                                                                                                                                                                                                                       | npm registry metadata for `@refinedev/core`                                                                                                             |
| `@refinedev/nextjs-router`     | **7.0.5** (`latest`, published 2026-03-05); peers `@refinedev/core ^5.0.0`, `react`/`react-dom` `^18 \|\| ^19`, `next *`                                                                                                                                                    | npm registry metadata for `@refinedev/nextjs-router/7.0.5`                                                                                              |
| Refine `DataProvider` contract | `getList`/`getOne`/`create`/`update`/`deleteOne`/`getApiUrl` required; `getMany`/`createMany`/`updateMany`/`deleteMany`/`custom` optional                                                                                                                                   | context7 `/refinedev/refine`, `documentation/docs/data/data-provider/index.md` and `documentation/docs/core/interface-references/index.md`              |
| Refine `AuthProvider` contract | `login`/`check`/`logout`/`onError` required; `register`/`forgotPassword`/`updatePassword`/`updateIdentity`/`getPermissions`/`getIdentity` optional                                                                                                                          | context7 `/refinedev/refine`, `documentation/docs/authentication/auth-provider/index.md`                                                                |
| Refine App Router integration  | `"use client"` on the layout rendering `<Refine routerProvider={routerProvider}>`; `@refinedev/nextjs-router`                                                                                                                                                               | context7 `/refinedev/refine`, `documentation/docs/routing/integrations/next-js/index.md` and `documentation/docs/guides-concepts/routing/index.md`      |
| Refine cursor pagination       | `getList` returns `{ data, total: 0, cursor: { next, prev } }`; `useInfiniteList`; `queryOptions.getNextPageParam`                                                                                                                                                          | context7 `/refinedev/refine`, `documentation/docs/data/hooks/use-infinite-list/index.md`                                                                |
| Refine v5 `useTable` shape     | `{ result: { data, total }, tableQuery, currentPage, setCurrentPage, pageSize, setPageSize, pageCount, filters, setFilters, sorters, setSorters, createLinkForSyncWithLocation }`, option `syncWithLocation: true`; v5 renamed `current` → `currentPage` and added `result` | context7 `/refinedev/refine`, `documentation/docs/migration-guide/4x-to-5x.md` and `documentation/tutorial/routing/syncing-state/react-router/index.md` |
| Refine v4→v5 auth renames      | `AuthPovider` → `AuthProvider`, `getUserIdentity` → `getIdentity`, `checkError` → `onError`, `checkAuth` → `check`, `useAuthenticated` → `useIsAuthenticated`                                                                                                               | context7 `/refinedev/refine`, `documentation/docs/migration-guide/auth-provider.md`                                                                     |
| `cratestack-*` crates          | **0.12.0** (twelve crates)                                                                                                                                                                                                                                                  | `Cargo.lock`; the CrateStack block in `Cargo.toml`                                                                                                      |
| `cratestack` CLI               | **0.12.0**                                                                                                                                                                                                                                                                  | `justfile:749`, `cratestack_version := "0.12.0"`                                                                                                        |
| `cratestack-client-typescript` | **0.12.0**; flags `--swr` / `--refine` / `--tanstack` / `--rtk` / `--no-native-cbor`, all additive                                                                                                                                                                          | crate `README.md` and `src/config.rs` (`DEFAULT_TANSTACK = false`, `DEFAULT_RTK = false`, `DEFAULT_NATIVE_CBOR = true`)                                 |
| `@cratestack/refine`           | **0.12.0** (published 2026-09-06); peer `@refinedev/core ^5.0.0`; emitted requirement `>=0.8.0 <0.13.0`                                                                                                                                                                     | npm registry metadata; `src/package_floors.rs:90` (`CRATESTACK_REFINE_FLOOR = "0.8.0"`) + `src/release_line.rs::ceiling`                                |
| `@cratestack/adapter-rtk`      | **0.12.0**; floor `0.8.0`; RPC transport only                                                                                                                                                                                                                               | npm registry; `src/package_floors.rs:151`; `src/rtk/mod.rs` module doc                                                                                  |
| Generated package's own deps   | `decimal.js ^10.6.0` (always); `typescript ^7.0.2` (dev)                                                                                                                                                                                                                    | `src/package_deps.rs::dependencies_for` / `::dev_dependencies_for`                                                                                      |
| Dashboard today                | `next ^15.5.25`, `react`/`react-dom` `^19.0.0`, `typescript ^5.7.0`, `vitest ^4.1.11`, `tailwindcss 4.3.3`, `daisyui 5.7.28`, `@base-ui/react 1.8.0`                                                                                                                        | `frontends/apps/dashboard/package.json`, `frontends/packages/ui/package.json`                                                                           |

**Two version hazards that follow from that table:**

- A generated client package declares `typescript ^7.0.2` as a devDependency
  while the dashboard is on `^5.7.0`. In one pnpm workspace that is two
  TypeScript majors; resolve it deliberately (pin the generated package's
  `typescript`, or leave the generated package out of `pnpm-workspace.yaml`)
  rather than discovering it at install.
- `@refinedev/nextjs-router` is at major **7** while `@refinedev/core` is at
  major **5**. They are versioned independently; `7.0.5` peers on `core ^5.0.0`,
  which is the pair to take. Do not assume matching majors.

---

## 6. Lane plan

Lanes are ordered; each names what lands, what it owns, and **the one check that
is decisive for it** — the thing that must go red if the lane's work is deleted.

### Lane 0 — `@vpay/ui`, on the separate track (not designed here)

The appearance complaint, and the per-component-folder / 200-line restructuring,
belong to the `@vpay/ui` track. **It is a dependency of Lane 3, not a part of
it**, and this plan does not design it. What the lanes below need from it, and
nothing more:

- the primitives the dashboard screens compose from — `Table`, `Select`,
  `Input`, `Field`, `Alert`, `Spinner`, `StatusBadge`, `Stack`, `PageShell`,
  `Link` — available per-component and unchanged in name;
- the `bumblebee` theme and `@vpay/ui/styles.css` unchanged, since
  `src/layout.test.tsx` pins `data-theme="bumblebee"` and
  `theme-contrast.test.ts` re-measures every tone pair.

- Decisive check (owned by that track, restated here so Lane 3 can rely on it):
  `just verify-ui` stays green — no palette colour, no arbitrary colour value,
  no daisyUI-4 class — and `src/a11y.test.tsx`'s eight axe cases stay green.
- Does not touch: anything under `frontends/apps/dashboard/src/server/`.

### Lane 1 — the seam, with no framework

Extract the data access the app already does into an explicit provider-shaped
module (`getList` / `getOne` over `/dash/v1`), still called from Server
Components, still using `readDash`. No Refine dependency yet.

- Owns: a new `src/dash/` module; `src/payments-query.ts` becomes its filter
  mapper.
- Decisive check: a test that asserts the backward-page `has_more` inversion —
  delete the inversion and it must fail. Plus `src/payments-query.test.ts`'s 15
  existing cases stay green unchanged.

### Lane 2 — the BFF, reviewed as a security change

Route handlers under `app/api/dash/…` that authenticate on the httpOnly session
cookie and proxy to `/dash/v1` using the server-read token. **This is a new
browser-reachable surface and it is the lane that needs a real security review**,
not a lane that gets folded into Lane 3.

- Owns: `app/api/dash/**`, plus the origin check applied to it.
- Decisive checks, all three:
  1. a request with no session cookie gets a refusal and **no** upstream call;
  2. a cross-origin request is refused by the same rule `csrf.ts` applies to
     Server Actions — delete the check and it must fail;
  3. the bearer token appears in **no** response body and **no** response header
     — assert on the serialised response, not on a mock.
- Must not land: any handler that forwards a caller-supplied merchant id,
  audience, or scope.

### Lane 3 — Refine core, behind the existing pages

`@refinedev/core` + `@refinedev/nextjs-router`, a `"use client"` layout, the
Lane-1 provider wired as `dataProvider`, the Lane-2 BFF as its transport, and
`authProvider` as the thin wrapper in §2.5. The payments list and detail render
through Refine hooks.

- Owns: a nested `app/(dash)/layout.tsx`, `src/dash/auth-provider.ts`.
- Decisive checks:
  1. `onError` maps `403` to an outage and **not** a sign-out — widen it to
     `>= 400` and a test must fail (issue #88 item 2);
  2. the 80 %-of-TTL re-mint still fires: `gate.test.ts`'s 19 cases stay green
     and the re-mint is still reached on a real render path;
  3. a `404` on the detail route still renders `notFound()`, and the response is
     byte-identical for a foreign id and an absent one — this is the client-side
     half of `dashboard_read_surface.rs:531` and must not be weakened by an error
     mapper that echoes the id differently.
- Explicitly out of scope: `useForm`; any `create`/`edit` route; any change to
  `@vpay/ui`, which is the separate track's.

### Lane 4 — the resource registry replaces `NAV_LINKS`

Refine's `resources` array becomes the single declaration, and
`src/layout.test.tsx`'s nav gate moves onto it.

- Decisive check: add a `resources` entry for a page nobody wrote and the gate
  must fail — the same mutation that fails today against `NAV_LINKS`. Two lists
  that can disagree is the failure this lane exists to prevent.

### Lane 5 (conditional, not scheduled) — revisit CrateStack generation

Opens **only** when upstream maps `jsonb` in both directions (D3). Until then
this lane is closed, and saying so in `docs/status.md` is part of Lane 1.

---

## 7. Risks, each with a mitigation

| #   | Risk                                                                                                                                                                                                         | Mitigation                                                                                                                                                                                                                                               |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R1  | **The BFF becomes a token leak.** A route handler that returns the upstream response verbatim can echo an `authorization` header, or a body that carries one.                                                | Lane 2's decisive check 3 asserts on the serialised response. The BFF whitelists the two upstream paths and re-serialises the body rather than streaming it.                                                                                             |
| R2  | **The BFF forgets the origin check.** Route handlers get nothing for free; `csrf.ts` currently fronts Server Actions only.                                                                                   | Lane 2 decisive check 2, written as a mutation: delete the check, the test fails.                                                                                                                                                                        |
| R3  | **Refine's default error handling turns a 403 into a sign-out.** A rolling vpay restart used to sign everyone out; that is issue #88 item 2 and it was fixed once.                                           | `onError` is `refusalFor` verbatim. Lane 3 decisive check 1.                                                                                                                                                                                             |
| R4  | **`useTable` moves filters out of the URL**, losing bookmarking, reload, and JS-off operation — an explicit documented decision.                                                                             | `syncWithLocation: true`, or keep `payments-filters.tsx` as the GET form and feed Refine from `searchParams`. Assert in a test that a filtered URL, loaded cold, renders the filtered table.                                                             |
| R11 | **`useTable`'s `pageCount` renders as a real page count against a surface that returns no total** (§2.3), giving First/Last/page-number controls that do nothing — a control that looks finished and is not. | Do not use `useTable` for pagination. `useList` with `pagination: { mode: "off" }` plus the existing `Pager`. Decisive check: a test asserting the rendered pager shows only prev/next and the "Newest first"/"End of results" text, never a page count. |
| R5  | **Offset paging arrives by the back door** via `@cratestack/refine` or a Refine default, and a money list starts skipping and duplicating rows under concurrent inserts.                                     | The provider is hand-written (§4) and returns `total: 0` with a cursor. Any change to offset is a reserved decision (§8 RD2), not an implementation detail.                                                                                              |
| R6  | **The cross-tenant 404 stops being byte-identical** because a client-side error mapper adds the id or the resource name to one branch and not the other.                                                     | Lane 3 decisive check 3, mirroring `dashboard_read_surface.rs:531`'s body comparison.                                                                                                                                                                    |
| R7  | **Two TypeScript majors in one workspace** if a generated client package lands (`^7.0.2` vs `^5.7.0`).                                                                                                       | Decide before install, not after: pin the generated package's `typescript`, or keep it out of `pnpm-workspace.yaml`.                                                                                                                                     |
| R8  | **The migration makes the app look more finished than it is.** Refine's scaffolding makes create/edit routes cheap to add, and `/dash/v1` refuses every write with a 403.                                    | `resources` declares `create: false, edit: false, canDelete: false`; the provider's write methods throw. Do not add a write route before the `audit_log` ADR-0008 requires.                                                                              |
| R9  | **`docs/status.md` and the dashboard README go stale** in the same commit that makes them wrong — the README currently states there is no browser-reachable surface.                                         | Every lane updates `docs/status.md` and the README in the same commit, per CLAUDE.md. Lane 2 must update the README's "what this app cannot do" section specifically.                                                                                    |
| R10 | **Bundle and dependency growth** on an app that currently ships no client data library.                                                                                                                      | Measure `next build` output before Lane 3 and after; record both numbers in the PR rather than asserting the change is small.                                                                                                                            |

Three smaller things noticed in passing, recorded so they are not rediscovered:

- `frontends/apps/dashboard/README.md` says "18 files, 136 tests"; it is **21
  files / 178 tests**.
- `@vpay/api-client` and `@vpay/config` are declared dependencies of
  `@vpay/dashboard` and imported by nothing in the app.
- `dash/mod.rs:18-20` says every dash route is "reachable over HTTP and by
  nothing a person can click". Six pages now exist; that comment is stale.

---

## 8. Reserved decisions I am **not** taking

| #       | Decision                                                                                                                                                                                                           | Why it is the maintainer's                                                                                                                                                                        |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **RD1** | **Adopt CrateStack's generated transport for any surface** — D2 in `datalayer-plan.md` §5. This document supplies evidence that it is foreclosed for `/dash/v1` today; it does not change D2's standing.           | D2 is a recorded reserved decision, and the evidence here narrows it rather than settles it.                                                                                                      |
| **RD2** | **Whether `/dash/v1` may change shape at all** — cursor → offset, adding `total_count`, adding `@@paged`. Every generation path in §3.4 needs at least one of these.                                               | The shape is a contract between two things the maintainer owns, and an offset page on a money list is a correctness trade, not a refactor.                                                        |
| **RD3** | **Whether the payments list becomes infinite scroll** (§2.4). A product change, not a plumbing one.                                                                                                                | Product shape is the maintainer's.                                                                                                                                                                |
| **RD4** | **Whether the `@vpay/ui` restructuring and the Refine migration land together or in sequence.** This plan assumes the `@vpay/ui` track lands first and Lane 3 composes from it.                                    | The message was cut off mid-sentence; assuming what it would have coupled is exactly the inference I was told not to make. The `@vpay/ui` shape itself is that track's decision, not this plan's. |
| **RD5** | **Whether a new browser-reachable surface on the dashboard origin is acceptable at all** (Lane 2). If the answer is no, Refine's client-side hooks are off the table and only the SSR/`initialData` shape remains. | It reverses a stated property of the app's security model.                                                                                                                                        |
| **RD6** | **`schemas/vpay.cstack` gains an `auth` block.** It has none today. Adding one is a schema-wide change affecting every model's policy vocabulary and the drift numbers.                                            | Schema ownership, and it moves `EXPECTED_DRIFT_CHANGES`.                                                                                                                                          |
| **RD7** | **Filing the `jsonb` round-trip bug upstream (D3).**                                                                                                                                                               | `datalayer-plan.md` already reserves it; cratestack is the maintainer's own project.                                                                                                              |

---

## 9. What lands where — file map

Nothing below exists yet.

```
frontends/apps/dashboard/
  app/api/dash/payment_intents/route.ts          Lane 2 (BFF list)
  app/api/dash/payment_intents/[id]/route.ts     Lane 2 (BFF detail)
  app/(dash)/layout.tsx                          Lane 3 ("use client", <Refine>)
  src/dash/provider.ts                           Lane 1 → Lane 3 dataProvider
  src/dash/auth-provider.ts                      Lane 3 (wraps requireStaff/signOut)
  src/dash/resources.ts                          Lane 4 (replaces src/nav.tsx's NAV_LINKS)
```

Unchanged and untouched by every lane: `src/server/session.ts`, `oauth.ts`,
`pkce.ts`, `gate.ts`, `csrf.ts`, `cookies.ts`, `api.ts`, `dash-read.ts`,
`actions.ts`. If a lane's diff touches one of these, that is the signal that the
lane has drifted into rewriting the auth stack.

---

## 10. Where this leaves "why bother with raw implementation"

The honest answer, in one paragraph, because it is the question that was asked.

For `/v1` the answer is that its shape is a public contract carried by two SDKs
and is not CrateStack's to choose. For `/dash/v1` the answer is narrower and
more interesting: the surface really is internal, really is CRUD-shaped, and
CrateStack 0.12.0 really does have a first-class Refine path — `--refine`,
`@cratestack/refine`, `@@internal` route suppression, and `FieldEqAuth` policies
that would give the merchant scope and the uniform cross-tenant 404 structurally
rather than by discipline. That is a genuinely attractive target and this
document does not want to talk anyone out of it. It is blocked by one thing and
it is not a preference: the five money models carry no policy arm because
`cratestack-core 0.12.0` demotes JSON integers through `as_f64()`, and
`payment_intents` — the only resource `/dash/v1` serves — carries two `jsonb`
columns the dashboard renders. Until that round-trip is fixed upstream,
"generate it" for this surface would mean either shipping a read that silently
corrupts merchant metadata, or shipping one that returns zero rows. Raw
implementation is not being preferred here; it is what is left.

---

## 11. What I could not determine

- **I ran nothing.** No `pnpm install`, no `next build`, no `cargo build`, no
  `just verify`, no `just ci`, no `cratestack generate-typescript`. Every claim
  is read from a file, a published artefact or a docs page.
- **I did not generate a TypeScript client from `schemas/vpay.cstack`.** §3.2's
  description of `--refine`/`--rtk` output is read from
  `cratestack-client-typescript` 0.12.0's sources, templates and README, not
  from a rendered package. The exact `src/refine.ts` vpay's schema would produce
  — in particular the resource accessor names the pluraliser derives — is
  unverified.
- **`TRANSPORT_STYLE` for `schemas/vpay.cstack` is inferred, not observed.** The
  file declares no `transport` line and `TransportStyle`'s `#[default]` is
  `Rest` (`cratestack-core-0.12.0/src/schema.rs:53-57`), so `--refine` would emit
  the `ResourceMap` (REST) form. I did not run `cratestack check` to confirm.
- **I did not measure bundle size** for `@refinedev/core` +
  `@refinedev/nextjs-router` + TanStack Query in this app. R10 asks for the
  measurement rather than asserting a number.
- **I did not verify that `@refinedev/nextjs-router` 7.0.5 works with
  `next ^15.5.25`.** Its peer range is `next: "*"`, which is a declaration, not
  evidence.
- **I did not read `@cratestack/refine`'s compiled provider implementation**,
  only its `.d.ts` files and README. The offset-pagination claim rests on
  `CratestackFetchQuery`'s type (`{ filters, sort, limit, offset }`) and the
  README's "refine's `{ current, pageSize }` maps to `limit`/`offset` directly",
  both unambiguous, but I did not trace the JavaScript.
- **Whether Lane 2's BFF is acceptable at all** is RD5, and the rest of Lane 3
  depends on it. If it is refused, §6 needs rewriting from Lane 2 down.
- **`docs/flows/dashboard.md`'s Status section** would need updating by every
  lane; I did not read it or draft the updates.
- **The restructured `@vpay/ui` does not exist yet at the commit I read.** At
  `6b1b7d8` the package is flat (`src/components/<name>.tsx` with co-located
  `.test.tsx`/`.stories.tsx`, 17 modules, one `src/index.ts` barrel). §2.1 and
  Lane 0 **assume** the per-component-folder / 200-line shape the separate track
  will produce, as instructed. Every component name this plan composes from is
  taken from today's `src/index.ts` exports; if that track renames or splits any
  of them, §2.3's tables need re-reading against the new names.
- **I did not verify `useList`'s exact v5 return shape**, only `useTable`'s and
  `useInfiniteList`'s. §2.3 recommends `useList` with `pagination: { mode: "off" }`
  on the strength of the `DataProvider` contract and `useTable`'s documented
  options; the precise destructuring an implementer writes should be checked
  against the docs at implementation time, not copied from this plan.
- **I did not check whether `@@paged` would change the generated `ListObject`
  shape in a way `/v1`'s SDKs would notice.** `/dash/v1` and `/v1` share
  `ListObject<T>`; §3.4 point 2 assumes adding a count to one would affect both,
  and I did not verify that the type is not already parameterised in a way that
  avoids it.
