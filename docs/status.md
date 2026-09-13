# STATUS

## Current limits

> **vpay is a scaffold. Do not deploy it.**

The project has not called a real MTN or Orange endpoint, prompted a real payer,
moved money, delivered to an external merchant endpoint, or run on Kubernetes.
Rails and webhook receivers used by tests are WireMock hosts reached over HTTP.

What exists today:

- Merchant authentication, payment intents, customers, invoices, events, and
  account-holder lookup are implemented and tested against local infrastructure.
- MTN MoMo and Orange Money adapters, worker settlement, and signed webhooks
  are tested against WireMock.
- Hosted and embedded checkout, dashboard sign-in, tenant-bound and admin
  payment reads, and the merchant SDKs are implemented. A CrateStack procedure
  transport exists alongside the dashboard's current `GET` reads; SDK parity is
  checked by `just verify`.
- Password and TOTP credentials use the generic credential model. Six other
  credential kinds are modeled but not implemented.
- Checkout and dashboard Storybooks run axe in a real browser.
- Images, Compose, Helm, migrations, and monitoring configuration are tested,
  but no deployment or monitoring system has operated vpay.

The work still required before a production claim includes real-rail validation,
external merchant webhook delivery, deployment validation, browser enforcement
of checkout framing controls, ingress protection for unauthenticated surfaces,
and refund support for MTN. Modeled credential kinds beyond password and TOTP
also remain unimplemented.

## Verification

Run `just verify` for repository invariants and `just ci` before review. The
`justfile` is the authoritative list of checks. `verify-docs` reports document
volume but does not pass or fail the build.

### Unimplemented items tracked by `verify-status`

- `mtn_momo::refund`
