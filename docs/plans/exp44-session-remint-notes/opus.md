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

- a typo sent the person back to the email-and-password form with no
  explanation — the commonest failure on that screen, and the one the comment
  there did not list;
- on a **first sign-in** it was worse than a bounce. The enrolment cookie is
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
"authenticated"}` for a session inside both bounds whose staff row is
> `active`; `401` for every other; **nothing about the person**.

Three properties, each deliberate:

- **`load_session`, not `authenticated_session`.** That is the whole point —
  and the decisive mutation, which turns the first assertion of
  `a_disabled_account_is_refused_at_the_stage_route` red.
- **No identity in the body.** A caller at this stage has presented a password
  and no second factor. A display name, an email or a merchant id would be
  identity moved to the wrong side of it. Two tests pin the key set: one unit
  (`the_stage_response_carries_one_field_and_no_identity`) and one over a
  booted server.
- **It does not touch `last_seen_at`.** Unlike `/staff/session`. Being asked
  for a credential is not use of a session, and moving the idle bound on a
  render of that page would let an unattended browser hold a
  half-authenticated session open indefinitely.

### The dashboard side

`server/gate.ts::totpGateFor` is the decision, pure and unit-tested, in the
same shape and for the same reason as `refusalFor` beside it:

| Stage read      | What the page does                                                         |
| --------------- | -------------------------------------------------------------------------- |
| `pending_totp`  | render the code form (with the enrolment panel, if the blob is there)      |
| `authenticated` | on to `/payments` — a back button, or a reload after the action redirected |
| `401`           | `/login`. **The only branch that ends a session**                          |
| anything else   | render the failure, keep the cookie (issue #88 item 2, on this route)      |
| neither         | `/login`. Fails closed                                                     |

`submitTotp` now clears nothing on a refusal, exactly as `changePassword`
does not.

### What this admits, stated rather than glossed

Somebody holding a stolen `pending_totp` session may keep guessing codes at
one screen instead of re-presenting a password per attempt. It is bounded by
the same limiter — `staff::totp_step` counts a wrong code against the shared
sign-in budget, ten per five minutes by default, against a six-digit space.

### Proof

- `a_wrong_code_leaves_the_session_live_and_the_stage_route_says_so` — over a
  booted server: `/staff/session` refused and `/staff/session/stage` not, at
  the same instant with the same token; the body's key set; a wrong code
  refused and the session still `pending_totp` afterwards; the **retry with
  the same blob** completing the enrolment; `authenticated` after it;
  sign-out, a forged token and no header all `401`.
- `a_disabled_account_is_refused_at_the_stage_route`.
- Five `totpGateFor` cases in `gate.test.ts`.
- `dashboard.cy.ts` leg 3a, in a real browser: a wrong code, then the alert,
  the surviving cookie **and the surviving QR** — the enrolment panel is the
  assertion that would be missed by a spec checking only the path.

Decisive mutations, all run:

| Mutation                                        | Gate               | Measured                                                                              |
| ----------------------------------------------- | ------------------ | ------------------------------------------------------------------------------------- |
| `session_stage` calls `authenticated_session`   | `staff_sign_in.rs` | **FAIL** — `a_disabled_account_is_refused_at_the_stage_route`, `left: 401 right: 200` |
| `totpGateFor` maps any failure to `'dead'`      | dashboard vitest   | see the run below                                                                     |
| `submitTotp` clears the cookie on a `401` again | `dashboard.cy.ts`  | see the run below                                                                     |

## 2. The proactive re-mint (issue #88 item 1)

### The arithmetic

`staff_auth.access_token_ttl_seconds` is 900 by default; a session is thirty
minutes idle and twelve hours absolute. `requireStaff` ran the
authorization-code leg only when the session row carried **no** token, so once
one was written it was used until sign-out — and a quarter of an hour into
every sign-in `/dash/v1` answered

    The bearer token is invalid, expired, or was not issued for this endpoint.

on every render. The exp28 review measured it (F4) and added the **reactive**
re-mint in `dash-read.ts`: one retry, on a `401` only. That works and costs a
refused request in front of every read once a token has died.

### There is no refresh token, and the OP was read before one was invented

`authkestra-op` offers a `RefreshTokenStore`; this deployment gives it
`vpay_api::op::refusing_stores`' fail-closed type, and
`docs/flows/dashboard-auth.md`'s "Token lifetimes" refuses one by name — there
is no endpoint to revoke it with. So **the refresh is the authorization-code
leg**, run again on the session the browser already holds, which
`completeAuthorizationCode` already did for a session with no token at all.

That is not a way around a check. `vpay_api::staff::oauth::authorize` re-reads
the staff row and re-checks the active status, the merchant binding and
`password_change_required` on **every** mint, so re-minting is strictly more
checking than carrying one token for twelve hours. The four refusals the brief
asked to see proven are exactly the four that leg already performs, and
`the_dashboard_token_is_re_minted_from_a_live_session_and_from_nothing_else`
now drives all of them with a restored control after each.

### What was added

| Piece                                                                          | Why it could not be skipped                                                                      |
| ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------ |
| `staff_sessions.access_token_expires_at` (migration `0040`)                    | The app has to decide _before_ the expiry and had no way to know when that was                   |
| `staff_sessions_token_expiry_is_paired`                                        | The column and the token are one fact; a token with no expiry reads as "re-mint on every render" |
| `access_token_expires_at` + `access_token_ttl_seconds` on `GET /staff/session` | The margin is a **fraction**, and the app cannot divide by a number it does not have             |
| `staff_auth.access_token_ttl_seconds`                                          | At 900 s no browser run crosses an expiry, which is why the fifteen minutes shipped              |
| `REMINT_AFTER_FRACTION` + the `stale-token` arm                                | The decision, pure and unit-testable, beside `refusalFor`                                        |

Two choices worth defending:

**The app does not read the JWT's `exp`.** It holds the token and presents it;
it is not a verifier of it. Reading a claim out of an unverified credential is
a habit worth not having in a payments app, and vpay already knows the answer.

**`stale-token` is a separate arm from `needs-token`**, because there is
something to fall back on. A vpay that cannot be _reached_ for the re-mint
leaves the page rendering with the token in hand — it has not expired, that is
what the margin bought — instead of replacing a working page with an error box.
A `401` is **not** fallen back on, stale or not: that is `/authorize` refusing
this session on this request, and reading on with the token it has just
invalidated is the hole the exp24 review's finding F1 closed one layer down.

### Migration 0040 and the rows that already exist

It **clears every `access_token` already stored** rather than inventing an
expiry for one. There is no honest value: the TTL may have changed, and
deriving 900 seconds from `last_seen_at` would write down a time that is not
when that token expires. Nobody is signed out — a session with no token is the
state every session is in between the second factor and its first render, and
the next render mints one, re-checking everything on the way. The `UPDATE` runs
**before** the CHECK is added, because the CHECK is validated against every
existing row and a live token with no expiry is exactly the row it refuses.

### The demo stack's thirty seconds

`demo_staff_token_ttl := "30"`, and `just demo` gets it too. That is a
deliberate difference from production and it is the point of the setting: at
900 seconds nothing that runs in under a quarter of an hour ever crosses an
expiry, so the bug was shipped by a suite that could not have caught it. At
thirty, `dashboard.cy.ts` crosses the margin (twenty-four seconds) and the
expiry in one leg with 4.5 seconds of slack. It was **twenty** first, and that
window is four seconds wide: measured, the leg landed 300 ms inside it, which
is a flake waiting for a slower machine. The waits are computed from the expiry
vpay reported rather than counted from the spec, so the slack is a stated
number rather than whatever a `cy.visit` happened to cost. `just demo_staff_token_ttl=900 demo` gets the production number and
regenerates the overlay to match — `gen-demo-keys`' shape check is keyed on the
current value, exactly as it is for `demo_dashboard_port`.

### Proof, and the one mutation a browser alone cannot make decisive

The reactive retry still exists, so a page renders identically whether the
token was replaced early or replaced after a read failed. A Cypress assertion
on the rendered page therefore proves nothing about the margin. The leg asserts
on `access_token_expires_at` instead, read from vpay through a Node task with
the httpOnly session cookie: **the expiry moved while the old one had not yet
passed.**

Run, on the final tree: with `marginMs` forced to `0` — "re-mint only when the
token is already dead", which is the reactive path alone — the leg fails at the
first of its two assertions, _"the same expiry back means nothing re-minted"_,
with `dashboard.cy.ts` 8 of 9 passing and 1 failing across all three Cypress
retries. Reverted, it is 9 of 9.

## 3. The three browser mutations, measured

| Mutation                                                     | Gate                                        | Measured                                                                                              |
| ------------------------------------------------------------ | ------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `session_stage` calls `authenticated_session`                | `staff_sign_in.rs`                          | **FAIL** — `left: 401 right: 200` on `a_disabled_account_is_refused_at_the_stage_route`               |
| `submitTotp` clears the cookie on a `401` again              | `dashboard.cy.ts`, real browser, real stack | **FAIL** — leg 3a: `-'/login' +'/login/totp'`, and the cascade takes the spec to 1 passing, 8 failing |
| the re-mint margin dropped (`marginMs = 0`)                  | `dashboard.cy.ts`                           | **FAIL** — "the same expiry back means nothing re-minted", 8 passing, 1 failing                       |
| the dashboard OP's TTL hard-coded to `ACCESS_TOKEN_TTL_SECS` | `staff_sign_in.rs`                          | **FAIL** — `expires_in` `900` against a configured `10`                                               |
| `staleTokenFor` returns `false` for an absent expiry         | dashboard vitest                            | covered by `re-mints when vpay sent no expiry at all`                                                 |
| `totpGateFor` maps any failure to `'dead'`                   | dashboard vitest                            | covered by `keeps the cookie when vpay could not be reached`                                          |

## 4. Corrections made in passing

- `postgres_smoke.rs`'s multi-column-CHECK list was commented "None of the
  eleven" and "Three of the eleven" while the list held seventeen. Both counts
  now say eighteen. A comment that miscounts the list beside it is the one a
  reader trusts instead of counting.
- `dash/mod.rs` and `docs/reference/vpay-api.md` said "seven unauthenticated
  routes"; there are eight.

## 5. What this pass did NOT do

- **No browser leg disables a staff member mid-session.** There is no `cy.task`
  that can reach the database or the CLI, and adding one is a wider change than
  this brief. The behaviour is proven over a booted server instead —
  `the_dashboard_token_is_re_minted_from_a_live_session_and_from_nothing_else`
  case 2, and `disabling_a_staff_member_refuses_their_live_session`.
- **`deploy/helm` does not carry `access_token_ttl_seconds`.** A chart
  deployment gets the 900 default. The chart writes no vpay-server config
  document at all today, so this would be the first.
- **Nothing was done about the residual ADR-0017 records**: a minted token
  stays cryptographically valid for its TTL after a sign-out. A shorter
  configured TTL narrows that window and does not close it, and closing it
  means binding every `/dash/v1` read to a live session row — a maintainer
  decision the exp24 review surfaced and nobody has taken.

## 6. The gate

`just ci`, on `a7e6971`, exit code read from a file: **0**.

| Recipe                                            | Measured                                                                                                    |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `fmt-check`                                       | clean                                                                                                       |
| `clippy --workspace --all-targets -- -D warnings` | clean                                                                                                       |
| `verify`                                          | the twelve gates; `verify-migrations` 40 files against the manifest, `verify-links` 1034 links in 197 files |
| `test-rust`                                       | **1665 run, 1665 passed, 0 skipped**, 45 binaries, 1144 s, against a real Postgres                          |
| `test-doc`                                        | **111 passed, 1 ignored**                                                                                   |
| `verify-ignored`                                  | 0 ignored (expected 0), 45 binaries (expected 45), 1665 total                                               |
| `lint-web`                                        | clean                                                                                                       |
| `test-web`                                        | all packages green; `@vpay/dashboard` **182 in 21 files**                                                   |
| `deny`                                            | advisories, bans, licenses, sources all ok                                                                  |

`just test-e2e`, on its own compose project (`demo_project=exp44`,
`demo_dashboard_port=13800`, `demo_port=18800`): **19 of 19 passing**, exit 0 —
dashboard 9, checkout 1, shop-hosted 3, shop-embedded 6.

`staff_sign_in.rs` is **23 → 26** cases across this branch; `postgres_smoke.rs`
+1; dashboard vitest **177 → 182**.
