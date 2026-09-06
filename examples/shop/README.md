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

| Page                     | What it is                                                                              |
| ------------------------ | --------------------------------------------------------------------------------------- |
| `/`                      | The catalogue. Five seeded products, priced in XAF.                                     |
| `/cart`                  | The cart. `localStorage`, no accounts (D13).                                            |
| `/checkout`              | An optional e-mail, a choice of surface, and the test-number panel.                     |
| `/orders/{id}`           | The order, **read from this shop's database only**, with what to do about a failed one. |
| `/orders/{id}/return`    | `success_url`. "We are confirming your payment", polling every 2 s.                     |
| `/orders/{id}/cancelled` | `cancel_url`. Writes nothing; the order stays open, and offers to cancel the intent.    |
| `/orders/{id}/embedded`  | The same unpaid order, paid inside an iframe.                                           |
| `POST /api/vpay/webhook` | vpay's deliveries. The only thing that moves an order out of `unpaid`.                  |
| `GET /healthz`           | Liveness. Touches neither Postgres nor vpay.                                            |

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
`@vaam-apps/vpay-sdk`'s `verifyWebhook`, dedupes on vpay's event id in `webhook_events`,
and answers 2xx **after** the write.

## The three ways to integrate, and how this shop picks one

**The integration mode is the developer's configuration**, not a thing a
payer chooses. `SHOP_CHECKOUT_MODE` is `hosted`, `popup` or `embedded`, it
defaults to `hosted`, and a real merchant sets it once and never shows a
switch. This shop shows one anyway — on `/checkout`, starting on the
configured value — because an example that could only demonstrate the mode
its environment file happened to name would be demonstrating a third of
itself.

| Mode       | What the payer sees                          | What the shop's server does                                                          |
| ---------- | -------------------------------------------- | ------------------------------------------------------------------------------------ |
| `hosted`   | The tab goes to vpay's page and comes back   | One **hosted** session; the browser follows `session.url`                            |
| `popup`    | vpay's page in a window this shop opened     | The **same** hosted session; the browser opens `session.url` in a popup              |
| `embedded` | vpay's page in an iframe on this shop's page | No session yet — one **embedded** session, minted when the frame asks for its secret |

`hosted` and `popup` send an **identical** `POST /v1/checkout/sessions`, so
they share an `Idempotency-Key` (`shop-order-{id}-session-hosted`). That is
deliberate: a payer whose popup is blocked falls back to a redirect and gets
the session they already had, rather than a second one.

### The popup, and why its completion message comes from _this_ shop

A popup is not a frame. Inside one `window.parent === window`, so vpay's
checkout page has no framer to post `vpay:complete` to and deliberately says
nothing (`frontends/apps/checkout/src/lib/frame.ts` returns `null` when there
is no parent). So the popup loads the **hosted** page, and what closes the
loop is `success_url` — this shop's own `/orders/{id}/return`, running
_inside_ the popup, calling `@vaam-apps/vpay-stripe-js`'s
`notifyCheckoutOpener`, which posts to `window.opener` and closes the window.

`notifyCheckoutOpener` answers `false` and does nothing when there is no
opener, which is what a payer who came back by an ordinary redirect has — so
one return page serves all three surfaces with no branch on a query
parameter.

Two things the shop does _not_ treat as authority: the completion message
(it navigates to the return page, which polls the shop's database) and the
popup being closed (`onCancel` sends the buyer to the order, because the
charge may still settle).

The window is opened **before** `orders.create` is awaited, because
`window.open` only succeeds inside the click that triggered it. A browser
that refuses it raises `CheckoutPopupBlockedError`, and the shop then creates
the order in `hosted` mode and navigates — a fallback no browser blocks. It
creates the order _at that point_ rather than in advance: a refused window is
refused before this shop's server has been asked for anything, so creating
one eagerly would leave an unpayable row behind every time the popup worked.

## Failure outcomes, and the numbers that reach them

A demo that can only show a payment working is showing the easy half. Every
outcome below is reachable from this shop, on the demo stack, by paying with
a documented fake number — and what the shop does about each is the point.
**Two of them are not reachable, and this document says which and why rather
than leaving them to be discovered:** `cancelled` (no event exists for it)
and MTN's `237600000400` (a decline at submit emits no event either). Both
are called out below, in the panel on `/checkout`, and in `docs/status.md`.

| The buyer sees                            | The order becomes | Where it comes from                                                  |
| ----------------------------------------- | ----------------- | -------------------------------------------------------------------- |
| A sentence written for the outcome        | `failed`          | `payment_intent.payment_failed`, and `last_payment_error.code` on it |
| "Try again" — for a payer-actionable code | a **new** order   | `orders.retry`: one charge per intent, forever                       |
| Nothing, and the order stays open         | `unpaid`          | The payer clicked "cancel" on the rail's page — a navigation         |
| "Cancel this payment"                     | `unpaid` — below  | `orders.cancel` → the intent is `canceled` at vpay → **no event**    |

> **`cancelled` is unreachable on today's vpay, and this shop does not pretend
> otherwise.** Measured on the demo stack on 2026-09-06: "Cancel this payment"
> reaches `POST /v1/payment_intents/{id}/cancel`, the intent really does become
> `canceled` — read out of vpay's own `payment_intents` row — and **vpay emits
> no event for that transition**. It writes three types and only three
> (`payment_intent.succeeded`, `payment_intent.payment_failed`,
> `checkout.session.expired`; `docs/status.md`'s "Events written by the worker"
> row says so, and the `events` table gained nothing during the run). So the
> order stays `unpaid`.
>
> The button and the procedure are left exactly as they are, because they are
> what a merchant's code should look like. What this shop will **not** do is
> write `cancelled` locally from its own request — that would be it deciding a
> settled status from something other than a signed event, which is the one
> thing the whole example exists to argue against. The gap is vpay's; it is in
> [`../../docs/plans/exp22-shop-demo-notes/opus.md`](../../docs/plans/exp22-shop-demo-notes/opus.md)
> and in `docs/status.md`.

The buyer-facing sentences live in `src/lib/failures.ts`, keyed on **vpay's**
closed `FailureCode` vocabulary (`docs/flows/failures.md`) rather than on
anything a rail said. That is the whole reason that vocabulary exists: one
message per outcome, not one per rail, and a rail added tomorrow reuses them.
`retryable` in that table is `FailureCode::payer_actionable` transcribed row
for row — a shop that offered "try again" after `invalid_payee` would be
inviting a buyer to fail identically a second time.

The order's `failure_code` and `failure_message` are written **by the webhook
handler and by nothing else**, in the same statement as the status, and only
on the write that moves it. A `paid` order that later receives a
`payment_intent.payment_failed` records the delivery and keeps both its
status and its empty failure columns.

### The test numbers

Nothing below is a phone number. They are documentation MSISDNs from the
`2376000000xx` block, and they mean something only because the demo stack's
WireMock rail stubs are _configured_ to answer particular things for them
(`backends/tests/conformance/wiremock/mtn/mappings/demo-outcomes.json` and
its Orange counterpart). **There is no branch on any of these values in
vpay, in this shop, or in either adapter.** Against a real rail they do
nothing at all.

`src/lib/test-numbers.ts` holds the same table, and
`src/lib/test-numbers.test.ts` fails if this document and that module
disagree in either direction — so the panel on `/checkout` and the table here
cannot drift.

#### MTN MoMo (`mtn_momo`)

Typed on **vpay's** checkout page: MTN is a push rail, so vpay prompts the
handset.

| Number         | What happens                                     | Order    | vpay code              | The rail said                   |
| -------------- | ------------------------------------------------ | -------- | ---------------------- | ------------------------------- |
| `237600000000` | Pays. Any number not listed below does the same. | `paid`   | —                      | `SUCCESSFUL`                    |
| `237600000101` | Declined — the wallet has too little money       | `failed` | `insufficient_funds`   | `NOT_ENOUGH_FUNDS`              |
| `237600000102` | The prompt expires — nobody enters the PIN       | `failed` | `payer_timeout`        | `COULD_NOT_PERFORM_TRANSACTION` |
| `237600000400` | Refused at submit — the rail has no such account. vpay's page says so; this shop never hears about it | `unpaid`   | `invalid_payer`        | `PAYER_NOT_FOUND (HTTP 400)`    |
| `237600000503` | The rail is unavailable                          | `failed` | `provider_unavailable` | `SERVICE_UNAVAILABLE`           |

> **`237600000400` leaves the order `unpaid`, and that is a second gap of
> the same shape as `cancelled`.** MTN refuses this MSISDN on the **submit**,
> before any charge is polled, so vpay commits the failure through
> `vpay_api::v1::payment_intents::persist_decline` — which writes the charge,
> writes `last_payment_error` on the intent, and **emits no event**.
> `payment_intent.payment_failed` is written by
> `vpay_db::settlement::apply_failed` and by nothing else, and only the
> worker's poll path calls it. So a decline **at submit** is a terminal
> outcome no signed event reports: the payer sees the real reason on vpay's
> page, the merchant can read it from `GET /v1/payment_intents/{id}`, and a
> shop that settles only from webhooks — this one — never learns of it. The
> order stays `unpaid` and "Try again" is not offered, because nothing told
> this shop there was anything to retry.
>
> The other four MTN numbers are unaffected: they are decided by the **status
> query**, which is the worker's path, which does emit. Pinned by
> `a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read` in
> `backends/tests/integration`, which asserts the `events` table stays empty
> for such a decline and will fail on the day vpay starts emitting one.
> Whether it should is a maintainer's call, written up in
> [`../../docs/plans/exp22-shop-demo-notes/opus-review.md`](../../docs/plans/exp22-shop-demo-notes/opus-review.md).

**No number produces `payer_declined` on this rail.** MTN documents no reason
for a payer who answered the prompt and refused it — its nine-row table has
none — so no MSISDN can produce one. `FailureCode::PayerDeclined` is
currently produced by **no adapter in this repository**; it is a code the
core defines and nothing emits.

#### Orange Money (`orange_money`)

Typed on **the rail's own** payment page, after vpay redirects you: Orange is
a redirect rail, so vpay never sees the number.

> **These do not work from a browser today.** The table below is what the stub
> would answer, not what you will see. Measured on the demo stack on
> 2026-09-06: vpay's confirm handler enqueues the first status query at
> `now()` — `poll_delay(0)` is the delay before the **second** attempt, not
> the first — and the worker's idle sleep is one second, so the stub's
> catch-all `SUCCESS` settles the charge long before a payer can reach this
> form. A run that typed `237600000400` came back **paid**: the submit was at
> T, the first `transactionstatus` at T+449 ms, and the form submission at
> T+12 s.
>
> The mappings themselves are right, and are proven at the adapter level by
> `a_test_number_typed_on_the_rails_hosted_page_reaches_the_documented_outcome`
> in `backends/tests/conformance`, which drives the same page and the same
> form against a real WireMock container and does not race a worker. What is
> missing is a way for the stub to answer `PENDING` while a payer is on the
> page — a change to a stub four suites share, one of which (Cypress) cannot
> be run from this branch's environment. The options are written up in
> [`../../docs/plans/exp22-shop-demo-notes/opus.md`](../../docs/plans/exp22-shop-demo-notes/opus.md).
>
> MTN's numbers are unaffected: a push rail takes the number in the
> merchant's own submit, so there is no window to lose.

| Number         | What happens                                     | Order    | vpay code        | The rail said |
| -------------- | ------------------------------------------------ | -------- | ---------------- | ------------- |
| `237600000000` | Pays. Any number not listed below does the same. | `paid`   | —                | `SUCCESS`     |
| `237600000102` | The payment window expires                       | `failed` | `payer_timeout`  | `EXPIRED`     |
| `237600000400` | Refused, with no reason the rail will name       | `failed` | `provider_error` | `FAILED`      |

Three outcomes this rail **cannot** express, stated rather than faked:

- `insufficient_funds` — Orange's documented statuses are `INITIATED`,
  `PENDING`, `SUCCESS`, `EXPIRED` and `FAILED`, and it documents no
  sub-reason for `FAILED`. A stub answering `NOT_ENOUGH_FUNDS` would be this
  repository inventing a rail vocabulary.
- `invalid_payer` — the number never reaches vpay on a redirect rail; the
  payer types it on Orange's page, after the charge was submitted.
- `provider_unavailable` — a rail that cannot answer is a _transport_ failure
  on Orange, not a status, and the poll ladder retries it for hours rather
  than failing the charge. Right, and not something a demo can show in a
  minute.

**`cancelled` is reachable from no number at all — and on today's vpay from
nothing else either.** Clicking "cancel" on the rail's page is a navigation:
the order stays `unpaid` and the charge may still settle. The order _would_
become `cancelled` when the shop cancels its PaymentIntent (the button on the
order page) and vpay delivered `payment_intent.canceled` — which it does not.
See the note under "Failure outcomes" above.

### Which rail can pay for what

Both test-number tables above are reachable from `just demo`, and that is a
property of the demo's **configuration** rather than a coincidence:
`just gen-demo-keys` writes a `providers:` block into
`.e2e/application-demo.yml` that puts `mtn_momo` on `currency: XAF`, and then
checks that it did. It has to. The catalogue is priced in XAF, and
`POST /v1/payment_intents/{id}/confirm` refuses a rail whose profile currency
is not the intent's — `vpay_api::v1::payment_intents::currencies_agree`.

Against `config/application.yml` **unmodified**, `mtn_momo` settles EUR —
MTN's real sandbox rejects XAF, and that file says so where it sets it — so
an XAF order there can only be paid on Orange, and a payer offered MTN would
be sent all the way to vpay's page for a `400` at the last step.

`SHOP_PAYMENT_METHOD_TYPES` is what a deployment says that with, and since
this change it takes a **per-currency** map as well as a plain list:

```
SHOP_PAYMENT_METHOD_TYPES=orange_money                    # one list, every currency
SHOP_PAYMENT_METHOD_TYPES=xaf:orange_money;eur:mtn_momo   # per currency
```

The rails are resolved against the **order's** currency at create time rather
than copied onto every intent, and a currency with no rails configured is a
refusal naming it — raised _before_ the order row is written, so the shop is
never left holding an order nothing can pay. Configuration either way, never
a code branch (ADR-0003), and no table of rail currencies for this shop to
keep in step with vpay's.

## Running it

The supported path is the demo stack: `just demo` brings up vpay, the rail
stubs, the checkout app and this shop, and
`docs/plans/step9-notes/lane-7.md` has the "buy a product" runbook.

Standalone, against a vpay you are already running:

```bash
cp examples/shop/.env.example examples/shop/.env   # then edit it
docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=shop postgres:16-alpine
pnpm --filter @vpay-examples/shop exec zen migrate deploy
pnpm --filter @vpay-examples/shop dev
```

`zen migrate deploy` both creates the tables and seeds the catalogue: the
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

| Piece                                              | Version    | Why                                                                                                |
| -------------------------------------------------- | ---------- | -------------------------------------------------------------------------------------------------- |
| Next.js (App Router)                               | 16.3.4     | D11. `output: 'standalone'` for the image.                                                         |
| tRPC                                               | 11.18.0    | D11. Server-side callers for pages, one HTTP client for the browser.                               |
| ZenStack                                           | 3.9.3      | D11. `zenstack/schema.zmodel` is the source of truth; `PolicyPlugin` enforces its `@@allow` rules. |
| `pg`                                               | 8.x        | ZenStack 3's Postgres driver. v3 has its own ORM — Prisma is not in the request path.              |
| Prisma                                             | (indirect) | Migrations only, through `@zenstackhq/cli`. Never imported by this package.                        |
| Zod                                                | 4.5.4      | tRPC input validation.                                                                             |
| `@vaam-apps/vpay-sdk`, `@vaam-apps/vpay-stripe-js` | workspace  | The merchant SDK and the browser SDK.                                                              |

**ZenStack 2 → 3, on 2026-09-06, and what it actually changed.** v3 is a
rewrite: it replaced Prisma at runtime with its own ORM over Kysely, so
`@prisma/client` and `@zenstackhq/runtime` are gone from this package and
`pg` is here instead. The **query surface is unchanged** — v3's ORM is
Prisma-API-compatible, which is why `zenstack-store.ts` reads as it did — and
so is the ZModel: the `@@allow` rules were not touched. What did change:

- `enhance(new PrismaClient())` became `new ZenStackClient(schema, {dialect})`
  `.$use(new PolicyPlugin())`. The policies are a plugin now, and a client
  without it enforces nothing — which is why `policies.test.ts` asserts the
  plugin is installed.
- The schema moved to `zenstack/schema.zmodel` (the v3 CLI's default) and
  `zen generate` writes `zenstack/{schema,models,input}.ts` there. Those are
  **generated, not committed** — the repository's `.gitignore` says why —
  where `prisma/schema.prisma` used to be committed because the container's
  entrypoint read it. It no longer does.
- `prisma migrate deploy` became `zen migrate deploy`, and the migrations
  moved with it to `zenstack/migrations/`. Prisma Migrate is still what
  applies the SQL; v3 derives a Prisma schema from the zmodel on the fly and
  drives it. **The migration files themselves were not rewritten** — the same
  three `migration.sql` files, moved.
- **Two things got worse, and they are recorded rather than smoothed over.**
  A denied _bulk_ write no longer throws: `product.deleteMany({})` resolves
  `{count: 0}` where v2 threw, so the effect is the same and the shape is
  not. And the policy decision now needs a database, so the refusals this
  package used to prove offline are no longer proven by anything that runs in
  CI. Both are measured, and both are in `policies.test.ts`'s header with the
  numbers.

The v2 note this replaces — "Prisma 6, not 7, because ZenStack 2.22.3 emits a
`datasource` with `url = env(...)` which Prisma 7 rejects (`P1012`), and
ZenStack 3 is still beta on npm" — is **retracted, dated 2026-09-06**: v3 is
not beta any more (`@zenstackhq/orm@3.9.3` is `latest`; it is
`@zenstackhq/runtime` that stopped at `3.0.0-beta.13`, because the package was
renamed), and Prisma's version no longer constrains this package because
nothing here imports it.

**Four `pnpm.overrides` came with this package**, all for advisories no
version bump clears: `@prisma/config>deepmerge-ts` (GHSA-ggr8-5vv4-36mx,
high) and three `lodash` entries under chevrotain. Both survive the v3
upgrade and for the same reasons, though the paths changed: `@prisma/config`
now arrives through `@zenstackhq/cli`'s own Prisma dependency rather than
through `@prisma/client`, so it is a _build_-time path and no longer a
production one; and chevrotain still arrives under `langium`, which
`@zenstackhq/language` uses. The root `package.json` carries the dated
justification for each.

**Why no `@trpc/react-query`.** Three call sites and one poll loop. A query
cache would be a dependency and a provider for no behaviour.

**Why no `@vpay/ui`.** It is vpay's own design system; a merchant integrating
vpay would not have it, so a demo built out of it would be demonstrating
something no merchant can reproduce.

## ZenStack, honestly

`zenstack/schema.zmodel` compiles to the TypeScript ZenStack 3's ORM reads
(`zenstack/{schema,models,input}.ts`, generated by `postinstall`, not
committed). The `@@allow` rules are the ones that are **true of every
caller**, because D13 gives this shop no authenticated principal:

- the catalogue is read-only to the application (it is seed data);
- `webhook_events` is append-only;
- no order or order line is ever deleted.

`src/server/store/policies.test.ts` proves that those are still the rules and
that the client the shop holds still has the plugin that enforces them — and
its header states plainly what it **stopped** proving at the v3 upgrade,
which is the refusals themselves. Those need a database now, and CI's `web`
job has none. The refusals were run by hand against a real Postgres on
2026-09-06 and the results are in that header.

What the policies do **not** do is stop one payer reading
another payer's order: there is no principal to compare against, and
`orders.get` is reachable by anyone holding an order id. A real shop would
put a session or a signed order token in front of it. This one says so
instead of writing a rule that would evaluate to `true`.

## Tests, and what they do not cover

`pnpm --filter @vpay-examples/shop test` — **93 cases** (measured
2026-09-06), no database, no Docker, no network.

- `src/server/orders.test.ts` runs a **real `VpayClient`** against a real
  local HTTP server and asserts the bytes: the amount, the
  `Idempotency-Key`, the exact session parameters (including the literal,
  unescaped `{CHECKOUT_SESSION_ID}`), the ids stored, that `popup` sends the
  hosted session's own key, that a retry is a _new_ order with its own keys,
  that a cancel reaches vpay and writes nothing here, and that the buyer's
  e-mail never goes on the wire to vpay.
- `src/server/webhook.test.ts` covers the four rules and the failure columns:
  the code and the message are carried onto the order, an event with no
  `last_payment_error` still fails it with nulls, and neither an
  already-settled order nor a replayed event id is ever stamped. The
  signature is built by an independent implementation of the header grammar
  rather than by the verifier under test.
- `src/lib/failures.test.ts` reads `vpay-core/src/failure.rs` and fails if the
  eleven codes, `payer_actionable` or `merchant_actionable` move out from
  under this shop's copy.
- `src/lib/test-numbers.test.ts` parses this document's own rail tables and
  fails if they and `src/lib/test-numbers.ts` disagree in either direction.
- `src/server/store/policies.test.ts` covers the ZenStack policy **rules**
  and that the plugin enforcing them is installed — and says in its own
  header what the v3 upgrade cost it.
- `src/testing/no-runtime-imports.test.ts` is the TypeScript counterpart of
  `cargo xtask verify-no-mocks`: nothing under `src/app`, `src/server`,
  `src/components` or `src/lib` may name `src/testing`.

**The rail stubs' side is proven in Rust, not here.** That the numbers above
actually reach those outcomes is
`a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin` (MTN, four
cases) and `a_test_number_typed_on_the_rails_hosted_page_reaches_the_documented_outcome`
(Orange, three cases) in `backends/tests/conformance`, which drive the real
adapters against real WireMock containers. A test in this package could only
have asserted that the table says what it says.

**Not covered by unit tests:** `ZenStackShopStore` (`PrismaShopStore` until
2026-09-06). The tests run against an
in-memory implementation of the same `ShopStore` port so that they need no
Postgres in CI's `web` job, which has none. The database-backed
implementation was verified by hand on 2026-09-04 against the built image and a real Postgres —
migrations applied, catalogue seeded and rendered, a signed delivery
`applied`, its replay `duplicate` with no second `webhook_events` row, an
unknown intent 2xx with no write. That hand-run is the only evidence there
is: **no automated test covers `ZenStackShopStore` anywhere in this
repository.** `just demo` does not, whatever it may look like — it brings the
`vpay-shop` container up, waits for its healthcheck and prints its URL, and
never places an order (`docs/runbooks/demo.md` §5 lists "that the shop works"
under _does not prove_). Lane 6's Cypress specs — `shop-hosted.cy.ts` and
`shop-embedded.cy.ts` — **have** merged, and they drive this class in a real
browser against the demo stack; but they assert on the shop's pages, never on
the store class itself, so the sentence above stands: no unit or integration
test covers it.

Re-run by hand on **2026-09-06** after the ZenStack 3 upgrade, against a
throwaway `postgres:16-alpine`: `zen migrate deploy` applied all three
migrations from empty (and a `zen migrate dev --create-only` afterwards
produced an _empty_ migration, so the zmodel and the migrations agree); the
catalogue seeded; `createOrder` with a **null** e-mail returned an order with
its line joined to the product name; `applyWebhookEvent` wrote `failed` with
`insufficient_funds` / `NOT_ENOUGH_FUNDS`; a later `payment_intent.succeeded`
for the same intent answered `already_settled` and left both the status and
the failure columns alone; an event naming an unknown intent answered
`unknown_intent` and wrote nothing.

Also not covered by unit tests: the React components, and the browser end of
`initEmbeddedCheckout` (that is `@vaam-apps/vpay-stripe-js`'s own suite, and
lane 6's merged `shop-embedded.cy.ts`).

**Nothing here or anywhere else drives the popup surface in a real browser.**
`@vaam-apps/vpay-stripe-js`'s `src/popup.test.ts` drives it against stub
windows — jsdom implements neither `window.open` nor cross-window
`postMessage` — and no Cypress spec covers it. `CheckoutForm`'s popup branch,
including the blocked-window fallback, has been read and not run by a test.

## The image

```bash
docker build -f examples/shop/Dockerfile .        # from the repository root
```

Root context, because `@vaam-apps/vpay-sdk` and `@vaam-apps/vpay-stripe-js` are workspace
dependencies. Standalone output, non-root (`node`, uid 1000), read-only root
filesystem with a tmpfs on `/tmp`, and an entrypoint that runs
`zen migrate deploy` before the server — so a container that starts has a
schema and a catalogue, or it does not start. The entrypoint copies
`/app/zenstack` to `/tmp` first, because the CLI derives a temporary Prisma
schema beside the zmodel and the root filesystem is read-only.
