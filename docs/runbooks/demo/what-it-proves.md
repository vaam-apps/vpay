# The demo — §5. What this proves, and what it does not

_Moved out of [docs/runbooks/demo.md](../demo.md) on 2026-09-11 by exp57, which split a 1 426-line runbook into the procedure and its steps. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a `§` cross-reference pointed at a section that is now on another page, the page it moved to._

## 5. What this proves, and what it does not

**Proves**, because a run that did not do it fails and exits non-zero:

- **The `private_key_jwt` handshake works end to end** against a merchant
  whose public JWK the server holds — discovery, JWKS, an access token whose
  `iss`/`aud`/`sub` are the OP's own, and a `401` with vpay's error envelope
  for the same path without one. ([ADR-0010](../../adr/0010-merchant-auth-private-key-jwt.md))
- **Both rails, six outcomes.** Push and redirect; succeeded, declined and
  expired on MTN; succeeded, expired and refused on Orange. Every intent's
  public fields are printed from the object the API actually returned.
- **The response and the stored row agree.** Every create and every confirm is
  followed by a retrieve, and the two must be the _same object_ (bar
  `client_secret`, which `confirm` omits by design). A status rendered but not
  committed fails the run.
- **The failure taxonomy, not merely "it failed".** Each failing outcome
  asserts its exact `last_payment_error.code` — `insufficient_funds`,
  `payer_timeout`, `provider_error` — which is the difference between showing
  a decline and showing the adapter's mapping table working.
  ([../flows/failures.md](../../flows/failures.md))
- **Settlement is the worker asking the rail.** Nothing in the demo fakes an
  approval. The `vpay-worker` container claims the `poll_charge` job the
  confirm committed _in the same transaction as the charge_, asks the stub over
  HTTP, and commits the charge, the intent and one event together.
- **The webhook a merchant actually receives.** Read out of the receiver's own
  request journal (`GET /__admin/requests` — the merchant-side view, not
  vpay's belief about what it sent), matched on `Vpay-Event-Id` _and_ the
  intent id in the body, `Stripe-Signature` asserted byte-identical to
  `Vpay-Signature`, and the recorded bytes verified with
  `vpay_sdk::webhooks::verify` — the same call a merchant's handler makes. The
  verified event's `type` is asserted, so a run in which every payment was
  delivered as `payment_intent.succeeded` could not pass.
- **The redirect rail hands over a real URL.** Outcome 4 prints
  `next_action.redirect_to_url`, committed before the response was built —
  and since Step 9 that URL is on a port this stack actually publishes, so it
  can be opened.
- **`POST /v1/checkout/sessions` answers, in both modes.** Step 5 creates one
  hosted and one embedded session on a fresh intent each, reads each back and
  fails if the stored session differs from the one that was returned. The
  hosted `url` it prints is served by the `vpay-checkout` container this stack
  brought up — see [§4a](hosted-page.md#4a-opening-the-hosted-page).

**Does not prove**, each named so it is a decision and not an omission:

- **That MTN or Orange work.** No real rail endpoint has ever been called by
  this code. Every outcome is chosen at a WireMock stub, by a field of the
  request a merchant genuinely controls — the MSISDN on MTN (a `GET` status
  query steers no other way), the amount on Orange (whose status query is a
  `POST` carrying it). **Nothing rewrites stored state to force an outcome.**
- **That a payer can complete Orange's hosted page.** The demo prints the URL
  and does not open it; the stub then answers the status query as though the
  payer had finished. Since Step 9 that URL _is_ openable — the stub is
  published and serves a page with a Pay link and a Cancel link — but nothing
  in `just demo` clicks it.
- **That vpay's own checkout page works.** Step 5 mints two sessions and
  **stops**. This program has no browser: neither page has been rendered,
  neither intent has been confirmed, no rail has been called on either
  session's behalf, and both are still `open`/`unpaid` when the demo exits.
  What proves the page is `frontends/tests/e2e` and a human doing
  [§4a](hosted-page.md#4a-opening-the-hosted-page).
- **That the shop works.** `just demo` brings the `vpay-shop` container up,
  waits for its healthcheck and prints its URL. It does not click through it.
- **That a rail can call us.** `POST /provider/{code}/callback` **exists** —
  Step 8, lane C, and `provider_callback.rs` covers it against WireMock — but
  no stub in this demo calls it. Every settlement above came from vpay asking,
  never from being told. The walkthrough prints that sentence itself.
- **`amount_received`.** The settlement transaction writes the column; the
  `payment_intent` object does not carry it, so neither a merchant's client
  nor this demo can see it. The demo says so rather than reading the database
  behind the API it exists to demonstrate.
- **Anything about a deployment.** No cluster has ever run vpay.
- **That the demo is a build gate.** It is an assertion harness a human reads.
  Nothing in CI fails if it regresses; the closest thing is
  `just stripe-compat`, which drives the official `stripe` package against the
  same stack.
