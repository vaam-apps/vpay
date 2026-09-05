# `examples/shop` — a merchant site that pays through vpay

A five-product shop, in Next.js, that integrates vpay the way a real merchant
would: the **server** holds the credentials and creates the PaymentIntent and
the Checkout Session; the **browser** either goes to vpay's hosted page or
frames it; and the order is marked paid by a **signature-verified webhook**,
never by the payer coming back.

It exists because Step 9's D11 says the end-to-end demo should be a merchant
site rather than a script — the demo then proves what a merchant actually
builds.

> Nothing here ships and no money moves. The rails behind vpay in the demo
> stack are WireMock stubs. Do not deploy.

## What it does

| Page                     | What it is                                                             |
| ------------------------ | ---------------------------------------------------------------------- |
| `/`                      | The catalogue. Five seeded products, priced in XAF.                    |
| `/cart`                  | The cart. `localStorage`, no accounts (D13).                           |
| `/checkout`              | E-mail, then "Pay on vpay's page" or "Pay without leaving the shop".   |
| `/orders/{id}`           | The order, **read from this shop's database only**.                    |
| `/orders/{id}/return`    | `success_url`. "We are confirming your payment", polling every 2 s.    |
| `/orders/{id}/cancelled` | `cancel_url`. Writes nothing; the order stays open.                    |
| `/orders/{id}/embedded`  | The same unpaid order, paid inside an iframe.                          |
| `POST /api/vpay/webhook` | vpay's deliveries. The only thing that moves an order out of `unpaid`. |
| `GET /healthz`           | Liveness. Touches neither Postgres nor vpay.                           |

## The three rules this example is built to demonstrate

**1. The server prices the order.** `orders.create` takes product ids and
quantities. There is no price field on the wire, so there is no price for a
browser to tamper with; the total is summed from the catalogue in integer
minor units (docs/flows/money.md — XAF is zero-decimal, so `12000` is 12,000
FCFA).

**2. The idempotency key is derived from the order id**, not random:
`shop-order-{id}-intent`, `shop-order-{id}-session-hosted`,
`shop-order-{id}-session-embedded`. Every retry of the same order therefore
sends the same key, and the order row is written first precisely so a stable
key exists before the first call. It does **not** deduplicate two separate
checkout submissions — those are two orders with two ids, and vpay is right
to create two intents for them.

**3. Only the webhook marks an order paid.** The return page displays the
`session_id` vpay substituted into `success_url` and takes no decision from
it. `POST /api/vpay/webhook` verifies the `Vpay-Signature` header with
`@vpay/sdk`'s `verifyWebhook`, dedupes on vpay's event id in `webhook_events`,
and answers 2xx **after** the write.

## Running it

The supported path is the demo stack: `just demo` brings up vpay, the rail
stubs, the checkout app and this shop, and
`docs/plans/step9-notes/lane-7.md` has the "buy a product" runbook.

Standalone, against a vpay you are already running:

```bash
cp examples/shop/.env.example examples/shop/.env   # then edit it
docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=shop postgres:16-alpine
pnpm --filter @vpay-examples/shop exec prisma migrate deploy
pnpm --filter @vpay-examples/shop dev
```

`prisma migrate deploy` both creates the tables and seeds the catalogue: the
catalogue is a data migration, not a seed script, so there is nothing else to
remember to run.

### `VPAY_OAUTH_AUDIENCE`, and why it exists

`VPAY_API_URL` is where this server sends its requests. The client
assertion's `aud` claim is a different fact: it is what **vpay's OP calls
itself**, and `authenticate_client` compares the claim against its own two
names — `{deployment.public_base_url}/v1/oauth/token` and that `/v1/oauth`
issuer — and against nothing else. The URL you POSTed to is never consulted.

They coincide for a shop on the public internet, which is why the variable is
optional. They do not coincide in the demo stack:

| Variable              | The demo stack's value, and what it names                              |
| --------------------- | ---------------------------------------------------------------------- |
| `VPAY_API_URL`        | `http://vpay-server:8080` — vpay as **this container** reaches it      |
| `VPAY_OAUTH_AUDIENCE` | `http://localhost:8080/v1/oauth/token` — vpay as **vpay** names itself |

Leave it unset there and every token request answers `invalid_client`, with
the signature, the `client_id`, the `kid` and the lifetime all correct and
nothing in the response pointing at the audience. `compose.e2e.yml` and
`compose.demo.yml` set it on `vpay-shop` for exactly this reason.

The `/v1/oauth` issuer works too — the OP accepts either.

## The stack, and why each piece is here

| Piece                          | Version   | Why                                                                             |
| ------------------------------ | --------- | ------------------------------------------------------------------------------- |
| Next.js (App Router)           | 16.3.4    | D11. `output: 'standalone'` for the image.                                      |
| tRPC                           | 11.18.0   | D11. Server-side callers for pages, one HTTP client for the browser.            |
| ZenStack                       | 2.22.3    | D11. `schema.zmodel` is the source of truth; `enhance()` enforces its policies. |
| Prisma                         | 6.19.3    | ZenStack 2.x's ORM. **Not 7** — see below.                                      |
| Zod                            | 4.5.4     | tRPC input validation.                                                          |
| `@vpay/sdk`, `@vpay/stripe-js` | workspace | The merchant SDK and the browser SDK.                                           |

**Why Prisma 6 and not 7.** ZenStack 2.22.3 prints _"Prisma 7 support is
untested and not planned"_ and generates a `datasource` block with
`url = env(...)`, which Prisma 7 rejects outright (`P1012`: connection URLs
moved to `prisma.config.ts`). Measured, not assumed — the first install of
this package was against Prisma 7.10.0 and failed exactly there. ZenStack 3
is still `beta` on npm. 6.19.3 is the newest stable Prisma the newest stable
ZenStack works with.

**Four `pnpm.overrides` came with this package**, all for advisories no
version bump clears: `@prisma/config>deepmerge-ts` (GHSA-ggr8-5vv4-36mx,
high, and it reaches the _production_ graph through `@prisma/client`) and
three `lodash` entries under chevrotain, which `zenstack` pins transitively.
The root `package.json` carries the dated justification for each.

**Why no `@trpc/react-query`.** Three call sites and one poll loop. A query
cache would be a dependency and a provider for no behaviour.

**Why no `@vpay/ui`.** It is vpay's own design system; a merchant integrating
vpay would not have it, so a demo built out of it would be demonstrating
something no merchant can reproduce.

## ZenStack, honestly

`schema.zmodel` compiles to `prisma/schema.prisma` (committed, because the
container's entrypoint reads it) and to the metadata `enhance()` uses. The
`@@allow` rules are the ones that are **true of every caller**, because D13
gives this shop no authenticated principal:

- the catalogue is read-only to the application (it is seed data);
- `webhook_events` is append-only;
- no order or order line is ever deleted.

`src/server/store/policies.test.ts` proves each refusal, and needs no
database to do it. What the policies do **not** do is stop one payer reading
another payer's order: there is no principal to compare against, and
`orders.get` is reachable by anyone holding an order id. A real shop would
put a session or a signed order token in front of it. This one says so
instead of writing a rule that would evaluate to `true`.

## Tests, and what they do not cover

`pnpm --filter @vpay-examples/shop test` — 49 cases, no database, no Docker,
no network.

- `src/server/orders.test.ts` runs a **real `VpayClient`** against a real
  local HTTP server and asserts the bytes: the amount, the
  `Idempotency-Key`, the exact session parameters (including the literal,
  unescaped `{CHECKOUT_SESSION_ID}`), and the ids stored.
- `src/server/webhook.test.ts` covers the four rules, with the signature built
  by an independent implementation of the header grammar rather than by the
  verifier under test.
- `src/server/store/policies.test.ts` covers the ZenStack policies.
- `src/testing/no-runtime-imports.test.ts` is the TypeScript counterpart of
  `cargo xtask verify-no-mocks`: nothing under `src/app`, `src/server`,
  `src/components` or `src/lib` may name `src/testing`.

**Not covered by unit tests:** `PrismaShopStore`. The tests run against an
in-memory implementation of the same `ShopStore` port so that they need no
Postgres in CI's `web` job, which has none. The Prisma implementation was
verified by hand on 2026-09-04 against the built image and a real Postgres —
migrations applied, catalogue seeded and rendered, a signed delivery
`applied`, its replay `duplicate` with no second `webhook_events` row, an
unknown intent 2xx with no write. That hand-run is the only evidence there
is: **no automated test covers `PrismaShopStore` anywhere in this
repository.** `just demo` does not, whatever it may look like — it brings the
`vpay-shop` container up, waits for its healthcheck and prints its URL, and
never places an order (`docs/runbooks/demo.md` §5 lists "that the shop works"
under _does not prove_). Lane 6's Cypress specs — `shop-hosted.cy.ts` and
`shop-embedded.cy.ts` — **have** merged, and they drive this class in a real
browser against the demo stack; but they assert on the shop's pages, never on
`PrismaShopStore` itself, so the sentence above stands: no unit or integration
test covers it.

Also not covered by unit tests: the React components, and the browser end of
`initEmbeddedCheckout` (that is `@vpay/stripe-js`'s own suite, and lane 6's
merged `shop-embedded.cy.ts`).

## The image

```bash
docker build -f examples/shop/Dockerfile .        # from the repository root
```

Root context, because `@vpay/sdk` and `@vpay/stripe-js` are workspace
dependencies. Standalone output, non-root (`node`, uid 1000), read-only root
filesystem with a tmpfs on `/tmp`, and an entrypoint that runs
`prisma migrate deploy` before the server — so a container that starts has a
schema and a catalogue, or it does not start.
