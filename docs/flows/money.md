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

Exactly one *conversion* renders an amount for a provider, in two encodings:

```rust
Money::to_provider_string()   // backends/crates/vpay-core/src/money.rs — "5000", "50.00"
Money::to_provider_minor()    // the same amount as an integer count of minor units
```

`to_provider_string` reads the exponent from the *currency*, because the exponent
is a property of the currency universally — not of a deployment, an environment
or a config row. `to_provider_minor` reads no exponent at all: it returns exactly
`Money::minor`, the number the amount is already stored as. Neither *scales*
anything — there is still one conversion, in two encodings — and the integer form
exists because Orange Money's request body takes `"amount": 5000` as a JSON
number while MTN's takes the string form
(`the_amount_is_a_json_number_in_minor_units` in
`vpay-adapter-orange-money`; `xaf_renders_the_same_digits_in_both_encodings` and
`eur_pads_the_fractional_part` in `vpay-core`). Sending minor units to a rail that
expects major units is a 100× error nothing downstream can detect, so an adapter
picks the encoding its rail's own documentation names — never the one that
happens to compile.

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

**The demo stack is the one place that diverges, deliberately, since
2026-09-04 (Step 9).** `.e2e/application-demo.yml` — the overlay
`just gen-demo-keys` writes — carries its own `providers:` block putting
**both** rails on XAF, because the demo shop prices its catalogue in XAF,
offers a payer both rails, and `currencies_agree` refuses a confirm whose rail
settles in another currency than the intent. That stack talks to a WireMock
host, not to MTN's sandbox, and **no MTN mapping matches on a currency at
all** — `vpay_adapter_mtn_momo::wire::StatusResponse` never deserialises one.
`config/application.yml` and `config/application-sandbox.yml` are unchanged and
still put `mtn_momo` on EUR. **Do not read the demo as "MTN accepts XAF".** It
does not; the sentence at the head of this section is why.

## Invariants

1. A negative amount cannot be constructed — `Money::new` rejects it.
2. Arithmetic across currencies is a compile-time-shaped error, returned as
   `MoneyError::CurrencyMismatch`.
3. Subtraction that would go below zero fails. A refund can never exceed what
   was captured, at the type level as well as in the database.

All three are covered by tests in `vpay-core::money`.
