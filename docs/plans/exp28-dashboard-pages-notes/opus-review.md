# exp28 review — breaking into the dashboard

A sabotage review of `claude/exp28-dashboard-pages` (base `05ae51a`, delivered
head `f6d4ce8`, 14 commits) against the implementer's [opus.md](opus.md) and
`CLAUDE.md`'s "The failure mode to avoid". This is a security surface, so the
method was to get in rather than to read carefully: a compose stack of its own
(`exp28-review`, non-default ports, the user's `vpay-demo` untouched), a real
staff member created through the shipped CLI, and every refusal exercised over
HTTP against the running app.

Everything below that says "measured" was measured on that stack. Everything
that was reasoned from the code rather than run says so.

## The short version

**Safe as delivered: no.** Not for the reason a reviewer expects, though — the
*authorisation* is right everywhere it was attacked. No page rendered another
tenant's data, no credential reached a browser or a log, sign-out and account
disablement both took effect on the very next render, and there is no open
redirect anywhere in the app. What was wrong is the shape of the answers, the
lifetime of the credential, and the shape of the deployment.

| # | Finding | Severity |
|---|---------|----------|
| F1 | **Every refusal on a protected page answered `500`, and kept answering it.** `requireStaff` cleared the session cookie during a page render, which Next refuses outright; the exception escaped and the redirect never happened. A forged cookie, a session signed out from another browser, and an account an operator had just disabled each got Next's error page instead of the sign-in form — and, the cookie never having been cleared, so did every request after it. | gate-hole |
| F2 | **The first `/payments` after a sign-in answered `500`.** The `needs-token` branch read `GET /staff/session` a second time to confirm the exchange; React memoises identical `GET`s within one render, so it got the pre-exchange body back and concluded the session was unusable. Deterministic, 3/3. | gate-hole |
| F3 | **The second factor was not rate limited.** ADR-0017 says "sign-in is rate limited"; the limiter was wired to `/staff/login` only. Thirty consecutive wrong TOTP codes: thirty `401`s, no `429`. Six digits behind a phished password, no argon2id on that path. | gate-hole |
| F4 | **The dashboard stopped working fifteen minutes into every sign-in.** The `/dash/v1` token's TTL is 900 s, a session's is up to twelve hours, and nothing re-minted: `requireStaff` exchanges a code only when the row carries no token at all. Every render after the first quarter of an hour was an error box. | correctness |
| F5 | **`just demo` published the dashboard on `0.0.0.0:3000`.** compose.demo.yml binds every publication to loopback as a stated security property and left the dashboard alone because the demo never started it — with the note "Bind it here the day it grows a data source". exp28 added it to `demo_services` and did not. Reachable from the LAN, measured. | gate-hole |
| F6 | **CI's `e2e` job cannot pass this branch.** It brings its own stack up rather than calling `just test-e2e`, so it never runs `just demo-staff`; `dashboard.cy.ts` has nobody to sign in as and five of its cases fail on `cy.task('staffPassword')`. Green locally, red on the pull request. | gate-hole |
| F7 | **The timeline presented an incomplete history as a complete one.** Five of the eight documented event types are written by nothing, so a succeeded payment's "Timeline" is one line — and the page said nothing about it. | misleading-claim |
| F8 | **The `created_to` filter dropped the last second of the day.** `23:59:59Z` against a microsecond `TIMESTAMPTZ` and a `<=` predicate. | correctness |
| F9 | **`just demo-staff` reported "created" when it had not written the password.** No `-e`, no check on the redirect, and `chmod 0600` ran on whatever `$out` named. The password is printed once. | correctness |
| F10 | Three stale claims left behind by the same edits: the `demo-up` recipe's "the EIGHT services … the dashboard is the one this recipe deliberately leaves down", compose.demo.yml's "`just demo-up` does not start it", and `docs/status.md`'s "the one Cypress spec asserts the dashboard's scaffold notice and never starts a login". | misleading-claim |
| F11 | `just ci` failed once on the delivered head at `test-web`, in `@vpay/ui`'s `select.test.tsx` — **not this branch's file**, and green 5/5 in isolation. A flake under parallel load, reported rather than fixed. | rule-break (pre-existing) |
| F12 | *Found in this review's own work, not in the delivered branch.* F1's fix added `GET /signed-out`, a `GET` that clears a cookie — which is the `<img src>` forced-logout `signed-in-bar.tsx` already warns about. Fixed in the same pass with `Sec-Fetch-Dest`. | correctness |

F1 and F2 are the same 500 with two different causes and were found in the
same minute, by `curl -H 'Cookie: vpay_dash_session=<garbage>' /payments`.
Everything after that came from asking the same question in the other
direction.

**Safe as reviewed: yes**, for the surface that exists, with the residuals in
"Maintainer decisions" written where an operator will find them.

## What the delivered branch got right, and it is most of it

Worth stating first, because the table above is a list of what broke and this
review found no way through any of the following.

* **Tenancy.** A staff member of `demo-merchant-tenant` opening another
  merchant's `pi_…` got `404`, and the only occurrence of that id in the HTML
  was the router segment they had typed themselves — no amount, no
  description, no rail. The list showed one row of two.
* **Revocation.** Signing out through the API and then rendering `/payments`
  produced no data; disabling the account in Postgres refused the very next
  render. The token really is re-read from the row, which is what the exp24
  review's F1 asked for.
* **Credentials in the clear.** The whole 5,672-line compose log of a full
  sign-in, enrolment, password change, token exchange and payments read
  contains **zero** occurrences of a JWT (`eyJ`), the session token, the
  one-time password, the TOTP secret, `code=`, `code_verifier` or
  `code_challenge`.
* **CSRF.** A cross-origin server-action POST is refused by Next's own check:
  `` `x-forwarded-host` header with value `localhost:3000` does not match
  `origin` header with value `evil.example` … Aborting the action. `` The
  session cookie is `SameSite=Lax`, so a cross-site POST would not have
  carried it anyway.
* **Open redirect.** There is none to find: every `redirect()` in the app
  names a module constant, no page reads a `next`/`returnTo` parameter, and
  `readAuthorizationResponse` checks the `Location`'s origin and path against
  the registration before it will read a code out of it.
* **The `state` parameter** round-trips and a mismatch is refused
  (`oauth.test.ts`); the live exchange exercised it.
* **The forced password change** cannot be skipped: `/payments` with an
  authenticated session carrying `password_change_required` answered `307` to
  `/login/password`, and `/authorize` refuses such a session anyway.
* **The enrolment secret** is shown once, from a cookie the app sets itself,
  and is gone on the next sign-in (`dashboard.cy.ts` asserts the QR is absent
  the second time).
* **Money.** `formatAmount` does integer arithmetic on the digit string and
  refuses to place a decimal point for a currency `Intl` does not know. Read,
  not attacked; `format.test.ts` covers the zero-decimal case.

## The attack table

Every row was run against `exp28-review`. "Data?" means: did the response body
carry any payment of the merchant's.

| # | Attack | Delivered head | Data? | After this review |
|---|--------|----------------|-------|-------------------|
| 1 | `GET /payments`, no cookie | `307 → /login` | no | unchanged |
| 2 | `GET /payments`, forged cookie | **`500`** | no | `307 → /signed-out → 303 → /login`, cookie deleted |
| 3 | `GET /payments`, session signed out through the API from "another browser" | **`500`**, and `500` on every retry | no | sign-in form |
| 4 | `GET /payments` and `/payments/{id}`, staff row set to `disabled` | **`500`** | no | sign-in form |
| 5 | first `GET /payments` by a freshly authenticated session (no token on the row) | **`500`**; the second request `200` | n/a | `200` on the first |
| 6 | `GET /payments` holding a token `/dash/v1` refuses (what an expired one is) | **error box on every render, forever** | no | the list, and a fresh token on the row |
| 7 | `GET /payments` with `password_change_required` | `307 → /login/password` | no | unchanged |
| 8 | staff of A opens B's `pi_…` | `404`, Next's not-found page | **no** | unchanged |
| 9 | `?after=<another tenant's id>` | `200` with vpay's "a cursor must be a payment intent id" in an error `Alert` and a request id | no | unchanged |
| 10 | `?status=not_a_status` | `200` with an error `Alert` and a request id — **not** an empty list | n/a | unchanged |
| 11 | `?created_from=yesterday` | filter dropped, rows still listed | n/a | unchanged |
| 12 | `/payments/..%2F..%2Fstaff%2Fsession` | `404` | no | unchanged |
| 13 | 30 wrong TOTP codes on one `pending_totp` session | **30 × `401`, no `429`** | n/a | `429` inside 12 |
| 14 | replayed TOTP code | `401` (`record_totp_step` CAS) — read, and pinned by `a_replayed_totp_code_is_refused` | n/a | unchanged |
| 15 | cross-origin server-action POST to `/login` | refused, "Aborting the action" | n/a | unchanged |
| 16 | `?next=`, a mismatched `state`, a `Location` to another origin | no such parameter exists; both refused in `oauth.ts` | n/a | unchanged |
| 17 | compose log grep for JWT / session / password / secret / code / verifier / challenge | **0 hits each** | n/a | unchanged |
| 18 | the dashboard from the host's LAN address (`10.10.0.227:3000`) | **`307` — it answered** | n/a | connection refused |
| 19 | vpay-server from the same LAN address | connection refused | n/a | unchanged |

Rows 2–6 are four separate defects that all present as a broken page; each is
listed because each is a different way in.

### F1 in detail — a page may not clear a cookie

`session.ts`'s own header said "the answer to all of them is the same — clear
the cookie and show the form — and clearing it is what stops the loop".
`requireStaff` did exactly that, in a Server Component. Next:

    ⨯ Error: Cookies can only be modified in a Server Action or Route Handler.
        at q (.next/server/chunks/908.js:1:4169)
        at async s (.next/server/chunks/908.js:1:4527)
        at async t (.next/server/app/payments/page.js:1:3382) { digest: '3035987532' }

`dashboard.cy.ts` was green throughout, and it is worth being precise about
why, because the spec is a good one:

* its first case calls `cy.clearCookies()` — no cookie, so `requireStaff`
  redirects without clearing;
* its sign-out case passes because the sign-out **action** clears the cookie,
  which is legal there, leaving the next request with none.

Neither ever holds a cookie vpay refuses. That branch — the one every session
expiry in production takes — was reachable only through a browser and was
tested by nothing. The clearing moved to `app/signed-out/route.ts`;
`dashboard.cy.ts` now forges a cookie in a real browser and asserts both
halves.

`NextResponse.redirect` demands an absolute URL and the only one available in
a handler is built from `request.url`, which inside the container is
`http://<container id>:3000/login` — measured, and a browser follows it into a
DNS failure. The `Location` is written by hand and is relative.

### F2 in detail — the second read never happened

The `needs-token` branch exchanged the code and then read the session again
"to prove the row was written". For one render, with one session:

    16:41:49.742037  /dash/v1/oauth/authorize  issued a dashboard authorization code
    16:41:49.752448  /dash/v1/oauth/token      issued a dashboard access token

and afterwards, in Postgres:

    staff_sessions.last_seen_at = 2026-09-07 16:41:49.744685+00

`GET /staff/session` touches `last_seen_at` on every accepted read, and the
value is *earlier* than the mint — so no session read was served after the
token existed. React memoises `fetch` for identical `GET`s within one render;
the second call answered with the first call's body, which had no token on it,
and the branch refused the session it had just equipped.

The row did carry the token: the very next request rendered the list. The fix
uses the token the exchange returned, for the remainder of the one request
that minted it. Every later render still reads the row, which is the property
ADR-0017 decision 2's revocation actually rests on.

### F4 in detail — fifteen minutes

`vpay_api::op::ACCESS_TOKEN_TTL_SECS` is 900; a staff session is thirty
minutes idle and twelve hours absolute. `requireStaff` runs the
authorization-code leg only when `session.access_token` is absent, so once a
token is on the row it is used until sign-out.

Measured by writing a token `/dash/v1` refuses onto a live session row — which
is what an expired one is, from the app's point of view — and rendering the
page:

    Signed in as ada@example.test · merchant demo-merchant-tenant
    The bearer token is invalid, expired, or was not issued for this endpoint.
    Request 4c8e220c-86ea-46f3-9731-6c020b863889

The row still held the same dead token afterwards, so this is every render
until the person signs out and back in. Nothing could have caught it:
`dashboard.cy.ts` runs in under thirty seconds and no unit test exercises a
token older than the request that minted it.

`readDash` retries once, on `401` only. It is not a way around a check —
`staff::oauth::authorize` re-reads the staff row and re-checks active status,
the merchant binding and `password_change_required` on **every** mint, so
re-minting is more checking than carrying one token for twelve hours.

### F5 in detail — the note that said what to do, and the commit that did not

compose.demo.yml's header states, with a measurement behind it, that every
publication in that file is bound to `127.0.0.1` and that this is a security
property rather than a tidiness one. The `dashboard` service was the one
exception, and the note explaining why ended: *"Bind it here the day it grows
a data source."*

exp28 was that day. Measured with the demo stack up:

    $ curl -o /dev/null -w '%{http_code}' http://10.10.0.227:3000/
    307                       <- the dashboard, from the LAN
    $ curl -o /dev/null -w '%{http_code}' http://10.10.0.227:18190/healthz
    (7) Failed to connect     <- vpay-server, like everything else here

`docker compose … config` over the three files reported `host_ip: 127.0.0.1`
for all five services the overlay publishes and no `host_ip` at all for
`dashboard`.

## Mutation table

Each applied to the tree, the gate run, the mutation reverted. Every row was
run; none is predicted.

| Mutation | Gate | Measured |
|---|---|---|
| `COOKIE_ATTRIBUTES.httpOnly` → `false` | dashboard vitest | **2 failed, 143 passed** |
| token exchange sends a *fresh* `createPkce().verifier` | dashboard vitest | **1 failed, 144 passed** |
| `NAV_LINKS` gains `{ href: '/webhooks' }` | dashboard vitest | **2 failed, 143 passed** (the constant case and the rendered-markup case) |
| `pagerHrefs` reads `has_more` the same way in both directions | dashboard vitest | **2 failed, 143 passed** (one per half of the inversion) |
| `endOfDayUtc` back to `T23:59:59Z` | dashboard vitest | **2 failed, 143 passed** |
| the timeline note drops one of the five type names | dashboard vitest | **1 failed, 144 passed** |
| `signedOutResponse` stops deleting the cookie | dashboard vitest | **4 failed, 141 passed** |
| `readDash` stops retrying a `401` | dashboard vitest | **2 failed, 148 passed** |
| delete `limiter.check` from `staff::totp_step` | `staff_sign_in` | **1 failed** — fifteen consecutive `401`s and no `429` |
| `requireStaff` clears the cookie in-render again | `dashboard.cy.ts`, real browser, real stack | **4 of 8 failed**, beginning at "shows the sign-in form — not an error page — for a cookie vpay refuses" |

## The paging fix, and what pins it now

The brief asked for "the test that pins direction-relative `has_more` at both
ends", on the grounds that the implementer found the bug by reading. Both ends
were in fact already pinned, in two places, and the mutation above measures it:

* `payments-query.test.ts` — "offers Next unconditionally, we came from the
  page below" and "offers Previous only while has_more says newer rows
  remain". Inverting the reading fails both.
* `vpay-db/tests/repositories.rs:2336` — cursor paging over 25 rows in both
  directions, asserting `has_more` true on a backward page with newer rows
  beyond it and false at the newest end. That is the *source* semantics the
  app's reading depends on.

What neither covers, and is worth naming: **nothing ties the two together.**
The dashboard's unit test asserts the app's interpretation and the db test
asserts the repository's behaviour; a change to `list_page_filtered`'s
`has_more` that the app then followed would leave both green. `/dash/v1`'s own
integration suite (`dashboard_read_surface.rs`) does not exercise
`ending_before` beyond the tenancy case, and the demo stack never has more
than one page of payments, so `dashboard.cy.ts` cannot cover it either. Left
as a maintainer decision rather than added here.

## Honesty checks

| Claim | Verdict |
|---|---|
| no fake rows anywhere | holds — `rows` is a prop and nothing under `app/` or `src/` invents one |
| the empty state appears only on zero rows | holds — a failed read renders an `Alert` with the request id; measured with a bogus `status` filter |
| the `—` payer mask is a real null and is explained | holds — rendered from the column, never derived; explained in `docs/flows/dashboard.md`, in `runbooks/demo.md` §6 and in the component header, and pinned in both directions by `payment-detail.test.tsx` |
| no payer column on the list | holds, and the doc gives the stronger reason: the list response carries no charge at all |
| the events timeline | **F7** — fixed here |
| nav links only to pages that exist | holds, and the gate is real: `{ href: '/webhooks' }` fails `layout.test.tsx` twice |
| `just verify-ui` green, every `className` one line | holds — `git grep className` under `app/` and `src/`, excluding tests: **zero matches** |
| axe on every screen | 8 cases in `a11y.test.tsx` over every component and the real rendered `<body>`, **0 violations**. Structural rules only; `color-contrast` computes nothing in jsdom and is still checked by nobody |
| `docs/flows/dashboard.md` rewritten honestly | holds, and unusually well — the "what the pages deliberately do not show" section is the right shape. One gap (F7) added here |
| `dashboard-auth.md` Status | holds |
| `runbooks/demo.md` §6 | the steps are accurate, "twelve characters minimum; length is the only rule" included (`MIN_PASSWORD_CHARS = 12`, no composition rule). §6 was only *true* because the same commit added the dashboard to `demo_services`, which is what F5 and F10 are about |
| `docs/status.md` rows dated | holds, except the "GitHub Actions" row (F10) |
| `README.md` untouched | holds — the repository root README is not in this branch's diff |
| the screenshots | re-taken on the review head by the spec itself, and looked at. `04-payment-detail.png` was a *viewport* capture of a page taller than a viewport, so it stopped above the charge and the timeline — the sections a reviewer most needs. It is `fullPage` now |

## `just demo-staff` against a project of its own

Run as `just demo_project=exp28-review demo_port=18190 … demo-staff` against
the review stack: it created `ada@example.test` for `demo-merchant-tenant`
inside `exp28-review` and touched nothing in `vpay-demo`. The six overrides on
`test-e2e`'s sub-invocation are the fix for the defect the implementer's own
notes record, and they work.

It also has F9: with `$out` naming a directory it printed "created", exited 0,
wrote no password, and `chmod 0600`'d the directory — which in that layout is
one the compose stack bind-mounts. Guarded now, and the guard was measured in
both directions.

## Gates

### On the delivered head `f6d4ce8`

| recipe | exit | measured |
|---|---|---|
| `fmt-check` | 0 | |
| `clippy` | 0 | `--workspace --all-targets -D warnings` |
| `verify` | 0 | eleven gates; `verify-docs` advisory |
| `test-rust` | 0 | **1550 run, 1550 passed, 0 skipped** (999 s) |
| `test-doc` | 0 | |
| `verify-ignored` | 0 | 0 ignored (expected 0), 46 binaries (expected 46), 1550 total (floor 1080) |
| `lint-web` | 0 | |
| `test-web` | **1** | `@vpay/ui` `select.test.tsx`, 1 failed / 73 passed — **F11**, a flake: 5/5 green when that package's suite is run alone. Not this branch's file |
| `deny` | not reached | |

So `just ci` did **not** exit 0 on the delivered head in this environment, and
the implementer's table saying it did is not thereby contradicted — the same
suite passes five times in a row on its own. It is a flaky gate, which is a
different problem and belongs to whoever owns `select.test.tsx`.

`just test-e2e` was not run on the delivered head: the review's own fixes
landed before it was worth spending forty minutes on a head nobody would
merge.

### On the review head `2952fe2`

Run from nothing, in the review's own compose project on non-default ports.
`2952fe2` is the branch's **last code-bearing commit**; the commits after it
change only `docs/`.

| recipe | exit | measured |
|---|---|---|
| `fmt-check` | 0 | |
| `clippy` | 0 | `--workspace --all-targets -D warnings` |
| `verify` | 0 | eleven gates, `verify-ui` among them; `verify-docs` advisory |
| `test-rust` | 0 | **1551 run, 1551 passed, 0 skipped** (906 s) |
| `test-doc` | 0 | |
| `verify-ignored` | 0 | 0 ignored (expected 0), 46 binaries (expected 46), 1551 total (floor 1080) |
| `lint-web` | 0 | |
| `test-web` | 0 | **1244 across nine projects** — `@vpay/dashboard` **150 in 20 files**, `@vpay/ui` 74 (green this run; F11 did not recur), `@vpay/checkout` 507, `examples/shop` 102, `@vaam-apps/vpay-sdk` 190, `@vaam-apps/vpay-stripe-js` 146, `@vpay/config` 63, `@vpay/tokens` 8, `@vpay/api-client` 4 |
| `deny` | 0 | advisories, bans, licenses, sources all ok |
| **`just ci`** | **0** | |

`just test-e2e`, same head, from nothing in the same project: **exit 0 — 16
Cypress tests across four specs, 16 passing, 0 failing, 0 skipped.**
`checkout.cy.ts` 1, `dashboard.cy.ts` **8**, `shop-hosted.cy.ts` 3 in pass 1;
`shop-embedded.cy.ts` 4 in pass 2. Stack torn down with `down -v`; no
container and no volume left behind, and the user's `vpay-demo` was never
addressed.

The `staff_sign_in` suite specifically: **17 tests, 17 passed, 0 skipped.**

## Maintainer decisions

1. **The `select.test.tsx` flake (F11).**
   `expect(document.activeElement).toBe(english)` races Base UI's focus under
   parallel load. The fix is a `waitFor` around the same assertion, which is
   not a weakening — but the file belongs to exp26 lane D and a focus-timing
   test is easy to make vacuous by accident. Not touched here.
2. **An end-to-end paging test.** Nothing ties the dashboard's reading of
   `has_more` to `/dash/v1`'s meaning of it (see "The paging fix" above). The
   natural home is `dashboard_read_surface.rs`.
3. **Re-minting versus a longer token (F4's fix).** `readDash` re-mints on a
   `401`. The alternatives are a longer `ACCESS_TOKEN_TTL_SECS` or a shorter
   session; both weaken something the re-mint does not, but the choice of
   session model is the maintainer's and this review took the option that
   changes no security property.
4. **A transient vpay outage signs a staff member out.** `requireStaff` treats
   a failed `/staff/session` read as a refusal, and `api.ts` reports an
   unreachable vpay as `status: 0` — indistinguishable, at that call site,
   from a `401`. Unchanged from the delivered behaviour; naming it because the
   fix (branch on `status === 0` and render an outage) is a product decision.
5. **The payer mask**, unchanged from the exp24 review's decision 4: nothing
   writes `charges.payer_ref_masked`, and whether to derive it in the API or
   fix the write is still open. The dashboard's handling of the gap is correct
   either way.
6. **`change_password` requires no current password**, unchanged from exp24's
   decision 5 — and the page now exists, so a stolen session inside the
   thirty-minute idle window can change the password and lock the owner out.
   The form says why there is no current-password field; the decision is still
   the maintainer's.
7. **`X-Forwarded-Host` and the CSRF check.** Next's server-action origin check
   compares `origin` against `x-forwarded-host` when one is present. Behind a
   proxy that copies a client-supplied `X-Forwarded-Host`, both sides of that
   comparison are attacker-influenced. The default deployment is safe; a
   `serverActions.allowedOrigins` policy is a deployment decision.
8. **"Sign out" reads as bold text, not a control.** In the committed
   screenshots the `ghost` button wraps below the signed-in line and has no
   affordance at that width. A styling judgement on `@vpay/ui`'s `Button`
   rather than a defect; noted rather than changed.

## What this review did NOT check

* **A real browser, by hand.** Every attack above was `curl` and `psql`
  against the running app; the browser coverage is `dashboard.cy.ts`, which
  this review extended by one case and two assertions.
* **A genuinely expired JWT.** F4 was measured with a token `/dash/v1`
  refuses, which is what an expired one is from the app's side, rather than by
  waiting fifteen minutes.
* **Concurrency.** No two-request race on enrolment, sign-out or the code
  exchange; the repository-level CAS races are the implementer's and exp24's.
* **`color-contrast` and any real-browser accessibility.** jsdom computes no
  layout. Unchanged position since exp26.
* **The Helm chart**, which has no dashboard service, and `helm-check`, which
  is not in `just ci`.
* **Load.** The rate limiter's memory bound, argon2id on the async runtime,
  and the app under concurrent renders are all unmeasured. F3's fix
  deliberately counts a **wrong** second factor only, so a successful sign-in
  still spends one unit and a deployment behind a proxy — where every staff
  member shares one per-IP bucket (ADR-0017's Consequences) — is no worse off
  than before; the first version of that fix counted every attempt and would
  have halved it.
* **`frontends/packages/ui` itself** beyond the flake — this branch does not
  change it.
