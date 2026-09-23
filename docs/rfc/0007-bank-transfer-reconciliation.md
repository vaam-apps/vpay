# RFC-0007: Bank transfers reconciled from bank statements

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-09-23
- **Related:** [RFC-0004](0004-billing-on-top-of-invoices.md) § 6 (manual
  payments) and § 7 (bank payments),
  [RFC-0002](0002-gdpr-policy-and-operator-decisions.md) (retention),
  [RFC-0008](0008-payment-method-routing.md) (routing),
  [docs/flows/crash-safety.md](../flows/crash-safety.md).

Nothing in this document is built.

## Problem

A bank transfer is the one payment method every merchant already has. It
needs no PSP, no card network and no mobile-money contract. What vpay lacks is
a way to know one has arrived:

- CEMAC has no open-banking standard.
- The 2026-09-23 provider evaluation found no PSP documenting bank-transfer
  collection in Cameroon.
- Today the merchant can only record the transfer by hand (RFC-0004 § 6,
  `paid_out_of_band`).

Every bank already produces the evidence, though: **the account statement**.
A payer who quotes a reference vpay issued, and a statement line carrying that
reference, is enough to settle a bill with no partnership at all.

## Proposal

### 1. The method: `bank_transfer`, into the merchant's own account

- A payment intent or invoice may offer method type `bank_transfer`. On
  confirm, the intent moves to `requires_action` with Stripe's
  `next_action.display_bank_transfer_instructions`, which carries:
  - the merchant's receiving account (holder name, bank, account identifier
    in the local format, IBAN or BIC where they apply);
  - the amount and currency;
  - a **reference** unique to this payment.
- **The money goes straight into the merchant's own account.** vpay never
  touches it, so pass-through holds trivially.
- **The receiving account is configuration, never a `/v1` write.** Account
  details shown to payers are the most valuable target in this design: a
  changed account number silently redirects every payment. They live in YAML
  under `merchant_clients[]`, changed by a reviewed pull request, for the same
  reason webhook endpoints do (`vpay-config/src/oauth.rs`), and are validated
  at boot.

### 2. The reference

- **ISO 11649 structured creditor reference** (`RF` + two check digits +
  up to 21 characters). It is the standard for this, it survives being typed
  into a bank's free-text field, and its mod-97 check digits catch the
  transpositions that make a payment unmatchable.
- The body uses Crockford's alphabet, as invoice numbers do
  ([invoices.md](../flows/invoices.md)), because the payer copies it by hand.
- It is short, because banks truncate transfer labels, and the maximum
  surviving length per bank is unknown (open question 1). An invoice's own
  number is **not** used as the reference: a number is per merchant, while a
  reference must identify the payment even when two merchants share a
  receiving bank.

### 3. Statements in

Three sources, with different **trust**. The source is recorded on every
statement and shown on everything it settles.

| Source                                                                                                                 | Trust                                                                                       |
| ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| **Fetched from the bank** (SFTP, or a bank API where one exists), by a worker job with operator-configured credentials | Authoritative for that account: the analogue of an authenticated `query_status`             |
| **Uploaded by the merchant** (`POST /v1/bank_statements`, multipart)                                                   | A merchant's statement, exactly as `paid_out_of_band` is. vpay cannot verify it and says so |
| **Uploaded by an operator** (needs ADR-0008's unbuilt dashboard writes)                                                | Not v1                                                                                      |

**Formats:**

- ISO 20022 `camt.053` (end-of-day statement) and `camt.054` (intraday
  notification);
- SWIFT `MT940`;
- **CSV with a per-bank column mapping** in configuration, because which
  formats Cameroon banks actually export is unknown (open question 1).

PDF statements are not parsed.

**Each statement line is stored once.** A unique index on the line's identity
makes re-importing the same file, or an overlapping one, idempotent:

- the bank's own entry reference (`AcctSvcrRef` in camt) where present;
- otherwise account, booking date, amount and a hash of the remittance text.

### 4. Matching

A worker job, one transaction per line, compare-and-swap throughout:

1. **Automatic match:** the reference parses with valid check digits, names
   one payment in `requires_action` for this merchant's account, and the
   credited amount and currency equal what was asked. The line is matched, and
   the payment settles through the **ordinary settlement transaction**. So an
   invoice it pays flips inside it, `invoice.paid` and
   `payment_intent.succeeded` are emitted, and the ledger posts, exactly as for
   a rail.
2. **Everything else goes to review:**
   - no reference, or bad check digits;
   - an unknown reference;
   - **less or more than asked** (partial payments stay out of scope;
     RFC-0004);
   - a reference already settled.
3. **A line matches at most once** (a unique index on the match's line id), and
   **a payment is settled by at most one line** (the settlement
   compare-and-swap). A duplicate transfer is a review item, never a second
   settlement.

**Review** is a merchant API:

- `GET /v1/bank_statement_lines?status=unmatched` lists the queue;
- `POST /v1/bank_statement_lines/{id}/match` with `payment_intent` or
  `invoice` matches a line by hand;
- `POST …/{id}/dismiss` with a reason dismisses it.

A manual match of an amount other than the one asked is refused, for the
partial-payment reason. The merchant settles that difference outside vpay.

**Bank charges.** If a payer's bank deducts a fee in transit, the credited
amount is short and the line goes to review. An auto-match tolerance is **not**
proposed: a tolerance is a rule for writing off money, and that belongs to the
merchant (open question 3).

### 5. Expiry

A transfer takes days, not minutes. The instructions carry a
`bank_transfer.expires_after_days` (per merchant, default 14). The checkout
session a bank-transfer payment runs in cannot be the 24-hour session every
intent gets today, so this RFC needs **method-dependent session expiry**. A
transfer that arrives after expiry is a review item.

### 6. Objects and events

| Object                | Prefix  | Notes                                                                  |
| --------------------- | ------- | ---------------------------------------------------------------------- |
| `bank_statement`      | `bst_…` | source, account, period, format, line count, imported_at               |
| `bank_statement_line` | `bsl_…` | amount, currency, booking date, remittance text, status, match, source |

**No new event types.** Stripe has none for this, and vpay adds only Stripe's
types. A merchant learns about a match from `payment_intent.succeeded` and
`invoice.paid`, which carry the payment method type, and about the review
queue by reading it.

### 7. Personal data

A statement line carries the **payer's** name, and often their account
number: someone else's personal data, received from a bank. Every column goes
into `schemas/privacy-inventory.yaml` with a retention class under ADR-0020 § 4.

- An **unmatched** line is deleted when the review closes, or at its class's
  horizon.
- A **matched** line is kept for as long as the payment's accounting record.

The retention periods themselves are RFC-0002's owner's decision
(open question 4).

## Alternatives considered

- **Virtual accounts per payer** (one receiving account number per payment).
  That matches perfectly without a reference, but it needs a bank or PSP that
  issues them. None in Cameroon is documented. It is a later adapter under
  RFC-0004 § 7's `Instructions` flow, not this RFC.
- **Match by amount and date alone.** Rejected: two customers paying the same
  subscription price on the same day are indistinguishable, and a wrong match
  settles someone else's bill.
- **Keep it manual** (RFC-0004 § 6). It stays available, and this RFC makes
  the common case automatic.

## Open questions

1. **What Cameroon banks export** (camt, MT940, CSV, PDF only), whether any
   offer SFTP or an API, and how many characters of a transfer label survive
   end to end. (Ask Ecobank Cameroun, Afriland, SG Cameroun, UBA, BICEC.)
2. **Local account format** on the instructions: the CEMAC RIB, IBAN where
   issued, BIC. (Confirm with a bank.)
3. **Bank charges:** does a merchant want a per-merchant auto-match
   tolerance, knowing it is a write-off rule? (Maintainer, merchants.)
4. **Retention** of unmatched and matched lines. (RFC-0002 owner.)
5. **Whether a fetched statement is enough evidence to post to the ledger** as
   a capture, or whether bank-transfer settlements post to a separate account
   until reconciled against a second statement. (Maintainer, accountant.)

## Impact on existing invariants

- **"Callbacks are hints; the authenticated status query moves money"**
  gains an analogue: **a fetched statement line moves money**, and an uploaded
  one is a merchant attestation, labelled as such, like `paid_out_of_band`.
- **Checkout session expiry becomes method-dependent** (§ 5).
- **The ordinary settlement transaction gains a non-rail caller**, the
  statement matcher. It still takes the charge terminal in one transaction.
- **A charge whose rail is not an adapter.** Every charge today names a
  `providers.code` with a linked `ProviderAdapter`, and boot refuses a
  configured rail without one (`ProviderWithoutAdapter`). `bank_transfer` has
  no external call to make, and its "status query" is the statement matcher.
  Two ways to reconcile that, to be chosen when this is built:
  - a `bank_transfer` adapter whose `submit` mints the instructions and whose
    `query_status` answers `Unsupported` (the matcher settles, not the poll
    ladder);
  - a second kind of settlement source beside rails, with its own
    configuration entry.

  The first keeps one vocabulary. The second keeps the port honest about what
  an adapter is.

- **Per-merchant configuration grows** a receiving bank account, validated at
  boot and changed only by review.
- **New job kinds** (`fetch_bank_statements`, `match_statement_lines`) widen
  `kind_is_known`.
