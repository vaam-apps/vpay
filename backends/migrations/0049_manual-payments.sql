-- Manual (out-of-band) payments: a merchant's statement that an OPEN invoice
-- was settled outside vpay — cash, a cheque, a bank transfer received
-- directly, or anything else (RFC-0004 section 6, step A).
--
-- `POST /v1/invoices/{id}/pay` with `paid_out_of_band=true` is the only
-- writer. It is a compare-and-swap `open -> paid` in the same transaction as
-- the `manual_payments` insert below and the `invoice.paid` event, and it is
-- the SECOND writer of `open -> paid` beside the settlement transaction
-- (migration 0036's `mark_paid_for_intent_in_tx`).
--
-- WHAT THIS RECORDS, AND WHAT IT CANNOT
--
-- A row here is a record of what a merchant SAID. No money crossed any rail
-- vpay talks to, nothing in vpay can verify the claim, and under
-- pass-through (RFC-0001) vpay has no account the money could have arrived
-- in. So nothing is posted to the ledger: `payer_clearing` never saw this
-- money and a posting would be a balance vpay invented.
--
-- `invoices.paid_out_of_band` — Stripe's own key
-- ==============================================
--
-- NO DEFAULT, and the backfill is the ADD's own DEFAULT dropped on the next
-- line — migration 0042's `amount_refunded` device, for 0042's two reasons:
-- every writer of `invoices` names every column it is responsible for
-- (`vpay_db::invoices::insert_in_tx` writes `false` as a literal beside the
-- zero amounts), and `schemas/vpay.cstack`'s `model Invoice` declares this
-- column as a plain `Boolean`, so a surviving DEFAULT would be one permanent
-- `column paid_out_of_band default value differs` drift line (0033's
-- problem) and a value no statement had to think about. Every row that
-- exists when this migration runs was paid, if at all, by a settlement.
ALTER TABLE invoices ADD COLUMN paid_out_of_band BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE invoices ALTER COLUMN paid_out_of_band DROP DEFAULT;

-- THREE MULTI-COLUMN CHECKS, and like the six before them on this table they
-- are INVISIBLE to `cratestack migrate baseline` in both directions (it
-- filters `array_length(conkey, 1) = 1`). Each is asserted by writing the
-- row it refuses straight past the API, in
-- `backends/tests/integration/tests/invoices.rs`'s
-- `the_out_of_band_invariants_are_enforced_by_the_database_itself`.

-- The flag means the document is settled. A draft, open, void or
-- uncollectible invoice that claims it was paid out of band is a
-- contradiction no statement should be able to store.
ALTER TABLE invoices
    ADD CONSTRAINT paid_out_of_band_means_paid CHECK (NOT paid_out_of_band OR status = 'paid');

-- A paid invoice says HOW: either a payment intent paid it (the settlement's
-- `WHERE payment_intent_id = $1`) or the merchant said it was paid out of
-- band. `paid` with neither is a settled bill nobody can account for, and
-- until this migration it was storable. Every row that exists today
-- satisfies it: the settlement is the only writer of `paid` before this
-- migration, and it matches on the intent.
ALTER TABLE invoices
    ADD CONSTRAINT paid_names_how CHECK (
        status <> 'paid' OR paid_out_of_band OR payment_intent_id IS NOT NULL
    );

-- `amount_refunded` (0042) is the running total of succeeded refunds against
-- the intent that PAID this invoice. An invoice paid out of band was paid by
-- no intent — at most it keeps a CANCELED one from an abandoned attempt,
-- whose charge never succeeded — so a rail refund recorded against it would
-- be money given back that vpay never collected. The refund settlement's
-- statement also carries `AND NOT paid_out_of_band`; this is the guard that
-- survives a writer that forgets it.
ALTER TABLE invoices
    ADD CONSTRAINT paid_out_of_band_is_never_refunded CHECK (
        NOT paid_out_of_band OR amount_refunded = 0
    );

-- `manual_payments` (`mp_…`)
-- ==========================
--
-- One row per invoice, ever (`manual_payments_invoice_id_key`): payment is
-- still all-or-nothing, and 0036's `paid_means_nothing_remaining` still
-- holds, so a second statement about the same bill has nothing to settle.
--
-- BORN WITH A `.cstack` MODEL (`model ManualPayment`), and shaped so it can
-- be: no `jsonb`, no `bytea`, no native enum, no `int4`, and no DEFAULT on
-- any column a writer names — 0035's discipline. The read goes through the
-- generated layer; the write is hand-written because it copies the amount,
-- the currency, the tenant and the mode off the invoice row INSIDE the
-- statement (`INSERT … SELECT … FROM invoices`), which a generated `create`
-- (values, not expressions) cannot say.
--
-- NO `seq`. There is no list route and no cursor, and an identity column
-- nothing pages by would be one permanent drift line for nothing.
CREATE TABLE manual_payments (
    -- Caller-supplied `mp_…` (vpay_core::ids::manual_payment_id), minted
    -- before the insert exactly as every other id is.
    id TEXT PRIMARY KEY,
    -- Copied from the invoice in the insert, never taken from the request.
    -- No FK — there is no merchants table (0003's comment).
    merchant_id TEXT NOT NULL,
    -- Copied from the invoice, so a test-mode bill cannot acquire a
    -- live-mode payment record.
    livemode BOOLEAN NOT NULL,
    -- The invoice this settles. NO ACTION on delete: only a draft is ever
    -- deleted, and a draft cannot be paid.
    invoice_id TEXT NOT NULL REFERENCES invoices (id),
    -- How the money arrived, in the merchant's words. TEXT + CHECK, never a
    -- native enum (0032, upstream issue #228), and the CHECK is named the
    -- way `naming.rs::check_name(table, column, "enum")` names it, so
    -- `model ManualPayment`'s `method ManualPaymentMethod` and the live
    -- constraint are one object to the diff engine — 0036's
    -- `invoices_status_enum_check` device.
    method TEXT NOT NULL,
    -- The merchant's free-text reference: a cheque number, a transfer
    -- reference, a receipt number. Optional.
    --
    -- PERSONAL DATA, and classified so (`schemas/privacy-inventory.yaml`,
    -- element `payment_reference`, `control: redact`): a cheque carries the
    -- drawer's name and account, and a bank transfer's reference routinely
    -- carries the payer's. The customer erasure replaces a non-null value
    -- with the `[redacted]` marker (`vpay_db::customers::redact_stored_copies`),
    -- which the lower bound of 1 admits.
    reference TEXT,
    -- When the merchant says the money arrived. Defaults to the request's
    -- `now` in vpay_api; never before the invoice was issued (checked there
    -- against `invoices.finalized_at`, which no statement moves once set).
    received_at TIMESTAMPTZ NOT NULL,
    -- Integer minor units, copied from `invoices.amount_paid` by the same
    -- statement that proves the invoice was just paid out of band. See
    -- below for why that is the guarantee and a CHECK is not.
    amount BIGINT NOT NULL,
    currency_code TEXT NOT NULL REFERENCES currencies (code),
    -- When vpay recorded the statement. `now()` for `checkout_sessions`'
    -- reason; the insert names it anyway.
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    CONSTRAINT manual_payments_method_enum_check CHECK (method IN (
        'cash', 'cheque', 'bank_transfer', 'other'
    )),
    -- 500, the ceiling a `metadata` value has: a reference is one value a
    -- merchant attaches, not a note on a document (`description` is 1000).
    -- The API refuses first with a `400` naming `out_of_band[reference]`.
    CONSTRAINT reference_length CHECK (
        reference IS NULL OR char_length(reference) BETWEEN 1 AND 500
    ),
    -- `POST /v1/invoices/{id}/pay` refuses an invoice with nothing left to
    -- pay; this is the backstop.
    CONSTRAINT amount_positive CHECK (amount >= 1),
    -- MULTI-COLUMN, so invisible to `migrate baseline`: a merchant cannot
    -- have received money after vpay recorded that they had. Thirty seconds
    -- of allowance for the merchant's clock, which is `vpay_api`'s bound too
    -- — one TOTP step (`vpay_api::staff_auth::totp`), the only tolerance
    -- this codebase applies to a caller's clock.
    CONSTRAINT received_before_recorded CHECK (
        received_at <= created_at + interval '30 seconds'
    )
);

-- ONE PER INVOICE. Named exactly as `naming.rs::index_name_unique` names a
-- `@unique` column, so `model ManualPayment.invoice_id`'s `@unique` and this
-- index are one object to the diff engine rather than a drop-and-add pair
-- (the `charges_payment_intent_id_key` lesson, `model Charge`).
CREATE UNIQUE INDEX manual_payments_invoice_id_key ON manual_payments (invoice_id);

-- WHY `amount = invoices.amount_paid` IS NOT A CHECK, AND WHAT GUARANTEES IT
--
-- A CHECK sees one row of one table. The guarantee is instead that no
-- caller ever supplies `amount`, `currency_code`, `merchant_id` or
-- `livemode`: `vpay_db::invoices::pay_out_of_band_in_tx` writes the invoice
-- (`open -> paid`, `amount_paid = amount_due`) and then inserts this row with
-- `INSERT … SELECT invoices.amount_paid, invoices.currency_code, … FROM
-- invoices WHERE invoices.id = $2 AND invoices.paid_out_of_band`, in ONE
-- transaction, while that transaction still holds the row lock its own
-- UPDATE took. Nothing can move the invoice between the two statements, and
-- nothing moves a paid invoice's `amount_paid` afterwards (every amount
-- write is guarded on `draft`, `open` or the settlement's intent, and 0042's
-- refund counter sits beside the amounts rather than inside them).
-- `paying_out_of_band_records_the_invoices_own_amount_and_posts_nothing` in
-- `vpay-db`'s `tests/repositories.rs` is the evidence.

COMMENT ON COLUMN invoices.paid_out_of_band IS
    'Stripe''s key: true exactly when POST /v1/invoices/{id}/pay recorded the merchant''s statement that this invoice was settled outside vpay (one manual_payments row). Implies status = paid (paid_out_of_band_means_paid) and amount_refunded = 0 (paid_out_of_band_is_never_refunded). Nothing in vpay can verify it and nothing is posted to the ledger for it.';
COMMENT ON TABLE manual_payments IS
    'A merchant''s statement that an invoice was settled outside vpay (RFC-0004 section 6): cash, cheque, bank_transfer or other. One per invoice. Written only by POST /v1/invoices/{id}/pay with paid_out_of_band=true, in the transaction that moves the invoice open -> paid and emits invoice.paid. amount, currency_code, merchant_id and livemode are copied from the invoice by the insert itself. NOT VERIFIED BY VPAY AND NOT POSTED TO THE LEDGER: no money crossed a rail vpay talks to.';
COMMENT ON COLUMN manual_payments.reference IS
    'The merchant''s free-text reference (<= 500 characters). Personal data: a cheque or transfer reference routinely names the payer, so the customer erasure replaces a non-null value with [redacted].';
