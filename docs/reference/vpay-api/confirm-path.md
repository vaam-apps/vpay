# `vpay-api` — the confirm path

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## The confirm path

`POST /v1/payment_intents/{id}/confirm` and
`POST /v1/browser/payment_intents/{id}/confirm` share one implementation,
`confirm_once`. Its six steps are the order
[crash-safety.md](../../flows/crash-safety.md) requires, and their order is the
whole safety property:

1. load the intent for this merchant (404), refuse a status that forbids a
   confirm (409), refuse an intent that already has a charge (409, and **before**
   any insert — "one charge per intent, forever");

   **1b**, in the same breath and from one read: ask the intent's **checkout
   session**, if it has one, whether this confirm may happen at all — a
   session that is not `open` refuses it with a `409`
   (`checkout_session_expired` / `checkout_session_complete`) — and, when one
   does drive it, where the payer must come back to. It is a step of its own
   rather than a seventh, because it is the same question step 1 asks (may
   this be confirmed?) about a different object, and because `confirm_once`'s
   own doc comment counts six;

2. resolve the rail from `payment_method_data[type]` and branch on its
   `vpay_provider::Capabilities::flow` only, never on its code
   ([ADR-0002](../../adr/0002-provider-port.md));
3. mint the `provider_reference_id` and commit the charge row in `submitting`,
   so the reference is durable before anything is sent;
4. record the attempt in `provider_requests` with no status;
5. call the adapter with the `return_url` step 3 committed on the charge row
   — `submit` is `async`, so the `.await` is what actually sends the request.
   (It read the merchant's URL out of the request here until Step 9's lane 1b
   moved the resolution to 1b and the value into `charges.return_url`, so that
   what the rail is told is what a crash would leave behind.);
6. record what came back and answer.

Each step is a named function in the source (`load_confirmable_intent`,
`return_trip::admit_confirm`, `resolve_rail` with `payer_instrument`,
`open_attempt` covering steps 3 and 4, `submit_to_rail`, `finish_confirm`);
`confirm_once` is the sequence and nothing else, so "what order do these
happen in" is answerable by reading a dozen lines.

Step 6 has three shapes, and which one runs is decided by the _error's_ own
classification rather than by anything this file knows about rails:

- **the rail accepted it** — one transaction moves the charge to `submitted`
  with the rail's key material and the intent to `processing`/`requires_action`,
  it commits, and only then is a response built (`persist_submitted`, and
  crash-safety.md's "the commit is the gate on the redirect");
- **the rail declined it** (`ProviderError::Rejected`) — one transaction fails
  the charge with its `failure_code` and stamps `last_payment_error` on the
  intent, which stays `requires_payment_method` because the lifecycle has no
  `failed` status; the merchant gets the `409` `charge_declined`;
- **anything else** — we do not know what the rail did, so _nothing_ moves. The
  `submitting` charge row and the status-less `provider_requests` row stay
  behind on purpose: they are exactly the state a crash between steps 4 and 6
  would leave, and are what the recovery pass reads.

`Malformed` is the one arm where "no answer" is not literally true — bytes came
back, they just did not parse — and it is grouped with the unknown cases
deliberately. What the recovery table decides is whether to go and _ask the
rail_, and an unparseable answer is exactly as unknown as a lost one: "every
ambiguity resolves toward 'find out', never 'give up'".

### What the checkout session says, and where the payer comes back to (`v1/return_trip.rs`)

**A confirm on an intent whose checkout session is over is refused before any
charge is opened.** Added 2026-09-05. A session's `status` is a promise vpay
has already made: the hourly sweep emits `checkout.session.expired`, and
`POST /v1/checkout/sessions/{id}/expire` is the merchant's own statement that
the checkout is done. Neither retracts the payer's credential — the intent's
`client_secret` is minted at create and lives as long as the intent — so a
payer holding a stale checkout link could pay anyway, and the settlement's own
`WHERE status = 'open'` guard would then correctly decline to touch the
session, leaving `expired`/`unpaid` under a `succeeded` intent and a merchant
holding a webhook that said the opposite.

- The refusal is `ApiError::CheckoutSessionNotOpen`, `Category::Conflict`
  (409, `invalid_request_error`, `Retry::Never`), with **two codes** chosen by
  the state: `checkout_session_expired` and `checkout_session_complete`. Not
  the category default `invalid_state`, for the reason
  `idempotency_key_in_flight` is not `idempotency_key_in_use` — a merchant
  must be able to tell "your payer walked away" from "this intent is already
  processing". Not one code plus a `param` either: `param` on this API names a
  _request parameter_ (`ApiError::param` renders it only for `InvalidParam`),
  and the request that trips this carries no reference to a session at all.
- It fires on **both** surfaces, because it lives in `confirm_once`, which
  both share. That is deliberate rather than incidental: `/v1`'s confirm is
  not authenticated by the payer's `client_secret` at all, so a merchant
  server that kept confirming after its own systems recorded the checkout as
  abandoned would produce exactly the contradiction the browser refusal
  prevents.
- An `open` session past `expires_at` that no sweep has reached is expired
  **on the read**, and the read writes nothing. Same rule and same reasoning
  as `browser::checkout_sessions::authenticate`'s sixth refusal: a worker that
  is down must not be the difference between a payer being able to pay and
  not, and a confirm is the wrong place to repair a row — flipping it here
  would emit no `checkout.session.expired` and would skip the `NOT EXISTS`
  live-charge guard the sweep's transaction carries.
- An intent with **no** session is unaffected, and an intent whose session was
  expired and replaced by a new open one is payable through the new one:
  `CheckoutSessions::find_latest_by_intent` reads the newest row, and
  `checkout_sessions_one_open_per_intent` is what makes "an open session is
  the newest" true rather than hoped for
  ([vpay-db.md](../vpay-db/payment-intents-and-checkout-sessions.md#find_latest_by_intent--the-same-question-with-the-status-filter-off)).

The refusal and the return URL are **one read**, not two. Asking separately
would race the hourly sweep: a gate that read `open`, followed by a
return-URL lookup running a millisecond after `expire_due` committed, would
admit the confirm and then submit it to the rail with no return URL at all.

`vpay_provider::ChargeRef::return_url` is filled here and nowhere else (Step
9's D2; [rails.md](../rails.md) has the rail-side half and what it replaced). The
question has two answers and they belong to different owners, which is why it
is a trait and not two lines inside the confirm:

- a charge driven by a **checkout session** returns to vpay's own return page
  for that session, because vpay has to poll the intent before it can forward
  the payer to the merchant's `success_url` or `cancel_url`;
- every other charge returns to the merchant's own `charges.return_url` — the
  URL they sent on `confirm`, already validated by `checked_return_url` and
  already echoed back to them as `next_action.redirect_to_url.return_url`.
  This is what closes [browser-checkout.md](../../flows/browser-checkout.md)'s D4
  for integrations that never create a session.

It runs **before** `open_attempt`, which is what makes both refusals above cost
no charge row, no `provider_requests` row and no job — and after
`load_confirmable_intent`, never before it, because this answer is not the
uniform 404 and asking it first would let a caller learn that some other
tenant's intent has a checkout session on it. (Until Step 9's lane 1b it ran
_after_ `open_attempt` and handed its answer straight to the adapter; the
value now goes into `charges.return_url` before the charge is committed, so
what the rail is told is the value that would survive a crash rather than a
second read that could differ from what was made durable.)

`CheckoutSessionGate`'s shipping impl is `SessionGate`, which holds the
repositories _and_ `ResourceConfig::checkout_public_base_url()` — both, because
the URL needs a row (`CheckoutSessions::find_latest_by_intent`) and a configured
origin, and `CheckoutSessionRow::return_page_url` is the one place the two are
joined. It was a blanket impl over `dyn Repositories` answering `None` for every
intent while Step 9's lane 2 was ahead of the `checkout_sessions` table; lane 1b
replaced it, and `a_session_driven_confirm_sends_vpays_return_page_to_the_rail`
(`backends/tests/integration/tests/confirm_rails.rs`) is what would fail if it
ever went back — measured: make the branch answer `Ok(None)` and the rail is
told `https://shop.example/order/1234/return` instead of the session's page.

A session on a deployment with **no** `checkout.public_base_url` is refused
(`ApiError::CheckoutNotConfigured`, message
`CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP`) rather than fallen back from. It is
unreachable by any merchant request — `POST /v1/checkout/sessions` refuses
before a session can exist — and reachable only by an operator deleting the key
while sessions are open. Falling back to the merchant's URL there is precisely
the silent failure this seam exists to prevent: the payer is forwarded one step
too early and nothing reports it. The refusal fires for a push rail too, which
would have ignored the URL, because the deployment's checkout page is gone
either way and an outage that depended on which rail a payer picked would be
worse to debug than one that does not. It is reached only _after_ the session
has admitted the confirm, so an expired session on a deployment that has lost
its checkout page still answers `checkout_session_expired` — the more specific
and more actionable of the two.

### Why `confirm_once` takes seven loose arguments and not a struct

Each one is a distinct dependency of the six steps — the repositories, the
deployment's rails, the linked adapters, the tenant, the object, the payer's
instrument, and what the response renders — and the two callers pass different
values for every one of them. A parameter struct would put a constructor between
the two surfaces and this function, which is exactly the place a
browser-specific default could hide unnoticed. It sits at clippy's threshold,
deliberately: an eighth would be the signal that the confirm has grown a second
responsibility.

### Idempotency

Every `POST` needs an `Idempotency-Key` (Step 2's D7). The key is claimed
_atomically_ — one `INSERT … ON CONFLICT`, in `vpay_db`'s `Idempotency::claim` —
so two concurrent requests carrying one key cannot both proceed, and the loser is
told "in progress" rather than being allowed to create a second intent. A claim
is always ended, on every path: stored (the response is replayable) or released
(the retry must re-execute). "Every path" is meant literally, including the ones
that fail _after_ the work was done: a body that cannot be read back, a body
that is not JSON, and a failed write to `idempotency_keys` each release before
returning, because the alternative is the key staying `in_flight` until it
expires and every retry under it being answered "still in progress".

The claim is carried as the `claim_id` the claim minted, never as the key alone:
an expired claim is reclaimable, so addressing the row by key would let a request
that stalled past its window overwrite or delete the claim that replaced it.

The refusal of unsupported Stripe parameters runs _before_ the claim, where the
body is decoded — so a refused confirm stores nothing and leaves the key unspent,
exactly as a body that fails to decode already does. Running it first is safe
because the check is config-independent (it reads the body alone), so a genuine
replay, whose body is byte for byte the one that was accepted, can never be
shadowed by it. What the ordering is visible in is the other case: a confirm
reusing a completed key with a newly added refused field answers that field's
`400` rather than `idempotency_key_in_use`.

---
