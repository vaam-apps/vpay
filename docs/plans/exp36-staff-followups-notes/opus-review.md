# exp36 review — the staff sign-in follow-ups, attacked

**Date:** 2026-09-10 · **Branch:** `claude/exp36-staff-followups` ·
**Rebased onto** `f55e002` (PR #94, the dashboard's port as a demo variable),
which is what made the one gate the implementer could not run runnable.

Read [opus.md](opus.md) first — it is the implementer's own account, and this
document only records what the attack found on top of it.

## The headline

The implementation is sound where it is proven, and **the one thing that was
not proven was wrong**. `just test-e2e` had never been run — the notes say so
plainly rather than hiding it, which is why it was the first thing this review
did. It fails: **dashboard.cy.ts, 6 of 8 failing**, and the first failure is
`leg 4`, the current-password field this change exists for. That is F1, below.

**This document is written as the review proceeds**, one section per finding,
each landing in the commit that fixes it. A finding is not written down here
until the mutation that proves the fix has actually been run.

## Findings

### F1 — a wrong current password signs the staff member out · **correctness / gate-hole**

`POST /dash/v1/staff/password` answers `401` for a wrong or absent current
password since this branch. `frontends/apps/dashboard/src/server/actions.ts`
still read **any** `401` from that endpoint as "the session is over" and
called `clearSessionCookie()` — code that was correct for exactly as long as
the endpoint had no credential to refuse, and that was not revisited when it
grew one.

So a staff member who mistypes their current password is signed out of the
browser instead of being told. And the Cypress leg written for this feature
cannot pass: it types a wrong current password, expects the page to stay put
with an alert, and instead the next render finds no cookie and bounces to
`/login`.

Measured, as delivered, at `demo_dashboard_port=13200`:

```
28 - contains  button, Set password
29 - click            (fetch) POST 200 /login/password
31 - assert  expected /login/password to equal /login/password
32   get [role="alert"]                                    0
33 - assert  expected [role="alert"] to be visible         FAILED
     (new url) http://localhost:13200/login
```

```
✖  dashboard.cy.ts      02:18   8   2   6   -   -
✔  checkout.cy.ts       00:13   1   1   -   -   -
✔  shop-hosted.cy.ts    00:14   3   3   -   -   -
✖  1 of 3 failed (33%)  02:46  12   6   6   -   -
```

The other five dashboard failures are the cascade: that spec runs
`testIsolation: false`, and its retries replay an enrolment that attempt 1 has
already committed, so attempt 2 looks for a QR code that will never be shown
again. The reported error for failure 1 is therefore the retry's
(`[data-testid="totp-qr"]` never found) and not the first attempt's, which is
the one quoted above.

**Fix:** a `401` from this endpoint clears nothing. Nothing is lost, because a
guard already existed for the case it was covering: `PasswordPage` reads the
session on every render and redirects to `/login` when vpay refuses it. A
session that really is over therefore still ends at the sign-in form — one
render later, decided by the page whose job it is from a fresh answer, rather
than by an action inferring it from a status that now means two things.

**The mutation:** put the two lines back and leg 4 fails again, in a browser,
exactly as above.

### F2 — a repeated `X-Forwarded-For` was read as its first line only · **correctness**

`client_address_from` read `HeaderMap::get`, which answers the **first** field
line. A proxy may append its hop as a **new** `X-Forwarded-For` line rather
than rewriting the caller's — a per-proxy configuration difference, not a
rarity — and RFC 9110 §5.2 makes repeated field lines one comma-separated list
in the order received.

On such a deployment the entire chain this module walked was therefore the one
the **caller** wrote. The "first untrusted hop from the right" was whatever
they put at the end of their own line, and the allow-list handed out **a fresh
rate-limit bucket per request** instead of closing one — the same hole
`a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget` refuses
from an untrusted peer, arriving instead from a trusted one, which is the
deployment the feature exists for.

**Fix:** `get_all`, walked right to left across every line. A line that is not
readable as text ends the walk at the peer rather than being stepped over —
skipping it would mean trusting what lies on the far side of a hop this
deployment could not check.

**The mutation:** restore `headers.get(..)`.

| Level | As delivered | Fixed |
|---|---|---|
| unit — `a_repeated_header_is_one_chain_and_the_callers_line_is_not_the_end` | `Some(198.51.100.99)`, the address the caller chose | `Some(203.0.113.7)`, the one the proxy vouched for |
| socket — `a_repeated_forwarded_for_is_one_chain_and_buys_no_fresh_budget` | `[401, 401, 401, 401, 401, 401]` — no `429` at all | `[401 × 5, 429]` |

### F3 — the configured origin was configured nowhere · **rule-break**

`VPAY_DASHBOARD_PUBLIC_ORIGIN` was read by `server/csrf.ts` and set by no
compose file and no helm value — the implementer's notes name that as the
follow-up and are honest about it. The cost is not hypothetical: it left the
`Host` fallback as the only path anything ever executed, so the branch's
headline claim — "the CSRF check compares a value no caller can set" — was
true of a code path no run of `just test-e2e` had ever taken.

**Fix, and it is two different things because the two deployments are:**

- `compose.e2e.yml` sets it to
  `http://localhost:${VPAY_DEMO_DASHBOARD_PORT:-3000}`, the same variable the
  publication and `VPAY_DASHBOARD_REDIRECT_URI` are keyed to. This one is
  *consumed*, and it is proven by the e2e run: a wrong value refuses every
  server action, so a green `dashboard.cy.ts` is the assertion.
- The chart gets `dashboard.publicOrigin` (values, schema, README) and an
  eighteenth guard, `dashboard-public-origin`, on its **shape** — a trailing
  slash or a bare hostname there refuses every action on the dashboard, and
  the values file is the only cheap place to catch it. **No template reads
  it**, because this chart writes no dashboard workload, and every place the
  key appears says so. Inventing a Deployment to give it a consumer is
  precisely what AGENTS.md forbids and what the chart's own
  `dashboard-not-templated` guard exists to refuse.

**Kept optional, deliberately.** The brief asked for it to be set "so a later
pass can make it required". It is set; required is the *later* pass, and it
belongs with the Deployment, because a required value on a workload nothing
renders fails a deployment for a setting nothing reads.

`just helm-check`: 18 guards, all fired by name (18 expected); kubeconform 23
resources valid. `ci/values-full.yaml` carries a well-formed value so the
guard's passing side is exercised by something too — a guard's own values file
only ever proves that it fires.

### F4 — the window rollover and the sweep were decided in SQL and run by nothing · **gate-hole**

`count_attempt` is one statement, and three of the things it decides are
decided **inside** it: whether the window has elapsed, what the answer resets
to, and which rows the sweep takes.

Nothing ran any of them. `vpay_api::staff::rate_limit`'s unit tests cover
`Verdict::of`'s arithmetic over an integer the statement hands back;
`the_statement_keeps_the_three_properties_that_make_it_safe` asserts the
**text** of six fragments, which is a test of a string; and every case over a
booted server runs inside one 300-second window. **A `CASE` that never reset
passed all of them.** Its symptom in production is a staff member locked out
of the dashboard for good by ten wrong passwords — the durable lockout
ADR-0017 refuses by name, arrived at by accident.

Three cases in `vpay-db/tests/repositories.rs`, each measured against its
mutation:

| Case | Mutation | As mutated |
|---|---|---|
| `an_elapsed_rate_limit_window_is_replaced_rather_than_extended` | `attempts = attempts + 1`, dropping the reset arm | `5` where it demands `1` |
| `the_rate_limit_table_grows_by_one_window_and_is_then_swept` | the sweep matches nothing | **1040 rows** where it demands ≤ 60 |
| `the_rate_limit_id_is_the_budget_and_the_scope_column_is_only_a_label` | — | pins that `scope` separates nothing, so a caller who stopped hashing it into the `id` would merge two budgets and never notice |

**And the bound is now stated as it is.** Migration 0038 and the module both
say a caller spending fresh keys "drains the table faster than they fill it".
That is true in the limit and **not** true inside one window — nothing has
elapsed, so there is nothing to sweep. Measured: 1000 fresh keys inside one
window leave **1000 rows**. The honest bound is two rows per attempt for the
width of one window and then flat — about `2 × rate × 300` at steady state.
Which is still a bound, and a bound is what ADR-0017's objection needed; it is
not "the table never grows", and the difference is a page an operator sizes a
disk from.

### F5 — the current-password budget was exercised by nothing · **gate-hole**

`change_password:session` exists because the check it guards is an argon2id
verification that somebody holding a stolen session cookie can drive at will.
The case that proves the current password is required makes **two** wrong
attempts against a default budget of **five** and stops — so a
`check_password_change` that had been deleted, or wired to a policy of a
thousand, passed the entire suite.

`the_current_password_check_has_its_own_budget_and_it_is_the_sessions`
configures three, and reads `401` where it demands `429` with the limiter call
deleted. It also pins the two properties that were nowhere written down as
tests: a **successful** change spends a unit too (`login`'s rule — the limiter
counts before it knows the answer), and the budget is the **session's**, so a
second browser of the same person still has its own. The second is what stops
a thief with a stolen cookie from locking the owner out of their own sign-in,
and it was an argument in a doc comment and nothing else.

## The two calls the implementer surfaced, judged

### The shared default sign-in budget: **ten stays**

The brief asked for this to be decided with ADR-0017's rationale in front of
it. The case for lowering is the one the implementer stated: the budget got
strictly stricter by becoming shared, so there is headroom to spend.

The argument that settles it against lowering had not been made. **The half
that would be spent is the per-address one**, and with
`staff_auth.trusted_proxies` empty — the default, and the state of every
deployment behind a proxy that has not been reconfigured — that half is shared
by *everybody*. Lowering the shared number therefore makes a proxy-fronted
deployment lock its whole staff out faster, which is the opposite of what
"the budget got stricter" is being offered as a reason for.

And the reason for ten was never replicas: it is that a person who mistypes a
generated one-time password twice and then fetches it from a terminal must not
be locked out of their own first login. Nothing about that changed. What ten
buys an attacker, shared: 2,880 guesses a day at one account behind argon2id;
nothing at all against a 130-bit printed password; under one percent a year
against six TOTP digits with three live.

**The item stays open in `docs/status.md` all the same, and that is the point
of it being configuration.** A deployment that measures its own traffic moves
it without a release and without an ADR. The review takes the *default*, not
the choice.

### `VPAY_DASHBOARD_PUBLIC_ORIGIN`: set now, required later

See F3. Set where it is consumed and proven (`compose.e2e.yml`), named and
shape-checked where it is not (the chart). Kept optional, because required
belongs with the dashboard Deployment this chart does not write.

## Left open on purpose

### `submitTotp` signs a person out for a mistyped code · **nit / misleading comment**

The same `401`-means-two-things shape as F1, one route over. A wrong six-digit
code is a `401`, and `submitTotp` clears the session cookie on any `401` from
`/staff/totp` — so a mistyped code sends somebody back to the email-and-
password form rather than telling them. The comment there lists "gone,
expired, idle, at the wrong stage" and not "wrong code", which is the
commonest of the five.

**Not fixed, and not because it is small.** It predates this branch, it is not
a hole (the extra sign-in bounds code guessing rather than loosening it), and
unlike `changePassword` the two lines cannot simply be dropped:
`/login/totp` reads no session on render, only the cookie's presence, so with
the cookie kept a session that really *is* over leaves the person retyping
codes at a form that will never accept one. Closing it means giving that page
the session read `PasswordPage` already has — a change to a route this brief
did not cover, in a review that would then be unreviewed. Recorded in
`docs/flows/dashboard.md` beside `refusalFor`.

### Proactive token re-mint (#88 item 1)

Still not done, and the implementer's account of why is accurate: the reactive
re-mint in `dash-read.ts` exists and works, and a proactive one needs a
`staff_sessions.access_token_expires_at` column, an API field, a `gateFor`
arm and a configurable TTL. This review did not attempt it either.
