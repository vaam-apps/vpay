# How the "vpay is a scaffold" banner was narrowed, step by step

_Archived from [docs/status.md](../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](README.md) is the index of them._

Nine dated addenda across eight steps, each retiring one specific claim. The banner
itself, and the load-bearing sentence that survived all nine, are still on
[docs/status.md](../status.md).

> _2026-09-03 (Step 2): `/v1/payment_intents` now exists and answers real
> requests with real rows._
>
> _2026-09-03 (Step 3): the sentence "no HTTP call to any rail has ever been
> made by this code" is **retired** — both adapters now make real HTTP calls,
> and `confirm` moves an intent to `processing` or `requires_action` on the
> strength of one. **Its replacement is narrower and just as load-bearing:
> no HTTP call to a real rail has ever been made.** Every call has gone to a
> `wiremock/wiremock` host. No payer has been prompted, no money has moved,
> nothing polls a submitted charge, and no intent has ever reached
> `succeeded`. Do not deploy it._
>
> _2026-09-03 (Step 4): the worker runs, and an intent reaches `succeeded`
> without anyone touching it. **The load-bearing sentence is unchanged: no
> HTTP call to a real rail has ever been made.** Every payment this code has
> settled was settled against a `wiremock/wiremock` host that answered the way
> these documents say a rail answers. No payer has been prompted, no money has
> moved, and nothing has ever run outside a developer machine. Webhooks are
> delivered as of Step 5 — to a WireMock receiver on a compose network, never
> to a merchant. Do not deploy it._
>
> _2026-09-03 (Step 5c): a payer's own browser can now confirm a push payment
> directly — `/v1/browser` plus `@vaam-apps/vpay-stripe-js`, with no merchant
> credential anywhere near the browser. **The load-bearing sentence is still
> unchanged: no HTTP call to a real rail has ever been made.** The MTN push
> `examples/checkout-browser` confirms goes to `wiremock-mtn`, the same as
> every other payment in this repository's history. The redirect (Orange)
> half of a browser checkout has no return-trip route and must not be
> shipped on this package yet — see [flows/browser-checkout.md](../flows/browser-checkout.md).
> Do not deploy it._
>
> _2026-09-03 (Step 7, Phase A): a cleanup pass, and it built nothing.
> `vpay-db` grew a repository seam and `ProviderError` grew a real
> `#[source]`; no feature moved from ⛔ or 🟡 to ✅ because of it, and **the
> load-bearing sentence is unchanged: no HTTP call to a real rail has ever
> been made.** The proof that nothing changed is the guard suites, which were
> not rewritten to fit: 26 conformance cases, the integration binaries and
> the SDK suites all still pass, and the only assertion lines that moved are
> the three the escalation named. Do not deploy it._
>
> _2026-09-03 (Step 7, the four parallel lanes): the same sentence, once
> more, for the rest of the step. Step 7 is error surfaces, a repository
> seam, docs moved out of module headers into `docs/reference/`, doctests
> that make the examples real, and the tooling that runs them. **It moved no
> capability.** Not one row below changed from ⛔ or 🟡 to ✅ because of it,
> nothing new is reachable over HTTP, and no rail was called. What did change
> is what the repository checks about itself: the workspace's doctests run in
> CI for the first time, and `just verify` prints a doc-volume report it
> cannot fail on. If a row below looks better than it did before this step,
> that is a defect — say so._
>
> _2026-09-04 (Step 8, the production gate): this step **did** move capability,
> and the banner still stands. What moved: a runtime egress guard on every
> webhook delivery (the last ⛔ on a shipping path); `POST
/provider/{code}/callback`, so a rail that tries to tell us about a charge is
> heard; a real `SIGKILL` of the shipping worker and the shipping server, so
> two of the three crash-safety kill points are caused rather than written; a
> demo that walks six payments across both rails; a fix for a `500` on confirm
> that this step's own demo found. **The load-bearing sentence is unchanged:
> no HTTP call to a real rail has ever been made.** The guard has never refused
> a real merchant's endpoint, the callback route has never been called by MTN
> or Orange — its test bodies are transcribed from this repository's own flow
> documents, so a document that is wrong about a rail would pass — and every
> payment in the demo settled against a `wiremock/wiremock` host. No payer has
> been prompted and no money has moved. Do not deploy it._
>
> _2026-09-04 (Step 9, hosted checkout): this step moved the most visible
> capability so far, and the banner is unchanged. What moved: vpay serves its
> own payment page, hosted and embedded — a `checkout.session` object, three
> new browser reads, `frame-ancestors` from a per-merchant origin list, the
> return trip a redirect rail needs, `initEmbeddedCheckout` in
> `@vaam-apps/vpay-stripe-js`, `checkout.sessions` in both merchant SDKs, a fourth image
> and a Helm workload, and a demo shop a human can buy from. **A real browser
> has walked the whole thing**: add to cart, checkout, vpay's page, an MTN
> prompt or Orange's own page, back to the shop, and `paid` written only by the
> shop's webhook handler after it verified vpay's signature. **The
> load-bearing sentence is unchanged: no HTTP call to a real rail has ever
> been made.** Every rail in that walk is a `wiremock/wiremock` host; no payer
> has been prompted on a real handset; no money has moved; and no merchant
> endpoint outside this repository has ever been POSTed to. Two more things
> this step is not: **no browser has been observed enforcing vpay's
> `frame-ancestors`** — Cypress strips the header, so it is proven sent and the
> refusal a browser was seen performing is the page's own origin check — and
> **no pod has ever run** the page. Do not deploy it._
>
> _2026-09-07 (exp31, docs only): **`README.md` was rewritten against this
> page** and no capability moved. Its banner had still read "vpay cannot take a
> payment yet … no HTTP call to any payment rail has ever been made by this
> code" — a sentence this section retired on 2026-09-03 (Step 3) — and it still
> said "nothing polls the charge and no intent has ever reached `succeeded`",
> which Step 4 closed. Its banner now reads **"vpay has never taken a real
> payment"**, which is this section's own load-bearing sentence in the README's
> voice. Eleven other stale claims were corrected against the rows below and
> against the tree; every one is listed with its evidence in
> [plans/exp31-readme-notes/opus.md](../plans/exp31-readme-notes/opus.md), together
> with six stale claims in **other** files (this page's MVP item 7,
> `runbooks/demo.md` §6, `flows/README.md`, AGENTS.md's gate count and its
> Headless UI line, and the `justfile`'s `ci` comment) that pass deliberately
> did not touch. **The load-bearing sentence is unchanged: no HTTP call to a
> real rail has ever been made.** Do not deploy it._
