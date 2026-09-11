# The demo — §4a. Opening the hosted page, the shop, and the failure numbers

_Moved out of [docs/runbooks/demo.md](../demo.md) on 2026-09-11 by exp57, which split a 1 426-line runbook into the procedure and its steps. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a `§` cross-reference pointed at a section that is now on another page, the page it moved to._

## 4a. Opening the hosted page

New in Step 9, and the only part of this runbook that needs a browser.

`just demo`'s step 5 ends by printing two things. The first is a URL — in the
run pasted above:

```
      HOSTED — open this in a browser:

        http://localhost:3080/c/cs_938t20sg8x07x5c08nh7nk7f?key=pk_test_demomerchantsandbox01#cs_938t20sg8x07x5c08nh7nk7f_secret_1ex13vm04s2bhbk6rbxyvahpkh79zfkd
```

**Open it.** That is vpay's own payment page, served by the `vpay-checkout`
container on `demo_checkout_port` (3080 by default). It shows 5 000 FCFA, the
merchant's name if the overlay configured one, and a rail selector; MTN asks for
a phone number and Orange sends you to the rail's stub page. The steering
numbers are the same ones the walkthrough uses, with one difference that
matters:

| What you want                | Type this MSISDN | Why not the one the demo prints                                                                                                                                                                                                                                                                               |
| ---------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The payment succeeds         | `237600000100`   | The page validates Cameroon E.164 — `237`, then `6`, then eight **digits** — and correctly refuses `237600000ce0`, which has hex letters in it. Both numbers enter the same WireMock scenario by the same mapping (Step 9, lane 2b)                                                                           |
| The payer has no balance     | `237600000101`   | as above, twin of `237600000f01`                                                                                                                                                                                                                                                                              |
| The prompt expires           | `237600000102`   | as above, twin of `237600000f02`                                                                                                                                                                                                                                                                              |
| The payer refuses the prompt | `237600000103`   | New 2026-09-10 ([issue #59](https://github.com/vaam-apps/vpay/issues/59)); digits-only and no hex twin, because the hex family predates the page's validator. It arms `mtn-demo-refused`, which answers `FAILED`/`PAYMENT_NOT_APPROVED` → `payer_declined` — a code that until that day no rail could produce |

For Orange, pick it in the selector and follow the redirect: you land on the
**rail's** stub hosted page on `demo_orange_port`, which has a Pay link and a
Cancel link, and either one brings you back to vpay's return page. Both links
carry the same URL — the stub has no cancel semantics of its own — so what you
are seeing is the return trip, not a cancellation.

The second thing step 5 prints is the embedded session, and its `client_secret`
is **redacted**:

```
      EMBEDDED — what a merchant's own page does with it:

        import { initEmbeddedCheckout } from '@vaam-apps/vpay-stripe-js';
        const checkout = await initEmbeddedCheckout({
          publishableKey: 'pk_test_demomerchantsandbox01',
          fetchClientSecret: async () => '[67 chars redacted]',
        });
        checkout.mount('#vpay-checkout');
```

That is deliberate, and it is the same treatment step 2 gives the access token:
this output ends up in CI logs and in pasted transcripts. The real value is one
call away — `vpay.checkout().sessions().retrieve("cs_…")` — and the page it
belongs to will only mount from an origin in that merchant's
`checkout_origins`. **Two mechanisms hold that, and only one of them has been
watched working:** vpay serves `Content-Security-Policy: frame-ancestors <that
list>` on the page, and the page independently compares its own framer against
the same list. It is the second that a browser has been observed performing —
[checkout.md](../checkout.md) §5 has the measurement, and Cypress strips the
header before a browser ever sees it. To _see_ the embedded mode working rather
than read about it, use the shop's own embedded page (below): it is a
registered origin and the demo merchant's `demo-merchant` is not.

**Why the hosted `url` is printed in full and the embedded secret is not.** The
hosted URL carries its credential in the **fragment**, which a browser never
sends to a server, never writes to an access log and never carries across a
redirect — you cannot use it without pasting it into an address bar, which is
exactly what this section asks you to do. The embedded secret is a bare
credential in a code sample. Same class of value, two different jobs.

### The shop, which is the whole end-to-end demo

```bash
just demo-shop      # prints http://localhost:3001
```

`examples/shop` is a merchant's own site: a catalogue in FCFA, a cart, a
checkout that creates a PaymentIntent and a hosted session **server-side**
through `@vaam-apps/vpay-sdk`, and an order page that turns `paid` only when vpay's
**webhook** lands — never from the return trip. The step-by-step walkthrough is
[checkout.md](../checkout.md) §"Buying something in the demo shop", which is also
where a merchant integrating vpay should start.

Two things about it that will otherwise surprise you:

- **Its database is created once, on a fresh volume.**
  `deploy/dev/postgres-init/10-shop-database.sql` runs from Postgres's
  entrypoint, which executes that directory exactly once on an empty data
  directory. A `pgdata` volume from before Step 9 has no `shop` database and
  `vpay-shop` dies in `zen migrate deploy`. **`just demo-down` removes
  volumes** (`down -v`), so tearing the stack down is the fix; nothing here
  creates the database defensively on every boot, because that would hide a
  stale volume rather than report one.
- **Its tables and its catalogue come from `zen migrate deploy`**, run by the
  container's entrypoint before the server starts. Idempotent, so a restart is a
  no-op:

  ```console
  $ DEMO_COMPOSE="-f compose.yml -f compose.e2e.yml -f compose.demo.yml"
  $ docker compose $DEMO_COMPOSE logs vpay-shop | head -4
  vpay-shop-1  | vpay-shop: applying migrations
  vpay-shop-1  | 3 migrations found in prisma/migrations
  vpay-shop-1  | Applying migration `20260904091557_init`
  vpay-shop-1  | Applying migration `20260904091600_seed_catalogue`
  vpay-shop-1  | Applying migration `20260906120000_optional_email_and_failure_columns`
  ```

  The transcript above is from before the ZenStack 3 upgrade of 2026-09-06
  (three migrations, not two, since the same day). It is `zen migrate deploy`
  now, and the "prisma/migrations" in the second line is **Prisma Migrate's
  own wording**, not a path that still exists — v3 drives Prisma Migrate from
  a schema it derives from the zmodel, and the files are in
  `examples/shop/zenstack/migrations/`.

### Three ways to pay, and the numbers that make a payment fail

Since 2026-09-06 the shop's `/checkout` page offers all three integration
surfaces and a panel of the demo's **test numbers**. Both are worth a minute,
because between them they are most of what a merchant actually has to build.

- **The surface switch** (Redirect / Popup / Embedded) starts on whatever
  `SHOP_CHECKOUT_MODE` names — `hosted` by default, and the demo stack does
  not set it. A real merchant picks one in configuration and ships no switch;
  this one exists so a reader can see each. Redirect and Popup are the _same_
  hosted session: the popup opens `session.url` in a window the shop owns, and
  the shop's own return page — running inside that window — tells the opener
  and closes it.
- **The test numbers** are documentation MSISDNs the rail stubs are configured
  to answer particular things for. `237600000000` (or anything unlisted) pays;
  `237600000101` is insufficient funds on MTN; `237600000102` is a timeout on
  either rail; `237600000103` is a payer who refuses the prompt on MTN;
  `237600000400` is a refusal on either; `237600000503` is an unavailable rail
  on MTN. The full table, including the **four** outcomes Orange cannot
  express, is on the page itself and in
  [../../examples/shop/README.md](../../../examples/shop/README.md).

  **`just demo-walk` does not send `237600000103`, so this runbook's six
  outcomes do not include `payer_declined`.** The walkthrough drives the hex
  family (`237600000ce0`, `…0f01`, `…0f02`) plus two Orange amounts, and that
  is what the transcript in §4 and every "six outcomes" count on this page
  mean. `237600000103` is reached by typing it into the checkout page in a
  browser, and it is proven — against a real WireMock container, through the
  adapter's `submit` and `query_status` — by
  `a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin` in
  `backends/tests/conformance/tests/adapter_conformance.rs`. No browser test
  types it either; `checkout.cy.ts` drives the hex family.

  Adding a seventh payment to `examples/merchant-demo` would make the
  walkthrough cover it, and would also invalidate §4's pasted transcript and
  the "six outcomes for six" wording in the measurement records further down
  this page — those are dated records of runs that happened, not
  descriptions to be edited. **That trade is left to the maintainer**
  (2026-09-11).

Two things to know before you drive them:

- **MTN's number goes on vpay's page** (it is a push rail); **Orange's goes on
  the rail stub's own hosted page**, in the "Or pay with one of the demo's
  test numbers" form beside the Pay link — Orange is a redirect rail and vpay
  never sees a number.
- **Orange's numbers work from a browser, and did not until 2026-09-10**
  ([issue #58](https://github.com/vaam-apps/vpay/issues/58)). vpay's confirm
  handler enqueues the first status query at `now()` — `poll_delay(0)` is the
  delay before the _second_ attempt — and the worker's idle sleep is a second,
  so the stub's catch-all used to answer `SUCCESS` and the order was **paid**
  before you could reach the form, whatever number you were about to type.
  Measured on 2026-09-06 from the stub's own journal: submit at T, first
  `transactionstatus` at T+449 ms, the form at T+12 s, order `paid` for
  `237600000400`.

  What changed is the **stub**, not vpay: nothing about the first poll moved,
  because a charge being asked about as soon as it exists is a deliberate
  property (`docs/flows/crash-safety.md`). The rail stub now answers `PENDING`
  once from the submit and four more times once your browser has actually
  loaded its page — about 105 seconds on the worker's ladder — and then
  `EXPIRED`. So: **type the number and press the button; do not leave the tab
  and come back after two minutes**, or you will get `payer_timeout` whatever
  you typed, which is also what the Cancel link now gives you. None of those
  seconds is a fact about Orange — see
  [../flows/adapter-orange-money.md](../../flows/adapter-orange-money.md).

  MTN's numbers are unaffected by any of it — a push rail carries the number
  in the merchant's own submit, so there was never a window to lose.

  **Drive one payment at a time.** The window is a single WireMock scenario on
  a single container, keyed on nothing per charge, because WireMock scenarios
  cannot be. Measured 2026-09-10: with two Orange charges in flight the second
  never gets its `PENDING` rung, and a payer clicking Pay or Cancel on one
  charge's page decides whichever charge the worker asks about next — a `5001`
  charge included, whose amount-keyed mapping the payer-action mappings
  outrank. `just demo-walk` is strictly sequential and opens no page, so it
  never meets this; two browser tabs on two orders will. See
  [../status.md](../../status.md) §"The Orange stub's hosted page grew a payer's
  window" for the measurements.

The one outcome no _number_ reaches is `cancelled`, because it is not a rail
outcome at all. Clicking "cancel" on the rail's page ends the payment, but
what the rail then reports is `EXPIRED`, so the order comes back **`failed`**
with `payer_timeout` — Orange documents no `CANCELLED` and the stub will not
invent one. (Before 2026-09-10 that link went straight back to the merchant,
the stub never learned you had clicked it, and the order came back `paid`.)

The order page's "Cancel this payment" button reaches
`POST /v1/payment_intents/{id}/cancel`, the intent becomes `canceled` at vpay,
and **since 2026-09-10 vpay emits one `payment_intent.canceled` in that same
transaction** ([issue #57](https://github.com/vaam-apps/vpay/issues/57)) — so
the fan-out delivers it and the shop's webhook handler moves the order to
`cancelled` from the signed event, exactly as it does for every other status.

_(This paragraph said the opposite until 2026-09-10, and the measurement
behind it was right at the time: on 2026-09-06 the intent's row really did
become `canceled` with the `events` table unchanged, because the vocabulary
carried `payment_intent.canceled` and nothing wrote it. The shop's code did
not change when the event arrived — that is the point of settling from
events. Written up in
[../plans/exp22-shop-demo-notes/opus.md](../../plans/exp22-shop-demo-notes/opus.md).)_

### One currency, and what it is not saying

Every payment in this runbook is **XAF**, on both rails. Until Step 9 the MTN
ones were EUR, and the reason they were is unchanged: **MTN's real sandbox
rejects XAF** ([../flows/money.md](../../flows/money.md)), which is why
`config/application.yml` still puts `mtn_momo` on `currency: EUR` and why
`application-sandbox.yml` inherits it.

What changed is the _demo overlay_ — `.e2e/application-demo.yml`, which `just
gen-demo-keys` writes — and only it. That stack does not talk to MTN's sandbox;
it talks to a WireMock host whose mappings match on no currency at all. The demo
shop prices its catalogue in XAF, offers a payer both rails, and `vpay_api`'s
`currencies_agree` refuses a confirm whose rail settles in another currency than
the intent — so one currency for both rails is what makes the shop's MTN button
payable.

`gen-demo-keys` regenerates the overlay when it stops settling `mtn_momo` in
XAF, and that check is an `awk` range over that provider's own sequence item
rather than a grep for the provider's name — the name is present in an overlay
edited back to EUR, which is the one state the check exists to catch.

**Do not read this page as "MTN accepts XAF".** It does not.
