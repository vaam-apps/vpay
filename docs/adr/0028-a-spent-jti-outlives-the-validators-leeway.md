# ADR-0028: A spent client-assertion `jti` outlives the validator's leeway

- **Status:** Accepted
- **Date:** 2026-10-08
- **Deciders:** the vpay maintainer, who chose this fix on 2026-10-08 ("fix it
  now") after a review of
  [ADR-0026](0026-the-database-clock-schedules-jobs.md) found the defect below
  in the item that ADR's D8 had named and not concluded. The shape (a relative
  duration, applied by the database) is ADR-0026 D1's; the number and the
  tests are how the implementing agent applied it.
- **Concludes:** [ADR-0026](0026-the-database-clock-schedules-jobs.md) D8's
  second bullet, `oauth_client_assertion_jtis.expires_at` — "named, not
  concluded". ADR-0026 is immutable and is not edited here; its Status block
  points at this ADR.
- **Implementation:** the change that carries this ADR. Its verification page,
  [2026-10-08-jti-purge-horizon.md](../status/verification/2026-10-08-jti-purge-horizon.md),
  says what was run and what was not. This ADR records the decision.
- **Number checked at branch time, not assumed:** `ls docs/adr` on this
  branch returns `0001`…`0027`.

## Context

A merchant authenticates to `/v1` with `client_credentials` and a
`private_key_jwt` assertion (RFC 7523;
[ADR-0010](0010-merchant-auth-private-key-jwt.md)). The assertion is a bearer
credential for as long as it verifies, and its single use is what makes that
safe: `vpay_db::client_assertion_store` records its `jti` with
`INSERT … ON CONFLICT (jti) DO NOTHING`, and a second presentation finds the
row and is refused `invalid_client`.

Two clocks decide how long that protection lasts.

**The validator's.** `authkestra-op` 0.7.1
(`src/client_assertion.rs`, `verify_client_assertion`) builds a
`jsonwebtoken::Validation` and leaves `leeway` at jsonwebtoken's default,
**60 seconds**, with a comment saying why. jsonwebtoken 11 accepts a token
while `now_secs <= exp + leeway`, with `now` truncated to whole seconds.
Real time after `exp` is therefore up to **61 s**. The same function refuses an
`exp` further than 300 s in the future. vpay cannot configure any of this; the
dependency is pinned `=0.7.1` and the leeway is a constant inside it.

**The sweep's.** The store keeps the client's raw `exp` as `expires_at`. The
worker's hourly `sweep_expired` job
(`vpay_worker::handlers::sweep_expired`, `SWEEP_INTERVAL` = 3600 s) ran
`DELETE FROM oauth_client_assertion_jtis WHERE expires_at < now()` — Postgres'
`now()`.

Those two disagree about when an assertion is over. The row was deletable from
`exp`; the assertion was acceptable until `exp + 61 s`. A sweep that landed
between them deleted the `jti` of an assertion the API would still accept,
and the next presentation of it found no row, inserted a fresh one, and was
**accepted: one replay of a captured, already-used assertion, minting a
merchant access token.**

The window, as arithmetic. Let `db_offset` be how far the database's clock is
ahead of the clock of the API replica that serves the replay, and `api_offset`
the same for the replica's own error against true time (only the difference
matters). The row is deletable once the database's clock passes `exp`; the
replica stops accepting once its own clock passes `exp + 61 s`:

```text
window = max(0, 61 s + db_clock_offset − api_clock_offset)    after exp
```

At zero skew that is 61 s. An hourly sweep lands at a uniformly distributed
point in the hour, so the chance that a given assertion's window contains a
sweep is 61 / 3600 ≈ **1.7 %**. A database clock a minute ahead makes it 122 s
and about 3.4 %. The defect is silent: nothing fails and nothing is logged,
because the replayed request is a perfectly ordinary successful token request.

What an attacker needs is an assertion that has **already been spent** —
captured from a request in flight, or from anywhere a merchant's integration
logs it — and a replay inside the window. An assertion that has not been
spent is the merchant's to spend regardless; that is not this defect. The
assertion's `aud` is vpay's token endpoint, its `exp` is at most 300 s out,
and it yields one token with the registered client's scopes. It is a narrow
window and a real one, on the credential that gates every money-moving route,
and a payment system should not carry it for a probability.

[ADR-0026](0026-the-database-clock-schedules-jobs.md) D8 had found the column
and written "whether a database clock running ahead of the API host can drop
a spent `jti` while the API still accepts its assertion depends on the
validator's leeway, which this change did not analyse. Named, not concluded."
This is the analysis. The finding is stronger than D8 guessed: skew is not
needed. The leeway alone opens the window at zero skew. Skew widens it.

The doc comment on `ClientAssertions::delete_expired_client_assertion_jtis`
said the opposite until this change: that "an assertion past its `exp` is
refused by `verify_client_assertion` before any store is consulted", and that
the database's `now()` made skew harmless. Both are wrong for the 61 s after
`exp`. It also said the worker's job loop "does not exist yet", which had
been wrong since 2026-09-03 (Step 4).

## Decision

### D1 — The sweep keeps a spent `jti` for a horizon past its `exp`

`ClientAssertions::delete_expired_client_assertion_jtis` takes a
`retain_after_exp: std::time::Duration`, and the statement is

```text
DELETE FROM oauth_client_assertion_jtis
WHERE expires_at < now() - ($1::BIGINT * INTERVAL '1 microsecond')
```

This is ADR-0026 D1's shape, with `vpay_db::jobs::as_micros` as the bind
(saturating, so a stray nanosecond is not a refusal). It takes a relative
duration and no instant, so the caller's clock cannot reach the comparison. The
statement is a plain literal with a bind, so `sql_audit`'s
`EXPECTED_ASSERT_SITES` is unchanged.

### D2 — The horizon is five minutes, named and derived

`vpay_worker::CLIENT_ASSERTION_JTI_RETENTION` is `Duration::from_secs(5 * 60)`,
next to `SWEEP_INTERVAL` and passed by `sweep_expired`. It is the sum of:

| Part  | Why                                                                                      |
| ----- | ---------------------------------------------------------------------------------------- |
| 60 s  | `jsonwebtoken`'s leeway, which `authkestra-op` pins and vpay cannot set                  |
| 1 s   | the whole-second truncation of the validator's `now`                                     |
| 239 s | a budget for the database's clock running ahead of any API replica's: about four minutes |

The horizon is a round five minutes and the skew budget is what is left of it,
not a measured skew. Hosts that run NTP are within milliseconds. A deployment
whose database clock can run more than about four minutes ahead of an API
replica's has a different problem, and this horizon does not claim to cover
it.

The cost is five more minutes of rows in a table the sweep already bounds at
an hour's worth of assertions. It is invisible.

### D3 — `expires_at` stays the client's raw `exp`

`record_jti` is not changed. `expires_at` is a **fact** under ADR-0026 D6: the
instant the client's assertion says it expires, recorded as received. The
horizon is policy, applied once, at deletion, by the database.

### D4 — The invariant is pinned from both sides

The horizon is sufficient only while the validator's leeway does not exceed
it. Two tests in `backends/tests/integration/tests/` hold that, and both use
the production constant rather than a copy of its value:

- `merchant_token_flow.rs`,
  `an_assertion_older_than_the_sweep_horizon_is_refused_by_the_validator`: an
  assertion whose `exp` is one second past the horizon is refused by the
  running validator. If an `authkestra-op` or `jsonwebtoken` bump widens the
  leeway beyond the horizon, this fails in CI, and the horizon must grow
  before the bump merges.
- `merchant_token_flow.rs`,
  `a_spent_assertion_past_its_exp_but_inside_the_leeway_cannot_be_replayed_after_a_sweep`:
  an assertion whose `exp` is 30 s past is accepted once, the real worker
  sweep runs, and the replay is refused. With the horizon at zero the replay
  is accepted, which is the defect.
- `client_store.rs` pins the cut itself on the database's clock: a row 30 s
  and a row 61 s past `exp` survive a sweep at the production horizon, and a
  row one second beyond it goes.

## Consequences

- **The window is closed for any pair of clocks within about four minutes**,
  and for the validator at its present leeway. It is not closed by a
  database clock more than `horizon − 61 s` ahead, which nothing here checks.
- **A spent `jti` row now lives five minutes longer than before.** No other
  behaviour changes: no wire, schema or migration change, and no change to
  what a merchant sees.
- **`delete_expired_client_assertion_jtis` takes an argument.** It is a trait
  method with one caller, the worker's `sweep_expired`, plus its tests.
- **Migration `0011`'s header comment is stale and stays so.** It says no
  cleanup job exists and that the table grows without bound. Migrations are
  checksummed (`verify-migrations`) and are not edited; the current account is
  this ADR and the doc comment on the method.
- **The `exp` cap of 300 s and the leeway are upstream's.** An upstream change
  to either is a reason to re-read this ADR, and D4's first test is what
  makes the leeway half of that automatic.
- Status pages and
  [merchant-auth.md](../flows/merchant-auth.md) § Status carry what is built.

## Alternatives considered

- **Store `exp + leeway` in `record_jti` and keep `expires_at < now()`.**
  Rejected. It hard-codes into the store a leeway vpay does not own: the
  value would be a copy of a constant inside a pinned dependency, and a bump
  would silently desynchronise them with no test to say so. It also mixes
  policy into a fact. `expires_at` would no longer be the client's `exp`, and
  D6's distinction between what the application observed and what it decided
  would stop holding for this column. The horizon at deletion keeps the fact
  intact and puts the number where a reviewer reading the sweep will see it.
- **Cap the assertion's lifetime, or require `iat`, in vpay.** Rejected as a
  fix, since it does not close the gap. `authkestra-op` already refuses an
  `exp` more than 300 s out, and `iat` does not move the point at which a
  validator that accepts the token stops accepting it. The window is between
  the row's deletion and the validator's last accepted instant. Neither check
  touches either end.
- **Re-check `exp` against the API's clock at replay time, inside vpay's
  store.** Not chosen. The validator has already accepted the assertion by
  the time `record_jti` runs, so the store would be second-guessing a
  decision it does not own, with a leeway it would have to copy (the first
  alternative again).
- **Sweep on the application's clock, or delete less often.** Rejected. A
  less frequent sweep shrinks the chance of landing in the window and does
  not remove the window, and reading the application's clock for a database
  predicate is the defect ADR-0026 removes.
- **Leave it, recorded as a known limitation.** The maintainer decided against
  it on 2026-10-08.

## Related findings

Recorded here because ADR-0026 is immutable and its maintainer asked that these
not wait for a superseding ADR.

1. **ADR-0026 D8's jti item is concluded by this ADR.** D8 said "named, not
   concluded". It is concluded: the defect was real, it did not need skew, and
   D1–D4 above close it. D8's other bullet, `oauth_signing_keys.expires_at`, is
   untouched and still open.
2. **ADR-0026 D6 omits one call site that follows its rule.** A review of
   ADR-0026 on 2026-09-30 found that D6's list of comparisons against a
   fact leaves out the confirm gate in
   `backends/crates/vpay-api/src/v1/return_trip.rs` (`SessionGate::admit_confirm`
   and `verdict`). It compares the application's clock,
   `OffsetDateTime::now_utc()`, with `checkout_sessions.expires_at`, and it is
   the gate that decides whether a charge may start. It is on the right side of
   D6's rule: both operands are application-clock facts, and its comment says it
   reads the same clock as `browser::checkout_sessions::authenticate` for the
   reason D6 gives. No behaviour changes. The omission is in the list, not in
   the code, and the list is what a later reader will use to decide whether a
   new site belongs.
