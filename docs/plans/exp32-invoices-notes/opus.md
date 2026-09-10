# exp32 — Invoices (S4b): implementation notes

2026-09-07. Branch `claude/exp32-invoices`, base `174e16b`.

What this file is for: the decisions that are _not_ obvious from the code, the
things that were measured rather than assumed, the mutations that were run,
and the questions that belong to the maintainer rather than to an
implementation pass.

---

## Maintainer decisions this change had to make, and would rather surface

These are choices where a defensible alternative exists and the code picked
one. Each is reversible; none is buried.

### D1 — a second `pay` needs the first intent **canceled**, not merely

"not processing"

`vpay_db::invoices::NO_LIVE_INTENT` refuses `pay`, `void` and
`mark_uncollectible` while the invoice's attached intent is anything other
than `canceled`.

The alternative is to treat a _declined_ intent as immediately re-payable. It
was rejected because a rail-declined intent lands back on
`requires_payment_method`, which is also where a **fresh, unconfirmed** intent
sits — so "not processing" would let a merchant mint a second intent one
millisecond after the first, before it was ever confirmed, which is the
double-charge this rule exists to prevent.

The cost is a manual step: a merchant whose payer abandoned a payment must
`POST /v1/payment_intents/{id}/cancel` before the invoice moves again. That is
consistent with the repository's standing rule that a retry is a _new_
PaymentIntent, and it is documented as the answer to "my invoice is stuck".

**What a maintainer might prefer instead:** an automatic cancel-and-retry
inside `pay`, or a shorter definition of "live" keyed on whether the intent
has a charge. Both are more code on a money path and neither was in scope.

### D2 — `POST /v1/invoices/{id}/pay` requires `success_url` and `cancel_url`

Stripe's `pay` takes neither, because it charges a stored payment method. vpay
has none on this market, so paying means creating a **hosted checkout
session**, and migration `0028`'s `urls_match_ui_mode` requires both on one.

They are required rather than defaulted because vpay does not know where a
merchant's "thank you" page is. The alternative — defaulting to the
deployment's `public_base_url` plus a path — would send a paying customer to a
`404` on any deployment that has not built that page.

**What a maintainer might prefer instead:** storing a per-merchant pair in
`config.yaml` so `pay` can be called with no body at all. That is a config
schema change, which this pass was not scoped to make.

### D3 — an invoice with no lines cannot be finalized

Stripe finalizes a zero-amount invoice and immediately marks it `paid`. vpay
answers `400` naming `invoice`.

vpay has no "paid without a payment" transition, and inventing one would be a
status a merchant could not explain. The other option — an `open` invoice for
zero — is a document nobody can pay, because `pay` would have to mint an
intent for zero and no rail accepts one.

### D4 — the invoice-number prefix is random, per merchant, and permanent

Eight upper-case Crockford characters, minted by a merchant's first finalize
and stored. Random rather than derived from `merchant_id` because an invoice
number is quoted in e-mail and read out loud, and a derived prefix would put
the deployment's internal tenant identifier on every one of them.

**Not decided here, and worth a maintainer's view:** whether a merchant should
be able to _choose_ their prefix (Stripe lets a customer's be set). Nothing in
the schema forecloses it — `invoice_number_sequences.prefix` is an ordinary
column — but there is no route to set it.

---

## Things that were measured, not assumed

### The enum CHECK converged, and it is the first one that has

Migration `0036` names the status constraint `invoices_status_enum_check` —
exactly what `naming.rs::check_name(table, column, "enum")` generates — so
`model Invoice`'s `status InvoiceStatus` and the live constraint are **one
object** to `diff/checks.rs`, which matches by name first. Measured: no drift
line for it in either direction.

Migration 0032 had to _rename_ `providers.flow`'s hand-named CHECK after the
fact to get the same result. A table born with a model does not have to, and
this is the first one that took the opportunity.

What it does **not** buy: `[lossy] column status type differs (live:
Scalar("String"), schema: Enum("InvoiceStatus"))` is permanent, for
`introspect/postgres/enums.rs`' documented reason.

### The drift moved 130 → 156 over 20 → 23 relations, unmappable 18 → 19

Measured against a fresh `postgres:16-alpine` with `cratestack migrate
baseline --strict` at **0.12.0** (the version `justfile` pins; the host's
`~/.cargo/bin/cratestack` is 0.11.1 and `just check-schema` warns about it, so
0.12.0 was installed into a private `--root` for these runs rather than into
the shared bin).

- `invoices`: 16 — eight hand-named CHECKs, six undeclared indexes, one `seq`
  identity default, one `status` type line.
- `invoice_items`: 9 — six CHECKs, two indexes, one `seq`. **Zero** unmappable
  columns, which is what the table was shaped for.
- `invoice_number_sequences`: 1 — the whole table, undeclared on purpose.

### The five multi-column CHECKs are invisible to the report

`introspect/postgres/constraints.rs:62` filters `array_length(c.conkey, 1) =
1`, so `number_is_assigned_at_finalize`, `paid_means_nothing_remaining`,
`only_a_live_invoice_has_an_intent`, `amounts_add_up` and
`amount_is_the_product` contribute nothing in either direction. They are in
`postgres_smoke.rs`'s multi-column inventory (which reads `pg_constraint`
directly) and each is exercised by
`the_invoice_invariants_are_enforced_by_the_database_itself`, which writes the
row each one refuses.

### `reqwest`'s `.form()` cannot send `metadata[order_id]`

It percent-escapes `[` and `]`, and `vpay_api::form::parse_key` splits a key on
its brackets _before_ any decoding — deliberately, so a decoded `[` can never
become structure. So `.form()` sends `metadata%5Border_id%5D`, which arrives as
one flat key and is silently ignored. The first draft of `invoices.rs` used it
and had a `metadata[order_id]` assertion that failed for exactly this reason;
the suite now encodes its own bodies with literal brackets, which is what both
merchant SDKs and Stripe's own clients send.

### `clippy::indexing_slicing` shapes how these suites read JSON

`body["id"]` is denied workspace-wide, and `serde_json::Value`'s `Index` impl
panics on a _type_ mismatch besides — so a handler that answered an array
where a case expects an object would fail as a panic rather than as an
assertion naming the field. `invoices.rs` carries `field`/`at`/`nth`/`only`/
`item`, which is `staff_sign_in.rs`' and `dashboard_read_surface.rs`'
convention. The first draft used indexing throughout and `just ci`'s clippy
step is what said so — the suite was green under `cargo nextest` the whole
time.

### The idempotency refusal is `400`, not `422`

The brief asked for "different body 422". This API's documented answer is
`400` `idempotency_error` / `idempotency_key_in_use`
(`docs/api/README.md`'s idempotency table, and `customers.rs`'s
`a_reused_key_with_a_different_body_is_the_400_envelope`). The suite asserts
the repository's own contract, on the `code` as well as the status.

---

## Decisive mutations, run rather than argued

Each was applied to the source, the named test was run, the source was
restored, and the restoration was verified.

| #   | Mutation                                                                            | Test                                                                                                                                   | Measured                                                                                                                                            |
| --- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | drop `AND status = 'open' AND {NO_LIVE_INTENT}` from `void_in_tx`                   | `the_two_terminal_transitions_and_the_transitions_they_refuse`                                                                         | **FAIL** — a draft void reached the database and tripped `number_is_assigned_at_finalize` (`503` where `409` was expected), so both enforcers spoke |
| 2   | replace `SET next_number = … + 1` with `SET next_number = …` in `next_number_in_tx` | `two_concurrent_finalizes_take_consecutive_numbers`                                                                                    | **FAIL** — the second finalize took the same number and `invoices_merchant_number_key` refused it (`409` where `200` was expected)                  |
| 3   | drop `AND {NO_LIVE_INTENT}` from `attach_intent`                                    | `attaching_a_second_intent_to_an_invoice_is_refused_by_the_statement`                                                                  | **FAIL** — one invoice acquired two live payment intents                                                                                            |
| 4   | delete `@@allow("update", auth().isSystem())` from `model Invoice`                  | `invoices::tests::every_action_this_module_calls_has_an_allow_arm`                                                                     | **FAIL** in 4 ms, no container                                                                                                                      |
| 5   | drop `'invoice.paid'` from `type_is_a_documented_event`                             | `the_event_vocabulary_holds_exactly_the_invoice_types_that_have_writers` **and** `apply_succeeded_pays_the_invoice_the_intent_was_for` | **FAIL** (both) — `23514` on the insert                                                                                                             |

Mutation 2 was **re-run after** the JSON-accessor refactor above, because that
refactor rewrote the very assertions it lands on (`numbers[0]`/`numbers[1]`
became `item(&numbers, 0, …)`). It still fails, and
`a_refused_finalize_does_not_burn_a_number` still passes under it, which is
the pair that says the two cases are testing different things.

Mutation 3 is the one worth dwelling on: **every wire-level case still passed
under it**, because `vpay_api::v1::invoices::pay` reads the invoice and refuses
first. That is why `attaching_a_second_intent_…` exists at the repository seam
at all — an integration test cannot tell an API guard from a statement guard,
and the statement guard is the one that runs when two `pay` requests race.

TX1 has its own pair rather than a single mutation:
`an_aborted_settlement_leaves_the_invoice_open_and_emits_nothing` aborts the
settlement transaction with a replayed `event_id` (a real error path, not an
injected fault) and asserts the invoice is still `open` with no event — which
is exactly what moving the flip out of TX1 would break.

---

## What was NOT done

Stated plainly, because the brief asked for it and because a reader of
`docs/status.md` should not have to infer it.

1. **Neither merchant SDK has one invoice method.** Not Rust, not Node, and
   the Stripe-compat suite has no invoice cases. Three dated ⛔/⛔ rows in
   `docs/sdks/parity.md`. This is the largest gap in the change, and it is why
   the integration suite drives raw HTTP: writing it against a client that did
   not exist would be the failure `CLAUDE.md` names by name.
2. **No Cypress case follows a `hosted_invoice_url` to a paid charge.** The URL
   is asserted to be a real checkout-session URL by the integration suite, and
   the page it points at is covered by `checkout.cy.ts`, but nothing joins the
   two in a browser.
3. **No dashboard screen.** `/dash/v1` exposes nothing about invoices.
4. **`invoice.*` webhook bodies carry `lines.data` empty.** A deliberate cost,
   documented in three places rather than hidden: rendering the lines inside
   the transition's transaction would put a second query on a connection
   holding the number sequence's row lock.
5. **No `invoice.marked_uncollectible` and no `invoice.payment_failed`.** Both
   are Stripe types; neither has a writer, so neither is in the vocabulary —
   migration `0023`'s rule, with `customer.created` as the standing precedent.
6. **No PDF, e-mail, tax, discount, credit note, dunning, or subscription.**
   Listed with dates in `docs/flows/invoices.md`.
7. **`due_date` is stored and read by nothing.** It is a column that looks like
   it drives something and does not; the flow doc says so under "What is not
   built" for that reason.
