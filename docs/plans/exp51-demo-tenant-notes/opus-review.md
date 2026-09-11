# exp51 — the review: what stood between a right diagnosis and a working demo

Branch `claude/exp51-demo-tenant`, reviewing its own four commits (`83221de`,
`daa8baa`, `b9fe8f0`, `fff0586`, notes `26b0ba6`) on base `6b1b7d8`.

[opus.md](opus.md) is the implementation's own account. Its **diagnosis is
right and its design is kept**: the dashboard reads exactly one tenant, two
merchant clients cannot share one, the shop is the surface a person clicks, so
the demo binds the dashboard to the shop's tenant and the staff member follows
it. Nothing below changes any of that.

What it did not do is **record** the run. §6 of its own notes — "Gates, recipe
by recipe … filled in below as each was run" — is empty, and the branch says
nothing about `just test-e2e` anywhere. It was run: twice, at 04:02 and 04:15
on 2026-09-11, the second of them ending `16 tests, 14 passing, 2 failing`
(`dashboard.cy.ts` 9/11). The notes commit is timestamped 04:33, after both.
So the position is worse than an unrun gate: the gate ran, it was red, and
the branch that carries it is silent. That is the failure mode CLAUDE.md
names first — making the work look more finished than it is.

It fails for **two independent reasons**, only one of which CI could see —
and a third one sits behind the override the branch tells a reader to use.

## 1. CI ran a different environment, because the workflow held a second copy

`just test-e2e` is not what CI's `e2e (compose)` job runs. That job brings the
same stack up in YAML and then runs `pnpm --filter @vpay/e2e e2e` with its
own `env:` block — a copy of the recipe's, added "so a port change in one
place fails loudly rather than in a spec's 60-second timeout".

exp51 moved the browser fixture's merchant in the recipe. The workflow's copy
was untouched, so CI ran:

```yaml
VPAY_MERCHANT_CLIENT_ID: demo-merchant
VPAY_MERCHANT_PRIVATE_KEY_PATH: …/.e2e/demo-merchant/oauth-signing-key.pem
```

against a `checkoutTasks.ts` whose publishable-key default had moved to
`pk_test_shopmerchantsandbox1`. That is not one mismatch but two:

- **`checkout.cy.ts`, 0/1.** The intent is minted into
  `demo-merchant-tenant`; the page presents the SHOP's publishable key; and
  `/v1/browser`'s `authenticate` answers the uniform `404` it answers for
  every refusal, deliberately. The page therefore never renders a status and
  the spec times out after 10 000 ms on
  `data-status="requires_payment_method"` — a symptom that names neither the
  key nor the tenant, which is the point of that 404.
- **`dashboard.cy.ts`, "lists this merchant's payments".** The intent
  `cy.task('mintCheckoutPaymentIntent')` creates lands in the tenant the
  dashboard does not show, so `table tbody tr` is never found. Exactly the
  bug the maintainer reported, reproduced by the fix for it.

Evidence: CI run
[34555068739](https://github.com/vaam-apps/vpay/actions/runs/34555068739), and
its own log header, which prints the job's `env:` before the specs run.

**The fix is not a third literal.** The Cypress run moved into its own recipe,
`just e2e-specs`, which holds the environment once; `test-e2e` calls it and
CI's job calls it. That is the arrangement CI already uses for
`just helm-check`, `just lint-web`, `just check-schema` and `just demo-staff`,
and each of those files says why. `CHECKOUT_PUBLISHABLE_KEY` was dropped from
the recipe rather than corrected: `checkoutTasks.ts` already defaults to that
literal, and it already exists in three places the recipe cannot reach.

`just --dry-run e2e-specs` with no overrides prints the eight variables the
workflow used to spell — same ports (8080, 3001, 3080, 8082), same
`ada@example.test`, same `.e2e/vpay-demo/staff-password.txt` — with
`shop-merchant` in place of `demo-merchant`, plus `VPAY_DASHBOARD_URL`, which
CI had been leaving to `cypress.config.ts`'s own `:3000` default.

## 2. `Cypress.env` does not survive this spec's own primary-origin change

This one CI and a local run fail identically, and it is the branch's, not the
workflow's.

`fff0586` split the shop payment from the dashboard read and recorded that
"`testIsolation: false` is what makes the split free: the dashboard session
and the id below both survive into the next test." The session does. The id
does not.

Cypress treats a differing **port** as a differing origin
(`getSuperDomainOrigin` = protocol + superdomain + port — `shop-hosted.cy.ts`
already says so in its header), a test's primary origin is fixed by its first
`cy.visit`, and the `Cypress` object a spec talks to belongs to that origin's
spec bridge. The shop test's first visit is `http://localhost:3001`; every
other test in the file is on the dashboard's port. So a `Cypress.env(key,
value)` written during the shop test is written to a bridge the next test is
not talking to.

Two cases failed on it, and the second is the one worth noticing:

- "shows the demo staff member that payment, in the list and by id" —
  `expected undefined to match /^pi_/`, before it made a single request;
- "signs the same staff member back in with the password they set" —
  `cy.task('totpCode')` … `Cannot read properties of undefined (reading
'replace')`. That secret is stored in the **second** test of the file and
  read in the last one; the shop leg in between is what loses it. This case
  is untouched by exp51 and passes on master. The branch broke it at a
  distance.

Reproduced on a local stack of this review's own (compose project `exp51f`,
server :19600, dashboard :14600, shop :14602, checkout :14601), on the
branch as delivered — the same two, with the same messages:

```
checkout.cy.ts     1 passing, 0 failing        ← green LOCALLY; CI's copy is why it was not there
dashboard.cy.ts    9 passing, 2 failing
shop-hosted.cy.ts  4 passing, 0 failing
```

and while those two were failing, the stack's own database held the shop's
payment in the tenant the dashboard is bound to:

```console
$ docker exec exp51f-postgres-1 psql -U vpay -d vpay -At \
    -c "select id, merchant_id, status from payment_intents order by seq"
pi_xrwz75z8e556v6y7q25d95k4|shop-merchant-tenant|requires_payment_method
pi_bnm34wk5ch5x7255zera1wpd|shop-merchant-tenant|succeeded
pi_876a4rg4zd1bv8g3t7ag3r2y|shop-merchant-tenant|requires_payment_method
pi_wh7f55adkx50fbjtp3tv6s36|shop-merchant-tenant|succeeded
$ docker exec exp51f-postgres-1 psql -U vpay -d vpay -At \
    -c "select email, merchant_id from staff_members"
ada@example.test|shop-merchant-tenant
```

**That is the shape of the whole thing: the tenancy fix was correct and the
spec that proves it could not read its own fixture.** A fix that only made
the tenancy right would have left both cases red.

The values move to **Node**: `carryForward`/`carriedForward` in
`cypress/tasks/dashboardTasks.ts`, a `Map` in the one process `cypress run`
has for the whole run, with no origins in it. `cypress/support/dashboard.ts`
wraps them so a missing value fails naming the key rather than as
`undefined` three commands later.

## 3. A third one, found only by running the override the branch documents

The branch's whole claim about `demo_dashboard_merchant` is that pointing it
back at `demo-merchant-tenant` makes the dashboard case fail — "that spec
failing is the proof this variable is load-bearing". So the review ran it:

```bash
just demo_dashboard_merchant=demo-merchant-tenant demo_project=… test-e2e
```

It failed. It failed at the **first leg of the sign-in**, before anything
about payments, and the reason was not the tenancy guard:

```console
$ grep -A2 '^dashboard_client:' .e2e/application-demo.yml
dashboard_client:
  merchant_id: demo-merchant-tenant
$ docker exec …-postgres-1 psql -U vpay -d vpay -At \
    -c "select email, merchant_id from staff_members"
ada@example.test|shop-merchant-tenant
```

```text
WARN vpay_api::staff::oauth: a staff member signed in against a deployment
whose dashboard is bound to another merchant; refusing to mint a code for a
tenant they may not read
```

`gen-demo-keys` is a `just` **dependency** of `test-e2e`, so it sees the
override and binds the overlay to the tenant asked for. `just demo-staff` is
a **sub-invocation**, and `just` passes the environment on but not variable
overrides — the recipe's own comment says exactly that about the ports, and
repeats all seven of them for that reason. `demo_dashboard_merchant` was not
on that list, so `demo_staff_merchant` — which is `demo_dashboard_merchant` —
resolved to its default.

So the documented override produced a stack whose dashboard and whose only
staff member named different tenants, and the spec died at `/authorize`.
**A spec that cannot sign in proves nothing about which payments a dashboard
shows**, and the failure looks, from the outside, exactly like the guard
firing. Fixed by adding the variable to the sub-invocation; the override then
fails where it is supposed to (§6).

## 4. What the review added to the case itself

`dashboard.cy.ts` asserted the by-id read only by clicking the row's link, so
the `404` half of the report had no way to fail on its own: a regression that
broke by-id and left the list alone would have failed at `[data-payment-id]`
and read as a list problem. The case now ends with a direct

```js
cy.visit(`/payments/${intentId}`);
cy.get('[data-testid="detail-id"]').should("have.text", intentId);
```

which is the reported symptom exactly — `dash::payment_intents::retrieve`
answers the uniform cross-tenant 404, the app maps it to Next's `notFound()`,
and `cy.visit` fails on a 404 status.

## 5. What was NOT changed

- **The design.** The binding, `demo_staff_merchant := demo_dashboard_merchant`,
  the `gen-demo-keys` guard and the regeneration trigger are exp51's and are
  kept as written.
- **The deliberate asymmetry.** The fixture stays pinned to `shop-merchant`
  rather than following `demo_dashboard_merchant`, so
  `just demo_dashboard_merchant=demo-merchant-tenant test-e2e` still fails.
  Verified — see §6.
- **`examples/merchant-demo`.** `just demo-walk` still pays as
  `demo-merchant` into a tenant this dashboard does not show, and
  `docs/runbooks/demo.md` §6 still says so where a person signs in.
- **No Rust, no schema, no migration.** As on the branch before this review.

## 6. Gates

Each exit code read from a file, not from a banner. Four compose stacks, all
of this review's own and all on non-default ports so nothing collided with the
other agent on this host.

### `just test-e2e`, on the review head, default binding

**Exit 0. 22 Cypress tests, 22 passing, 0 failing, 0 skipped**, across four
specs and the two `cypress run`s `pnpm run e2e` chains:

```
checkout.cy.ts       1 passing        (default run)
dashboard.cy.ts     11 passing        (default run)
shop-hosted.cy.ts    4 passing        (default run)   ✔ All specs passed  16/16
shop-embedded.cy.ts  6 passing        (framed run)    ✔ All specs passed   6/6
```

Compose project `exp51j` — server :19900, dashboard :14900, shop :14902,
checkout :14901, orange stub :19902 — real Postgres, a real `vpay-worker`
driving the poll ladder, WireMock rails. The two cases the maintainer's report
is about are `dashboard.cy.ts`'s "takes a payment through the shop, the way
the person who reported this did" and "shows the demo staff member that
payment, in the list and by id".

### The override, which must fail — and now fails in the right place

`just demo_dashboard_merchant=demo-merchant-tenant demo_project=exp51h …
test-e2e`, on the head of this review:

```
demo-staff: created ada@example.test for demo-merchant-tenant
checkout.cy.ts   1 passing, 0 failing    ← unaffected: mint and key are both the shop's
dashboard.cy.ts  ✓ sends an unauthenticated visitor to the sign-in form
                 ✓ signs a staff member in through the real OP, enrolling …
                 ✓ keeps the session token httpOnly …
                 ✓ shows the sign-in form … for a cookie vpay refuses
                 ✗ lists this merchant's payments and opens one of them
                 ✓ takes a payment through the shop, the way the person who reported this did
                 ✗ shows the demo staff member that payment, in the list and by id
                 ✗ renders the masked payer as a dash …   (nothing in this tenant has a charge)
```

The staff member signs in — all four sign-in legs pass — the shop takes a real
payment, and then every case that reads `/dash/v1` for a payment fails,
because the payment is in `shop-merchant-tenant` and this dashboard reads
`demo-merchant-tenant`. That is the guard doing its job, and it is a
different failure from the one §3 describes: there, nothing got past `/login`.

The spec's own line was `11 tests, 5 passing, 4 failing, 2 skipped` — the run
was **stopped by hand** once the three cases above had failed, because the two
remaining ones (the token-refresh leg and the second sign-in) are about a
session and not about a tenant, and this host had another agent's build on it.
The fourth failure is the token-refresh leg caught by that stop; it is not
evidence of anything and is not claimed as any.

### `just ci`

**Exit 0**, read from a file. Twelve gates, `test-rust` **1696 tests run,
1696 passed, 0 skipped** across 46 binaries, `test-doc` 111 passed / 1
ignored, `verify-ignored` 0 ignored / 46 binaries / 1696 total, `test-web` 0
skipped, `deny`. `docs/status.md`'s entry for this branch carries the full
line, and also carries the two earlier exits of 100 — both
`testcontainers: failed to create a container`, both this rootless-Docker host
rather than this branch, and both explained there rather than dropped.

### What was NOT run

`just docs-check-citations` (needs the network and a GitHub token),
`just helm-check` (needs the network; no chart changed), `just demo-walk`
(untouched by this branch).
