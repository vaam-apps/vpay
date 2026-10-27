# MCP parity with `/v1`

Every `/v1` route vpay mounts, and what the merchant MCP endpoint
(`POST /mcp/v1`, [ADR-0033](../adr/0033-mcp-is-the-primary-agent-surface.md))
serves it as. **`cargo xtask verify-mcp-parity` reads this table**, and so does
`just verify`. It checks three things:

- every route in `V1_ROUTES` (`backends/crates/vpay-api/src/v1/mod.rs`) has
  exactly one row here, and every row names a mounted route;
- every answer is one of three forms: a tool `schemas/vpay.cstack` declares
  (`` tool `name` ``), a dated gap (`gap YYYY-MM-DD: reason`), or an exclusion
  (`excluded: reason`);
- every `@mcp(tool: …)` in the schema is named by some row.

So a route added to `/v1` with no row here fails the build, and so does a
tool added with no route.

What the gate does not check is that a tool behaves like its route.
`backends/tests/integration/tests/mcp_merchant.rs` checks that, for the tools
marked below. A tool's output is the `/v1` object reshaped in three places:
`metadata` as a list of `{key, value}`, `next_action.type` renamed to
`kind`, and `payment_method_details.type` renamed to `method_type`.
`schemas/vpay.cstack`'s MCP section says why.

`/v1/browser/*` (the payer's checkout), `/provider/*` (rail callbacks) and
`/v1/oauth/*` (the authorization server itself) are not in `V1_ROUTES`. They
are not an agent's surface, so they have no rows here.

| Route                                       | MCP                                                                          |
| ------------------------------------------- | ---------------------------------------------------------------------------- |
| `POST /v1/payment_intents`                  | tool `create_payment_intent`                                                 |
| `GET /v1/payment_intents`                   | tool `search_payment_intents`                                                |
| `GET /v1/payment_intents/{id}`              | tool `retrieve_payment_intent`                                               |
| `POST /v1/payment_intents/{id}/confirm`     | tool `confirm_payment_intent`                                                |
| `POST /v1/payment_intents/{id}/cancel`      | tool `cancel_payment_intent`                                                 |
| `POST /v1/refunds`                          | tool `create_refund`                                                         |
| `GET /v1/refunds`                           | tool `search_refunds`                                                        |
| `GET /v1/refunds/{id}`                      | tool `retrieve_refund`                                                       |
| `POST /v1/refunds/{id}`                     | gap 2026-10-26: the metadata update; ADR-0033 step 4                         |
| `POST /v1/refunds/{id}/cancel`              | tool `cancel_refund`                                                         |
| `GET /v1/events`                            | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/events/{id}`                       | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/account_holders`                   | gap 2026-10-26: ADR-0033 step 4; its payer lookup needs its own threat row   |
| `POST /v1/checkout/sessions`                | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/checkout/sessions`                 | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/checkout/sessions/{id}`            | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/checkout/sessions/{id}/expire`    | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/customers`                        | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/customers`                         | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/customers/{id}`                    | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/customers/{id}`                   | gap 2026-10-26: ADR-0033 step 4                                              |
| `DELETE /v1/customers/{id}`                 | gap 2026-10-26: ADR-0033 step 4; an erasure an agent triggers needs a ruling |
| `POST /v1/invoices`                         | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/invoices`                          | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/invoices/{id}`                     | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/invoices/{id}`                    | gap 2026-10-26: ADR-0033 step 4                                              |
| `PATCH /v1/invoices/{id}`                   | gap 2026-10-26: ADR-0033 step 4                                              |
| `DELETE /v1/invoices/{id}`                  | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/invoices/{id}/finalize`           | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/invoices/{id}/void`               | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/invoices/{id}/mark_uncollectible` | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/invoices/{id}/pay`                | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/invoice_items`                    | gap 2026-10-26: ADR-0033 step 4                                              |
| `GET /v1/invoice_items/{id}`                | gap 2026-10-26: ADR-0033 step 4                                              |
| `POST /v1/invoice_items/{id}`               | gap 2026-10-26: ADR-0033 step 4                                              |
| `PATCH /v1/invoice_items/{id}`              | gap 2026-10-26: ADR-0033 step 4                                              |
| `DELETE /v1/invoice_items/{id}`             | gap 2026-10-26: ADR-0033 step 4                                              |

## Where a tool's answer differs from its route's

- **`search_payment_intents` and `search_refunds`.** These are the dashboard's
  existing search procedures, not `/v1`'s list. They page by `offset` and
  `limit` rather than by cursor, and return summary rows with database-shaped
  names (`currency_code`, `customer_id`, `created_at` as a timestamp) rather
  than the `/v1` object. They answer the same question for the same tenant,
  so they stand for the list routes. A `/v1`-shaped list is a step 4 tool.
- **Refusals.** A refusal is CrateStack's error envelope (`code`, `message`).
  `code` is the class (`BAD_REQUEST`, `NOT_FOUND`, `CONFLICT`, `FORBIDDEN`).
  `message` begins with `/v1`'s own `error.code`, then `(param)` when there is
  one: `parameter_invalid (amount): …`.
