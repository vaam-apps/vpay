-- invoices and invoice_items: the merchant's bill to a payer, and the lines it
-- is made of (S4b of docs/plans/2026-09-06-data-layer.md).
--
-- The second and third vpay tables born with a `schemas/vpay.cstack` model
-- rather than acquiring one afterwards — `customers` (0034) was the first —
-- plus one that deliberately has no model (`invoice_number_sequences`, see
-- below).
--
-- WHY AN INVOICE IS A TABLE AND NOT A VIEW OVER PAYMENT INTENTS
--
-- A payment intent is one *attempt* to move money and is created at the
-- moment somebody is about to pay. An invoice exists before anybody pays,
-- may never be paid at all, and is the thing a merchant amends, sends, voids
-- and writes off. The two lifecycles do not nest: one invoice has at most
-- one intent today (`invoices_payment_intent_key` below) and zero for most
-- of its life.
--
-- THE STATE MACHINE, AND WHERE IT IS ENFORCED
--
--   draft ──finalize──> open ──pay/settle──> paid
--     │                  ├────void────────> void
--     │                  └────mark_uncollectible──> uncollectible
--     └────void────> void        (a draft may be voided; it may also be
--                                 DELETEd, which removes it entirely)
--
-- Three enforcers, deliberately, and none of them is a validation function
-- called before the write:
--
--   1. `invoices_status_enum_check` closes the vocabulary.
--   2. Every transition is a compare-and-swap `UPDATE ... WHERE id = $1 AND
--      merchant_id = $2 AND status = '<from>'` in `vpay_db::invoices`. A
--      transition from the wrong state matches zero rows and is refused; it
--      is never decided by comparing a status read a moment earlier.
--   3. The cross-column CHECKs below (`number_is_assigned_at_finalize`,
--      `paid_means_nothing_remaining`, `only_a_live_invoice_has_an_intent`)
--      make the *combinations* a broken transition would produce
--      unstorable — which is the guard that survives a future writer that
--      forgets rule 2.
--
-- Rule 3 is multi-column and therefore INVISIBLE to `cratestack migrate
-- baseline` in both directions (`introspect/postgres/constraints.rs:62`
-- filters `array_length(c.conkey, 1) = 1`), exactly as 0034's
-- `at_least_one_identifier` is. `postgres_smoke.rs` asserts each one
-- directly against a real Postgres for that reason: the drift report cannot
-- be the guard here.
--
-- NO COLUMN DEFAULTS CRATESTACK MUST WRITE
--
-- 0034's rule, unchanged and for the same measured reason
-- (`cratestack-macros`' `model/inputs.rs:20-23` drops every `@default(...)`
-- field from `Create{Model}Input`): every column below that CrateStack may
-- ever write carries no `DEFAULT`. The exceptions are the two columns
-- nothing may supply — `seq` (GENERATED ALWAYS) — and `created_at` /
-- `updated_at`, which are `now()` for `checkout_sessions`' reason.

CREATE TABLE invoices (
    -- Caller-supplied `in_…` (vpay_core::ids::invoice_id), generated before
    -- the insert exactly as `pi_…`, `cus_…` and `cs_…` are.
    id TEXT PRIMARY KEY,
    -- The list cursor. `customers.seq` (0034)'s argument unchanged.
    seq BIGINT GENERATED ALWAYS AS IDENTITY,
    -- No FK: there is no merchants table (ADR-0003; see 0003's comment).
    -- Every query in vpay_db::invoices filters on it in SQL.
    merchant_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    -- REQUIRED, unlike `payment_intents.customer_id` and
    -- `checkout_sessions.customer_id`, which are both nullable.
    --
    -- An invoice is a bill *to somebody*: it carries a number a merchant
    -- quotes in a conversation, it may be chased for months, and on this
    -- market the payer's identity is a phone number
    -- (`customers.phone`, the maintainer's decision of 2026-09-05). An
    -- invoice with no customer would be a bill nobody could be asked to pay
    -- and nobody could be reminded about — and `POST /v1/invoices/{id}/pay`
    -- has no payer to bind an intent to.
    --
    -- ON DELETE omitted, i.e. NO ACTION, for 0034's reason: the customer a
    -- bill was issued to cannot be erased out from under it. That is why
    -- `vpay_db::customers::UNREFERENCED` grew its third `NOT EXISTS` in the
    -- same commit as this migration — without it the twelve-month retention
    -- sweep would *render* a `customer.deleted` event for a customer the
    -- foreign key then refuses to delete, doing work for nothing on every
    -- pass.
    customer_id TEXT NOT NULL REFERENCES customers (id),
    -- The invoice's currency, and the currency every line must be in.
    -- Denormalised onto the item rows as well (`invoice_items.currency_code`)
    -- so a line can never be in a different currency from its invoice —
    -- see `invoice_items` below.
    currency_code TEXT NOT NULL REFERENCES currencies (code),
    -- TEXT + CHECK, never a native Postgres enum: cratestack's generated row
    -- decoders read an enum column with `try_get::<String>()`, so a native
    -- enum fails to decode on every read through that layer (migration 0032's
    -- own header, upstream issue #228). A table born with a model does not
    -- repeat the conversion 0032 had to perform.
    status TEXT NOT NULL,
    -- `{prefix}-{000001}`, assigned at finalize out of the merchant's own
    -- sequence (`invoice_number_sequences` below) and NEVER REUSED.
    --
    -- NULL exactly while the invoice is a draft — `number_is_assigned_at_
    -- finalize` makes that an invariant rather than a convention. A draft has
    -- no number on purpose: a merchant who sees a number believes the
    -- document is issued, and a number burnt on a draft that is then deleted
    -- would leave a hole in a sequence an accountant reads as a missing
    -- document.
    number TEXT,
    -- All three amounts are integer minor units (docs/flows/money.md), BIGINT
    -- to match Money's i64 exactly.
    --
    -- DERIVED FROM THE LINES, AND FROZEN AT FINALIZE. While the invoice is a
    -- draft these are recomputed by `vpay_db::invoices` from `invoice_items`
    -- on every line change, so a draft's total always equals its lines. At
    -- finalize they are computed once more and the lines become immutable
    -- (`invoice_items`' writes all carry a draft-parent guard), so an issued
    -- invoice's total can never stop matching the document that was sent.
    amount_due BIGINT NOT NULL,
    amount_paid BIGINT NOT NULL,
    -- Stored rather than computed on read, and `amounts_add_up` below is what
    -- keeps it honest. A GENERATED column would be the obvious alternative
    -- and is rejected for the reason every other default is: cratestack has
    -- no representation for one, so it would be a column the generated layer
    -- could neither write nor introspect.
    amount_remaining BIGINT NOT NULL,
    -- When the merchant says this is due. Advisory: nothing in vpay acts on
    -- it — there is no dunning, no reminder job and no automatic
    -- `uncollectible` transition. `docs/flows/invoices.md` says so plainly
    -- under "What is not built", because a `due_date` column that looks like
    -- it drives something is worse than no column at all.
    due_date TIMESTAMPTZ,
    description TEXT,
    -- The merchant's own key/value pairs. `JSONB NOT NULL` with **no
    -- DEFAULT**, which is what keeps `Invoices::create` and the draft writes
    -- hand-written `sqlx` statements: `schemas/vpay.cstack`'s `model Invoice`
    -- does not declare this column (see that model's GAP note, which is
    -- `model Customer`'s two measured reasons unchanged), so a generated
    -- INSERT omits it, and without a DEFAULT that omission is a loud `23502`
    -- rather than a silent `{}` over a merchant's data. The absence of the
    -- DEFAULT is the tripwire; it is not an oversight.
    metadata JSONB NOT NULL,
    -- The intent that is paying, or has paid, this invoice. NULL until
    -- `POST /v1/invoices/{id}/pay`.
    --
    -- At most one, enforced by `invoices_payment_intent_key` below, and that
    -- is the whole of "partial payments are out of scope": a second `pay`
    -- while an intent is live is refused by `vpay_db::invoices::attach_intent`'s
    -- `WHERE payment_intent_id IS NULL` compare-and-swap, not by a preceding
    -- read. `docs/flows/invoices.md` records the gap.
    payment_intent_id TEXT REFERENCES payment_intents (id),
    -- Stripe's `status_transitions`, one column each. Written by the
    -- transition that sets them and never by a trigger — 0034's rule.
    finalized_at TIMESTAMPTZ,
    paid_at TIMESTAMPTZ,
    voided_at TIMESTAMPTZ,
    marked_uncollectible_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),

    -- SINGLE-COLUMN, and named exactly as `naming.rs::check_name(table,
    -- column, "enum")` would name it, so `model Invoice`'s `status
    -- InvoiceStatus` and this constraint are the same object to
    -- `diff/checks.rs` (which matches by name first). Migration 0032's
    -- `providers_flow_enum_check` is the precedent; the difference is that
    -- 0032 had to rename a hand-named CHECK after the fact and this table
    -- is born with the generated name.
    CONSTRAINT invoices_status_enum_check CHECK (status IN (
        'draft', 'open', 'paid', 'void', 'uncollectible'
    )),

    -- THE THREE CROSS-COLUMN INVARIANTS. Multi-column, therefore invisible to
    -- `cratestack migrate baseline` in both directions — their loss would
    -- move no drift line — so `postgres_smoke.rs` asserts each one against a
    -- real Postgres. See this file's header.

    -- A number exists exactly when the invoice is not a draft. Both halves
    -- matter: a draft with a number is a burnt number (see the `number`
    -- column), and a finalized invoice without one is a document an
    -- accountant cannot file.
    CONSTRAINT number_is_assigned_at_finalize CHECK ((status = 'draft') = (number IS NULL)),
    -- `paid` means paid in full. Partial payments are out of scope
    -- (docs/flows/invoices.md); this is what makes "out of scope" a property
    -- of the database rather than a sentence in a document.
    CONSTRAINT paid_means_nothing_remaining CHECK (status <> 'paid' OR amount_remaining = 0),
    -- A draft has no intent — nothing may pay a document that has not been
    -- issued — and a `void` invoice keeps whichever intent it had, because
    -- the payment record survives the document. Only the first half is
    -- constrained.
    CONSTRAINT only_a_live_invoice_has_an_intent CHECK (
        status <> 'draft' OR payment_intent_id IS NULL
    ),
    -- The arithmetic that makes `amount_remaining` a stored column rather
    -- than a lie. Every writer computes all three together.
    CONSTRAINT amounts_add_up CHECK (amount_paid + amount_remaining = amount_due),

    CONSTRAINT amount_due_non_negative CHECK (amount_due >= 0),
    CONSTRAINT amount_paid_non_negative CHECK (amount_paid >= 0),
    CONSTRAINT amount_remaining_non_negative CHECK (amount_remaining >= 0),

    -- The two bounds the API also enforces, so a merchant gets a `400` naming
    -- the parameter and this is the backstop. 0034's argument on `name`
    -- applies verbatim.
    CONSTRAINT number_length CHECK (number IS NULL OR char_length(number) BETWEEN 1 AND 64),
    CONSTRAINT description_length CHECK (
        description IS NULL OR char_length(description) BETWEEN 1 AND 1000
    ),

    -- `metadata_is_object`, exactly as `payment_intents` (0014), `refunds`
    -- (0017) and `customers` (0034) spell it.
    CONSTRAINT metadata_is_object CHECK (jsonb_typeof(metadata) = 'object')
);

-- An identity column is not implicitly unique, and every cursor below assumes
-- a total order — `customers_seq_key` (0034)'s reasoning.
CREATE UNIQUE INDEX invoices_seq_key ON invoices (seq);

-- `GET /v1/invoices`: merchant-scoped, newest first.
CREATE INDEX invoices_merchant_seq_idx ON invoices (merchant_id, seq DESC);

-- `GET /v1/invoices?status=open`, the filter's own index. Composite with
-- `seq DESC` so a filtered page is an index scan rather than a filter over
-- the merchant's whole history.
CREATE INDEX invoices_merchant_status_seq_idx ON invoices (merchant_id, status, seq DESC);

-- `GET /v1/invoices?customer=cus_…`, and the retention sweep's third
-- `NOT EXISTS` guard (`vpay_db::customers::UNREFERENCED`). Not partial,
-- unlike `payment_intents_customer_idx`: `invoices.customer_id` is NOT NULL.
CREATE INDEX invoices_customer_seq_idx ON invoices (customer_id, seq DESC);

-- NUMBERS ARE UNIQUE PER MERCHANT AND NEVER REUSED.
--
-- The sequence table below is what *assigns* them and the lock in
-- `Invoices::finalize` is what serialises two concurrent finalizes; this
-- index is what makes "never reused" true even if both of those were wrong.
-- Partial, because `number` is NULL for every draft and NULLs are distinct
-- in a Postgres unique index anyway — the `WHERE` makes the intent explicit
-- and keeps drafts out of the index entirely.
CREATE UNIQUE INDEX invoices_merchant_number_key ON invoices (merchant_id, number)
    WHERE number IS NOT NULL;

-- ONE INVOICE PER INTENT, FOREVER — the `one_charge_per_intent` device
-- (0004) applied one level up.
--
-- `POST /v1/invoices/{id}/pay` mints an intent and attaches it with a
-- compare-and-swap on `payment_intent_id IS NULL`; this index is the second
-- enforcer, and it is the one that survives a future writer that forgets the
-- guard. It is also what makes the settlement hook's lookup ("which invoice
-- is this intent paying?") a single-row read that cannot be ambiguous.
CREATE UNIQUE INDEX invoices_payment_intent_key ON invoices (payment_intent_id)
    WHERE payment_intent_id IS NOT NULL;

-- The lines an invoice is made of.
--
-- WHY `amount` IS STORED AND NOT COMPUTED ON READ
--
-- `amount = quantity * unit_amount` is checked by the database
-- (`amount_is_the_product` below) rather than computed by it, for the reason
-- `invoices.amount_remaining` is stored: a GENERATED column is not something
-- cratestack can express, so it would be a column the generated layer could
-- neither write nor introspect. The CHECK gives the same guarantee — no
-- committed row can disagree with its own arithmetic — without that cost.
CREATE TABLE invoice_items (
    -- Caller-supplied `ii_…` (vpay_core::ids::invoice_item_id).
    id TEXT PRIMARY KEY,
    -- The order lines are rendered in, and the only order that exists: a
    -- merchant reads their invoice's lines in the order they added them.
    seq BIGINT GENERATED ALWAYS AS IDENTITY,
    -- ON DELETE CASCADE, and this is the one cascade in the schema.
    --
    -- `DELETE /v1/invoices/{id}` is refused for anything but a draft
    -- (`vpay_db::invoices::delete_draft`'s `AND status = 'draft'`), and a
    -- draft's lines have no independent existence: they are not billable,
    -- not numbered, and not referenced by anything. Leaving them behind would
    -- be rows no query can reach. The alternative — NO ACTION, and a delete
    -- that fails until the merchant removes every line by hand — makes
    -- "delete this draft" a loop rather than a call.
    invoice_id TEXT NOT NULL REFERENCES invoices (id) ON DELETE CASCADE,
    -- Denormalised from the parent so `GET /v1/invoice_items/{id}` is
    -- merchant-scoped in ONE table's `WHERE` rather than through a join.
    -- `events.merchant_id` (0018)'s argument: a tenancy filter that needs a
    -- join is a tenancy filter a future query can forget.
    --
    -- It cannot drift from the parent's: nothing updates either column, and
    -- `Invoices::add_item` copies it out of the same guarded read that
    -- proves the parent is a draft of this merchant's.
    merchant_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    -- NOT NULL, unlike `invoices.description`: a line with no description is
    -- a charge a payer cannot identify, and this is the text on the document.
    description TEXT NOT NULL,
    quantity BIGINT NOT NULL,
    unit_amount BIGINT NOT NULL,
    amount BIGINT NOT NULL,
    -- Copied from the parent invoice, never taken from the request. A line in
    -- a different currency from its invoice is a total that means nothing,
    -- and the API has no parameter for it.
    currency_code TEXT NOT NULL REFERENCES currencies (code),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    CONSTRAINT description_length CHECK (char_length(description) BETWEEN 1 AND 1000),
    -- A zero-quantity line is a line that bills nothing and still prints on
    -- the document; the API answers a `400` naming `quantity`.
    CONSTRAINT quantity_positive CHECK (quantity >= 1),
    CONSTRAINT unit_amount_non_negative CHECK (unit_amount >= 0),
    CONSTRAINT amount_non_negative CHECK (amount >= 0),
    -- MULTI-COLUMN, therefore invisible to `migrate baseline`; asserted
    -- directly in `postgres_smoke.rs`. See `invoices`' header.
    CONSTRAINT amount_is_the_product CHECK (amount = quantity * unit_amount)
);

CREATE UNIQUE INDEX invoice_items_seq_key ON invoice_items (seq);

-- "The lines of this invoice, in order" — the only read this table has, and
-- the one `InvoiceObject.lines` is rendered from.
CREATE INDEX invoice_items_invoice_seq_idx ON invoice_items (invoice_id, seq);

-- THE PER-MERCHANT INVOICE NUMBER SEQUENCE.
--
-- WHY A TABLE AND NOT A POSTGRES SEQUENCE
--
-- A Postgres `SEQUENCE` is non-transactional by design: `nextval` is not
-- rolled back, so a finalize that fails after taking a number leaves a hole.
-- Stripe's numbering has holes and that is defensible for Stripe; it is not
-- defensible here, because a Cameroonian merchant's invoice numbers are
-- read by a tax authority that treats a missing number as a destroyed
-- document. A row in an ordinary table is rolled back with everything else,
-- so a rolled-back finalize burns nothing and the next finalize takes the
-- same number. `invoices.rs`'s `finalize` and the concurrency case in
-- `backends/tests/integration/tests/invoices.rs` are the evidence.
--
-- It also needs one sequence PER MERCHANT, and a Postgres sequence per
-- merchant would mean DDL at runtime.
--
-- WHY IT HAS NO `.cstack` MODEL
--
-- Its only write is `next_number = invoice_number_sequences.next_number + 1`
-- — a SET whose right-hand side names the column being set.
-- `Update{Model}Input` carries values, not expressions, and cratestack 0.12.0
-- has no representation for one (the same shape `Customers::touch_last_used`
-- wanted `GREATEST` for and could not have). A model would be a declaration
-- of a table nothing could write through, which is what this schema's header
-- says it does not do.
CREATE TABLE invoice_number_sequences (
    -- One row per merchant, created lazily by the first finalize.
    merchant_id TEXT PRIMARY KEY,
    -- The human-readable part of every number this merchant's invoices carry
    -- — `A7K3M9QP` in `A7K3M9QP-000001`.
    --
    -- Minted once, from `vpay_core::ids::invoice_number_prefix`, and never
    -- changed: it is printed on documents that have already been sent.
    --
    -- Random rather than derived from `merchant_id`, and that is a privacy
    -- choice rather than an aesthetic one: an invoice number is quoted in
    -- e-mails, printed on receipts and read out over the phone, and a prefix
    -- derived from the tenant identifier would put the deployment's internal
    -- tenant name on every one of them.
    prefix TEXT NOT NULL,
    -- The number the NEXT finalize will take. 1-based, so the first invoice a
    -- merchant issues is `-000001` and not `-000000`.
    next_number BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    -- Eight characters of `vpay_core::ids`' alphabet, upper-cased. The shape
    -- is fixed so a number is recognisable as a vpay invoice number by
    -- looking at it, and so `{prefix}-{000001}` always fits `number_length`.
    CONSTRAINT prefix_shape CHECK (prefix ~ '^[0-9A-HJKMNP-TV-Z]{8}$'),
    CONSTRAINT next_number_positive CHECK (next_number >= 1)
);

-- THE EVENT VOCABULARY, REOPENED FOR FOUR TYPES
--
-- 0023/0024/0029/0034's mechanism unchanged: the database is what refuses a
-- type no code writes, so the CHECK moves in lockstep with the code that
-- writes it.
--
-- All four have a writer in the same commit, which is the rule 0034's own
-- comment states and which `customer.created`/`customer.updated` are the
-- standing counter-example to (they are still unwritten and still absent —
-- issue #66; this migration deliberately does not add them):
--
--   invoice.created    <- vpay_db::invoices::Invoices::create        (in TX)
--   invoice.finalized  <- vpay_db::invoices::Invoices::finalize      (in TX)
--   invoice.paid       <- vpay_db::settlement's TX1, via
--                         invoices::mark_paid_for_intent_in_tx
--   invoice.voided     <- vpay_db::invoices::Invoices::void          (in TX)
--
-- Every one is Stripe's own type name, which is docs/flows/webhooks.md's
-- standing rule: an invented type has no branch in a Stripe-shaped handler
-- and is silently dropped by `stripe-node`'s typed event union.
--
-- `invoice.marked_uncollectible` is Stripe's fifth and is deliberately NOT
-- here: `POST /v1/invoices/{id}/mark_uncollectible` is a single statement on
-- the pool (it is the one transition with no event), and adding the label
-- would put a value in a closed vocabulary that no code can produce. See
-- docs/flows/invoices.md.
ALTER TABLE events DROP CONSTRAINT type_is_a_documented_event;
ALTER TABLE events ADD CONSTRAINT type_is_a_documented_event CHECK (type IN (
    'payment_intent.created',
    'payment_intent.processing',
    'payment_intent.succeeded',
    'payment_intent.payment_failed',
    'payment_intent.canceled',
    'charge.refunded',
    'charge.refund.updated',
    'checkout.session.expired',
    'customer.deleted',
    'invoice.created',
    'invoice.finalized',
    'invoice.paid',
    'invoice.voided'
));

COMMENT ON TABLE invoices IS
    'A merchant''s bill to one customer (S4b). draft -> open -> paid | void | uncollectible, enforced by invoices_status_enum_check, by compare-and-swap in vpay_db::invoices, and by the three multi-column CHECKs this table carries. Amounts are integer minor units and are derived from invoice_items while the invoice is a draft, then frozen at finalize.';
COMMENT ON COLUMN invoices.number IS
    'The document number, {prefix}-{000001}, out of the merchant''s own invoice_number_sequences row. NULL exactly while the invoice is a draft (number_is_assigned_at_finalize). Unique per merchant (invoices_merchant_number_key) and never reused: the sequence is an ordinary table row, so a rolled-back finalize does not burn a number.';
COMMENT ON COLUMN invoices.customer_id IS
    'The payer this invoice bills, REQUIRED (unlike payment_intents.customer_id). NO ACTION on delete: the customer a bill was issued to cannot be erased out from under it, so an invoice pins its customer against both DELETE /v1/customers/{id} and the twelve-month retention sweep.';
COMMENT ON COLUMN invoices.payment_intent_id IS
    'The intent paying this invoice, or NULL. At most one, ever (invoices_payment_intent_key), which is how "partial payments are out of scope" is enforced rather than documented. Written by POST /v1/invoices/{id}/pay as a compare-and-swap on IS NULL; read by the settlement transaction to mark the invoice paid in TX1.';
COMMENT ON COLUMN invoices.due_date IS
    'When the merchant says this is due. ADVISORY ONLY: nothing in vpay reads it. There is no dunning, no reminder job and no automatic transition to uncollectible — see docs/flows/invoices.md, "What is not built".';
COMMENT ON COLUMN invoices.metadata IS
    'The merchant''s own key/value pairs (<=50 keys, <=40-char key, <=500-char value, enforced by vpay_api). JSONB NOT NULL with NO DEFAULT on purpose: schemas/vpay.cstack''s model Invoice does not declare this column, so a generated INSERT that omitted it would be a loud 23502 rather than a silent {} over a merchant''s data.';
COMMENT ON TABLE invoice_items IS
    'One line of one invoice (S4b). Writable only while the parent invoice is a draft — every write in vpay_db::invoices carries an EXISTS guard on the parent''s status, which is what freezes an issued document. ON DELETE CASCADE from invoices, the schema''s only cascade: a draft''s lines have no independent existence.';
COMMENT ON COLUMN invoice_items.merchant_id IS
    'Copied from the parent invoice so a line''s tenancy filter is one table''s WHERE and never a join. Cannot drift: nothing updates it, and add_item copies it out of the same guarded read that proves the parent is this merchant''s draft.';
COMMENT ON TABLE invoice_number_sequences IS
    'One row per merchant: the prefix every invoice number carries and the number the next finalize will take. An ordinary table and NOT a Postgres SEQUENCE, deliberately: nextval is non-transactional, so a rolled-back finalize would burn a number and leave a hole a tax authority reads as a destroyed document. Has no .cstack model because its only write is a self-referencing SET expression, which cratestack 0.12.0 cannot represent.';
COMMENT ON COLUMN events.type IS
    'Constrained to the thirteen event types in docs/flows/webhooks.md. Only real Stripe event types, so a merchant''s existing Stripe-shaped handler recognises every one of them. The four invoice.* types (0036) each have a writer in the same commit that added them, which is this vocabulary''s standing rule; invoice.marked_uncollectible and customer.created/updated are deliberately absent because nothing writes them.';
