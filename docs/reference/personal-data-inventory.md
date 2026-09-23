# Personal-data inventory

_Issue [#144](https://github.com/vaam-apps/vpay/issues/144). The machine-readable
companion is [`schemas/privacy-inventory.yaml`](../../schemas/privacy-inventory.yaml);
this page is the human-facing version. The architecture is
[ADR-0020](../adr/0020-privacy-controls-and-evidence.md); the unresolved policy
decisions (legal basis, roles, retention periods) are
[RFC-0002](../rfc/0002-gdpr-policy-and-operator-decisions.md)'s D1–D13 and
[open-decisions.md](../open-decisions.md)._

## What this is

An inventory of every personal-data processing surface vpay owns: every
database column across the authoritative schema, and every non-database surface
(logs, metrics, traces, events, webhooks, the payer's browser, the rails,
the merchant API, the observability listener, backups).

It is **not** a legal document. It classifies _what the product stores,
derives, logs, emits and sends_, so that the engineering controls (erasure,
retention, minimisation, export, incident evidence) can be built on a shared,
code-grounded map. Controller/processor roles, lawful bases, notices, retention
**periods** and disclosure approval are human decisions and are recorded here as
`unresolved` rather than guessed — RFC-0002 keeps each one open with a named
owner.

## How it is checked

`cargo xtask verify-privacy-inventory` (a gate in `just verify`) reads
`schemas/privacy-inventory.yaml` and derives the authoritative database column
set by parsing `backends/migrations` (the source of truth — `schemas/vpay.cstack`
deliberately models less than the whole database). It fails in **both**
directions, exactly like `verify-status`:

- a migrated column with no element in the inventory → fail;
- an inventory element copy naming no live column → fail.

So a new privacy-relevant column cannot land silently, and a stale inventory
row cannot survive the column it named. The gate also validates every element's
six fields and every registered non-database surface.

_(The parser models `CREATE TABLE`, `ALTER TABLE … ADD/DROP/RENAME COLUMN` and
`DROP TABLE`. **`DROP TABLE` was added on review, 2026-09-17, and the omission
had cost exactly what this page exists to prevent:** migration `0009` drops
`merchant_api_keys` outright, so all eight of that table's columns stayed in
the derived set, both directions of the gate agreed about a table no database
has, and this page published `merchant_api_keys.key_digest`/`key_prefix` as
stored merchant credentials in the `oauth_client_secret` row below. The eight
rows and that claim are gone; the column count moved 303 → 295.
`a_dropped_table_leaves_no_columns_behind` and
`a_stale_row_naming_a_dropped_tables_column_fails_direction_b` in `.xtask` pin
both halves.)_

_(**A second review pass, 2026-09-18, found the same shape three more times,
and this time closed the class rather than the instances.** The parser matched
its keywords as one literal space: `CREATE  TABLE` with two spaces, a tab, or a
line break after `CREATE` derived **nothing at all** — the whole table, every
column of it, invisible to both directions. `ALTER  TABLE … ADD COLUMN email`
lost `email`; `DROP  TABLE t` left `t` standing, re-opening the `0009` hole
through a second space. Nothing here reformats SQL; the only thing keeping the
gate honest was that all 48 migrations happen to be typed with one space. The
match also had no word boundary and no string-literal awareness — these
migrations' `COMMENT ON` bodies are essays that discuss migrations, so one
containing the words `DROP TABLE customers` would have removed the real
`customers` table from the derived set. `kw_end`/`words_at` in `.xtask` replace
the whole approach, and `a_keyword_pair_survives_any_whitespace_between_its_words`,
`a_keyword_inside_an_identifier_is_not_a_keyword` and
`a_ddl_keyword_inside_a_string_literal_is_data` pin all three.)_

_(**And the forms the parser does not model are now refused rather than
skipped.** This paragraph used to end by naming `ALTER TABLE … RENAME TO`,
`CREATE TABLE … (LIKE …)` and `PARTITION OF` as unmodelled, and said the last
two "would produce a table with no columns, which does not [fail loudly]".
That was the honest thing to write and the wrong thing to leave: a table with
no columns is one **neither direction of the gate can see**, so the gate stays
green over an unclassified table — the `0009` failure again, in a form nobody
had written yet. `create_table_parts` now returns a three-armed `CreateTable`,
and `AS SELECT`, `PARTITION OF`, `INHERITS`, a `(LIKE …)` body and
`ALTER TABLE … RENAME TO` each **fail the gate by name**, telling the author to
model the form before landing the migration. None of the five appears in
`backends/migrations` today, which is now a fact the gate keeps rather than one
a reviewer has to re-establish.
`a_create_table_the_parser_cannot_read_is_refused_not_skipped` and
`a_table_rename_is_refused_rather_than_silently_ignored` pin it.)_

_(**Schema qualification is dropped, not kept**, and this page publishes the
bare names because of it. `0006` and `0013` create five tables as
`authkestra.oauth_clients`, `…oauth_codes`, `…oauth_device_codes`,
`…oauth_dpop_jti` and `…oauth_refresh_tokens`; the derived set — and therefore
the inventory, and therefore the tables named below — files them as
`oauth_clients`, `oauth_codes` and so on. Two tables of the same bare name in
different schemas would merge into one entry, and a column of either would then
satisfy a classification written for the other. No such pair exists: 31 created
tables, 31 distinct bare names. Said here rather than left in
`normalise_sql_name`'s doc comment, because this page is where someone looks to
find out where a value is stored.)_

## The element model

Each entry in the inventory is a **data element**: a stable identifier shared by
every copy of one piece of data (ADR-0020 §1). Each element carries the eight
fields below and a `copies` list of the database columns (and, once
the non-DB registry is complete, the non-database surfaces) that hold it.
_(This said "the six-field classification" and then listed eight until
2026-09-17. Six is what the gate checks is non-empty — `subject`, `purpose`,
`tenant_boundary`, `retention`, `owner`, `control`; `necessary` is a boolean
and `recipients` a list, so "present" is all either can be.)_

_(The `subject` row below listed **four** values until 2026-09-18, folding
"system/technical" into `none`. The gate enforces **five** —
`PRIVACY_SUBJECT_VOCAB` in `.xtask/src/main.rs` — and `system` is not a synonym
for `none` here: one element uses it, `oauth_signing_key`, and the honesty
note on that element below already treated `system` as its own value while
this table denied it existed. A field table that disagrees with the
closed vocabulary its own gate enforces is the kind of thing this page exists
not to be.)_

| Field             | Meaning                                                                              |
| ----------------- | ------------------------------------------------------------------------------------ |
| `subject`         | data-subject category: `payer`, `staff`, `merchant`, `none`, or `system`             |
| `purpose`         | the product purpose for which the value is processed                                 |
| `necessary`       | whether the value is required for that purpose to function                           |
| `tenant_boundary` | `merchant` or `system` — which boundary the value is scoped to                       |
| `recipients`      | third parties that receive it (e.g. `mtn_momo`, `orange_money`, `merchant-endpoint`) |
| `retention`       | the retention **class** (not the period — the period is an RFC-0002 decision)        |
| `owner`           | the accountable retention owner (`maintainer`, `operator`, `merchant`)               |
| `control`         | the erasure/privacy control at `DELETE`/retention time: `redact`, `none`, `forbid`   |

`control` is the bridge to the controls already built: `redact` is what the
customer erasure and retention sweep do (write the `[redacted]` marker, or
`NULL` where no integer is not a place); `forbid` is what the Debug-redaction
rule and `RefundTarget`'s redacting `Debug` implement — a value that must never
reach a log or export.

### `control: redact` names two columns that still need a decision

**Added on review, 2026-09-17; the coverage half was rewritten 2026-09-18.**
Read `control` as the control this element _should_ carry, not as a promise
that one runs today. The customer erasure and the retention sweep are the only
`redact` implementations in this repository, and between them they write
exactly ten statements (`vpay_db::customers`,
`backends/crates/vpay-db/src/customers.rs:1294` and `:1445`–`:1648`):
`customers`' own identifier columns, `events.data`,
`charges.payer_ref`/`payer_ref_masked`/`failure_raw`, `refunds.failure_raw`,
`idempotency_keys.response_body`, `webhook_deliveries.payload_sha256`, and —
added 2026-09-18 — `payment_intents.last_payment_error_code`/
`last_payment_error_message` (NULLed together, forced by `lpe_paired`) and
— added 2026-09-23 with migration `0049`, five more statements in
`redact_out_of_band_references` / `redact_stored_invoice_responses_in_tx` —
`manual_payments.reference` and its copies in `invoice.*` event bodies,
their deliveries' digests and excerpts, and stored invoice responses (see
`payment_reference` below), and
`webhook_deliveries.response_excerpt` (the `[redacted]` marker when non-null).

_(**Added 2026-09-23, [ADR-0027](../adr/0027-erasure-reaches-through-checkout-sessions.md):
no statement was added, but three reach further.** The erasure's `charges`
statement (`payer_ref`, `payer_ref_masked`, `failure_raw`), its `refunds`
statement (`failure_raw`) and its `payment_intents` statement
(`last_payment_error_*`) used to find a payment only through
`payment_intents.customer_id`. They now find it through
`vpay_db::customers::PAYERS_INTENTS`. That constant also covers a
customer-less intent that one of the customer's checkout sessions names,
which is the shape a session created with `customer=` on such an intent left
before vaam-apps/vpay#253. It never covers an intent that names another
customer. No column and no classification moved, so
`schemas/privacy-inventory.yaml` is unchanged. What moved is the evidence
that the `payer_msisdn` and `rail_failure_text` controls run on those rows:
`an_erasure_reaches_a_payment_whose_only_link_to_the_payer_is_a_checkout_session`,
`an_erasure_through_a_session_never_reaches_an_intent_that_names_another_customer`,
and the whole-database scan, which now seeds that shape
(`backends/tests/integration/tests/customers.rs`; see
[../status/verification/2026-09-23-erasure-through-checkout-sessions.md](../status/verification/2026-09-23-erasure-through-checkout-sessions.md)).
The line references in the paragraph above are from 2026-09-18 and are now
out of date. Search for the function names instead.)_

Of the five columns that used to be reached by **none** of those statements,
the erasure now covers three. Two remain, each a documented maintainer decision
that is **not taken**:

| Column                                       | Element             | What actually protects it today                                                                    |
| -------------------------------------------- | ------------------- | -------------------------------------------------------------------------------------------------- |
| `payment_intents.last_payment_error_code`    | `rail_failure_text` | the erasure NULLs it, with `_message`, forced by the `lpe_paired` CHECK — 2026-09-18               |
| `payment_intents.last_payment_error_message` | `rail_failure_text` | the erasure NULLs it, with `_code`, forced by the `lpe_paired` CHECK — 2026-09-18                  |
| `webhook_deliveries.response_excerpt`        | `rail_failure_text` | the erasure writes the `[redacted]` marker when non-null — 2026-09-18                              |
| `provider_requests.error_kind`               | `rail_failure_text` | a 128-character `CHECK`, and nothing else — a maintainer decision **not** to redact (below)        |
| `staff_members.email`                        | `staff_email`       | the `Debug` impl (`vpay_db::staff`) — no staff erasure; staff erasure is a separate concern (#145) |

The heading that named five columns "nothing yet redacts" is itself the record
of the change: as of 2026-09-18 only **two** of the five are reached by no
erasure, and neither is an oversight. `provider_requests.error_kind` holds only
vpay's own operator labels (timeout/TLS/not_implemented), never rail prose —
the inventory classifies it `rail_failure_text`, but reclassifying it is a
maintainer decision and one that has **not been taken**. `staff_members.email`
is a staff subject, not a payer: the customer erasure structurally cannot reach
it, and a staff erasure is a separate concern, issue #145. Both stay listed here
so the decision is named rather than rediscovered.

This is an **unmet criterion, not a gap in the file**: the gate reads `control`
for its closed vocabulary and cannot know which statements exist, and no gate
here claims otherwise. The criterion is now **met for three of the five**
columns — the customer erasure covers them — and stays open only for
`error_kind` and `staff_members.email`, each pending the maintainer decision
above. It keeps #144 open alongside the six non-enumerable surfaces below, for
those two columns and for the two decisions. The two `rail_failure_text`
columns that _were_ the only ones redacted — `charges.failure_raw` and
`refunds.failure_raw` — are redacted because they are "unbounded text a rail
wrote _about_ this payer and may quote their number back" (`customers.rs:1176`);
the same leak argument holds for `response_excerpt`, which is an un-parsed copy
of what a merchant's webhook endpoint said and can echo the payload back.
`last_payment_error_message` is the exception and is stated honestly: every
writer stores the vpay-authored `public_message()` (the rail's prose lives in
`charges.failure_raw`), so NULLing it is defense-in-depth — redacted because the
inventory classifies the column `rail_failure_text`, not because it holds rail
prose today.

_(Two corrections here, 2026-09-18, both found by reading the migrations rather
than the schema. The `error_kind` row said "and a closed operator-label
vocabulary"; there is no such vocabulary — `backends/migrations/0016_create-provider-requests.sql:32-36`
says the column is "Free text on purpose", and the only constraint on it is the
128-character `CHECK`. And the sentence above said the rail-quotes-your-number
argument "is true of the four above it"; it is not true of `error_kind`, which
the same migration says is set "when the attempt failed **without** an HTTP
status: a timeout, a TLS failure" — vpay's own connectivity label, written
precisely when the rail said nothing to quote. That makes `error_kind`'s place
in `rail_failure_text` itself doubtful; see § "Classifications the review
could not settle".)_

_(**And, 2026-09-18, the erasure now covers three of the five columns this
section used to list as reached by nothing.** `redact_stored_copies`
(`customers.rs:1445`) NULLs `payment_intents.last_payment_error_code` and
`last_payment_error_message` together — `lpe_paired` forces the pair both-NULL
or both-set, and the code column's closed-vocabulary CHECK refuses a text
marker, so the redaction is an absence exactly as it is for the coordinate —
and replaces a non-null `webhook_deliveries.response_excerpt` with the marker,
for every delivery of the payer's `customer.*` events, the terminal ones
included. The two that remain — `error_kind` (vpay's own operator label, never
rail prose) and `staff_members.email` (a staff subject, structurally out of
reach of a customer erasure) — are maintainer decisions that are **not taken**,
named in the table above.)_

## The personal-data columns

Not an exhaustive table here — the authoritative copy is the YAML. The elements
whose `subject` is a person are:

| Element                   | Copies (DB)                                                                                                                                                  | Control  |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------- |
| `payer_msisdn`            | `customers.phone`, `charges.payer_ref`, `charges.payer_ref_masked`                                                                                           | `redact` |
| `customer_name`           | `customers.name`                                                                                                                                             | `redact` |
| `customer_email`          | `customers.email`                                                                                                                                            | `redact` |
| `address_formal`          | `customers.address_line1`…`address_country`                                                                                                                  | `redact` |
| `address_gps`             | `customers.address_latitude_microdeg`, `customers.address_longitude_microdeg`                                                                                | `redact` |
| `rail_failure_text`       | `charges.failure_raw`, `refunds.failure_raw`, `webhook_deliveries.response_excerpt`, `payment_intents.last_payment_error*`, `provider_requests.error_kind`   | `redact` |
| `staff_email`             | `staff_members.email`                                                                                                                                        | `redact` |
| `staff_display_name`      | `staff_members.display_name`                                                                                                                                 | `none`   |
| `staff_credential_secret` | `credentials.material`/`subject`/`issuer` (migration 0044 moved the staff credential columns here from `staff_members`)                                      | `forbid` |
| `staff_oauth_token`       | `staff_sessions.access_token`                                                                                                                                | `forbid` |
| `session_oauth_code`      | `oauth_codes`, `oauth_refresh_tokens`, `oauth_device_codes`, `oauth_authorization_codes`, `oauth_client_assertion_jtis`, `oauth_dpop_jti`                    | `forbid` |
| `webhook_url`             | `webhook_deliveries.url`, `checkout_sessions.success_url`/`cancel_url`/`return_url`, `oauth_*redirect_uri`, `charges.redirect_url`/`return_url`              | `forbid` |
| `oauth_signing_key`       | `oauth_signing_keys.kid`/`public_jwk`                                                                                                                        | `forbid` |
| `oauth_client_secret`     | `oauth_clients.client_secret_hash`/`jwks`, `checkout_sessions.publishable_key`/`client_secret_suffix`/`return_token`, `payment_intents.client_secret_suffix` | `forbid` |
| `merchant_metadata`       | `customers.metadata`, `payment_intents.metadata`/`description`, `refunds.metadata`, `invoices.metadata`                                                      | `none`   |
| `stored_api_body`         | `events.data`, `idempotency_keys.response_body`                                                                                                              | `redact` |
| `merchant_note`           | `refunds.reason`, `invoices.description`, `invoice_items.description`                                                                                        | `none`   |
| `payment_reference`       | `manual_payments.reference` (migration `0049`, 2026-09-23)                                                                                                   | `redact` |

**`payment_reference` is a merchant's free text and is still `subject: payer`,
and that is the difference from `merchant_note`** (added 2026-09-23, RFC-0004
§ 6). `out_of_band[reference]` exists to say _how the payer paid_ — a cheque
number beside the drawer's name, a bank transfer's reference, which routinely
carries the payer's name — so it is data about the payer that a merchant typed,
where `invoices.description` is the merchant's note about their own bill. It is
`control: redact` and the control **runs**: the customer erasure
(`vpay_db::customers::redact_out_of_band_references`) writes the marker over
the column and over every copy — the `invoice.paid` body in `events.data`, the
live deliveries' `payload_sha256` (cleared, for the same re-render reason as
the customer bodies) and `response_excerpt`, and the stored
`POST /v1/invoices/{id}/pay` response in `idempotency_keys.response_body`, the
last also on the far side of the issue-#111 race through
`ResponseSubject::OutOfBandInvoice`. And after an erasure the API **refuses** a
new reference on that customer's invoices (`400` naming
`out_of_band[reference]`) rather than re-attach payer detail to a payer vpay
has erased; a payment without one still records.
`erasing_the_customer_redacts_the_out_of_band_reference_everywhere` (every
copy, delivery digests and excerpts included),
`an_erasure_racing_an_out_of_band_payment_neither_deadlocks_nor_leaves_the_reference`,
`a_touch_of_an_invoice_paid_out_of_band_replays_redacted_after_an_erasure`
and `a_stored_invoice_response_written_after_an_erasure_is_redacted_as_it_lands`
(`backends/tests/integration/tests/invoices.rs`), and the whole-database scan
`an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`
(`customers.rs`), which now seeds an out-of-band reference naming the payer,
are the evidence, all green against a real Postgres on 2026-09-23; see
[../status/verification/2026-09-23-manual-payments.md](../status/verification/2026-09-23-manual-payments.md).

Everything else is a `sys_*` element (`subject: none`) — identifiers, timestamps,
statuses, amounts, tenancy references, configuration, worker state — classified
for completeness so that a future column is caught in both directions, not
because they hold personal data.

Two honesty notes on that sentence, both added on review, 2026-09-17:

- **`oauth_signing_key` is in the table above and its `subject` is `system`**,
  not a person. It is listed there because its `control` is `forbid` and a
  reader looking for the protected columns should find it, not because a
  signing key identifies anyone. The 17 elements the gate counts as
  personal-data are the ones whose `subject` is `payer`, `staff` or `merchant`
  (16 until `payment_reference` joined them on 2026-09-23).
- **`jobs.last_error` is `sys_job`, and that classification is the one this
  page is least sure of.** It stores
  `vpay_core::error::display_with_chain` of the worker's own error, bounded to
  2 000 characters (`vpay_worker::run_loop`,
  `backends/crates/vpay-worker/src/run_loop.rs:352`), and
  `ProviderError::Rejected { code, message }` renders the **rail's own
  message** into it — the same class of text as `charges.failure_raw`, which
  this inventory calls `rail_failure_text`, `subject: payer`, `control:
redact`. Calling it `rail_failure_text` would be no better while it stays
  here, because that would assert a `redact` control no statement performs on
  `jobs` (§ above). It is left as `sys_job` **and named here** rather than
  moved quietly, because which of the two it is is a review question, not one
  a parser can answer (ADR-0020 §1: "No gate can determine … whether a stated
  purpose is necessary").

### The `none` controls deserve a note

`merchant_metadata`, `merchant_note` and `staff_display_name` carry `control:
none` deliberately, matching what the erasure already does: `metadata` is the
merchant's own key/value data and is deliberately **not** destroyed on erasure
([customers.md](../flows/customers/privacy-and-erasure.md)); a refund `reason` is
the merchant's free text, the same kind of thing `metadata` is.

`events.data` and `idempotency_keys.response_body` are **not** `merchant_metadata`.
They are their own element, `stored_api_body` (`subject: payer`, `control:
redact`): each stores a complete rendered API object, and when that object is a
customer body it holds payer personal data — which is exactly why the erasure
**rewrites** these copies in the same transaction that erases the customer
([customers.md](../flows/customers/privacy-and-erasure.md)). The honest reading of
`control: redact` here is "the erasure rewrites the payload this container holds",
which is what the erasure statement in `vpay_db::customers` does.

## Non-database surfaces

`schemas/privacy-inventory.yaml` registers the disclosure surfaces ADR-0020 §1
names. Each is marked `enumerable` — whether the surface's contents are
exhaustively pinned by something outside this file, so that a source set
independent of the inventory could be derived from it.

**The gate derives none of them.** _(Corrected 2026-09-17; this sentence read
"whether the gate can currently derive its contents", which was true of no
row in the table.)_ `verify-privacy-inventory` checks that surface ids are
unique, that none collides with an element name, and that `surface` and
`description` are present — and then **counts** the flag. An `enumerable: true`
row is a claim that the surface is closed and knowable (OTLP traces do not
exist; the event store is one already-classified column; browser storage is one
file; the observability listener is two routes), **not** evidence that anything
re-checks it. Nothing fails if OTLP tracing is added tomorrow.

| Surface           | Enumerable | Status                                                                                                            |
| ----------------- | ---------- | ----------------------------------------------------------------------------------------------------------------- |
| `logs`            | no         | no static scanner over `tracing!` sites yet; the Debug-redaction rule is the only guard                           |
| `metrics`         | no         | metric-label derivation is not centrally registered                                                               |
| `traces`          | yes        | OTLP traces deliberately absent (`docs/status.md`: deferred)                                                      |
| `event_store`     | yes        | `events.data` — redacted on customer erasure                                                                      |
| `webhook_payload` | no         | per-event projections (RFC-0002 D2) not yet implemented; payload == API object today                              |
| `browser_storage` | yes        | checkout page local memory on the payer's own device                                                              |
| `provider_rail`   | no         | no static scan of adapter request builders; guarded by `RefundTarget`'s redacting `Debug`                         |
| `merchant_api`    | no         | wire object projections are per-resource, not centrally registered                                                |
| `observability`   | yes        | `/livez` and `/metrics` on `--observability-bind`                                                                 |
| `backup`          | no         | PITR/WAL and full backups — external store; retention of restore copies is a deployment obligation (RFC-0002 D13) |

A surface with `enumerable: false` is an **unmet criterion**: the gate records it
and keeps #144 open on it rather than manufacturing a self-check (RFC-0002 PR 2).
This page is the record of which surfaces still lack an independent source set.

## Classifications the review could not settle

_Added 2026-09-18._ A second review read ten `subject: payer|staff|merchant`
columns and ten `subject: none|system` columns against the migration that
defines each one. It found no fabricated compliance claim, no invented lawful
basis and no retention period stated as a commitment. It did find **six
classifications that the migrations' own comments contradict**. None is
changed here: each needs an element split or a judgement that RFC-0002 D1
(legal basis, controller/processor, purpose necessity) has not yet made, and
quietly re-deciding a GDPR artifact in a review pass is the failure this
directory exists to prevent. They are **unmet criteria of #144**, listed with
the line that contradicts them so the decision can be made rather than
rediscovered.

| Classified as                                                                                                                                                   | The migration says                                                                                                                                                                         | Where                                                                                       |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| `checkout_sessions.publishable_key` — `oauth_client_secret`, `subject: merchant`, `control: forbid`                                                             | "**NOT a secret**: it names a tenant and authorises nothing"                                                                                                                               | `backends/migrations/0028_create-checkout-sessions.sql:284`                                 |
| `checkout_sessions.client_secret_suffix`, `checkout_sessions.return_token`, `payment_intents.client_secret_suffix` — `oauth_client_secret`, `subject: merchant` | "Two **payer** credentials"; "the second half of this session's **payer-facing** `client_secret`"                                                                                          | `0028_create-checkout-sessions.sql:282`, `:286`; `0026_payment-intent-client-secret.sql:64` |
| `authkestra.oauth_clients.jwks` and `oauth_signing_keys.public_jwk` — `control: forbid`                                                                         | both exist to be **published**: "must never hold a private component"; "so `/jwks.json` can publish every currently-valid key"                                                             | `0013_add-authkestra-op-0-7-columns.sql:82`, `0010_reshape-oauth-signing-keys.sql:47`       |
| `payment_intents.last_payment_error_code` — `rail_failure_text`, `subject: payer`                                                                               | a **closed** `failure_code` enum, exactly like `charges.failure_code` and `refunds.failure_code`, which this inventory classifies `sys_status`, `subject: none`                            | `0014_payment-intent-api-fields.sql:86`                                                     |
| `provider_requests.error_kind` — `rail_failure_text`, `subject: payer`                                                                                          | vpay's **own** operator label, set "when the attempt failed without an HTTP status: a timeout, a TLS failure" — written precisely when the rail said nothing to quote                      | `0016_create-provider-requests.sql:32`                                                      |
| `charges.provider_ref_extra` — `sys_provider_ref`, `subject: none`, `control: none`                                                                             | an open rail-supplied `BTreeMap<String, String>` "captured from a previous `submit`", reached by no erasure statement — the same untrusted-rail-input class `rail_failure_text` exists for | `0004_create-charges.sql:42`                                                                |

Four of the six are one defect wearing four hats: an element carries **one**
classification and `oauth_client_secret` holds both genuine merchant secrets
and payer-facing session credentials. That is § "What ADR-0020 §1 asks for and
this file does not yet carry", below, with a name and a line number attached.

## What ADR-0020 §1 asks for and this file does not yet carry

_Added on review, 2026-09-17, because the alternative was for the gap to be
visible only by reading the ADR beside the schema._ ADR-0020 §1 requires every
entry to identify "its source … and code or configuration reference", and
every additional copy to name "why it is necessary and which control removes or
protects it". This inventory carries necessity and control **per element**, and
a copy is `{kind, table, column}` with nothing of its own. So:

- there is no per-copy necessity or control, and no per-element code or
  configuration reference;
- an element whose copies are protected differently — `rail_failure_text` is
  the live example, § "`control: redact` names five columns nothing yet
  redacts" — cannot say so in the schema.

Both are **unmet criteria of #144**, not decisions. Adding either field is a
schema change the gate would have to require in both directions, which is
PR 2's successor work, not PR 1's.

## Unresolved questions

Every element's `retention` names a **class**, never a period: the customer
class, the auth class, the operational class. The **periods** are RFC-0002 D9
(and the current hard-coded 365-day customer rule is in explicit conflict with
D9's "approved, configurable" requirement — see RFC-0002). Legal basis,
controller/processor role and purpose-necessity approval are RFC-0002 D1 and
are `unresolved`, not guessed.

## Status

**2026-09-16, amended 2026-09-17 on review.** The inventory exists and is
machine-checked in both directions against `backends/migrations` by `cargo xtask
verify-privacy-inventory`, wired into `just verify`. **307** database columns
across 26 elements (17 personal-data), and 10 non-database surfaces, are
registered — the gate's own output on 2026-09-23, after migration `0049`
added twelve columns (`invoices.paid_out_of_band` and the eleven of
`manual_payments`, the last its always-`true` `paid_out_of_band`, classified
`sys_status`) and the `payment_reference` element; it said 295 across 25 (16)
until then, and 306 for the few hours `0049` had eleven.

What is **not** done, and each of these keeps #144 open:

- the six `enumerable: false` surfaces lack independent static scanners
  (recorded above, not faked), and the gate enumerates none of the other four
  either — it counts the flag;
- three of the five columns that carried `control: redact` and no redacting
  statement are now covered by the customer erasure (§ above); the remaining
  two — `provider_requests.error_kind` and `staff_members.email` — still have
  no redacting statement and are open maintainer decisions (a reclassification,
  and a staff-erasure feature, issue #145), not oversights;
- six classifications are contradicted by the migration that defines the column
  (§ "Classifications the review could not settle", 2026-09-18) and are left for
  a maintainer, not re-decided here;
- ADR-0020 §1's per-copy necessity/control and per-element source reference are
  absent from the schema (§ above);
- the `@vpay/api-client` and per-event webhook projections RFC-0002 D2 needs are
  unbuilt;
- every RFC-0002 D1–D13 policy answer remains open.

This is the foundation stage of the GDPR epic, not the epic.

_(This section read "Personal-data columns and the non-database surfaces are
registered" with three unmet items until 2026-09-17. It was not wrong, but it
was shorter than the truth: the review added `DROP TABLE` to the parser — which
removed eight columns of a table migration `0009` deleted, 303 → 295 — and
found the two claims now listed above it.)_
