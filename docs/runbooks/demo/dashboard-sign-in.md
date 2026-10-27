# The demo — §6. Reading the dashboard's data

> **Rewritten 2026-10-26 (VP-6a,
> [ADR-0036](../../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).**
> This section used to say how to sign in to the dashboard with a password and a
> TOTP code. Both are gone, with the session, the authorization code and the
> bearer token they led to. **The dashboard application cannot sign in at all
> until VP-6b** ships the console's own key generation and signing, and vaam's
> admin-phone pairing. What follows is what works today.

## Which payments

**`/dash/v1` shows the SHOP's payments.** It does not show `just demo-walk`'s,
and that is a choice rather than a gap: a request reads exactly one tenant's rows
— the one `dashboard.merchant_id` names, checked at boot and filtered on in every
query ([dashboard.md](../../flows/dashboard.md)) — and this stack registers two
merchant clients on two tenants (`ConfigError::DuplicateMerchantId` refuses to
merge them).

| client          | tenant                 | who pays as it                         |
| --------------- | ---------------------- | -------------------------------------- |
| `shop-merchant` | `shop-merchant-tenant` | `examples/shop`, and the browser demos |
| `demo-merchant` | `demo-merchant-tenant` | `just demo-walk`                       |

To look at the walkthrough's payments instead, move the binding on **both**
commands, because the member has to exist in the tenant the dashboard is bound
to:

```bash
just demo_dashboard_merchant=demo-merchant-tenant demo-up
just demo_dashboard_merchant=demo-merchant-tenant demo-staff
```

`just gen-demo-keys` regenerates an overlay whose `dashboard.merchant_id` no
longer matches, so nothing has to be cleaned up by hand.

## Reading it by hand: the break-glass

```bash
just demo-staff        # creates ada@example.test, writes the stf_… id to .e2e/vpay-demo/staff-id.txt
```

`demo-staff` creates the member and **no credential**. A member reads `/dash/v1`
once a console key is attached to them, and the way to attach one without the
phone pairing is the operator's command:

```bash
docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml run --rm --no-deps -T vpay-server \
  staff pair-key --staff "$(cat .e2e/vpay-demo/staff-id.txt)" --public-key <SEC1 uncompressed P-256 key, base64url>
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

The `demo_staff_token_ttl` override, the OAuth redirect URI the overlay used to
register on `demo_dashboard_port`, the pepper and TOTP key `gen-demo-keys`
generated, `staff-password.txt` and the Cypress `dashboard.cy.ts` spec that
signed in with all of them. The dashboard container still starts with the rest
of the stack and renders one notice page saying the console signs in with a
paired key, which is not available until VP-6b.
