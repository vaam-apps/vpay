# exp33 — invoices in both merchant SDKs

Branch `claude/exp33-sdk-invoices`, base `1513639` (the reviewed S4b invoices
branch). Three commits, `sdks/**` and docs only; `backends/**` untouched.

## What was built

Thirteen capabilities in each merchant SDK, mirroring
[`../../api/README.md`](../../api/README.md)'s eleven invoice routes:

|          | `sdks/rust`                                                                                                             | `sdks/nodejs`                                                                                                                         |
| -------- | ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| accessor | `client.invoices()` / `client.invoice_items()`                                                                          | `client.invoices` / `client.invoiceItems`                                                                                             |
| invoice  | `create` `retrieve` `update` `list` `del` `finalize` `void` `mark_uncollectible` `pay`                                  | same, and `mark_uncollectible` is spelled snake_case — see below (**superseded 2026-09-08 by the review: it is `markUncollectible`**) |
| line     | `create` `retrieve` `update` `del`                                                                                      | same                                                                                                                                  |
| object   | `Invoice` (18 keys), `InvoiceLine`, `InvoiceStatus`, `InvoiceStatusTransitions`, `DeletedInvoice`, `DeletedInvoiceItem` | the same six                                                                                                                          |
| events   | four `KnownEventType` variants + `Event::invoice()`                                                                     | four union members + `isInvoiceEvent`                                                                                                 |

No `invoice_items.list` in either: the server mounts no collection `GET`, and
an invoice's lines are read off the invoice.

## Four things that had to be decided rather than copied

**1. `mark_uncollectible`, not `markUncollectible`, in the Node SDK.** ~~Reversed by the review on 2026-09-08 — see `opus-review.md`; the gate was taught a per-column spelling table instead.~~ The one
snake_case method in `@vaam-apps/vpay-sdk`. Stripe's own Node SDK says
`markUncollectible`; `sdks/rust` cannot spell that (`non_snake_case` is a
rustc lint, not a preference); and
[ADR-0015](../../adr/0015-sdk-parity.md) plus the gate want one name per
capability, because `verify-sdk-parity` keys rows on `<resource>.<method>`
verbatim. Two spellings would be two rows for one capability, each showing a
⛔ in the column that does not use its spelling — a matrix reporting a
divergence where there is none. The same rule that made `customers.del` `del`
in Rust, applied in the only direction both languages allow. **Recorded as a
decision a maintainer may want to revisit**, in the method's doc comment, in
[`../../sdks/parity.md`](../../sdks/parity.md) and in
[`../../status.md`](../../status.md), because renaming a public method later
is a breaking change.

**2. `InvoiceLine` for a `line_item` reached at `/v1/invoice_items`.** Both
spellings are the wire's. The type is named for the object it decodes, the
resource for the route it calls; neither name is invented.

**3. The two update shapes differ, and had to.** An invoice patch field is
three-state (`Option<Option<T>>` / `T | null | undefined`), because
`description=` clears. An invoice _item_ patch field is two-state in both
languages, because all three of its columns are `NOT NULL` and `description=`
is a `400` naming the parameter rather than a clear.

**4. The event union grew by four, not six.** `invoice.marked_uncollectible`
and `invoice.payment_failed` are real Stripe types vpay does not write.
Adding either would be the `customer.created` mistake made knowingly; each
SDK's proving test asserts they are **not** known.

## Gate numbers

Both measured on this tree — the base by restoring `sdks/` and the matrix from
`1513639` (and moving the new `invoices.ts` aside, since `git checkout <sha> --
sdks` does not delete a file that tree never had) and re-running the gate:

|                                          | before                                    | after                                         |
| ---------------------------------------- | ----------------------------------------- | --------------------------------------------- |
| `verify-sdk-parity`                      | 407 proving, 41 gaps, 19 methods, 23 rows | **443 proving, 37 gaps, 32 methods, 36 rows** |
| `cargo nextest run -p vpay-sdk`          | 149, 0 ignored                            | **164, 0 ignored**                            |
| `cargo test --doc -p vpay-sdk`           | 6 passed, 1 ignored                       | **8 passed, 1 ignored**                       |
| `pnpm --filter @vaam-apps/vpay-sdk test` | 190, 0 skipped                            | **207, 0 skipped**                            |

`just verify` (twelve gates), `just lint-web`, `just test-web` (1133 across the
workspace) all pass on the final head.

## Mutations — each applied, run, and reverted

| #   | Mutation                                                              | Gate                            | Exit  | Verdict                                                                     |
| --- | --------------------------------------------------------------------- | ------------------------------- | ----- | --------------------------------------------------------------------------- |
| 1a  | delete `InvoicesResource::void` from **`sdks/rust` only**             | `verify-sdk-parity`             | **0** | **not caught** — see below                                                  |
| 1a′ | the same                                                              | `cargo nextest run -p vpay-sdk` | 101   | caught (E0599, the test does not compile)                                   |
| 1b  | delete `void` from **both** SDKs                                      | `verify-sdk-parity`             | 1     | caught (doc→code: the row names a method nothing declares)                  |
| 2   | drop `"invoice.paid"` from Rust `KnownEventType::from_wire`           | `cargo nextest -p vpay-sdk`     | 100   | caught — `the_four_invoice_event_types_are_known_and_their_payloads_decode` |
| 3   | typo `"invoice.paid"` in the Node union                               | `pnpm typecheck`                | 2     | caught — TS2322 at `client.test.ts`                                         |
| 4   | rename a body key in the Node resource (`unit_amount` → `unitAmount`) | `pnpm test`                     | 1     | caught — 2 cases                                                            |
| 5   | collapse the Node invoice patch to two states (`null` ≡ `undefined`)  | `pnpm test`                     | 1     | caught — the leave-alone/set/clear case                                     |
| 6   | collapse `UpdateInvoiceParams` to a single `Option` in Rust           | `cargo nextest`                 | 100   | caught — the same case, other language                                      |
| 7   | drop `check_amount` from `invoice_items().create()`                   | `cargo nextest`                 | 100   | caught                                                                      |
| 8   | rename a Rust proving test                                            | `verify-sdk-parity`             | 1     | caught                                                                      |
| 9   | rename a Node proving test                                            | `verify-sdk-parity`             | 1     | caught                                                                      |

**Mutation 1a is the one worth reading twice.** Deleting a method from _one_
SDK does **not** fail `verify-sdk-parity`: the doc→code direction is satisfied
while _either_ SDK declares the capability, and the ✅ cell's named test still
exists as source text, because the gate is a scanner and not a compiler. This
is the hole [`../../status.md`](../../status.md) already records —
"no rule compares a row's per-column `✅`/`⛔` cell against whether _that_ SDK
declares the method" — measured again here, on invoice rows, and left as it
is: closing it is a change to `.xtask`, not to this branch. What catches it in
practice is the suite (mutation 1a′), because a `✅` cell names a test that
calls the method.

## What this does NOT do

- **No Stripe-compat invoice cases.** Item (3) of the brief, **not done**.
  `sdks/stripe-compat` needs a live compose stack; the only one on this host
  belongs to another session (project `vpay-demo`, port 8080), standing up a
  second one means a cold musl release build of `vpay-server`, and this
  worktree has no `.e2e/demo-merchant` keypair the running stack would accept.
  Writing the cases without running them would be a suite claiming what it has
  not shown. The brief's own fallback allows reporting it here instead.
- **Neither SDK has run invoices against a running vpay.** Every server in
  these cases is `wiremock` / `node:http`. That is the one ⛔/⛔ row still
  standing in [`../../sdks/parity.md`](../../sdks/parity.md), and it was
  written on 2026-09-07 precisely so building the methods against stubs would
  not close it by accident. It is not closed.
- **`@vaam-apps/vpay-sdk/stripe` needed nothing.** That subpath exports
  `createStripeAuthenticator` and no resources at all, so there is no invoice
  surface there to add: a merchant driving invoices through the real `stripe`
  package already reaches vpay's routes through the authenticator, unchanged.
- **`sdks/stripe-js` needed nothing**, as the brief guessed. It is a browser
  package that authenticates a _payer_, holds no merchant credential and calls
  only `/v1/browser`; an invoice is a merchant-side object and the payer meets
  it as an ordinary hosted checkout via `hosted_invoice_url`.
- **No dashboard, Cypress or example changes.** `examples/shop` sells a cart,
  not a bill.
- **The `finalize` amount ceiling is the server's alone.** Both SDKs bound a
  line's `unit_amount` at `2^53-1`, but neither sums the lines, so an invoice
  whose _total_ passes the ceiling is refused by `POST
/v1/invoices/{id}/finalize` with a `400` and not before. That matches the
  server (the S4b review put the check at finalize deliberately) and is not a
  gap the SDKs should close: they do not know the other lines.
