# Dashboard auth — ending other sessions, and every refusal being one answer

_Split out of [docs/flows/dashboard-auth.md](../dashboard-auth.md) on 2026-09-11 by exp57, which broke a 732-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

### Changing a password ends every other session

`POST /dash/v1/staff/password` takes the password **in force** as well as the
new one, and on success deletes every session of that staff member **except
the caller's own**. Neither was true until 2026-09-10 (issue #79 item 3), and
each absence was its own defect:

- **No current password** meant the credential protecting an irreversible
  account takeover was the session cookie alone. The argument for leaving it
  out was written down — "the session making the change has already presented
  both factors" — and what it misses is _when_: a session lives twelve hours
  and the factors were presented once, at its start. An unattended browser, a
  stolen cookie or an XSS on the dashboard's origin bought the account in one
  request, `password_change_required` included.
- **No revocation** meant that changing a password did nothing about the
  person you changed it because of. Their `staff_sessions` row stayed
  `authenticated` with its `access_token` column intact until the absolute
  bound, up to twelve hours later.

The caller's own session survives on purpose: it is the one session here known
to have just proved two factors _and_ the current password, and signing it out
would make the success case look like a failure. The cascade on
`oauth_authorization_codes.session_id` takes any code the deleted sessions had
in flight.

The current-password check has **its own rate-limit budget**, keyed by the
session (`change_password:session`, five per five minutes by default) — the
narrowest thing identifying that caller, and deliberately not the sign-in
budget: a thief holding a stolen cookie must not be able to lock the owner out
of their own login by guessing here. A **successful** change spends a unit too,
for `login`'s reason and no other: the limiter counts before it knows the
answer, because an attempt over budget must not cost an argon2id
verification.

> _Added 2026-09-10 by the exp36 review (finding F5)._ That budget was
> exercised by nothing. The case proving the current password is required
> makes two wrong attempts against a default of five and stops, so a
> `check_password_change` that had been deleted — or wired to a policy of a
> thousand — passed the whole suite, and the check it guards is an argon2id
> verification anybody holding a stolen cookie can drive at will.
> `the_current_password_check_has_its_own_budget_and_it_is_the_sessions`
> reads `401` at the fourth attempt against a budget of three with the
> limiter call deleted, and asserts the second half too: a second browser of
> the same person still has its own budget.

Every refusal is the same `401` this document's next section describes. Absent
and wrong are one answer.

**And a `401` from this endpoint ends nothing.** _Corrected 2026-09-10 by the
exp36 review (finding F1)._ The dashboard cleared its session cookie on any
`401` from `POST /staff/password` — which was right while the endpoint's only
refusal was an unauthenticated session, and became wrong the moment it grew a
credential to refuse. "Every refusal is one answer" cuts both ways: a caller
cannot tell "your session is over" from "that is not your password", so the
only safe reading on a credential endpoint is that neither ends the session.
Whether the session is over is the _next render's_ question, and
`PasswordPage` asks it on every render and redirects to `/login` when vpay
refuses. Measured before the fix, in a browser at
`demo_dashboard_port=13200`: a wrong current password produced
`(new url) /login` and no alert at all, and `dashboard.cy.ts` was 6 of 8
failing.

Proof:
`changing_a_password_needs_the_current_one_and_ends_every_other_session`. The
decisive mutations, one per half: delete the `verify_password` and the first
two assertions read `200`; delete the `delete_others` and the other browser's
next render reads `200` where it must read `401`.

### A mistyped code does not end a session either, and closing that took a route

_Added 2026-09-10 (issue #88, the exp36 review's finding F6, left open there
with the reason.)_ `POST /dash/v1/staff/totp` has exactly the shape the
previous section describes: it answers one `401` for a **wrong six-digit
code** and for every session it refuses. `submitTotp` read all of them as "the
session is over", cleared the session cookie _and_ the sealed enrolment blob,
and sent the person back to the email-and-password form. A typo is the
commonest of the five cases the comment there listed and was not one of them;
on a first sign-in it was worse than a bounce, because the retry then carried
no secret to commit and the enrolment could not be completed at all.

The two lines could not simply be deleted, which is why F6 stayed open.
`/login/password` could drop its own copy (F1) because `PasswordPage` reads
the session on every render — but `/login/totp` read **no** session, only the
cookie's presence, so with the cookie kept a session that really was over
would leave somebody typing codes at a form that could never accept one.

It reads one now, and `GET /dash/v1/staff/session` cannot serve it: that route
is refused for a session that has not presented a code, with the same `401` it
answers for a session that is over. So there is an eighth staff route:

> **`GET /dash/v1/staff/session/stage`** — `{"stage": "pending_totp" |
"authenticated"}` for a session inside both bounds whose account is active,
> `401` for every other, and **nothing about the person**. A caller here has
> presented a password and no second factor; a display name, an email or a
> merchant id would be identity moved to the wrong side of it. It does not
> touch `last_seen_at` either — being asked for a credential is not use, and
> moving the idle bound on a render of that page would let an unattended
> browser hold a half-authenticated session open indefinitely.

So the answer to a wrong code is now the sentence and the form, with the
enrolment panel still on it; the only thing that sends somebody back to
`/login` from that page is a `401` from the stage read; and a vpay that cannot
be reached renders the failure and keeps the cookie, as everywhere else since
issue #88 item 2.

What this admits, stated rather than glossed: a caller holding a stolen
`pending_totp` session may now keep guessing codes at one screen instead of
re-presenting a password for each attempt. That is bounded by the same
limiter, which counts a wrong code against the shared sign-in budget — ten
per five minutes by default, against a six-digit space.

Proof: `a_wrong_code_leaves_the_session_live_and_the_stage_route_says_so` and
`a_disabled_account_is_refused_at_the_stage_route` over a booted server, the
`totpGateFor` cases in `gate.test.ts`, and the mistyped-code leg of
`dashboard.cy.ts` in a real browser. The decisive mutations: `session_stage`
calling `authenticated_session` instead of `load_session` (the first two
assertions read `401`); `totpGateFor` mapping any failure to `'dead'` (the
outage case signs everybody out mid-sign-in); `submitTotp` clearing the cookie
on a `401` again (the browser leg finds `/login`).

## Every refusal is one answer

No such address, wrong password, disabled account, wrong code, replayed code,
expired session, idle session, forged session, session at the wrong stage:
one `401`, one sentence, `error.code = "authentication_error"`. The step that
refused reaches the log and never the body.

The **timing** half of the same property is
`vpay_api::staff_auth::StaffCredentials::verify_absent_account`: an address
with no account still costs one argon2id verification, so "no such account"
and "wrong password" take the same time as well as the same shape. A login
form that answered differently for either would be an account-enumeration
oracle.

Sign-in is rate limited per email **and** per address, fixed window, ten
attempts per five minutes by default. `POST /staff/login` is refused **before**
any credential work happens, because an attempt over budget must not cost an
argon2id verification. Both counters move on every attempt including a refused
one — short-circuiting would let an attacker who exhausted one address keep
hammering a thousand others from the same host with that host's counter
frozen.

**The counters were in one process's memory until 2026-09-10** and are rows in
Postgres now (issue #79 item 2) — see "The budget is the deployment's" below.

**The second factor spends from the same budget, and it did not until
2026-09-07** (the exp28 review). The limiter was wired to `/staff/login` alone,
and a TOTP code is six digits with three of them live at any instant — on a
path that costs one HMAC-SHA1 and no argon2id, so a caller holding one phished
password and the `pending_totp` session it produces could guess at whatever
rate the network allowed. Measured against a real stack: thirty consecutive
wrong codes, thirty `401`s, no `429`.

`POST /staff/totp` counts on the **failure** path rather than before the
verification, which is the one place it differs from the password leg. The
budget is shared between the two, behind a proxy the per-IP half of it is
shared by the whole deployment, and a correct code that spent a unit would
have halved how many people can sign in per window to close a hole only wrong
codes exploit. There is no argon2id here to protect, so the count can wait
until the answer is known.
