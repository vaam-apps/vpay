# The demo — §6. Signing in to the dashboard

_Moved out of [docs/runbooks/demo.md](../demo.md) on 2026-09-11 by exp57, which split a 1 426-line runbook into the procedure and its steps. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a `§` cross-reference pointed at a section that is now on another page, the page it moved to._

## 6. Signing in to the dashboard

~~The dashboard is out of scope, and why~~ — **rewritten 2026-09-07 (exp28).**
This section said "there is no data source to show", and that was true right up
to the moment `/dash/v1` and the staff sign-in landed. The dashboard is a real
screen now, it shows real payments, and this section says how to get into it.

### Which payments, and why not the walkthrough's

**This dashboard shows the SHOP's payments.** It does not show
`just demo-walk`'s, and that is a choice rather than a gap.

A `/dash/v1` request reads exactly one tenant's rows — the one
`dashboard_client.merchant_id` names, checked at boot and filtered on in every
query ([dashboard.md](../../flows/dashboard.md)). This stack registers **two**
merchant clients, on two tenants, and D12 gave them separate credentials on
purpose so neither walkthrough can break the other:

| client          | tenant                 | who pays as it                         |
| --------------- | ---------------------- | -------------------------------------- |
| `shop-merchant` | `shop-merchant-tenant` | `examples/shop`, and the browser demos |
| `demo-merchant` | `demo-merchant-tenant` | `just demo-walk`                       |

They cannot be merged into one tenant: `ConfigError::DuplicateMerchantId`
refuses a config where two `merchant_clients` share a `merchant_id`, because
two credentials on one tenant could read each other's objects. So the
dashboard shows one of the two, and `demo_dashboard_merchant` is which —
defaulting to the shop's, because the shop is the surface a person clicks.

**Until 2026-09-11 it defaulted the other way, and the consequence was
reported rather than predicted:** a payment made by hand through the shop was
absent from the list, and its id answered `404` on the detail page. Nothing
was broken — the dashboard was showing its tenant, correctly, and its tenant
was the one nobody had clicked anything in. `dashboard.cy.ts` now buys a tote,
pays for it, and asserts the payment is there, so this cannot go back.

To look at the walkthrough's payments instead, move the binding — and move it
on **both** commands, because the staff member has to exist in the tenant the
dashboard is bound to or `/authorize` refuses to mint a code for it:

```bash
just demo_dashboard_merchant=demo-merchant-tenant demo-up
just demo_dashboard_merchant=demo-merchant-tenant demo-staff
```

`just gen-demo-keys` regenerates an overlay whose binding no longer matches,
the same way it does for a moved `demo_dashboard_port`, so nothing has to be
cleaned up by hand. What the override does **not** do is keep `just test-e2e`
green: the spec above asserts the shop's payment is visible, and under the
override it is not. That failure is the guard working.

**Both commands, and that is not belt-and-braces.** The staff member has to be
created in the tenant the dashboard is bound to or `/authorize` refuses them —
the server says so in as many words, `a staff member signed in against a
deployment whose dashboard is bound to another merchant; refusing to mint a
code for a tenant they may not read` — and `demo-staff` on its own resolves
the binding to its default. `just test-e2e` had the same gap internally until
2026-09-11: it forwards this variable to its own `demo-staff` call now, so
that under the override the dashboard spec fails on the payment it cannot
find rather than on a sign-in it cannot complete. Measured both ways;
[../plans/exp51-demo-tenant-notes/opus-review.md](../../plans/exp51-demo-tenant-notes/opus-review.md)
§3 has the transcript.

### Create the staff member

There is **no sign-up**. ADR-0017 decision 1: a dashboard account is a
decision an operator takes, not a form a visitor fills in, and
`vpay-server staff add` is the only thing that creates one. Against a stack
that is already up:

```bash
just demo-staff
```

It runs `staff add` as a one-off container inside the stack — same
configuration, same database — and writes the **one-time password** it printed
to `.e2e/<demo_project>/staff-password.txt`, mode 0600 and git-ignored.

```console
$ just demo-staff
demo-staff: created ada@example.test for shop-merchant-tenant; the one-time password is in .e2e/vpay-demo/staff-password.txt
$ cat .e2e/vpay-demo/staff-password.txt
```

> **The password is printed once.** Only its argon2id hash is stored, so if you
> lose the file the account cannot be recovered — `just demo-down` (which
> deletes the volumes) and start again. Running `just demo-staff` twice hits
> the `staff_members_email_key` unique index; the recipe says so and keeps the
> file from the first run.

If you moved ports, the sub-invocation needs them too — `just
demo_project=vpay-demo-b demo_port=18088 demo-staff`.

### Sign in

The dashboard is on **http://localhost:3000** by default. Since 2026-09-10
(issue #78) that is `demo_dashboard_port`, a variable like every other demo
port, so a stack brought up as `just demo_dashboard_port=13000 demo` serves it
on 13000 instead — and `just demo-dashboard`, or `just
demo_dashboard_port=13000 demo-dashboard`, prints the origin for whichever
stack you mean rather than making you remember.

Moving it moves three other things with it, which is why it took until #78 to
become a variable: the dashboard app's own `VPAY_DASHBOARD_REDIRECT_URI`, the
`redirect_uris` the OP has registered in the generated overlay, and Cypress's
`baseUrl`. authkestra matches a redirect URI byte for byte, so a stack whose
overlay disagreed with its app would answer every sign-in with a 400 at
`/authorize` naming `redirect_uri`. `just gen-demo-keys` writes the overlay
from the same variable and regenerates one that names a different port, so
they cannot drift — that is the whole reason a _generated_ overlay makes this
possible at all.

1. **Work email and password.** `ada@example.test` and the contents of the
   file above.
2. **Set up your authenticator.** This is a _first_ sign-in, and enrolment is
   mandatory — a session never reaches `authenticated` while
   `staff_members.totp_secret` is `NULL`. Scan the QR with any TOTP app, or
   type the base32 key shown beneath it. Nothing is written to your account
   until the next step succeeds, so a failed scan is not a lockout: go back to
   `/login` and start again for a fresh secret.
3. **Enter the six-digit code.** This is what commits the enrolment.
4. **Choose a password.** The printed one is refused by every authenticated
   route until it is replaced, `/oauth/authorize` included, so there is no
   `/dash/v1` token at all until this is done. Twelve characters minimum;
   length is the only rule.
5. You land on **/payments**, listing the intents made for
   `shop-merchant-tenant` — everything bought through the shop, and anything
   `examples/checkout-browser` minted, which defaults to the same merchant.
   Click an id for the charge, the refunds, the last error and the event
   timeline. `just demo-walk`'s payments are **not** here; see "Which
   payments, and why not the walkthrough's" above.

**"Sign out" is a real revocation**, not a cookie clear: it deletes the
`staff_sessions` row, which is the only place the dashboard's server can read
the `/dash/v1` access token from. The JWT itself stays cryptographically valid
until it expires — ADR-0017's Consequences says so rather than letting
"revocation" carry more weight than it can.

**This stack's `/dash/v1` tokens live thirty seconds, and a real deployment's
live 900.** `demo_staff_token_ttl` is the variable
(`staff_auth.access_token_ttl_seconds` in the generated overlay), and the
difference is deliberate: at 900 seconds nothing that runs in under a quarter
of an hour ever crosses an expiry, so the re-mint the dashboard performs — and
the fifteen-minute bug it exists to fix (issue #88 item 1) — was exercised by
no browser run at all. At thirty, clicking around the demo for half a minute
does it. You will not notice: the app replaces the token when a fifth of its
life is left, so a page never waits on a refused read. `just
demo_staff_token_ttl=900 demo` gets the production number and regenerates the
overlay to match.

### What you will see that looks wrong and is not

- **The payer column is an em dash.** `charges.payer_ref_masked` is never
  written by anything (`docs/status.md`), so the detail page renders the column
  as null rather than deriving a mask from the payer's unmasked number. The day
  the column is written the value appears.
- **The list has a "Methods" column and no "Rail" column.**
  `GET /dash/v1/payment_intents` returns no charge, so the only rail-shaped
  value there is the set of rails the intent _may_ be confirmed against. The
  rail that actually took it is on the detail page.
- **There is no page count.** The API serves cursors and a `has_more`, and no
  count anywhere.

### Two things it still cannot do

**Anything at all to a payment.** `/dash/v1` refuses every non-`GET` method at
the boundary, before the router matches. There is no re-poll, no replay, no
refund and no annotation — and therefore no `audit_log`, because there is
nothing yet to audit (ADR-0008 wants one row per dashboard write).

**Any other slice.** Webhooks, checkout sessions, balances, settings and rail
health are not built, and the navigation does not link to them —
`frontends/apps/dashboard/src/layout.test.tsx` fails if it ever does.
