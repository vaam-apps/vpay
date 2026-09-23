# Card and bank providers against RFC-0004 § 7

- **Date:** 2026-09-23
- **Request, verbatim:** "Evaluate the card providers against the RFC-0004
  criteria"
- **Status:** A desk evaluation of **public documentation only**. No account
  was opened, no sandbox key obtained, no API called, and no provider
  contacted. Every "Yes" below means "the provider's documentation says so", not
  "seen working". [AGENTS.md](../../AGENTS.md)'s question applies: nothing here
  has been seen to work.

## The verdict

**No candidate is shown to meet the disqualifying criteria for Visa and
Mastercard acceptance by a Cameroon merchant.** Every one of the four named in
[RFC-0004 § 7](../rfc/0004-billing-on-top-of-invoices.md#7-cards-and-bank-through-third-party-providers)
is a **mobile-money aggregator** first. Its card story is undocumented, retired
or unconfirmed for Cameroon specifically. **No candidate documents bank-transfer
or virtual-account collection in Cameroon.**

| Provider           | Cards for CM merchants, hosted + 3DS                                                            | Status query      | Refund API | Verdict                                                                                                      |
| ------------------ | ----------------------------------------------------------------------------------------------- | ----------------- | ---------- | ------------------------------------------------------------------------------------------------------------ |
| Flutterwave        | Priced for CM (4.8 %); v3 hosted page does not document XAF or 3DS; v4 has no hosted page       | Yes               | Yes        | **Closest candidate — blocked on written confirmation** of hosted XAF cards and of the Cameroon payout pause |
| CinetPay           | Documented on the v2 API, whose host no longer resolves; the current v1 SDKs carry no card flow | Yes               | **No**     | **Disqualified** (no refund API); cards unconfirmed on the live API                                          |
| Notch Pay          | A `cm.card` channel name and a phrase in an FAQ; no guide, no 3DS, no price                     | Yes (weak key)    | Partial    | **Not a card rail on its documentation**                                                                     |
| Maviance Smobilpay | Its supported-methods page lists mobile money only                                              | Yes (HMAC-signed) | **No**     | **Disqualified** (no cards, no refund API); poor pass-through fit                                            |

## Criterion by criterion

`Y` yes, `P` partial, `N` no, `?` the documentation does not say. **Bold** marks a
disqualifying miss.

| RFC-0004 § 7 criterion                         | Flutterwave                                          | CinetPay                                   | Notch Pay                                   | Smobilpay                               |
| ---------------------------------------------- | ---------------------------------------------------- | ------------------------------------------ | ------------------------------------------- | --------------------------------------- |
| 1. Onboards CM merchants, XAF to the merchant  | P — onboards CM; an XAF payout pause, now a 404 page | P — balance held 8 days, manual withdrawal | P — balance, withdrawal; bank "coming soon" | P — Maviance trust account, payout plan |
| 2. Visa/MC, 3DS, hosted page, for CM           | P — see above                                        | **P** — retired API only                   | **?**                                       | **N**                                   |
| 3. Authenticated status query                  | Y — Bearer secret (v3), OAuth2 (v4)                  | Y — JWT (v1)                               | Y — the key the docs call public            | Y — HMAC-SHA1 request signing           |
| 4. Refund API with a status read               | Y (v4 single read ?)                                 | **N**                                      | P — in the guide, not in the OpenAPI        | **N**                                   |
| 5. Sandbox or stub-grade spec                  | Y — v4 sandbox with forced scenarios                 | P — sandbox, no spec; stubs from SDK code  | Y — OpenAPI 2.1.0, with gaps                | Y — staging host, generated SDKs        |
| 6. Signed webhooks                             | P (v3 echoes a secret) / Y (v4 HMAC-SHA256)          | P — shared token (v1)                      | Y — HMAC-SHA256, no timestamp               | P — undocumented scheme                 |
| 7. Reusable tokens, merchant-initiated charges | Y — needs approval; XAF ?                            | ?                                          | N                                           | N                                       |
| 8. Bank transfer or virtual accounts in CM     | N — NGN and GHS only                                 | N                                          | N                                           | ?                                       |
| Mobile money in CM                             | MTN, Orange                                          | MTN, Orange, Express Union                 | MTN, Orange, Express Union, Yoomee          | MTN, Orange, Express Union              |

### Flutterwave

The only candidate whose documentation covers every technical criterion. Two
business facts block it.

- **Cameroon XAF payouts.** Flutterwave's help page "About the payout pause in
  Cameroon" said XAF balances could not be moved to a bank account or mobile
  money, with no timeline. The page now returns 404, and its text was read only
  in a search-engine snippet. The 404 is evidence that the page moved or that
  the pause ended; it is not evidence of which. Cameroon is on the list of
  registration countries, and Flutterwave announced a BEAC-licensed service in
  Cameroon (with Ecobank) in June 2025.
- **Hosted cards in XAF.** The v3 Standard checkout returns a hosted link, but
  its documentation lists no XAF card support and does not describe 3DS. v4 is
  a public beta with **no hosted page**: the integrator encrypts card data
  itself. That breaks rule 2 (vpay never sees a card number) and puts merchants
  outside SAQ-A. **So only v3 can qualify.**
- v3's webhook "signature" is the merchant's secret echoed in a header. Under
  "callbacks are hints" that costs vpay nothing (see below). v4's HMAC is better,
  but v4 is disqualified by the hosted-page point.

Sources: [requirements](https://flutterwave.com/eu/support/general/what-are-the-requirements-for-using-flutterwave),
[Cameroon onboarding](https://flutterwave.com/gb/support/my-account/onboarding-requirements-for-using-flutterwave-in-cameroon),
[Cameroon pricing](https://flutterwave.com/cm/pricing),
[licence announcement](https://flutterwave.com/ke/blog/flutterwave-powers-business-growth-in-cameroon-with-fully-licensed-payment-services),
[v3 Standard](https://developer.flutterwave.com/docs/flutterwave-standard-1),
[v3 verification](https://developer.flutterwave.com/docs/transaction-verification),
[v3 refunds](https://developer.flutterwave.com/docs/refunds),
[v3 tokenization](https://developer.flutterwave.com/docs/tokenization),
[v3 francophone mobile money](https://developer.flutterwave.com/docs/francophone),
[v4 card](https://developer.flutterwave.com/v4.0.0/docs/card),
[v4 webhooks](https://developer.flutterwave.com/v4.0.0/docs/webhooks),
[v4 bank transfer](https://developer.flutterwave.com/v4.0.0/docs/pay-with-bank-transfer).

### CinetPay

- **`docs.cinetpay.com` and `api-checkout.cinetpay.com` do not resolve**
  (NXDOMAIN, 2026-09-23), and `cinetpay.com` refuses automated reads. The v2 API
  — the one whose documentation describes `channels=CREDIT_CARD` sending the
  payer to "the bank's form (3DSecure available)" — was read from Wayback
  snapshots of January–April 2026.
- The live API is a new "v1" on `api.cinetpay.co`, known only from CinetPay's own
  SDKs (github.com/cinetpay; the PHP SDK is 3.0.1 of 2026-08-11). **Those SDKs
  list mobile-money methods only.**
- **No refund API in either version.** The SDKs' "Remboursement" example is a
  payout, which is a new transfer, not a refund.
- One account per country, and funds are held eight days by default before the
  merchant withdraws them.

### Notch Pay

- A mobile-money aggregator (MTN, Orange, Express Union, Yoomee) with a clean
  OpenAPI spec (2.1.0), a usable test mode and HMAC-SHA256 webhooks.
- **Cards appear only as a channel name and a phrase.** There is no card guide,
  no 3DS, no test card and no card price.
- The status read accepts the key the documentation itself calls public. That
  is weaker than rule 3 intends: anyone holding a merchant's publishable key can
  read its payment statuses.
- The merchant agreement limits merchants to "natural persons … residing in
  French-speaking Africa" in one clause and defines legal persons in another.
  No official page names a legal entity, a licence, a partner bank or
  card-scheme membership.

Sources: [collect](https://developer.notchpay.co/accept-payments/collect),
[authentication](https://developer.notchpay.co/api-reference/authentication),
[refunds](https://developer.notchpay.co/accept-payments/refunds),
[webhook verification](https://developer.notchpay.co/get-started/webhooks/verify),
[testing](https://developer.notchpay.co/get-started/testing),
[merchant agreement](https://notchpay.co/policies/merchant-agreement).

### Maviance Smobilpay

- Two products. **S3P** is a partner API for bill payment and cash-in/cash-out,
  whose collections land in the _integrator's_ account. That is the aggregation
  RFC-0001 forbids, unless every merchant is its own S3P partner. **Enkap**
  ("Smobilpay for e-commerce") is a hosted merchant checkout that pays out from
  a Maviance-administered trust account.
- The supported-methods page lists mobile money only. No 3DS, no refund API.
- S3P's request signing (HMAC-SHA1, OAuth-1.0a style, 300-second freshness) is
  the most rigorous status-query authentication of the four.
- **Caveat on the evidence:** `enkap.cm`, `support.enkap.cm` and `maviance.cm`
  resolved to `0.0.0.0` on the machine this was researched from, which looks
  like a local DNS block. Enkap findings are from search-engine snippets of the
  official pages. S3P findings are from `apidocs.smobilpay.com` and Maviance's
  own SDKs.

Sources: [S3P authentication](https://apidocs.smobilpay.com/s3papi/Authentication.1578338286.html),
[S3P concepts](https://apidocs.smobilpay.com/s3papi/S3P-API-Concepts.1578338258.html),
[smobilpay-php](https://github.com/maviance/smobilpay-php),
[smobilpay-python](https://github.com/maviance/smobilpay-python).

## Beyond the four

A screen of other names, one line each. Entries marked † rest on third-party
pages only.

| Candidate                  | CM merchants | Visa/MC for them                                     | Source                                                                             |
| -------------------------- | ------------ | ---------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Stripe                     | No           | No — Africa through Paystack                         | [stripe.com/global](https://stripe.com/global)                                     |
| Paystack                   | No           | No                                                   | [country launch](https://paystack.com/blog/company-news/civ-rwanda-egypt-beta)     |
| DPO Pay                    | No           | No                                                   | [about](https://dpogroup.com/about-us/)                                            |
| PayDunya                   | No           | No — cards in SN, CI, BJ only                        | [terms](https://paydunya.com/terms-of-service)                                     |
| KKiaPay                    | No           | No                                                   | [kkiapay.me](https://kkiapay.me/?lang=en)                                          |
| pawaPay                    | Yes          | No — mobile money only                               | [product](https://www.pawapay.io/product-payments)                                 |
| Monetbil                   | Yes          | ? — cards through unnamed partners †                 | [monetbil.com](https://www.monetbil.com/)                                          |
| Campay                     | Yes          | ? — a blog claims payment-link cards                 | [campay.net](https://www.campay.net/en/)                                           |
| Hub2                       | Likely †     | ? — cards listed generically                         | [docs](https://docs.hub2.io/integration/en/getting_started/introduction)           |
| Ecobank Cameroun           | Yes (bank)   | Plausible — a Mastercard gateway with an API †       | third-party description only                                                       |
| Afriland, UBA, SG Cameroun | Yes (banks)  | ? — no e-commerce acquiring offer found              | —                                                                                  |
| GIMAC                      | Not a PSP    | The CEMAC interbank switch; merchants join via banks | [BEAC](https://www.beac.int/systemes-paiement/systeme-de-monetique-interbancaire/) |

**The regulatory frame.** Payment services in CEMAC are reserved to credit
institutions, microfinance institutions and licensed payment institutions,
under Règlement n° 04/18/CEMAC/UMAC/COBAC of 21 December 2018
([BEAC PDF](https://www.beac.int/wp-content/uploads/2019/07/REGLEMENT-N-04-18-CEMAC-UMAC-COBAC-du-21-d%C3%A9cembre-2018.pdf);
its text could not be extracted, and what it says about foreign PSPs is
**unverified**). Third-party summaries also report that GIMAC clears
CEMAC-issued cards locally in FCFA. **Both point the same way: card acquiring
for Cameroon merchants is most likely to come from a local bank acquirer or a
licensed local partner, not from a pan-African aggregator.**

## Two corrections to the criteria themselves

Reading real providers against RFC-0004 § 7 showed two criteria were worded
more strictly than vpay's own invariants require.

1. **Criterion 1 — "settles XAF to the merchant's own bank".** Every
   aggregator holds the merchant's funds in a balance at the PSP, under the
   merchant's own contract, until the merchant withdraws it. RFC-0001's line is
   that **vpay** never holds merchant money, and a balance the merchant holds at
   the PSP does not cross it. The criterion should read: _"the merchant's own
   account with the PSP, withdrawable to the merchant's bank or mobile money in
   XAF"_. What stays disqualifying is a PSP that credits **vpay** (Smobilpay
   S3P's partner model).
2. **Criterion 6 — signed webhooks.** Callbacks are hints: a callback only
   triggers an authenticated `query_status`. So a forged webhook costs vpay one
   status read and moves no money. A signature is a quality signal, not a
   disqualifier.

Criterion 3 is sharpened instead. A status read authenticated by a
**publishable** key (Notch Pay) passes the letter of "authenticated" and fails
its purpose. The criterion should say _secret-authenticated_.

## What to ask, in writing, before any adapter is written

- **Flutterwave:**
  - Is the Cameroon XAF payout pause lifted, and since when?
  - Does v3 Standard accept Visa/Mastercard in XAF for a Cameroon-registered
    merchant, with 3DS on the hosted page?
  - Is `tokenized-charges` available in XAF, and on what approval?
  - Is a v4 hosted checkout coming, and when?
- **CinetPay:**
  - Is card acceptance available on the v1 API, and where is its documentation?
  - Is there any refund API, planned or existing?
- **Ecobank Cameroun, and the other Cameroon banks:**
  - Do you offer e-commerce card acquiring to Cameroon merchants: hosted page,
    3DS, a server-to-server status query, refunds?
  - Do you accept GIMAC cards as well as Visa and Mastercard?
- **Hub2, Monetbil, Campay:**
  - Which acquirer processes your card payments?
  - Does it settle XAF to the merchant?
  - Is there a refund API?

## What this changes in RFC-0004

- § 7's "None has been checked" line is replaced by a pointer to this page,
  and the two criteria are reworded as above.
- Open question 6 ("which PSP first") becomes: **no PSP can be chosen until the
  written answers above are in.** The first card adapter (delivery step F) is
  blocked on them, not on engineering.
- Nothing about the port design moves. A hosted-page card rail is still a
  `Redirect` rail, and every candidate that offers cards at all offers them that
  way (Flutterwave v3, CinetPay v2).
