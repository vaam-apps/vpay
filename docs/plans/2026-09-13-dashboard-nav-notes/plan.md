# Dashboard navigation — the plan, and the four decisions behind it

_Written 2026-09-13. Base: `7730284c` (`origin/master`)._

## The problem, stated accurately

The dashboard's rail has **one** entry. That is not a frontend omission:
`frontends/apps/dashboard/src/dash/resources.ts` derives `NAV_ENTRIES` from
`DASH_RESOURCES`, and `src/layout.test.tsx` fails the build if any entry has
no page — so the rail cannot grow past what the backend serves without
either building the backend or shipping dead links.

What the backend serves is two routes.
`backends/crates/vpay-api/src/dash/mod.rs`'s `DASH_ROUTES` is
`GET /payment_intents` and `GET /payment_intents/{id}`, and its own comment
says the other slices are _"deliberately absent rather than
present-and-empty"_.

Meanwhile `schemas/vpay.cstack` declares eighteen models and one
`procedure searchPaymentIntents`, whose body in
`backends/crates/vpay-db/src/schema/search_payment_intents.rs` is complete,
container-tested, and **reachable from nothing**: its module doc says _"No
transport serves it."_ Nothing in the workspace mounts a CrateStack axum
router or RPC dispatcher, and `vpay_db::persistence::system_context` is the
only `CratestackContext` vpay mints — a system context carries no tenant, so
the procedure's own policy would refuse it.

So the schema already describes far more than the transport exposes. That
gap is the lever.

## The four decisions (maintainer, 2026-09-13)

1. **Mount CrateStack's transport.** Mount the generated router and mint a
   tenant-carrying context from the staff token, so a new read slice is one
   `procedure search*` declaration plus a body — not a bespoke axum route,
   handler, error mapping and test set per slice.
2. **A cross-tenant admin role.** A real admin who may read across
   merchants. This is a change to `require_dashboard_token`'s
   registration-based authorisation and therefore **gets its own ADR and its
   own PR, landed before any UI depends on it.**
3. **Reads only.** `/dash/v1` keeps answering `403` to every non-`GET`.
   ADR-0008 gates writes behind an `audit_log` that does not exist; nothing
   here builds one, and no resource declares `create` or `edit`.
4. **Stacked PRs, one per lane.**

Nav slices: refunds, events / webhook deliveries, customers, settings
(read-only). Drawer: overflow menu + theme switcher + account block.

## The lanes

| PR  | Lane                                         | Depends on | Surface       |
| --- | -------------------------------------------- | ---------- | ------------- |
| A   | Shell: drawer, theme, account, rail          | —          | frontend only |
| B   | ADR + cross-tenant admin role                | —          | backend, auth |
| C   | CrateStack transport + tenant context        | B          | backend       |
| D   | Slices: refunds, events, customers, settings | C          | backend + BFF |
| E   | Nav IA, screens, screenshots                 | A, D       | frontend      |

Lane A and lane B are independent and start together. Nothing in lane A
depends on a route existing; it is the chrome around whatever the rail
contains.

## Lane A — the shell

What the maintainer asked for, mapped onto `@vaam-apps/ui`'s actual API
(`.agents/skills/vaam-ui/references/primitives-layout.md`):

- **Sticky left rail, floating horizontally on small screens.** This is
  `SideNav`'s default `smallScreen="floating"` — already what
  `app-shell.tsx` passes, and its comment records that `"off-canvas"` was
  measured and broke the layout (`main` computed to 0px wide at 375px).
  Keep it. The lane's job is to keep the content column's padding correct,
  since both rails are `position: fixed` and portalled and **cannot reserve
  their own space**.
- **Theme switcher in a bottom-sheet drawer.** Today `accountSlot` is a
  bare `<ThemeSwitcher />`, and `SideNav` drops `accountSlot` entirely
  below `lg` — so on a phone there is no theme control at all. It moves
  into the drawer.
- **Most menus in the drawer.** The rail keeps the top destinations; a
  "More" control opens the drawer holding the rest of the tree.
- **Account block.** `SignedInBar`'s email + merchant id + sign-out move
  into the drawer as a proper account block. Sign-out stays a `<form>`
  POST — a sign-out reachable by `GET` is what a prefetcher or an
  `<img src>` fires (issue #88 item 4).

The constraint that outranks all of the above: **the identity must stay
reachable at the e2e viewport.** `dashboard.cy.ts` asserts
`cy.contains(staffEmail()).should("be.visible")` and eight tests follow it
in a `testIsolation: false` sequence. If the account block is only in a
closed drawer, that assertion fails and takes the eight with it.

## Lane B — the admin role

Its own ADR (next number: **0018**). What it has to decide, not assume:

- what an admin **is** — a `StaffMember` flag, a registration property, or
  a scope on the token;
- how `require_dashboard_token` tells one from a tenant-bound staff session,
  given that today it authorises **by client registration and by nothing in
  any token**;
- what a cross-tenant read answers when no tenant is named, and that it is
  still reads-only;
- that the uniform-404 property survives — `/dash/v1` answers `404`
  identically for "not yours" and "does not exist", and an admin role must
  not turn that into an oracle for non-admins.

## Lane C — the transport

Mount the generated router and mint the context. `searchPaymentIntents` is
the proving case: it already has a body and container tests, so the lane is
provably done when that procedure answers over HTTP with the same rows its
tests assert, refuses an unauthenticated caller, and refuses a tenant
mismatch. No new procedure is declared in this lane.

## Lane D — the slices

One `procedure search*` per slice, each with a hand-rolled `type` filter and
summary. Three constraints, all measured and all in
`docs/reference/vpay-db/cratestack.md`:

- **`search` prefix is mandatory.** Every model generates
  `handle_{list,create}_<plural>` and `handle_{get,update,delete}_<singular>`
  whether routed or not, so `listRefunds` collides and fails to compile.
- **A generated model read is not an option** for these tables. `Refund`,
  `Customer` and `PaymentIntent` carry no `@@allow("read", …)` arm, and a
  model with no arm is deny-by-default: the generated read renders `FALSE`
  and answers zero rows, forever. The procedure body is ours; that is the
  point of a procedure.
- **Never publish a filter over every scalar.** `FindMany<PaymentIntent>`
  was rejected because it would expose `startsWith`/`lt`/`gt` on
  `client_secret_suffix` — a prefix oracle on a live payer credential.
  Name the fields.

Each slice is container-tested against real Postgres before it is claimed.

## Lane E — the IA

`DASH_RESOURCES` grows to the slices that now have routes; the rail and the
drawer render from it, unchanged in mechanism. Screenshots at 375px, 1000px
and 1440px — the three bands `SideNav` actually has.

## What this plan does not do

- No writes, anywhere. No `audit_log`.
- No fake rows, no empty-list route for a slice nobody built, no nav entry
  without a page. If a slice does not land, its nav entry does not either.
- No change to `/v1`. The merchant API is untouched.
