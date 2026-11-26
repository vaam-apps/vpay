# Issue #91 item 5: a Cypress case follows `hosted_invoice_url`, and the bug that found

_Written 2026-11-20 on `test/91-cypress-hosted-invoice`, over `master` at
`81cf2467`. Every number below was measured on the tree this page was
committed with, unless the line says otherwise._

What was built is in [backend.md](../backend.md) (the invoice row) and
[frontend.md](../frontend.md) (the D6 row); the flow docs are
[invoices.md](../../flows/invoices.md) § Paying and
[hosted-checkout.md](../../flows/hosted-checkout.md) § Status. This page is
what was read, what was run, what was broken on purpose, and what could not
be run.

## The bug item 5 found, and where it was

`POST /v1/invoices/{id}/pay` mints a `pi_…` and a hosted checkout session for
it. It minted the intent with `payment_method_types: []`
(`vpay_api::v1::invoices`' `intent_for`). The browser session read renders one
`RailSpec` per code in that list
(`vpay_api::browser::checkout_sessions`' `build_rails`), so an empty list is
an empty `rails` member, and `frontends/apps/checkout` refuses a session with
no supported rail (`machine.ts`: `no_supported_rail`). A payer following a
`hosted_invoice_url` therefore never reached a payment page — so the D6
invoice number added 2026-11-17 could not be read off one either, and no
Cypress case could have passed before this change.

The fix names the deployment's enabled rails that settle the invoice's
currency (`ResourceConfig::enabled_rails_for_currency`) on that intent, and
refuses `pay` with `checkout_not_configured` when none exists rather than
minting an intent the page cannot render.

## What was run

| Command                                                                    | Result                                                                                                                                                                                                        |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test -p vpay-tests-integration --test invoices -- --test-threads=2` | **34 passed, 0 failed, 0 ignored** (was 32; the two new cases are `a_payer_following_a_hosted_invoice_url_is_offered_the_deployments_rails` and `paying_an_invoice_in_a_currency_no_rail_settles_is_refused`) |
| `cargo test -p vpay-api --lib`                                             | **575 passed, 0 failed, 0 ignored**                                                                                                                                                                           |
| `pnpm --filter @vpay/e2e typecheck`                                        | clean                                                                                                                                                                                                         |
| `pnpm --filter @vpay/e2e lint`                                             | clean                                                                                                                                                                                                         |
| `cargo xtask verify-links` / `verify-doc-counts`                           | clean                                                                                                                                                                                                         |
| `hosted-invoice.cy.ts` against the compose stack                           | **not run here** — see below; CI's `e2e (compose)` job is its first execution                                                                                                                                 |
| `just ci`                                                                  | **not run here** — see below                                                                                                                                                                                  |

## What was broken on purpose

| Mutation                                                                                            | Caught by                                                                                                                                                                                                                                |
| --------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `intent_for`'s `payment_method_types` returned to `Value::Array(Vec::new())` (the code as it stood) | `a_payer_following_a_hosted_invoice_url_is_offered_the_deployments_rails`, at its first assertion: `left: Array []`, `right: Array [String("mtn_momo")]`                                                                                 |
| the `if payment_method_types.is_empty()` guard deleted from `pay_hosted_once`                       | `paying_an_invoice_in_a_currency_no_rail_settles_is_refused`: the `500` becomes a `200`, and the body is an `open` EUR invoice carrying a `hosted_invoice_url` and a `pi_…` — the intent the browser then refuses `no_supported_rail` on |

## What was not run, and why

- **`hosted-invoice.cy.ts` against the compose stack.** Not run on the machine
  that wrote it: `just test-e2e` builds four images, and the two the compose
  file builds in parallel from `backends/Dockerfile` (`vpay-server` and
  `vpay-worker`) each run `cargo build --profile dist` inside the builder and
  were OOM-killed on this 31 GiB host before either finished, twice, with
  `COMPOSE_PARALLEL_LIMIT` unset and with it set to `1`. The spec is
  **typechecked and linted**, every selector in it is the one
  `shop-hosted.cy.ts` already drives against the same page, and the API-level
  behaviour it depends on is proven by the two cases above — but **a browser
  spec nothing has run is a claim nothing has checked**, so this page says it
  has not. CI's `e2e (compose)` job runs it; if it is red there, this page is
  wrong and must be corrected.
- **`just ci`.** Not run here — the brief's own note says it needs the
  network, and on this host the Rust release image build above already showed
  the memory ceiling. CI runs it on the PR. The pieces of it this change could
  exercise locally did run: `cargo test -p vpay-api --lib`, the integration
  suite above, `cargo xtask verify-links`/`verify-doc-counts`, `cargo fmt
--all`, `pnpm exec prettier`, and the e2e package's `typecheck`/`lint`.
- **`just test-storybook`.** Not run: this change touches no checkout story
  and no theme.
- **The `vpay-skills` companion.** A PR against
  [vaam-apps/vpay-skills](https://github.com/vaam-apps/vpay-skills) updates the
  three references that recorded the gap (`vpay-invoices`'
  `not-built.md` and `constraints-and-transitions.md`, `vpay-checkout`'s
  `SKILL.md` and `testing-and-gates.md`); `node tools/verify-coverage.mjs`
  passes against this tree. It is linked from the PR body.
