# ADR-0018: A cross-tenant admin role for `/dash/v1`

- **Status:** Accepted
- **Date:** 2026-09-13
- **Deciders:** vpay maintainers (delegate, 2026-09-13)
- **Extends:** [ADR-0017](0017-staff-authentication.md), which this document
  does not revisit except where named. Every vocabulary word below —
  `staff_members`, the re-read on every request, `DashboardBinding`,
  `MerchantScope`, the uniform `404` — is ADR-0017's.

## Context

[docs/plans/2026-09-13-dashboard-nav-notes/plan.md](../plans/2026-09-13-dashboard-nav-notes/plan.md)
("Lane B") asks for a cross-tenant admin role: a staff member who may read
across merchants, not only the one merchant their dashboard client is
registered to. Reads only — `/dash/v1` still answers `403` to every
non-`GET`, ADR-0008's write boundary is gated behind an `audit_log` this
slice does not build, and nothing here changes that.

### What "one merchant" means today, precisely

`vpay_config::Config` registers **exactly one** `dashboard_client`
(`Option<DashboardClient>`, never a list — `config.rs`'s own comment: "unlike
a merchant client, there is exactly one of these ever"), bound to one
`merchant_id`. A staff row belongs to exactly one merchant too
(ADR-0017 decision 1), and `staff::oauth::authorize` refuses to mint a code
unless the staff member's own `merchant_id` equals the registration's — so
every staff member of a given deployment shares that one merchant. `/v1`,
meanwhile, may serve **several** merchants from one deployment
(`config.merchant_clients: Vec<MerchantClient>`) — the existing
`dashboard_read_surface.rs` test harness already configures two
(`acme-cameroon-tenant`, `beta-douala-tenant`) to prove the tenancy filter has
something to fail against.

So "cross-tenant" cannot mean "a staff member registered against several
dashboard clients" — there is only ever one per deployment. It means: this
deployment's `/v1` may already serve merchants the dashboard's own
registration is not bound to, and today nothing on `/dash/v1` can ever read
one of them, for anybody, at all. The admin role is the first thing that can.

### The boundary today, precisely

`vpay_api::require_dashboard_token` authorises a `/dash/v1` request by three
things and reads nothing from a database until the third:

1. the token validates against vpay's own JWKS, audience = the dashboard
   client's own `client_id`;
2. its `vpay_merchant_id` claim equals `DashboardBinding::merchant_id` —
   which tenant, config-bound, never resolved from anything the token says;
3. it carries the registration's scope.

Only then is `staff_members` re-read, by the token's `sub`, to catch a
disabled account or one moved to another merchant since the token was
minted. What that re-read answers today is binary: may this person read at
all. It has no third state. Every one of these four checks is **by client
registration and by nothing in any token** — the problem statement's own
words — and none of it has ever asked "which merchant" beyond the one the
registration names.

## Decision

### 1. What an admin is: a column on `staff_members`, read on the same re-read

`staff_members.is_admin BOOLEAN NOT NULL`, no default (migration `0043`, no
`@default` in `schemas/vpay.cstack`'s `model StaffMember` — the rule
migration `0035` established for every column on this table). Read by
`require_dashboard_token` in the same statement that already answers "is this
account active, and is it still this merchant's" — no second query, no second
round trip.

**Three shapes were on the table, and here is what each would have cost.**

**A. A property of the token (a scope, or an `is_admin` claim).** Rejected
for the same reason ADR-0017 rejected reading tenancy from the token at all:
a claim, once minted, is true for the rest of the token's TTL regardless of
what changes afterward. ADR-0017's whole "disabling a staff member takes
effect at once" property — the exp24 review's finding F1, the reason the
staff row is re-read on every request rather than trusted from the token —
would have a hole a claim-based admin flag reopens on purpose: revoking
`is_admin` from a compromised or mis-granted account would leave that
account able to read every merchant this deployment serves for up to fifteen
more minutes (`staff_auth.access_token_ttl_seconds`, default 900). A
mis-granted **write** scope would at least be caught by ADR-0008's write
boundary refusing every method regardless of scope; a mis-granted **read**
scope for every tenant has no such backstop. The row is checked on every
request precisely because a token cannot be.

There is a second reason, sharper than the first: nothing today mints a
token with any claim beyond audience, subject, scope and the one merchant
claim (`staff::oauth::token`). Adding an `is_admin` claim would mean the
authorization-code grant itself has to look up the staff row to decide what
to stamp — the exact database read this design keeps _out_ of the token
mint, on purpose, because a token that is wrong about who is an admin cannot
be corrected without re-minting it, and a row that is wrong about it is
corrected the moment an operator runs a command.

**B. A property of the dashboard client's registration
(`DashboardClient.admin: bool`, or a second `dashboard_client` for admins).**
Rejected because it is answering the wrong question. A registration property
would make _every_ staff member of an "admin dashboard client" an admin — an
all-or-nothing switch per client, when the actual requirement is per
_person_: one operator with cross-tenant duties beside colleagues who have
none. It would also multiply the one place `DashboardBinding` is already
config-bound and boot-checked (`vpay_config::ConfigError::DashboardUnknownMerchant`)
into a second registration nobody asked for, for a question — "may _this_
person read another tenant" — that a registration cannot answer per person at
all. And it inherits the token problem from option A a second way: a
registration is read once at boot into `ResourceConfig`, not on every
request, so flipping it would need a restart to take effect — the opposite
of "disabling a staff member takes effect at once."

**C. A column on `staff_members` (chosen).** Costs one column, one migration,
one schema edit, and nothing else: `require_dashboard_token` already re-reads
this exact row on every request for `status` and `merchant_id`, so a third
fact costs no new query, no new round trip, and inherits the "takes effect
on the next request" property for free — the same guarantee ADR-0017 built
for disabling an account and for moving one between merchants. Revoking
`is_admin` closes the door as fast as disabling the account entirely.

**Whether `staff_members` being one of the three CrateStack-only tables is a
reason to prefer this or to avoid it: a reason to prefer it, and only that.**
Every one of `Staff`'s six repository methods already runs through
CrateStack (ADR-0017's own account, `docs/reference/vpay-db/cratestack.md`),
so adding a column here means one more field in an existing
`CreateStaffMemberInput` literal and one more `row_from_model` line — no new
query shape, no new builder, no new place a raw `sqlx::query!` could drift
from the model. The one real cost CrateStack imposes anywhere on this table
is `@@allow` completeness (a missing arm on `update` silently fails the TOTP
replay guard, per ADR-0017's own account) — and this change touches no
`@@allow` arm at all: `is_admin` is read through the same `read` policy that
already serves `find` and `find_by_email`, and written through the same
`create` policy `staff add` already exercises. There is no new drift risk
either: a `BOOLEAN` with no `@default` and no hand-named `CHECK` is the shape
`password_change_required` already proved drift-neutral on this exact table.

### 2. How the boundary tells an admin from a tenant-bound staff session

Read straight off `StaffRow::is_admin`, at the same point in
`require_dashboard_token` that already reads `status` and `merchant_id` — see
Decision 1. A non-admin session's tenant is `DashboardBinding::merchant_id`,
unconditionally, exactly as before this ADR: nothing on the wire is even
consulted. An admin session's tenant is resolved by
[`DashboardTenancy`](../../backends/crates/vpay-api/src/dash/mod.rs) — see
"The seam" below — from an optional `?merchant_id=` query parameter, parsed
**only when `staff.is_admin` is true**. A non-admin's copy of that parameter,
well-formed or not, is never parsed at all, which is what makes "nothing a
non-admin caller writes can move the tenant" a property of the code path
taken rather than a property of what the parameter happens to say.

### 3. What a cross-tenant read answers when no tenant is named: the bound one, same as anyone else

Three shapes were possible, and the pagination consequence is what decides
among them.

**Merge every merchant's rows into one page.** Rejected. Every repository
method this surface calls —
`PaymentIntents::list_page_filtered`, `get_for_merchant`,
`Refunds::list_for_intent`, `Events::list_for_objects` — takes exactly one
`merchant_id` and orders its cursor within that one tenant
(`payment_intents` is indexed on `(merchant_id, seq DESC)`, per
`dash::payment_intents`'s own doc). A merged view needs a cursor that is
stable across an arbitrary, deployment-specific set of tenants with no
shared ordering key — a new pagination design this ADR has no mandate to
invent, and one the plan itself does not ask for: "a staff member who may
read across merchants" is satisfied by letting an admin choose which one,
not by flattening all of them into one firehose nobody asked to see merged.

**Refuse with `400` until one is named.** Rejected too, on the operator
argument the problem statement itself makes: "an operator staring at an
empty list needs to tell 'this merchant has no payments' from 'I am looking
at the wrong merchant'." A `400` wall on every admin request with no
override would make that distinction _harder_ for the common case — an admin
who has not yet reached for the picker should see something, the same
"empty list versus wrong merchant" ambiguity every other staff member
already has to read from the merchant id on screen, not a refusal that
regresses the ordinary view.

**Default to the bound tenant — the decision.** No override, or an override
naming the same tenant the registration is already bound to
(`DashboardTenancy::Bound`), reads exactly what a non-admin reads. Naming a
_different_ one this deployment serves (`DashboardTenancy::ChosenByAdmin`) is
the one case that is actually cross-tenant, and it is opt-in, one tenant at a
time, through the same single-tenant repository calls every other request
already uses — no new query shape, no merged cursor, nothing to invent.

**The operator consequence:** an admin's default view is indistinguishable
from a non-admin's — same merchant id on screen, same empty-list ambiguity,
same everything — until they deliberately pick another tenant, at which
point the screen must say so unambiguously (a UI concern for the plan's Lane
E, not this one, but the API gives it `DashboardTenancy::is_cross_tenant()`
to render from). **The pagination consequence:** there is none, because
nothing about pagination changed — every page is still one tenant's cursor,
exactly as `/dash/v1` has always paginated.

This is revisitable. If a future ADR decides operators want a genuine
merged, multi-tenant overview, the plan's Lane C is where it would be built —
CrateStack's own generated pagination is uniform across a `find_many` with no
tenant predicate in its policy, which is a materially different starting
point than hand-rolled per-tenant SQL. Nothing here forecloses that; it
declines to build it now, for a request nobody has made.

### 4. The uniform-404 property survives, for a non-admin absolutely and for an admin by construction

`/dash/v1` answers `404` identically for "this is not yours" and "this does
not exist" so that a non-admin cannot use it as an existence oracle for
another merchant's ids. Two guarantees keep that true here:

- **A non-admin's boundary is byte-for-byte what it was before this ADR.**
  `?merchant_id=` is not parsed at all unless the re-read staff row already
  says `is_admin`, so there is no code path by which a non-admin's request
  can change which tenant a query filters by — the parameter is inert for
  everyone it must stay inert for. `a_non_admin_cannot_move_the_tenant_with_the_query_parameter`
  is the container-backed proof, and it asserts the two 404 bodies are
  byte-identical modulo the id, exactly as the pre-existing
  `another_merchants_intent_is_indistinguishable_from_one_that_never_existed`
  does for the non-admin case this ADR does not touch.
- **An admin's read is always scoped to exactly one tenant, never two at
  once**, because Decision 3 refused the merged view that would have made
  "which tenant is this 404 about" ambiguous. An admin who chooses tenant A
  and names an id from tenant B gets the same `404` a nonexistent id would —
  not a _different_ answer, because the query is still a single-tenant,
  single-predicate read; the only thing that moved is which tenant that one
  predicate names, and an admin choosing that value explicitly, on a
  request they control, is not an oracle over anything they did not already
  choose to look at.

An admin **does** gain the ability to enumerate whether a _merchant_ exists
(Decision 3's `is_known_merchant` check answers `400` for a merchant this
deployment does not serve, `200` with a possibly-empty page for one that
does). That is deliberately not extended to a non-admin, and it is a much
smaller fact than "does payment intent X exist": which merchants a
deployment serves is operational information already visible in
`config/application.yml` to anyone who administers it, not a payer-facing
credential's existence oracle.

### 5. The blast radius if the flag is wrong, and what makes that detectable

**What it exposes.** A `staff_members` row with `is_admin = true` that
should not be lets that one person read every payment intent, charge,
refund and event this deployment's `/v1` serves, across every registered
merchant — not just the one their session is bound to. It grants **no**
write capability whatsoever: ADR-0008's write boundary is a property of
`dash::required_scope` and is checked before the staff row is read at all,
so a mis-set flag cannot reach a write path that does not exist to reach.

**What makes it detectable, in three independent ways, none of which needs a
new table:**

1. **The flag's current state is a plain, queryable column.** Unlike a claim
   sealed inside already-issued tokens, `SELECT id, email, merchant_id,
is_admin FROM staff_members WHERE is_admin` enumerates the entire blast
   radius, at any moment, with no reconstruction from logs.
2. **Every cross-tenant read is logged as it happens**, structured, at
   `info`, naming the subject, the home merchant and the merchant actually
   read — the one line `require_dashboard_token` emits from
   `DashboardTenancy::ChosenByAdmin`'s branch. A mis-set flag that is never
   exercised cross-tenant produces no such lines; one that is, is visible in
   the same place every other refusal and grant on this surface already
   logs from.
3. **Containing it is the same one break-glass switch ADR-0017 already
   built.** `staff_members.status = 'disabled'` refuses the row on its very
   next request, `is_admin` included — there is no separate "revoke admin"
   procedure to remember, because the row that grants the capability is the
   row the existing kill switch already reads.

**What creates the exposure in the first place is a plain CLI flag with no
additional confirmation** — `staff add --admin` — symmetric with every other
`staff add` argument, including `--merchant`, which already has no second
confirmation step either. `staff add`'s own log line now names `is_admin`
explicitly (`created a staff member; is_admin=…`), so the creation itself is
part of the same audit trail as the row, not a fact only a row scan can
recover.

## The seam Lane C consumes

[`vpay_api::dash::DashboardTenancy`](../../backends/crates/vpay-api/src/dash/mod.rs)
is a two-variant enum — `Bound(String)` and `ChosenByAdmin(String)` — with a
`merchant_id()` accessor and an `is_cross_tenant()` predicate, inserted into
the request's extensions by `require_dashboard_token` alongside the
`MerchantScope` every existing `/dash/v1` handler already reads. It is a real
type rather than an inline `bool` at the one call site that computes it
because the plan's Lane C mints a CrateStack `auth()` context from exactly
this value, and a type with a named accessor is the contract that seam is
written against — a future edit to _how_ an admin's tenant is chosen (a
header instead of a query parameter, say) changes one function's inside and
not Lane C's understanding of what it consumes.

## What this ADR does not do

- **No write path of any kind.** `/dash/v1` still answers `403` to every
  non-`GET`, checked before the staff row and therefore before `is_admin` is
  even read. `an_admin_still_cannot_write` is the container-backed proof.
- **No merged, multi-tenant page.** Decision 3.
- **No change to who may become an admin.** `vpay-server staff add --admin`
  is the only way, exactly as `staff add` (no flag) is the only way a staff
  member is created at all — ADR-0017 decision 1, unchanged.
- **No change to `/v1`.** The merchant API is untouched, as the plan's own
  "What this plan does not do" already states for the whole of Lane B
  through Lane E.
