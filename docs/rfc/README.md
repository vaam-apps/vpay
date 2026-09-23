# RFCs

An RFC proposes a change that is not yet decided. Once accepted, the _decision_
is recorded as an ADR and the RFC is marked resolved.

- ADR = a decision that has been made (immutable).
- RFC = a proposal under discussion (mutable, then closed).

| RFC                                                                | Title                                                       | Status                                                                                   |
| ------------------------------------------------------------------ | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| [0001](0001-settlement-and-payouts.md)                             | Settlement and payouts                                      | Draft                                                                                    |
| [0002](0002-gdpr-policy-and-operator-decisions.md)                 | GDPR policy and operator decisions                          | Under review                                                                             |
| [0003](0003-refunds-destinations-and-the-first-ledger-postings.md) | Refunds, refund destinations, and the first ledger postings | Draft                                                                                    |
| [0004](0004-billing-on-top-of-invoices.md)                         | Billing on top of invoices                                  | Draft; §§ 5–6 accepted → [ADR-0024](../adr/0024-customer-filters-and-manual-payments.md) |
| [0005](0005-prepaid-customer-balances.md)                          | Prepaid customer balances                                   | Draft                                                                                    |
| [0006](0006-card-processing-without-a-psp.md)                      | Card processing without a PSP                               | Draft                                                                                    |
| [0007](0007-bank-transfer-reconciliation.md)                       | Bank transfers reconciled from bank statements              | Draft                                                                                    |
| [0008](0008-payment-method-routing.md)                             | Payment-method routing                                      | Draft                                                                                    |

## Template

```markdown
# RFC-NNNN: Title

- Status: Draft | Under review | Accepted (→ ADR-XXXX) | Rejected
- Author:
- Date:

## Problem
## Proposal
## Alternatives considered
## Open questions
## Impact on existing invariants
```
