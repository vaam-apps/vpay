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
- Hosted and embedded checkout, dashboard sign-in and payment reads, and the
  merchant SDKs are implemented. SDK parity is checked by `just verify`.
- Images, Compose, Helm, migrations, and monitoring configuration are tested,
  but no deployment or monitoring system has operated vpay.

The work still required before a production claim includes real-rail validation,
external merchant webhook delivery, deployment validation, browser enforcement
of checkout framing controls, ingress protection for unauthenticated surfaces,
and refund support for MTN.

## Verification

Run `just verify` for repository invariants and `just ci` before review. The
`justfile` is the authoritative list of checks. `verify-docs` reports document
volume but does not pass or fail the build.

### Unimplemented items tracked by `verify-status`

- `mtn_momo::refund`
