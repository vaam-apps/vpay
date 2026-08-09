# Money

## The rule

Money is an **integer count of a currency's minor unit**. There is no floating
point anywhere in the money path, and `clippy::float_arithmetic` is denied
workspace-wide to keep it that way.

## XAF is zero-decimal

The Central African CFA franc has no centimes in circulating use. Its minor unit
*is* its major unit.

```
amount: 5000, currency: "xaf"   →   5,000 FCFA
```

Not 50.00. This matches Stripe's own zero-decimal currency handling, so a
developer who knows Stripe gets it right by default.

## The single conversion point

Exactly one function renders an amount for a provider:

```rust
Money::to_provider_string()   // backends/crates/vpay-core/src/money.rs
```

It reads the exponent from the *currency*, because the exponent is a property of
the currency universally — not of a deployment, an environment or a config row.

| Currency | Exponent | `Money::new(5000, …)` renders |
|---|---|---|
| XAF | 0 | `5000` |
| EUR | 2 | `50.00` |

The frontend mirrors this in `@vpay/api-client`'s `formatAmount`, covered by the
same table of cases.

## Why EUR is here at all

MTN's sandbox rejects XAF and accepts EUR only. That is a property of a
*provider profile*, expressed as a config value — never a code branch. It has a
useful side effect: teams using the sandbox exercise the two-decimal formatting
path daily, so the decimal branch is never untested code.

**Amounts against a EUR profile are notional.** No FX happens and none is implied.

## Invariants

1. A negative amount cannot be constructed — `Money::new` rejects it.
2. Arithmetic across currencies is a compile-time-shaped error, returned as
   `MoneyError::CurrencyMismatch`.
3. Subtraction that would go below zero fails. A refund can never exceed what
   was captured, at the type level as well as in the database.

All three are covered by tests in `vpay-core::money`.
