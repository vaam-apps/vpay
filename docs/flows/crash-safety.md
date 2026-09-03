# Crash safety

## The invariant

> **Never let a payer act on a transaction you cannot later name.**

Everything below is an application of that one sentence. The two flow shapes
enforce it at different moments, because the payer becomes able to act at
different moments.

## Push rails (MTN MoMo)

MTN acknowledges `requesttopay` with **202 and an empty body**. There is no
transaction id in the response — *the id is the `X-Reference-Id` you sent*.

So the payer's handset starts buzzing before you learn whether your request
succeeded. Generate the reference in memory, call the rail, crash before writing
it down, and you have created a payment you can never observe.

```
BEGIN;
  INSERT INTO charges (…, provider_reference_id, state='submitting');
COMMIT;                        -- the reference is now durable
  INSERT INTO provider_requests (…, status_code = NULL);   -- "about to send"
  POST /collection/v1_0/requesttopay   (X-Reference-Id = provider_reference_id)
  UPDATE provider_requests SET status_code = …;            -- "heard back"
UPDATE charges SET state='submitted';
```

**Write first, network second. Always.**

### Retry rule

If submit times out or errors, **do not generate a new reference.** Retry with
the same one. The adapter contract requires a duplicate submission to be
reported as `Submitted` rather than an error — that is what makes this safe. A
fresh reference on retry is how you double-charge a customer.

### Recovering a `submitting` charge

`submitting` covers two physically different situations. Disambiguate with
`provider_requests`:

| Evidence | What happened | Action |
|---|---|---|
| No `provider_requests` row | Crashed before the POST | **Resubmit**, same reference |
| Row exists, `status_code IS NULL` | POST issued, response lost | **Poll**. On `NotFound`, retry the poll; only after 3 consecutive `NotFound` over ≥60s treat it as never-received and resubmit with the same reference |
| Row has a status code | Normal path | Advance state from the code |

A bare `NotFound` is **never** on its own grounds to fail a charge. Resubmission
is always safe, so every ambiguity resolves toward "find out", never "give up".

## Redirect rails (Orange Money)

The ordering is reversed, and it is safe for a reason worth stating plainly.

```
INSERT charge (state='submitting', provider_reference_id = order_id);  COMMIT
POST /webpayment  →  { pay_token, payment_url }
UPDATE charge SET provider_ref_extra = {pay_token…}, state='submitted';  COMMIT
                                     ↑
                    the payer is redirected ONLY after this commit
```

**If the submit response is lost, no payment can have occurred** — the payer was
never given the URL. That `order_id` is dead: abandon it and let the merchant
create a new PaymentIntent. This is the one place where "the response was lost"
is genuinely benign, and it is benign only because the payer's route in is a URL
you must hand them.

What is *not* safe is emitting `redirect_to_url` before `ref_extra` is
committed. Do that and a crash strands a payer mid-payment on the rail's page
against a charge you cannot query.

**The commit is the gate on the redirect.**

## Why Orange is integrable at all

Orange's `transactionstatus` requires `order_id` + `amount` + `pay_token`, and
`pay_token` exists only in the submit response. Under a naive reading of the push
precondition ("status must be queryable by a reference you generated"), that
disqualifies it.

It does not, because of the asymmetry above. This is exactly why the
preconditions are stated **per flow shape** rather than universally.

## Tests

The crash tests kill the worker at three points — after the charge insert and
before any `provider_requests` row; after that row and before the response;
after the response and before the state update — and assert the recovery table
resolves all three without double-charging.

**Status: the ordering is implemented for `confirm`, on both flow shapes;
the recovery is not.** Updated 2026-09-03 (Step 3).

**What is built.** `POST /v1/payment_intents/{id}/confirm`
(`backends/crates/vpay-api/src/v1/payment_intents.rs`) performs exactly the
ordering this document requires, and in this order:

1. mint the `provider_reference_id`;
2. **commit** the charge row in `submitting` carrying that reference, in its
   own transaction, before any network call
   (`vpay_db::charges::insert_for_intent` takes a connection precisely so the
   commit point is the caller's);
3. insert a `provider_requests` row with `status_code IS NULL` (migration
   `0016`);
4. call the adapter's `submit`;
5. record what came back on that row.

**And, since 2026-09-03, the redirect half:** the merchant's `return_url` is
committed on the charge row at step 2 (`charges.return_url`, migration
`0019`), and the rail's `pay_token` + `redirect_url` are committed at step 5
in **one** transaction — after which, and only after which, `next_action` is
built. It is built by re-reading the committed row, not from the adapter's
return value, so there is no code path that can emit a URL the database does
not already hold. That is "the commit is the gate on the redirect", enforced
by construction rather than by discipline.

**What proves it.**

- `redirect_confirm_commits_the_rails_material_before_it_answers`
  (`backends/tests/integration/tests/confirm_rails.rs`) — the decisive one:
  the `next_action.redirect_to_url.url` the merchant is handed is asserted
  equal to the `redirect_url` already on the committed `charges` row, and
  the `return_url` likewise.
- `an_unreachable_rail_leaves_the_charge_where_recovery_expects_it` — a
  submit that never got an answer leaves a `submitting` charge with its
  reference and a `provider_requests` row with `status_code IS NULL`,
  `responded_at IS NULL` and `error_kind = 'provider_unavailable'`. Retrying
  the same confirm meets that live charge and is refused with the "poll,
  do not create a new PaymentIntent" `409`.
- `a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read` — the
  answered-but-declined case: the attempt is stamped with `status_code = 0`
  (the sentinel migration `0020` documents: *answered, but the port carries
  no HTTP status*) and `error_kind = 'charge_declined'`, so an operator can
  tell it apart from an unanswered attempt without reading a body.
- `provider_requests_record_attempts_and_keep_status_and_responded_at_in_step`
  (`backends/crates/vpay-db/tests/repositories.rs`) pins the
  `response_is_paired` CHECK, so a row can never claim a status without a
  `responded_at`.
- `a_second_confirm_cannot_produce_a_second_charge` proves the reference is
  not regenerated — there is no second charge to regenerate it for.

**Those rows are deliberately left behind.** A confirm whose submit was lost
leaves precisely the state a crash between steps 3 and 5 would leave. That is
the point: it is what a recovery pass has to read.

**What is not built, and is the whole rest of this document:**

- **No recovery pass.** Nothing reads the table above. No code resubmits, no
  code polls, no code advances a state from a status code. The worker's job
  loop does not exist.
- **No retry of any kind.** The "retry with the same reference" rule is
  written down and executed by nothing.
- **No crash tests.** The three kill points above are not exercised by
  anything; nothing kills a process mid-confirm. What is proven is the
  *ordering* and the rows a lost answer leaves — not that a `SIGKILL` at
  each point resolves without a double charge.
- **The recovery table above is read by nothing**, so the two rules it
  states — "no `provider_requests` row → resubmit" and "row with
  `status_code IS NULL` → poll, and require 3 consecutive `NotFound` over
  ≥60 s before treating the request as never received" — are written down
  and executed by no code. Step 3 shipped only the adapter API that
  recovery needs: `query_status` returning a canonical
  `ChargeStatus::NotFound` rather than a failure
  (`not_found_is_never_on_its_own_a_failure`, both rails).

*(The "no redirect-rail ordering" item that stood here on 2026-09-02 is
resolved: `charges.return_url` exists, and the commit gates the redirect.)*

See [../status.md](../status.md).
