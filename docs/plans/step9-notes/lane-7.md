# Step 9, lane 7 — the shop (`examples/shop`)

**Date:** 2026-09-04 · **Branch:** `claude/step9-lane-7-shop` · **Base:**
`386c9d5` (gate branch with lane 5's SDK work merged).

What landed, what it is verified by, and — the part other lanes need — the
**exact** compose, config and `gen-demo-keys` additions lane 4 must make.
Lane 7 does not edit `compose*.yml`, the `justfile`, `deploy/helm`,
`docs/status.md` or `docs/flows/*`.

---

## 1. What landed

`examples/shop` (`@vpay-examples/shop`): a Next.js App Router site — catalogue,
cart, checkout, order, return, cancelled and embedded pages; a tRPC router
(`products.list`, `orders.create`, `orders.get`, `orders.embeddedSecret`) over
a ZenStack `schema.zmodel` compiled to Prisma on Postgres; a webhook route
handler; a root-context Dockerfile; and 49 vitest cases.

Versions pinned exactly: **Next 16.3.4**, **tRPC 11.18.0**, **ZenStack
2.22.3**, **Prisma 6.19.3**, **Zod 4.5.4**, React 19.2.8.

> **Prisma 6, not 7, and that is the newest pair that works.** ZenStack 2.22.3
> prints _"Prisma 7 support is untested and not planned"_ and emits a
> `datasource` with `url = env(...)`, which Prisma 7 rejects (`P1012` — URLs
> moved to `prisma.config.ts`). Measured: the first install of this package
> was on Prisma 7.10.0 and failed there. ZenStack 3 is `beta` on npm.

## 2. The compose service block (lane 4)

Add to **`compose.demo.yml`** and **`compose.e2e.yml`** (the e2e copy without
the `${VPAY_DEMO_SHOP_PORT}` indirection, exactly as `wiremock-webhook` is
handled today):

```yaml
vpay-shop:
  build:
    context: .
    dockerfile: examples/shop/Dockerfile
  environment:
    # Its own database in the shared `postgres` container — see §3.
    DATABASE_URL: postgres://vpay:vpay@postgres:5432/shop
    # vpay as THIS SERVER reaches it: the compose service name.
    VPAY_API_URL: http://vpay-server:8080
    # vpay as a BROWSER reaches it: the published host port. The two differ
    # inside compose, which is the whole reason there are two variables.
    VPAY_BROWSER_API_URL: http://localhost:${VPAY_DEMO_PORT:-8080}
    VPAY_CLIENT_ID: shop-merchant
    VPAY_PRIVATE_KEY_FILE: /secrets/shop-merchant.pem
    VPAY_PUBLISHABLE_KEY: pk_test_shopmerchantsandbox1
    # The checkout app, as a browser reaches it (it is an iframe `src`).
    VPAY_CHECKOUT_URL: http://localhost:${VPAY_DEMO_CHECKOUT_PORT:-4200}
    # Must be byte-identical to the secret in the generated overlay's
    # `shop-merchant.webhooks[0].secrets` (§4).
    VPAY_WEBHOOK_SECRET: ${SHOP_WEBHOOK_SECRET:-whsec-shop-demo-secret-32-bytes-xx}
    # Where vpay redirects a payer back to.
    SHOP_PUBLIC_URL: http://localhost:${VPAY_DEMO_SHOP_PORT:-3000}
    # See §6 before deciding whether to narrow this.
    # SHOP_PAYMENT_METHOD_TYPES: orange_money
  volumes:
    # The merchant's PRIVATE key. Unlike `demo-merchant`'s, this one is
    # mounted into a container, because the shop is the merchant's own
    # server. Read-only; `just gen-demo-keys` must chmod it 0644 (the
    # image runs as uid 1000, not root) — see §4.
    - ./.e2e/shop-merchant/oauth-signing-key.pem:/secrets/shop-merchant.pem:ro
  depends_on:
    postgres: { condition: service_healthy }
    vpay-server: { condition: service_started }
  ports: ["${VPAY_DEMO_SHOP_PORT:-3000}:3000"]
  # The image writes nothing outside /tmp: non-root (`node`, uid 1000),
  # read-only root filesystem, and `HOME`/`XDG_CACHE_HOME` already point at
  # /tmp in the Dockerfile.
  read_only: true
  tmpfs:
    - /tmp
  healthcheck:
    test:
      [
        "CMD",
        "node",
        "-e",
        "fetch('http://127.0.0.1:3000/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))",
      ]
    interval: 5s
    timeout: 3s
    start_period: 25s
    retries: 12
  restart: unless-stopped
```

Two collisions to resolve, both in lane 4's hands:

- **`dashboard` already publishes `3000:3000`** in `compose.e2e.yml`. Nothing
  in the demo starts it, but a `docker compose -f compose.yml -f
compose.e2e.yml up` with no service list would now fail on "port is already
  allocated". Either move `demo_shop_port` off 3000 or `!reset` the
  dashboard's publication the way `compose.demo.yml` already does for
  `postgres` and the rail stubs.
- **`just demo-up` names its services explicitly.** `vpay-shop` (and
  `vpay-checkout`) have to be added to that list or they will not start.

## 3. The `shop` database (lane 4)

The shop gets its own database in the **same** `postgres` container, created
by an init script. Postgres's entrypoint runs everything in
`/docker-entrypoint-initdb.d` once, on an empty data directory:

```yaml
postgres:
  volumes:
    - ./deploy/dev/postgres-init/10-shop-database.sql:/docker-entrypoint-initdb.d/10-shop-database.sql:ro
```

```sql
-- deploy/dev/postgres-init/10-shop-database.sql
-- The demo shop's database (`examples/shop`), beside vpay's own.
-- Separate database, same server: the shop is a MERCHANT's system, and a
-- merchant does not share a schema with its payment provider.
CREATE DATABASE shop OWNER vpay;
```

**It only runs on a fresh volume.** A developer with a `pgdata` volume from
before this change gets no `shop` database and a shop that fails at
`prisma migrate deploy`. `just demo-down -v` (or a note in the runbook) is the
answer; do not paper over it by making the entrypoint create the database.

The shop's tables and its catalogue are then created by
`prisma migrate deploy`, which the container's entrypoint runs before the
server starts (`examples/shop/docker-entrypoint.sh`). It is idempotent, so a
restart is a no-op. The catalogue is the second migration —
`prisma/migrations/20260904091600_seed_catalogue` — deliberately a data
migration rather than a `prisma db seed` script, so there is nothing extra to
remember to run.

## 4. `shop-merchant` in the generated overlay (lane 4, `gen-demo-keys`)

`gen-demo-keys` currently generates one key pair and writes one
`merchant_clients` entry. It needs to generate a **second** pair and write a
**second** entry. `merchant_clients` is a list, and a list in a profile
overlay replaces the base list wholesale, so both entries must be in the one
generated document.

Additions to the recipe:

```bash
    shop_key=.e2e/shop-merchant/oauth-signing-key.pem
    # ... the same freshness checks the demo key already gets, plus one more:
    #     grep -q '^\s*client_id: shop-merchant' "$overlay"
    # An overlay generated before this step has no shop merchant, and the
    # shop's first token request would answer `invalid_client` with nothing
    # pointing at a stale file — the same class of failure the existing
    # `merchant_id` / `publishable_keys` checks exist for.

    shop_generated=$(cargo xtask gen-signing-key --out .e2e/shop-merchant)
    shop_jwk=$(printf '%s\n' "$shop_generated" | grep -m1 '^{"kty"')
    shop_n=$(printf '%s' "$shop_jwk" | jq -er .n)
    shop_e=$(printf '%s' "$shop_jwk" | jq -er .e)
    shop_kid=$(printf '%s' "$shop_jwk" | jq -er .kid)

    # 0644, NOT 0600: unlike demo-merchant's key this one is bind-mounted into
    # a container (the shop is the merchant's own server) and that image runs
    # as uid 1000. Throwaway, git-ignored, regenerated per checkout.
    chmod 0644 "$shop_key"
```

and this entry in the heredoc, **after** `demo-merchant`:

```yaml
      - client_id: shop-merchant
        merchant_id: shop-merchant-tenant
        jwks:
          keys:
            - kty: RSA
              use: sig
              alg: RS256
              kid: "$shop_kid"
              n: "$shop_n"
              e: "$shop_e"
        grant_types: [client_credentials]
        # `payments:write` for POST /v1/payment_intents and
        # POST /v1/checkout/sessions.
        scopes: ["payments:write"]
        allowed_audiences: ["vpay:v1"]
        # Fixed, like demo-merchant's, and for the same reason: the shop's
        # compose environment names it literally.
        publishable_keys: ["pk_test_shopmerchantsandbox1"]
        # D4 / D12. The shop's own origin, as a BROWSER sees it — this is what
        # becomes `Content-Security-Policy: frame-ancestors` on vpay's
        # embedded page. `http://` is permitted only because this overlay does
        # not set `livemode`, so the base file's `false` stands.
        checkout_origins: ["http://localhost:{{demo_shop_port}}"]
        webhooks:
          - id: shop
            url: http://vpay-shop:3000/api/vpay/webhook
            secrets: ["\${SHOP_WEBHOOK_SECRET}"]
```

`SHOP_WEBHOOK_SECRET` must be resolvable in **`vpay-server`'s and
`vpay-worker`'s** environments (an unresolved `${...}` in the config is exit
78 at boot for both) _and_ be the same bytes as the shop's
`VPAY_WEBHOOK_SECRET`. The default above is 32 bytes, over the livemode floor,
for the reason `MERCHANT_WEBHOOK_SECRET` already documents.

`webhooks.allow_private_targets: true` is already in the overlay for
`wiremock-webhook`; `vpay-shop` is a compose service too, so the same flag is
what lets its deliveries out.

`demo-merchant` and its WireMock receiver stay exactly as they are (D12).

## 5. Justfile additions lane 4 owns

- `demo_shop_port := "3000"` and `demo_checkout_port := "4200"`, exported as
  `VPAY_DEMO_SHOP_PORT` / `VPAY_DEMO_CHECKOUT_PORT` from `demo-up`.
- The overlay freshness check gains `grep -q '^\s*client_id: shop-merchant'`
  and a `checkout_origins` check keyed on `{{demo_shop_port}}`, so changing
  the port regenerates rather than silently leaving a `frame-ancestors` that
  no longer names the shop.
- `vpay-shop` in `demo-up`'s explicit service list.
- A `demo-shop` recipe that prints `http://localhost:{{demo_shop_port}}`.

## 6. A cross-lane finding lanes 4 and 6 must decide on

**The catalogue is XAF; the demo stack's MTN rail settles EUR.**

`config/application.yml` gives `mtn_momo` `currency: EUR` (MTN's sandbox
rejects XAF) and `orange_money` `currency: XAF`.
`vpay_api::v1::payment_intents::currencies_agree` refuses a **confirm** whose
rail settles in a different currency from the intent —
`rail 'mtn_momo' settles in EUR; this PaymentIntent is XAF`. It is checked at
confirm, not at create, so an XAF order with both rails on it is created
happily and then refused on vpay's page if the payer picks MTN.

The charter for this lane is explicit that the catalogue is priced in XAF
(docs/flows/money.md's own example currency), so that is what shipped. The
consequence is not lane 7's to resolve:

- **Lane 6** wants the shop's hosted spec to pay by **MTN push**
  (`237600000ce0`). As things stand that confirm is refused.
- Three ways out, in the order I would try them: (a) set
  `SHOP_PAYMENT_METHOD_TYPES=orange_money` on the shop service and drive
  lane 6's shop specs on Orange only, keeping MTN on
  `examples/checkout-browser`'s existing EUR spec; (b) give the demo overlay
  a `providers:` block with `mtn_momo` on `currency: XAF` — note that a list
  in an overlay replaces the base list **wholesale**, so both providers would
  have to be written out; (c) price the catalogue in EUR, which contradicts
  this lane's charter and docs/flows/money.md's worked example.

I have taken (a) as far as making it one environment variable and no code
branch. **The choice itself is lane 4's and lane 6's, and it is called out
here rather than decided quietly.**

## 7. Runbook — "buy a product"

For `docs/runbooks/demo.md` (lane E's file, not mine). Assumes `just demo` is
green and the shop and checkout services are up.

```
1. Open http://localhost:3000
   Five products, priced in FCFA. "Add to cart" on any of them.

2. Open http://localhost:3000/cart
   The line total and the cart total. Change a quantity if you like; the
   number here is for display only.

3. "Checkout" -> enter any e-mail -> "Pay on vpay's page".
   The shop's server has now, in this order:
     - written an `orders` row (status `unpaid`),
     - created a PaymentIntent with `Idempotency-Key: shop-order-<id>-intent`,
     - written the `pi_…` onto the order,
     - created a hosted Checkout Session with
       `Idempotency-Key: shop-order-<id>-session-hosted`,
       `success_url = http://localhost:3000/orders/<id>/return?session_id={CHECKOUT_SESSION_ID}`,
       `cancel_url  = http://localhost:3000/orders/<id>/cancelled`.
   Your browser is now on vpay's page at http://localhost:4200/c/cs_…

4. Pay on vpay's page (Orange: the stub's "Pay" link; MTN: MSISDN
   237600000ce0 — but read §6 of docs/plans/step9-notes/lane-7.md first).

5. You land back on http://localhost:3000/orders/<id>/return
   It says "We are confirming your payment" and polls every 2 seconds. It is
   polling the SHOP's database, not vpay. The `session_id` it prints is the
   one vpay substituted into the URL; the page takes no decision from it.

6. Within a few seconds it turns to "Paid".
   That happened because vpay's worker delivered `payment_intent.succeeded`
   to http://vpay-shop:3000/api/vpay/webhook, the shop verified the
   `Vpay-Signature` header, and wrote the row. Watch it:

       docker compose $DEMO_COMPOSE logs vpay-shop | grep 'vpay webhook'
       # vpay webhook: payment_intent.succeeded evt_… -> 200 applied

   Deliver it twice and the second is `200 duplicate`, with no second
   `webhook_events` row:

       docker compose $DEMO_COMPOSE exec postgres \
           psql -U vpay -d shop -c 'SELECT id, status FROM orders' \
                                 -c 'SELECT id, type FROM webhook_events'

7. Cancel instead, on another order: vpay sends you to
   http://localhost:3000/orders/<id>/cancelled, and the order is still
   `unpaid`. The cancel URL writes nothing.

8. The embedded mode: from the order page of an unpaid order, or from
   "Pay without leaving the shop" at checkout, open
   http://localhost:3000/orders/<id>/embedded
   vpay's page is in an iframe served from http://localhost:4200/e/cs_…,
   allowed to frame here because `http://localhost:3000` is in
   `shop-merchant`'s `checkout_origins`. Paying inside it produces a
   `vpay:complete` message; the shop treats that as a cue and sends you to the
   return page, which — again — waits for the webhook.
```

## 8. Verification

Run in this worktree on 2026-09-04.

| Gate                                          | Result                          |
| --------------------------------------------- | ------------------------------- |
| `pnpm --filter @vpay-examples/shop typecheck` | pass                            |
| `pnpm --filter @vpay-examples/shop lint`      | pass                            |
| `pnpm --filter @vpay-examples/shop test`      | 49 passed, 0 skipped, 0 ignored |
| `pnpm --filter @vpay-examples/shop build`     | pass                            |
| `pnpm -r typecheck` / `just lint-web`         | pass                            |
| `just test-web`                               | see the lane report             |
| `just audit-web`                              | see the lane report             |
| `docker build -f examples/shop/Dockerfile .`  | pass                            |

### The four `pnpm.overrides` this lane added

`just audit-web` failed twice on this dependency set before it passed, both
times on advisories with **no version bump that clears them**. Each override
is scoped to its dependent and carries a dated justification in the root
`package.json`, beside the three that were already there.

| Override                                  | Advisory                                                                 | Why a bump does not fix it                                                                                                                                                         |
| ----------------------------------------- | ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `@prisma/config>deepmerge-ts: ^8.0.2`     | GHSA-ggr8-5vv4-36mx (**high**)                                           | `@prisma/config` pins `deepmerge-ts: 7.1.5` exactly in 6.19.3 _and_ in 7.10.0. It reaches the **production** graph through `@prisma/client`, so it failed the `--prod` half too.   |
| `chevrotain>lodash: ^4.18.1`              | GHSA-r5fr-rjxr-66jc (**high**), GHSA-f23m-r3pf-42rh, GHSA-xxjr-mmjv-4gpg | zenstack 2.22.3 pins `langium: 1.3.1` exactly, which pins `chevrotain: 10.4.2`, whose three packages each pin `lodash: 4.17.21` exactly. Dev-only (it arrives through `zenstack`). |
| `@chevrotain/gast>lodash: ^4.18.1`        | as above                                                                 | Same lodash, a different direct parent.                                                                                                                                            |
| `@chevrotain/cst-dts-gen>lodash: ^4.18.1` | as above                                                                 | Same lodash, a third direct parent.                                                                                                                                                |

Final result, verbatim, after all four:

```
audit-web: production dependency graph only (attempt 1 of 4)
No known vulnerabilities found
audit-web: whole workspace, dev dependencies included (attempt 1 of 4)
...
audit-web: whole workspace, dev dependencies included (attempt 3 of 4)
No known vulnerabilities found
audit-web: ok — no high or critical advisory in the workspace
```

The elision is the registry: `https://registry.npmjs.org/-/npm/v1/security/audits`
answered `503 Service Unavailable` through attempts 1 and 2 on 2026-09-04 and
succeeded on the third. That is what the recipe's four attempts are for, and
it is worth knowing before reading a CI failure on this step as a dependency
problem. A separate `pnpm audit --audit-level=high --json` over the whole
workspace on the same tree returned zero advisories.

Both were measured rather than assumed. `prisma migrate deploy` was run from
the built image against a real Postgres with the deepmerge override in place
and applied both migrations; `examples/shop/Dockerfile` installs its runtime
Prisma CLI from a manifest carrying the same override, and the only
`deepmerge-ts` in the image is 8.0.2. `prisma/schema.prisma` regenerates byte
for byte with the lodash override applied, which is the thing chevrotain is
actually used for.

**Guard-failure proofs** (each reverted immediately):

- Replacing `verifyWebhook(...)` with `JSON.parse(request.rawBody)` in
  `src/server/webhook.ts` makes **4** cases in `webhook.test.ts` fail
  (`expected 200 to be 400`), including "a bad signature is 400 and writes
  nothing". Restored: 12 passed.
- Replacing `Object.hasOwn(SETTLING_EVENTS, event.type) ? … : undefined` in
  `src/server/webhook.ts` with a bare `SETTLING_EVENTS[event.type]` — the
  form the file shipped with until this was caught — makes the
  `constructor` case fail with
  `expected { outcome: 'applied' } to deeply equal { outcome: 'ignored' }`:
  an event typed `constructor` reads a truthy function off the prototype and
  **settles the order**. The same bare form in `src/money.ts` renders
  `NaN.NaN CONSTRUCTOR` on a price tag.
- Widening `Product`'s policy from `@@allow('read', true)` to
  `@@allow('all', true)` in `schema.zmodel` and regenerating makes **3**
  cases in `policies.test.ts` fail. Restored: 7 passed.

**No secret reaches the browser bundle.** Of the 17 client chunks
`next build` emits, none contains `VPAY_WEBHOOK_SECRET`,
`vpayWebhookSecret`, `VPAY_PRIVATE_KEY_FILE`, `node:crypto` or
`readFileSync` — grepped, not reasoned about. The only `console` call in the
shipping source is the webhook route's one line, and it prints an event id,
an event type and an outcome.

**Manual proof of the Prisma store**, which no unit test covers: the built
image (`docker build -f examples/shop/Dockerfile .` on the committed tree)
was run non-root with `--read-only --tmpfs /tmp` against a throwaway
Postgres. `prisma migrate deploy` applied both migrations; `/healthz` answered
200; the catalogue rendered `12 000 FCFA` / `45 000 FCFA`; a signed delivery
answered `{"received":true,"outcome":"applied"}` and the order became `paid`;
its replay answered `duplicate` with `count(*) = 1` in `webhook_events`; an
event for an unknown `pi_…` answered `unknown_intent` with no new row; a bad
signature answered `400`.

## 9. What lane 7 did NOT do

- **No end-to-end proof through a real vpay.** Lane 1's
  `/v1/checkout/sessions` does not exist on this base, so nothing here has
  ever spoken to a running vpay server. The tests speak to a local HTTP
  server shaped like the wire contract. That is lane 6's job.
- **No automated test of `PrismaShopStore`** — see §8 and the README.
- **No component or browser tests.** The React components and the browser end
  of `initEmbeddedCheckout` are untested here.
- **No compose, justfile, helm, `docs/status.md` or `docs/flows/*` edits** —
  §2 to §5 are what lane 4 applies.
- **No i18n.** The shop is English only; vpay's page is the bilingual one.
- **No `enhance()`-based tenancy.** `orders.get` is reachable by anyone
  holding an order id, which is stated in the README rather than hidden
  behind a policy that would evaluate to `true`.
