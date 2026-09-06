-- customers: the merchant-owned record of a payer they expect to see again
-- (S4a of docs/plans/2026-09-06-data-layer.md), and the first vpay table born
-- with a `schemas/vpay.cstack` model rather than acquiring one afterwards.
--
-- WHY THIS OBJECT EXISTS AT ALL, WHEN /v1 ALREADY IGNORES `customer`
--
-- `POST /v1/payment_intents` has accepted a `customer=cus_…` and dropped it
-- since Step 5b (docs/api/README.md, "accepted and ignored"), because a
-- Stripe SDK sends one and refusing it would refuse a request vpay can
-- otherwise satisfy. Dropping it is honest only while there is nothing to
-- point at. This migration is what makes it point somewhere: the column
-- below on `payment_intents` and `checkout_sessions` is a real foreign key,
-- and the API stops ignoring the field in the same change.
--
-- THE MAINTAINER'S THREE DECISIONS (2026-09-05), AND WHERE EACH ONE LIVES
--
--   1. "Phone is the customer identity and is stored by default."  -> `phone`
--      below is an ordinary nullable column with no opt-in anywhere; a
--      merchant who sends one gets it stored and echoed back.
--   2. "Phone-only customers are allowed."  -> `at_least_one_identifier`
--      below requires *one* of name/email/phone, not a name and not an
--      email. A `cus_…` whose only content is a phone number is a legal,
--      complete customer.
--   3. "Retention is twelve months."  -> `last_used_at` below, and the
--      `sweep_idle_customers` job the same migration opens `kind_is_known`
--      for. The horizon itself is `vpay_worker::handlers::CUSTOMER_IDLE_AFTER`
--      and deliberately not a column default: it is a product rule, exactly
--      as the checkout session's 24 hours is (0028).
--
-- NO COLUMN DEFAULTS, AND THAT IS A CONSEQUENCE OF THE MODEL, NOT A STYLE
--
-- `cratestack-macros` drops every `@default(...)` field from
-- `Create{Model}Input` and from `upsert_update_columns`
-- (`model/inputs.rs:20-26`, `model/descriptor/columns.rs:92-101`), which is
-- what migration 0033 had to undo for `providers` after the fact. A table
-- born with a model does not have to: every column below that CrateStack may
-- ever write carries no `DEFAULT`, so the generated input can name it. The
-- two exceptions are deliberate and are the two columns nothing may supply —
-- `seq` (GENERATED ALWAYS) and `created_at`/`updated_at`, which are
-- `now()` for `checkout_sessions`' reason.
CREATE TABLE customers (
    -- Caller-supplied `cus_…` (vpay_core::ids::customer_id), generated before
    -- the insert exactly as `pi_…`, `ch_…` and `cs_…` are.
    id TEXT PRIMARY KEY,
    -- The list cursor. `payment_intents.seq` (0014), `events.seq` (0018) and
    -- `checkout_sessions.seq` (0028)'s argument, unchanged: `created_at` ties
    -- under a burst and a cursor that can skip a row is a page a merchant
    -- never sees.
    seq BIGINT GENERATED ALWAYS AS IDENTITY,
    -- No FK: there is no merchants table (ADR-0003; see 0003's comment).
    -- Every query in vpay_db::customers filters on it in SQL.
    --
    -- IT IS ALSO THE PRIVACY BOUNDARY. A customer is personal data a
    -- *merchant* collected. There is deliberately no unique index on
    -- (phone), on (email), or on anything that would let two merchants of the
    -- same deployment discover they share a payer: cross-merchant identity is
    -- a product vpay does not have and must not acquire by accident.
    merchant_id TEXT NOT NULL,
    -- Live or test money. Copied from the deployment's own configuration at
    -- creation, exactly as `payment_intents.livemode` is, and never updated.
    livemode BOOLEAN NOT NULL,
    -- The three identifiers, every one nullable and at least one required —
    -- see `at_least_one_identifier`.
    name TEXT,
    email TEXT,
    -- The canonical Cameroon MSISDN, `2376XXXXXXXX`, twelve digits and no
    -- `+`, exactly as `vpay_api::v1::account_holders` canonicalises one
    -- before it reaches a rail. Stored canonical rather than as typed so that
    -- a merchant who created a customer from `+237 6 00 00 02 00` and one who
    -- created it from `600000200` have one value between them, and so the
    -- column can be compared to a rail's `payer_ref` without a normaliser in
    -- the middle.
    --
    -- The CHECK below is a *shape* backstop and deliberately looser than that
    -- rule — 8 to 15 digits, E.164's own bound. `vpay_api`'s canonicaliser
    -- owns the market rule and may widen it (a second country code, a second
    -- mobile prefix), and a constraint that restated two thirds of it would
    -- be a second, drifting copy that refused a number a later vpay accepts.
    -- What this catches is a writer that put something else in the column.
    phone TEXT,
    -- The merchant's own key/value pairs. `JSONB NOT NULL` with **no
    -- DEFAULT**, which is what keeps `Customers::create` and
    -- `Customers::update` hand-written `sqlx` statements rather than
    -- CrateStack calls: `schemas/vpay.cstack`'s `model Customer` does not
    -- declare this column (see that model's GAP note for the two measured
    -- reasons), so a generated INSERT omits it, and without a DEFAULT that
    -- omission is a loud `23502` rather than a silent `{}` over a merchant's
    -- data. The absence of the DEFAULT is the tripwire; it is not an
    -- oversight.
    metadata JSONB NOT NULL,
    -- When something last *used* this customer, which is the whole input to
    -- the twelve-month retention sweep.
    --
    -- "Used" is defined once, here, and means: created, updated, or named by
    -- a payment intent or a checkout session (`vpay_db::customers::
    -- Customers::touch_last_used`, called on create, on update, on intent
    -- create, on confirm and on session create). Invoices would be the third
    -- referencing object and do not exist — see docs/flows/customers.md.
    --
    -- NOT NULL and written by the writer, never by a trigger or a DEFAULT:
    -- the sweep deletes on this column, so a row whose value was filled in by
    -- something no code names is a row whose deletion nobody can explain.
    last_used_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Maintained by the writers in vpay_db::customers, not by a trigger — the
    -- same choice 0014 and 0028 made, and for the same reason (a trigger is a
    -- write nothing in the code names).
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),

    -- AT LEAST ONE OF name / email / phone, AND WHY IT IS A CONSTRAINT
    --
    -- A customer with all three null is a `cus_…` that names nobody: it can
    -- never be matched to a payer, never be reached, and never be recognised
    -- by the merchant who created it. It is also the shape a form with every
    -- field left blank produces, so it is the one an integration creates by
    -- accident and then accumulates.
    --
    -- Which one is present is deliberately unconstrained (the maintainer's
    -- decision 2, above): phone-only is a first-class customer on a mobile
    -- money rail, and requiring an email would make vpay refuse the payer
    -- population it exists for.
    --
    -- MULTI-COLUMN, therefore INVISIBLE to `cratestack migrate baseline` in
    -- both directions — `introspect/postgres/constraints.rs:62` filters
    -- `array_length(c.conkey, 1) = 1`, so this constraint contributes nothing
    -- to the drift count and its *loss* would contribute nothing either. That
    -- is why `postgres_smoke.rs`'s
    -- `a_customer_with_no_name_email_or_phone_is_refused_by_the_database`
    -- asserts it directly against a real Postgres, exactly as the money
    -- CHECKs are asserted: the drift report cannot be the guard here.
    CONSTRAINT at_least_one_identifier CHECK (
        name IS NOT NULL OR email IS NOT NULL OR phone IS NOT NULL
    ),

    -- The three bounds the API also enforces, so a merchant gets a `400`
    -- naming the parameter and this is the backstop. 0028's argument on
    -- `success_url` applies verbatim: trip the CHECK and an over-long value
    -- comes back as a `500` telling a merchant vpay is broken.
    --
    -- The floors are 1, not 0: an empty string is not a name, and `name=` is
    -- what a client templating an absent field emits. The API turns blank
    -- into absent before it gets here; this refuses a writer that does not.
    CONSTRAINT name_length CHECK (name IS NULL OR char_length(name) BETWEEN 1 AND 256),
    CONSTRAINT email_length CHECK (email IS NULL OR char_length(email) BETWEEN 1 AND 512),
    -- Digits only, 8 to 15 of them — E.164's own length bound, and
    -- deliberately wider than the twelve-digit Cameroon MSISDN
    -- `vpay_api::v1::account_holders::canonical_msisdn` produces today. The
    -- market rule (which country codes, which mobile prefixes) belongs to
    -- that canonicaliser and may widen; what this refuses is a value that is
    -- not a phone number at all — a `+`, a name, a whole `cus_…`, an empty
    -- string. See the column comment.
    CONSTRAINT phone_is_a_canonical_msisdn CHECK (
        phone IS NULL OR phone ~ '^[0-9]{8,15}$'
    ),

    -- `metadata_is_object`, exactly as `payment_intents` (0014) and `refunds`
    -- (0017) spell it: the wire contract is a string map, `vpay_api::model`'s
    -- `metadata_of` renders `Map<String, Value>`, and an array or a scalar in
    -- this column is a `500` on every later read of the row.
    CONSTRAINT metadata_is_object CHECK (jsonb_typeof(metadata) = 'object')

    -- DELIBERATELY NOT HERE: `CHECK (last_used_at >= created_at)`.
    --
    -- It looks like a free invariant and is a latent outage. Both instants
    -- are this process's `OffsetDateTime::now_utc()` (see the column comment
    -- on `last_used_at` for why neither is `now()`), and two vpay processes
    -- do not share a clock. A server whose clock is a second behind the one
    -- that created a customer would trip it on the very next
    -- `touch_last_used`, turning a `POST /v1/payment_intents` into a `500`
    -- for a reason no merchant could act on. The rule it would express —
    -- "nothing sets last_used_at backwards" — is worth having and is
    -- enforced where it can be enforced without a clock comparison:
    -- `touch_last_used` is `SET last_used_at = GREATEST(last_used_at, $2)`.
);

-- An identity column is not implicitly unique, and every cursor below assumes
-- a total order — the same reasoning as `payment_intents_seq_key` (0014).
CREATE UNIQUE INDEX customers_seq_key ON customers (seq);

-- `GET /v1/customers`: merchant-scoped, newest first. The same shape as
-- `payment_intents_merchant_seq_idx` and `checkout_sessions_merchant_seq_idx`.
CREATE INDEX customers_merchant_seq_idx ON customers (merchant_id, seq DESC);

-- The retention sweep's backlog query: "which customers have been idle
-- longest?", asked once an hour. Ordered by the column the horizon is
-- compared against, so the sweep's `ORDER BY last_used_at` is an index scan
-- rather than a sort of every customer in the deployment.
--
-- NOT partial, unlike `checkout_sessions_open_by_intent_idx` and
-- `events_pending_idx`: there is no status column to make a partial index
-- over, because every customer is eligible for the sweep and what disqualifies
-- one is a *reference from another table*, which no index on this one can see.
CREATE INDEX customers_idle_idx ON customers (last_used_at);

-- `customer` becomes a real reference on the two objects that can name one.
--
-- WHY A COLUMN AND A FOREIGN KEY RATHER THAN A JOIN TABLE
--
-- An intent has at most one customer and a customer has many intents, which
-- is the shape a nullable column with a foreign key expresses exactly. The FK
-- is real for `charges.payment_intent_id`'s reason (0004): `customers` is a
-- table this schema owns, unlike the merchant, so there is something to point
-- at.
--
-- ON DELETE is deliberately **omitted**, i.e. `NO ACTION`. A customer that an
-- intent references cannot be deleted, and that is the whole safety property
-- of the retention sweep: `DELETE /v1/customers/{id}` and
-- `sweep_idle_customers` both answer to this constraint, so "the merchant's
-- payment history survives a customer deletion" is enforced by Postgres
-- rather than by two call sites remembering. `ON DELETE SET NULL` was the
-- alternative and is worse: it would let a delete succeed and silently
-- detach a payment from the payer it was taken from, which is the record a
-- dispute is settled with.
--
-- The API's answer to a delete the FK refuses is a `409` naming the
-- constraint's meaning, not a `500` — see `vpay_api::v1::customers`.
ALTER TABLE payment_intents ADD COLUMN customer_id TEXT REFERENCES customers (id);
ALTER TABLE checkout_sessions ADD COLUMN customer_id TEXT REFERENCES customers (id);

-- Both directions of "which intents belong to this customer?" — the sweep's
-- `NOT EXISTS` guard and a future `GET /v1/payment_intents?customer=`. Without
-- them the sweep's guard is a sequential scan of every intent per candidate
-- customer.
CREATE INDEX payment_intents_customer_idx ON payment_intents (customer_id)
    WHERE customer_id IS NOT NULL;
CREATE INDEX checkout_sessions_customer_idx ON checkout_sessions (customer_id)
    WHERE customer_id IS NOT NULL;

-- THE EVENT VOCABULARY, REOPENED FOR THE THREE CUSTOMER TYPES
--
-- Same mechanism as 0024's `fanout_state_is_known`, 0022/0023's
-- `jobs.kind_is_known` and 0029's own reopening: the database is what refuses
-- a type no code writes, so the CHECK moves in lockstep with the code rather
-- than being written permissively ahead of it.
--
-- All three are Stripe's own type names, which is docs/flows/webhooks.md's
-- standing rule: a custom type is silently dropped by any merchant using
-- `stripe-node`'s typed event union or an exhaustive `switch`, so an invented
-- one would have no branch at all in a Stripe-shaped handler.
--
-- `customer.deleted` is the one that had to exist. A hard delete is
-- unobservable by polling — the object is gone, and a `GET` afterwards is
-- byte-identical to a `GET` for an id that never existed — so without an
-- event a merchant whose customer was swept has no way to learn it happened.
-- Stripe emits `customer.deleted` for exactly this and merchants already
-- handle it.
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
    'customer.created',
    'customer.updated',
    'customer.deleted'
));

-- THE JOB VOCABULARY, REOPENED FOR THE RETENTION SWEEP
--
-- 0023's argument unchanged: a kind spelled here and not in
-- `vpay_worker::jobs::JobKind` is a row no worker can dispatch; one spelled
-- there and not here is refused at the insert.
--
-- Its own kind rather than a fourth statement inside `sweep_expired`, which
-- is the shape 0023 chose for `scan_deliveries` and the opposite of what
-- `sweep_expired` did with checkout sessions. The difference that decides it
-- is what a failure means: the three statements inside `sweep_expired` are
-- bounded deletes of vpay's own bookkeeping whose healthy answer is zero,
-- while this one deletes a merchant's personal-data records and emits a
-- merchant-visible event for each. Sharing a job would mean a customer sweep
-- that started failing showed up as "the housekeeping sweep is unhealthy",
-- with the idempotency-key deletes it has nothing to do with in the same
-- number, and would put a merchant-visible event behind the same lease as an
-- internal delete.
ALTER TABLE jobs DROP CONSTRAINT kind_is_known;
ALTER TABLE jobs ADD CONSTRAINT kind_is_known CHECK (kind IN
  ('poll_charge','resubmit_charge','sweep_expired','scan_live_charges','fan_out_events','deliver_webhook','scan_deliveries','sweep_idle_customers'));

COMMENT ON TABLE customers IS
    'A merchant-owned record of a payer they expect to see again (S4a). Personal data: hard-deleted by DELETE /v1/customers/{id} and by the twelve-month sweep_idle_customers job, never soft-deleted. No cross-merchant identity — nothing here is unique across merchant_id, deliberately.';
COMMENT ON COLUMN customers.phone IS
    'The canonical Cameroon MSISDN (2376XXXXXXXX, twelve digits, no +) as vpay_api::v1::account_holders canonicalises one. Stored by the maintainer''s decision of 2026-09-05: phone is the customer identity on a mobile money rail and is present by default. The CHECK is a shape backstop; the market rule is the API''s canonicaliser and may widen.';
COMMENT ON COLUMN customers.last_used_at IS
    'When this customer was last created, updated, or named by a payment intent or a checkout session. The sole input to the twelve-month retention sweep (sweep_idle_customers). Written by vpay_db::customers::Customers::touch_last_used, never by a trigger or a DEFAULT.';
COMMENT ON COLUMN customers.metadata IS
    'The merchant''s own key/value pairs (<=50 keys, <=40-char key, <=500-char value, enforced by vpay_api). JSONB NOT NULL with NO DEFAULT on purpose: schemas/vpay.cstack''s model Customer does not declare this column, so a generated INSERT that omitted it would be a loud 23502 rather than a silent {} over a merchant''s data.';
COMMENT ON CONSTRAINT at_least_one_identifier ON customers IS
    'A customer names somebody: at least one of name, email, phone. Which one is unconstrained — phone-only is a first-class customer (maintainer decision, 2026-09-05). Multi-column, therefore invisible to cratestack migrate baseline in both directions; postgres_smoke.rs asserts it directly.';
COMMENT ON COLUMN payment_intents.customer_id IS
    'The customer this intent is for, or NULL. Was accepted-and-ignored as the request field `customer` from Step 5b until migration 0034 made it real. NO ACTION on delete, deliberately: an intent pins its customer against both DELETE /v1/customers/{id} and the retention sweep, so a payment can never be detached from the payer it was taken from.';
COMMENT ON COLUMN checkout_sessions.customer_id IS
    'The customer this session is for, or NULL. Copied from the session''s intent at create when the intent has one, or supplied directly. Same NO ACTION reasoning as payment_intents.customer_id.';
COMMENT ON COLUMN events.type IS
    'Constrained to the eleven event types in docs/flows/webhooks.md. Only real Stripe event types, so a merchant''s existing Stripe-shaped handler recognises every one of them. customer.created/updated/deleted (0034) are the only ones whose data.object is a customer; customer.deleted is the only way a merchant can learn about a hard delete, since a GET afterwards is byte-identical to one for an id that never existed.';
COMMENT ON COLUMN events.object_id IS
    'The id of the object this event is about: pi_ for payment_intent.*, ch_/re_ for the refund types, cs_ for checkout.session.expired (0029) and cus_ for customer.* (0034). Untyped and un-foreign-keyed on purpose — it points into five different tables depending on type, and a polymorphic reference cannot be a foreign key. It is NOT a foreign key onto customers in particular for a second reason: customer.deleted describes a row that no longer exists.';
COMMENT ON COLUMN jobs.kind IS
    'What to run, from a vocabulary closed by kind_is_known and mirrored exactly by vpay_worker::jobs::JobKind: poll_charge, resubmit_charge, sweep_expired, scan_live_charges, fan_out_events, deliver_webhook, scan_deliveries, and sweep_idle_customers (0034 — the twelve-month retention sweep, a singleton on dedupe key sweep:customers). A kind spelled here and not in that enum is a row no worker can dispatch; one spelled there and not here is refused at the insert.';
