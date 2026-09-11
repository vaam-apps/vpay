# exp51 — the demo dashboard shows the payments a person actually made

Branch `claude/exp51-demo-tenant`, base `6b1b7d8` (master).

## 1. The report

The maintainer used the stack by hand on 2026-09-11: payments created through
the demo shop did not appear on the demo dashboard, and looking one up by
payment id answered `404`.

## 2. The diagnosis, confirmed before anything was changed

**Confirmed. Neither the dashboard nor the 404 is broken.** Three
independent pieces of evidence, taken on the unmodified base commit:

### 2a. The generated overlay

`just gen-demo-keys` writes `.e2e/application-demo.yml`, and on the base
commit it registers two merchant clients on two tenants and binds the
dashboard to the first:

```yaml
dashboard_client:
  merchant_id: demo-merchant-tenant
merchant_clients:
  - client_id: demo-merchant
    merchant_id: demo-merchant-tenant # what `just demo-walk` pays as
  - client_id: shop-merchant
    merchant_id: shop-merchant-tenant # what `examples/shop` pays as (D12)
```

`demo_staff_merchant` was a second literal spelling `demo-merchant-tenant`,
so `just demo-staff` created the staff member in the same tenant — the
binding and the staff member agreed with each other, and both disagreed with
the shop.

### 2b. The merchant binding in `vpay_api::dash`

`backends/crates/vpay-api/src/dash/mod.rs`'s module doc states the boundary:
a `/dash/v1` request reads exactly one tenant's rows, the one
`dashboard_client.merchant_id` names, and every query filters by it through
`MerchantScope`. `dash::payment_intents::retrieve` reads the intent with
`PaymentIntents::get_for_merchant(.., scope.merchant_id(), &id)` and maps
`None` to `ApiError::NotFound` — its own doc says "another merchant's id is
the **same `404`** a nonexistent one gets", deliberately, so the page cannot
be an oracle for which ids exist in other tenants. The dashboard app maps
that 404 to Next's `notFound()` for the same stated reason.

The two tenants cannot be merged either: `ConfigError::DuplicateMerchantId`
refuses a config where two `merchant_clients` share a `merchant_id`.

### 2c. A live stack

Own compose project `exp51` (server :19500, dashboard :14500, shop :14502,
checkout :14501, orange stub :19502, receiver :19501), brought up from the
base commit with `just demo-up`.

```console
$ just … demo-staff
demo-staff: created ada@example.test for demo-merchant-tenant; …
```

Then one payment made **by hand in a real browser**: the shop's catalogue →
Njangi tote bag → cart → checkout (e-mail `payer@example.test`, hosted
surface) → vpay's hosted page → MTN Mobile Money → `237600000100` → "Payment
received" → back to the shop, which rendered "Paid" from its own database
after vpay's signed webhook arrived. The shop's own runbook block named
`pi_5xr2f3cmds6vh9py620ch1t1` and `cs_g9tpne507s69f93fn05aypm1`.

The stack's database then held exactly one payment intent and one staff
member, in two different tenants:

```console
$ docker exec exp51-postgres-1 psql -U vpay -d vpay -At \
    -c "select id, merchant_id, status from payment_intents order by seq desc"
pi_5xr2f3cmds6vh9py620ch1t1|shop-merchant-tenant|succeeded
$ docker exec exp51-postgres-1 psql -U vpay -d vpay -At \
    -c "select email, merchant_id from staff_members"
ada@example.test|demo-merchant-tenant
```

That is the whole report. The dashboard was showing its tenant, correctly,
and its tenant was the one nobody had clicked anything in.

## 3. What was chosen, and why

**The demo's dashboard follows the SHOP's tenant.**

The brief allowed either choice or a better arrangement. There is no
arrangement in which both merchants are visible — a dashboard client binds to
one tenant, and two clients sharing a tenant is refused at boot — so the
question is only _whose payments is this dashboard for_.

It is for the person holding the mouse. `examples/shop` is the clickable
surface: it is the thing the runbook tells a reader to open, the thing the
two shop specs drive, and the thing the maintainer used. `just demo-walk` is
a scripted console walkthrough that prints its own six outcomes as it goes; a
reader of it is reading a terminal, not looking for a list. Binding the
dashboard to the surface with no clicks in it is what produced a report.

Two things follow, and both are implemented rather than described:

1. **One variable, not two literals.** `demo_dashboard_merchant` is the
   binding, and `demo_staff_merchant` is _that variable_ rather than a second
   spelling of the same string. Two literals is how the binding and the staff
   member could have gone out of step; now they cannot.
2. **The override moves both.** `just demo_dashboard_merchant=demo-merchant-tenant demo-up`
   and the same on `demo-staff` points the dashboard at the walkthrough's
   payments. `gen-demo-keys`' presence check is keyed on the current value —
   the pattern the recipe already used for `demo_dashboard_port` and
   `demo_staff_token_ttl` — so a stale overlay is regenerated rather than
   kept.

A third thing that is not obvious and is deliberate: **`gen-demo-keys`
refuses a value that names neither registered tenant**, before it generates a
key. Without that check a typo is `ConfigError::DashboardUnknownMerchant` and
exit 78 on every restart — a `demo-up` that spends its whole readiness budget
in a crash loop while the recipe that caused it reports success.

### The browser fixtures moved with it

`cy.task('mintCheckoutPaymentIntent')` and its hand-run twin
`examples/checkout-browser/mint.mjs` now mint as `shop-merchant`. They are
the other two ways a person puts a payment on this stack through a browser,
and both used to pay into the tenant the dashboard does not show — the same
invisibility, reached by a different route. Which merchant pays makes no
difference to anything `checkout.cy.ts` asserts: it drives a publishable key
and a `client_secret`, and both merchants have one.

`test-e2e` pins that fixture to `shop-merchant` rather than deriving it from
`demo_dashboard_merchant`, on purpose. Under the override the dashboard spec
**must** fail; a recipe that moved the fixture with the binding would make
the override look harmless.

### What did NOT move

`examples/merchant-demo` (`just demo-walk`) still authenticates as
`demo-merchant` into `demo-merchant-tenant`. D12 separated the two
credentials so neither walkthrough can break the other, and that is still
worth having; the demo now simply says out loud which of the two the
dashboard is for.

## 4. The change, file by file

| File                                                 | What                                                                                                                                                                                                                                                                                                       |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `justfile`                                           | new `demo_dashboard_merchant` (default `shop-merchant-tenant`); `demo_staff_merchant` is now that variable; the overlay heredoc and `dashboard_binding_present` are keyed on it; `gen-demo-keys` refuses a value naming neither registered tenant; `test-e2e` mints its browser fixture as `shop-merchant` |
| `frontends/tests/e2e/cypress/tasks/checkoutTasks.ts` | defaults moved to `shop-merchant` / its key / its publishable key                                                                                                                                                                                                                                          |
| `examples/checkout-browser/mint.mjs`                 | the same three defaults, kept in step with its twin                                                                                                                                                                                                                                                        |
| `frontends/tests/e2e/cypress/support/shop.ts`        | `buyOnVpaysPage` moved here — two specs drive those screens now                                                                                                                                                                                                                                            |
| `frontends/tests/e2e/cypress/e2e/dashboard.cy.ts`    | the new case; header section explaining it                                                                                                                                                                                                                                                                 |
| `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts`  | imports the moved helper                                                                                                                                                                                                                                                                                   |
| `docs/runbooks/demo.md`                              | §6 "Which payments, and why not the walkthrough's", the corrected `demo-staff` transcript, the corrected step 5                                                                                                                                                                                            |
| `docs/flows/dashboard.md`                            | Status: the demo's binding, and that the flow itself did not change                                                                                                                                                                                                                                        |
| `examples/checkout-browser/README.md`                | which credential `mint.mjs` uses and why                                                                                                                                                                                                                                                                   |
| `docs/status.md`                                     | the measured counts below                                                                                                                                                                                                                                                                                  |

No Rust changed. No schema changed. This is configuration, a test and prose.

## 5. The new case, and why it is written the way it is

`dashboard.cy.ts`, "shows the demo staff member a payment somebody made
through the shop":

1. buys a Njangi tote in the shop and submits the checkout form (hosted
   surface) — `buyOnVpaysPage`, the same three screens `shop-hosted.cy.ts`
   drives;
2. on vpay's page, picks MTN, types `237600000100` and waits for the
   `succeeded` outcome. `vpay-worker` polling the MTN stub is what moves it;
   nothing in the spec pushes the status;
3. lands back on the shop's return page, which renders `paid` from the
   shop's own database after vpay's signed webhook arrived;
4. reads the payment intent id out of the **shop's** `orders.get` — so the id
   asserted on is the one the merchant stored, not one the spec minted;
5. signs into nothing new (still signed in from the sign-in leg) and asserts
   the payment is in `/payments` by its `data-payment-id` row, that its link
   lands on `/payments/{id}`, that `detail-id` is that id, and that
   `detail-rail` is `mtn_momo` — i.e. a payment with a charge behind it.

**Deliberately not `cy.task('mintCheckoutPaymentIntent')`.** That task holds a
merchant private key; a test built on it proves only that a tenant can read
its own rows, which `dashboard_read_surface.rs` already proves 13 ways. What
is under test here is the arrangement of the **demo**, so the payment has to
arrive the way the person who reported this made theirs.

## 6. Gates, recipe by recipe

_(filled in below as each was run; every exit code is read from a file, not
from a banner.)_
