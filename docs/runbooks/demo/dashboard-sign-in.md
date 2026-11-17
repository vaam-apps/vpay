# The demo — §6. Reading the dashboard's data

> **Rewritten 2026-10-26 (VP-6a,
> [ADR-0036](../../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)),
> and the password sign-in is back since 2026-10-30.** VP-6a removed the password
> and TOTP sign-in with the session, the authorization code and the bearer token
> behind it. The `human_login` method (VPA-3) and the dashboard application's
> half of it (VPA-5a) restored it, and the generated demo overlay serves it since
> VPA-5c: "Signing in with a password" below is how. **What the dashboard cannot
> do is read `/dash/v1`:** it has no payments pages until VP-6b, so a signed-in
> person sees an account page and nothing else, and the paired key is still the
> way to read the data by hand.

## Signing in with a password

```bash
just demo-up                  # the overlay serves human_login; its secrets are generated into .e2e/staff-auth.env
just demo-staff               # creates ada@example.test and keeps the one-time password in .e2e/vpay-demo/staff-password.txt
```

Open **`http://localhost:3000`**, not another name for the same machine: the
session cookie is `__Host-` prefixed and `Secure`, and a browser keeps one over
plain `http` only on `localhost`. Then, one question per screen: the email
(`ada@example.test`), the password (`cat .e2e/vpay-demo/staff-password.txt`),
the authenticator — a first sign-in shows a QR code and, behind a button, the
key to type, and the six-digit code it produces — and a new password of at least
twelve characters, replacing the one-time one. You land on an account page that
says who you are and which merchant you read, with Sign out. The one-time password is printed once and only its hash is stored;
if the file is lost, `just demo-down` and start again. There is no single
sign-on button: the demo stack has no identity provider.

## Which payments

**None — the dashboard reads no `/dash/v1` yet.** It signs a staff member in and
lands on an account page, and no page reads the payments API until VP-6b. This
supersedes the older distinction this section used to draw between the SHOP's
payments and `just demo-walk`'s: even `demo_dashboard_merchant`'s binding to one
tenant (`dashboard.merchant_id`, checked at boot and filtered on in every query —
see [dashboard.md](../../flows/dashboard.md)) cannot be _seen_ anywhere until a
payments page exists. The two tenants are still registered and still distinct —

| client          | tenant                 | who pays as it                         |
| --------------- | ---------------------- | -------------------------------------- |
| `shop-merchant` | `shop-merchant-tenant` | `examples/shop`, and the browser demos |
| `demo-merchant` | `demo-merchant-tenant` | `just demo-walk`                       |

— but `demo_dashboard_merchant` only chooses which tenant's rows the _future_
payments page will read, so moving it now (`just demo_dashboard_merchant=
demo-merchant-tenant demo-up` + `demo-staff`) is preparing for VP-6b rather
than making the walkthrough's intents visible today. The way to read either
tenant's data by hand is the console key in the next section.

## Reading it by hand: the break-glass

```bash
just demo-staff        # creates ada@example.test and keeps the one-time password in .e2e/vpay-demo/staff-password.txt
```

`demo-staff` creates the member and a password credential. A member **reads
`/dash/v1`** only once a console key is attached to them (a password session
opens the dashboard application's sign-in and nothing more), and the way to
attach one without the phone pairing is the operator's command. It needs the
member's `stf_…` id, which `demo-staff` no longer keeps now that it prints a
password; read it from the database:

```bash
docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml exec postgres \
  psql -U vpay -d vpay -Atc "select id from staff_members where email = 'ada@example.test'"
docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml run --rm --no-deps -T vpay-server \
  staff pair-key --staff <that id> --public-key <SEC1 uncompressed P-256 key, base64url>
```

It prints `<staff id> <key id> <thumbprint> <expires_at>` and the key is good
for 30 days. To end it sooner, with the thumbprint it printed:

```bash
docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml run --rm --no-deps -T vpay-server \
  staff revoke-key --thumbprint <the 64 hex characters>
```

It takes effect on the key's next request and is idempotent. A client that holds the private half signs each request as
[dashboard-auth.md](../../flows/dashboard-auth.md) describes. There is no
browser that does this yet; that is VP-6b.

## What is not here any more

The `demo_staff_token_ttl` override (a thirty-second access token for a spec
that waited out an expiry) and that spec's cases: the payments, refunds and
deliveries pages it read do not exist until VP-6b. The OAuth redirect URI on
`demo_dashboard_port`, the pepper and TOTP key `gen-demo-keys` generates,
`staff-password.txt` and a Cypress spec that signs in with all of them came back
with VPA-5c (`dashboard-human-login.cy.ts`).
