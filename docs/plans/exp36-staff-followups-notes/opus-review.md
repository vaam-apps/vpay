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
