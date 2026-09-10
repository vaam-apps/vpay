# exp44 — the dashboard session's remaining gaps

**Branch:** `claude/exp44-session-remint` · **Base:** `231f51e` (master) ·
**Date:** 2026-09-10

Two things were asked for: the **proactive access-token re-mint** (issue #88
item 1) and the **TOTP page's mistyped-code sign-out** (the exp36 review's
finding F6, recorded there and left open with its reason). The brief said to
deliver the TOTP page first if the re-mint turned out to be larger than one
pass. This document is written as the work lands, so what is here is what is
done.

## 1. A mistyped code no longer signs a staff member out

### What was wrong

`POST /dash/v1/staff/totp` answers one `401` for a **wrong six-digit code**
and for every session it refuses. That uniformity is deliberate
(`docs/flows/dashboard-auth.md`, "Every refusal is one answer"): a caller must
not learn that the password was right and only the code was wrong.

`submitTotp` read every one of those `401`s as "the session is over" and
cleared the session cookie **and** the sealed enrolment blob. So:

* a typo sent the person back to the email-and-password form with no
  explanation — the commonest failure on that screen, and the one the comment
  there did not list;
* on a **first sign-in** it was worse than a bounce. The enrolment cookie is
  what carries the sealed secret between the login and the first code, so with
  it gone the retry had nothing to commit and the enrolment could not be
  completed at all.

This is the same shape as F1 (`changePassword`) one route over.

### Why F6 could not be closed by deleting two lines

`changePassword` could drop its copy because `PasswordPage` reads the session
on **every** render and redirects when vpay refuses it. `/login/totp` read no
session at all — only whether a cookie was present — so with the cookie kept, a
session that really was over would leave somebody typing codes at a form that
could never accept one. The exp36 review said exactly this and stopped there.

### The route that closes it

`GET /dash/v1/staff/session` is refused for a `pending_totp` session, with the
same `401` a dead session gets, so it cannot answer "is my session still
alive?" mid-sign-in. The eighth staff route does:

> **`GET /dash/v1/staff/session/stage`** → `{"stage": "pending_totp" |
> "authenticated"}` for a session inside both bounds whose staff row is
> `active`; `401` for every other; **nothing about the person**.

Three properties, each deliberate:

* **`load_session`, not `authenticated_session`.** That is the whole point —
  and the decisive mutation, which turns the first assertion of
  `a_disabled_account_is_refused_at_the_stage_route` red.
* **No identity in the body.** A caller at this stage has presented a password
  and no second factor. A display name, an email or a merchant id would be
  identity moved to the wrong side of it. Two tests pin the key set: one unit
  (`the_stage_response_carries_one_field_and_no_identity`) and one over a
  booted server.
* **It does not touch `last_seen_at`.** Unlike `/staff/session`. Being asked
  for a credential is not use of a session, and moving the idle bound on a
  render of that page would let an unattended browser hold a
  half-authenticated session open indefinitely.

### The dashboard side

`server/gate.ts::totpGateFor` is the decision, pure and unit-tested, in the
same shape and for the same reason as `refusalFor` beside it:

| Stage read | What the page does |
|---|---|
| `pending_totp` | render the code form (with the enrolment panel, if the blob is there) |
| `authenticated` | on to `/payments` — a back button, or a reload after the action redirected |
| `401` | `/login`. **The only branch that ends a session** |
| anything else | render the failure, keep the cookie (issue #88 item 2, on this route) |
| neither | `/login`. Fails closed |

`submitTotp` now clears nothing on a refusal, exactly as `changePassword`
does not.

### What this admits, stated rather than glossed

Somebody holding a stolen `pending_totp` session may keep guessing codes at
one screen instead of re-presenting a password per attempt. It is bounded by
the same limiter — `staff::totp_step` counts a wrong code against the shared
sign-in budget, ten per five minutes by default, against a six-digit space.

### Proof

* `a_wrong_code_leaves_the_session_live_and_the_stage_route_says_so` — over a
  booted server: `/staff/session` refused and `/staff/session/stage` not, at
  the same instant with the same token; the body's key set; a wrong code
  refused and the session still `pending_totp` afterwards; the **retry with
  the same blob** completing the enrolment; `authenticated` after it;
  sign-out, a forged token and no header all `401`.
* `a_disabled_account_is_refused_at_the_stage_route`.
* Five `totpGateFor` cases in `gate.test.ts`.
* `dashboard.cy.ts` leg 3a, in a real browser: a wrong code, then the alert,
  the surviving cookie **and the surviving QR** — the enrolment panel is the
  assertion that would be missed by a spec checking only the path.

Decisive mutations, all run:

| Mutation | Gate | Measured |
|---|---|---|
| `session_stage` calls `authenticated_session` | `staff_sign_in.rs` | **FAIL** — `a_disabled_account_is_refused_at_the_stage_route`, `left: 401 right: 200` |
| `totpGateFor` maps any failure to `'dead'` | dashboard vitest | see the run below |
| `submitTotp` clears the cookie on a `401` again | `dashboard.cy.ts` | see the run below |

## 2. The proactive re-mint (issue #88 item 1)

See the final report for what was and was not delivered.
