# exp23 — sabotage review of dashboard slice 1

Branch `claude/exp23-dashboard-slice1`, reviewed at `1521e5d` on 2026-09-06.
The implementer's own account is [opus.md](opus.md); this file records what a
second pass measured, what it found, and what it changed.

The headline: **the tenancy boundary the slice set out to build is real and
the mutations prove it.** What the mutations also proved is that two of the
three reads the detail route assembles could return an empty list forever
without a single test noticing — which is the one shape `AGENTS.md` rule 2
names by hand.

## `just ci` as delivered, recipe by recipe

Run end to end on `1521e5d` before any change, `CARGO_BUILD_JOBS=4`, Node
22.23.2 (`.nvmrc`), `pnpm install --frozen-lockfile`, rootless Docker.

| recipe | exit | note |
|---|---|---|
| `fmt-check` | 0 | |
| `clippy` | 0 | |
| `verify` | 0 | ten gates; `verify-docs` advisory. No new `#[allow]`, no ```` ```ignore ```` fence |
| `test-rust` | 0 | **1418 run, 1418 passed, 0 skipped, 0 ignored** (1216 s) |
| `test-doc` | 0 | |
| `verify-ignored` | 0 | 0 ignored (expected 0), **44 binaries (expected 44)**, 1418 total (floor 1080) |
| `lint-web` | 0 | |
| `test-web` | 0 | |
| `deny` | 0 | |

The counts the branch claims are the counts that were measured.

## Mutations

`+` = caught, `-` = **not** caught. Each was applied to a clean tree, built,
run, and reverted; the helper asserts the branch name and refuses a dirty
tree.

| # | mutation | result |
|---|---|---|
| M1 | `MerchantScope::for_dashboard(claims.client_id)` instead of `binding.merchant_id` | + `the_dashboard_lists_only_the_merchant_it_is_bound_to` |
| M2 | delete the `claims.client_id != binding.client_id` arm | + `a_dashboard_token_for_an_unregistered_client_is_refused` |
| M3 | `Surface::Dashboard.audience()` returns `MERCHANT_AUDIENCE` | + 8 of 10 fail |
| M4 | delete the `validate_dashboard_binding` call | + `a_dashboard_client_bound_to_an_unregistered_merchant_is_rejected` |
| M5 | delete the merchant-claims-dashboard-audience check | + `a_merchant_client_that_lists_the_dashboard_audience_is_rejected` |
| M6 | mount the `/dash/v1` nest unconditionally | **-** nothing fails (known, documented by the implementer: the middleware's `None` guard answers the same 404) |
| M7 | `dash::required_scope` returns the scope for **every** method | **-** nothing fails — finding F3 |
| M8 | `Events::list_for_objects` **and** `Refunds::list_for_intent` return `Ok(vec![])` | **-** nothing fails — finding F1 |
| M10 | drop `AND merchant_id = $1` from **both** list cursor subqueries | **-** 35 tests across `dashboard_read_surface` + `payment_intents` all pass — finding F2 |

M8 is the important one. Every assertion the suite makes about the detail
route's `refunds` and `events` sections is `== []`, over fixtures that have
neither, so the two repository methods that exist to fill them are covered
only in the case where returning nothing is correct.

## Findings

Severity: gate-hole / correctness / rule-break / misleading-claim / nit.

### F1 — correctness. The timeline and the refunds are never proven to render

`Events::list_for_objects` and `Refunds::list_for_intent` are new, have
exactly one caller each (`dash::payment_intents::retrieve`), and can both
`return Ok(Vec::new())` unconditionally with the whole suite green (M8).

`list_for_objects`' `merchant_id = $1` predicate is load-bearing and also
untested: `events.object_id` is a plain `TEXT NOT NULL` with **no foreign
key** (migration `0018`) that points into three different tables depending on
`type`, so it is the only thing that keeps another tenant's event row out of
this tenant's timeline.

`list_for_intent`'s `p.merchant_id = $1` is, by contrast, genuinely redundant
— `refunds.payment_intent_id` is a real FK onto `payment_intents(id)`, and
the intent was already fetched tenant-scoped — so it is defence in depth and
no mutation of it can be caught. That is stated rather than tested.

**Fixed** by `the_detail_read_renders_the_timeline_and_the_refunds_it_has`.

### F2 — correctness. The list cursor's tenant predicate is untested

Removing `AND merchant_id = $1` from both cursor subqueries in
`list_page_filtered` leaves every test green (M10). With it removed, a cursor
naming **another tenant's** intent id resolves to that row's `seq` instead of
`NULL`, so the page comes back populated — which turns the list into an
existence oracle for other tenants' ids and breaks, through the cursor, the
exact property `another_merchants_intent_is_indistinguishable_from_one_that_never_existed`
pins for `retrieve`.

The predicate itself is **pre-existing and correct** — byte-identical at
`3694e34` — and no `/v1` test covered it either. It is in scope here because
this branch put a second surface on that statement.

**Fixed** by `a_cursor_naming_another_merchants_intent_answers_an_empty_page`.

### F3 — rule-break. "No method but GET/HEAD reaches a handler" is claimed five times and tested nowhere

`dash/mod.rs`, `require_dashboard_token`'s doc, `docs/flows/dashboard.md`,
`docs/reference/vpay-api.md` and `docs/status.md` all state that a non-read
method is refused **before the router matches**, and that this is what makes
read-only "structural rather than a promise". Making `required_scope` answer
the scope for every method changes nothing observable (M7).

**Fixed** by `a_write_method_is_refused_by_the_boundary_not_by_the_route_table`.

### F4 — misleading-claim. The `405` that is a `403`

`dash::required_scope`'s doc: "The caller (`require_dashboard_token`) answers
`405` for it". It answers `403`, and `require_dashboard_token`'s own doc
argues at length for `403` over `405`. One commit, two answers.

### F5 — misleading-claim. The charge read is not tenant-scoped in its own right

`retrieve`'s doc says the charge, the refunds and the events are "each ...
also tenant-scoped in its own right", and warns that "the first read already
checked" is "precisely the reasoning that stops being true when someone
reorders the function". `Charges::get_for_intent` takes no `merchant_id`;
its own doc says it is deliberately not merchant-scoped and that `charges`
has no `merchant_id` column to filter on. The charge read relies on exactly
the reasoning the paragraph warns against — safely, because the intent id it
is given came from a scoped read, but a reader is told the opposite.

### F6 — misleading-claim. "The dashboard half is unmounted", still said in three places

The same three now-false clauses — `AuthenticatedDashboard` exists, no
`/dash/v1/*` route does, nothing constructs a `DashboardJwtValidator` —
survive in `resource_auth.rs`'s module header, in
`docs/reference/vpay-api.md`'s § resource-server JWT validation **Status**,
and in `docs/status.md`'s "Resource-server JWT validation" row, which now
contradicts the "Dashboard auth" row three rows above it that the same commit
amended.

### F7 — misleading-claim, and a maintainer decision. The `client_id` check cannot survive the login it is waiting for

`require_dashboard_token` compares `claims.client_id` — which is the token's
`sub` (`ResourceClaims`' `From<RawClaims>`) — against
`DashboardBinding::client_id`, and three documents call it one of the four
checks that make the boundary hold.

Under `client_credentials` that is right: `TokenManager::issue_client_token`
sets `sub: client_id.to_string()`. Under the **authorization-code** grant the
dashboard is blocked on, `default_handle_authorization_code` calls
`issue_user_token_with_extra(auth_code.identity, …, Some(client_id))` —
`sub` is `identity.external_id`, the **staff member**, and `client_id` is the
*audience*. So the check as written refuses every token a real dashboard
login would issue.

Nothing else in this branch makes ADR-0017 harder; centralising
`DASHBOARD_AUDIENCE` in `vpay-config` makes the audience change easier, and
`Surface::audience()` returning `&'static str` is unchanged from the base
commit. This check is the one thing that will have to move, and **which claim
identifies the dashboard credential once a human is in the loop is a
maintainer decision** (`azp`? a `client_id` claim? the audience itself?), not
something a review picks. Documented, not changed.

### F8 — nit. Two boot-refusal messages carry a ten-space gap

`ConfigError::DashboardUnknownMerchant` and
`ConfigError::MerchantClaimsDashboardAudience` both render `…which no
          merchant_clients entry…` / `…the dashboard          surface's
audience…`. These are the sentences an operator reads when the process
refuses to start.

### F9 — nit. The sandbox overlay's comment does not mention the field that picks the tenant

`config/application-sandbox.yml` says figment's dict merge "leaves client_id
and scope untouched from the base file". It now also leaves `merchant_id`,
which is the field deciding whose payments the dashboard reads.

### F10 — nit. The list is not asserted free of `client_secret`

`the_detail_read_carries_the_charge_and_never_the_client_secret` asserts over
the **serialised** body precisely because swapping `PaymentIntentObject` for
`PaymentIntentWithSecret` is a one-word change that still compiles. The list
handler renders the same object and has no such assertion.

## Checked and held — not findings

- **No test-only issuer leaks into production.** Every token the suite
  presents is minted through `LoadedSigningKey::token_manager()` — a `pub`
  API since before this branch, used the same way by seven other integration
  suites — i.e. the shipping mint path. The one hand-assembled token (the
  expired one) uses `jsonwebtoken`, added under `[dev-dependencies]` of
  `vpay-tests-integration`, a `publish = false` crate whose `[dependencies]`
  section is empty. `cargo tree -p vpay-server -e normal` reaches no test
  crate; `verify-no-mocks` passes.
- **A foreign cursor leaks nothing today.** With the predicate present, an
  id belonging to another tenant resolves to `NULL`, `seq < NULL` is `NULL`,
  and the page is empty — no rows and no ids. (F2 is that nothing pins it.)
- **The limit is bounded**: `/dash/v1` shares `v1::paging::parse_limit`,
  `MAX_LIMIT = 100`, capped rather than refused.
- **The uniform 404/401 set holds**: foreign detail id byte-for-byte equal to
  a never-written id; no `dashboard_client` → 404 with and without a token;
  expired → 401; merchant audience → 401; no token → 401 on every route in
  `DASH_ROUTES`.
- **`payer_ref_masked` is honest.** It is rendered from the column and never
  derived from the unmasked `payer_ref`; the always-`NULL` fact is stated in
  the struct's own doc, in `docs/flows/dashboard.md`, in
  `docs/reference/vpay-api.md`, in `docs/status.md`, and the test fixture
  writes `None` with a comment saying why.
- **The SQL audit is right**: 39 `AssertSqlSafe` sites, every interpolation a
  crate `const`, every caller value bound, and the events read binds a whole
  `&[String]` to `= ANY($2)` rather than generating an `IN (…)` list.
- **`/dash/v1` rather than `/dashboard/v1`** is correct: the brief was wrong
  and two accepted ADRs say `/dash/v1`.

## Not checked

- The frontend. `frontends/apps/dashboard` is byte-identical to the base
  commit, so there was nothing to review; `test-web` and `lint-web` pass.
- `just test-e2e` (Cypress). Out of `just ci`, and the branch changed no
  frontend file or compose service.
- Whether any of this is reachable in the compose stack. It is not: no grant
  can mint a token for `/dash/v1`.
