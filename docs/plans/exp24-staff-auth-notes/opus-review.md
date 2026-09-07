# exp24 — staff sign-in: sabotage review

Reviewer's notes. Base `9edec7c`, reviewed head `f1aef74` (8 commits), review
head at the bottom. Written 2026-09-07.

The brief for this review was to break in. What follows is what was tried,
what happened, and what was changed.

## The short version

The implementation is unusually good and its own notes
([opus.md](opus.md)) are unusually honest — the mutation table records two
mutations as *not caught* and explains each, which is the opposite of the
failure mode this repository worries about. Eleven of the fourteen attacks
below found nothing.

**Three found something, and two of them are the same bug seen twice: a
security property that was implemented, documented, tested at the unit level,
and never actually wired to the request path.**

| # | Finding | Severity |
|---|---|---|
| F1 | A **disabled** staff member's already-minted bearer token kept reading `/dash/v1` for the rest of its 15-minute TTL. `require_dashboard_token` never read `staff_members` at all. | gate-hole |
| F2 | The **per-IP** half of the sign-in rate limit did not exist. `ConnectInfo` was never installed, so every attempt in the process shared one `ip:unknown` bucket — and ten unauthenticated requests locked the whole deployment out of the dashboard. | gate-hole |
| F6 | A staff member **moved to another merchant** kept reading the old merchant's rows with the token they already held, for the same window. Same root cause as F1, found by asking the same question again. | gate-hole |
| F3 | Mutation M17 (`Staff::create` → `upsert`), recorded as uncaught, was catchable at the repository layer. Now caught. | correctness |
| F4 | ADR-0017 said deleting a session row "is a **revocation**"; its own Consequences and the suite say the JWT stays valid for its TTL. Overstated. | misleading-claim |
| F5 | ADR-0017 decision 1 said `model Staff`; the model is `StaffMember`, which migration `0035`'s header explains at length. | nit |

All three gate-holes are fixed, with decisive regression tests. F3, F4 and F5
are fixed. One thing is **surfaced and deliberately not taken** — see Maintainer
decisions.

## `just ci` on the delivered head, recipe by recipe

Run end to end on `f1aef74`, `CARGO_BUILD_JOBS=4`, Node 22.23.2 (`.nvmrc`),
`pnpm install --frozen-lockfile` (the implementer skipped this; without it
`lint-web` dies on `tsc: not found`), rootless Docker.

| recipe | exit | measured |
|---|---|---|
| `fmt-check` | 0 | |
| `clippy` | 0 | |
| `verify` | 0 | ten gates; `verify-docs` advisory |
| `test-rust` | 0 | **1546 run, 1546 passed, 0 skipped** (964 s) |
| `test-doc` | 0 | |
| `verify-ignored` | 0 | 0 ignored (expected 0), **46 binaries (expected 46)**, 1546 total (floor 1080) |
| `lint-web` | 0 | |
| `test-web` | 0 | vitest across eight packages; `apps/dashboard` reports "No test files found" — consistent with "the pages were not built" |
| `deny` | 0 | advisories, bans, licenses, sources all ok |

Every number the implementer reported is confirmed. `verify-ignored` initially
exited 101 on my run; that was **my own scratch attack file** failing to
compile, and it passed on the clean tree.

## The attack table

Fourteen cases against a real `vpay_api::router` on a real socket over a real
Postgres, plus one in-process timing case.

| # | What was tried | What happened |
|---|---|---|
| A1 | Wrong password vs unknown address, 20 samples each over HTTP | **No oracle.** Medians 32 507 µs vs 30 553 µs, ratio **1.064** |
| A1b | The same, in-process, no socket and no limiter | **No oracle.** 20 991 µs vs 20 019 µs, ratio **1.049** |
| A2 | Disable the account, then use the bearer token it already holds | **BROKE IN — `200`** with the merchant's list envelope. **F1** |
| A3 | PKCE `plain`; `code_challenge_method` absent; `code_challenge` absent | Refused, three times, no code issued. The refusal is an error *redirect* to the registered URI, which is RFC 6749 §4.1.2 and correct |
| A4 | `state` echoed on the redirect | Echoed verbatim |
| A5 | Four unregistered `redirect_uri`s, including a traversal, a query-string variant and a case variant | `400` each, **no `Location` at all** — not an open redirector, per RFC 6749 §4.1.2.1 |
| A6 | Redeem a code with another `client_id`; `client_credentials` on `/dash/v1/oauth/token` | Both `400` |
| A7 | 13 attempts on one address; then a *correct* password on another address; then back | `429` from the 11th; a success elsewhere does **not** reset the counter |
| A8 | TOTP at offsets −2, −1, 0, +1, +2 | Exactly ±1. −2 and +2 refused |
| A9 | Password of length 0 and 10 KB at `/login`; the same at `/staff/password` | `401` in 34.7 ms and 29.7 ms — argon2's cost is its parameters, not the input, and the nest carries a 64 KiB `RequestBodyLimitLayer`. `/staff/password` bounds 12–256 chars |
| A10 | A code from the *previous* TOTP secret after the stored one was rotated | `401` |
| A11 | An authorization code aged past `expires_at` | `401` |
| A12 | Does the `302` or anything else leak the session token? | No. The `Location` carries `code` and `state` only |
| A13 | Burn one source address's budget, then a first-ever attempt from another source with a fresh email | **BROKE IN — `429`.** **F2** |
| A14 | Move the staff member to merchant B, then use the token minted for merchant A | **BROKE IN — `200`** with merchant A's intent in the body. **F6** |

## F1 — a disabled staff member kept reading `/dash/v1`

`staff_members.status` is the only per-person kill switch this deployment has.
ADR-0017 says so; `docs/flows/dashboard-auth.md` says "what can be disabled per
person is `staff_members.status`, which is the granularity that matters";
`docs/status.md` claimed "a disabled account refused on its **live** session".

All three were true of the session routes, where `load_session` re-reads the
staff row on every request. None of them was true of the credential that
actually reads a merchant's rows. `require_dashboard_token` checked the
signature, the audience, the merchant claim, the scope and the method — and
never touched `staff_members`.

The delivered test is the reason it survived. It reads:

```rust
let (session, _token) = harness.access_token().await?;
```

It obtains the bearer token and **discards it**, then disables the account and
checks `/dash/v1/staff/session`. The one credential under test is the one it
threw away.

Measured: with `status = 'disabled'`, `GET /dash/v1/payment_intents` with that
token answered `200` and the merchant's list envelope.

**Fixed** in `require_dashboard_token`: the `staff_members` row named by `sub`
is read, and a `disabled` or missing one is refused with the same `403` (telling
them apart would say which `stf_…` values name a row). Placed after the
audience, tenant and scope checks so an unauthenticated caller cannot make the
surface touch Postgres, and failing closed on a database error.
`disabling_a_staff_member_refuses_their_live_session` now asserts both
credentials before and after the `UPDATE`. Decisive: removing the `is_active()`
arm gives `left: 200, right: 403`.

## F6 — a staff member moved to another merchant kept reading the old one

Found by asking F1's question a second time: *everything*
`require_dashboard_token` checked was a statement about the **token**, and
none of it was a statement about the **person**. The merchant *claim* says
which tenant the token was minted for; nothing said which tenant the person
belongs to now.

`oauth_authorization_codes.merchant_id` is a copy of `staff.merchant_id` taken
at issue, and migration `0035`'s comment says why: "so that a staff row edited
between issue and exchange cannot silently move a token to another tenant."
That is real and it protects the tenant being moved **to**. Nothing protected
the tenant being moved **from**.

Measured: with `merchant_id` updated to merchant B, the token minted for
merchant A answered `200` with merchant A's payment intent in the body.

**Fixed** in the same read F1 added, at no extra cost: the row's `merchant_id`
must equal the binding. It refuses nobody who was ever allowed in, because
`staff::oauth::authorize` already requires that equality before it will mint a
code. `moving_a_staff_member_to_another_merchant_refuses_their_existing_token`
is the guard, and it asserts the refusal carries none of the tenant's data.
Decisive: dropping `&& staff.merchant_id == binding.merchant_id` gives
`left: 200, right: 403` with `pi_moved_a` in the body.

## F2 — the per-IP rate limit did not exist

`SignInLimiter::check` takes an `Option<IpAddr>` and counts a `None` under one
shared `ip:unknown` key rather than exempting it. That is the fail-closed
reading and the module argues for it well. What supplies the `Some` is axum's
`ConnectInfo` — and `ConnectInfo` is in request extensions **only** when the
service is built with `into_make_service_with_connect_info`.

```rust
// vpay-server/src/main.rs, as delivered
let serve_fut = axum::serve(listener, vpay_api::router(deps))
// tests/support/mod.rs, as delivered
let _ = axum::serve(listener, vpay_api::router(deps)).await;
```

Neither did. So `peer` was `None` on every request in the binary and in every
suite, and the entire limiter was one global ten-per-five-minute bucket.

Two consequences:

1. **The claim was false.** ADR-0017 decision 2, `docs/flows/dashboard-auth.md`,
   `docs/status.md` and the module header all said "per email **and** per IP".
2. **It was a trivial, unauthenticated, deployment-wide login DoS.** Ten
   requests from anywhere refused every staff sign-in for five minutes,
   repeatable forever. `rate_limit`'s own header rejects a lockout because it
   "is a denial of service an attacker triggers by guessing at somebody else's
   address" — this was that, at deployment scale rather than per account.

Every unit test in `rate_limit` passed throughout, and had to: they call
`check` directly and pass a `Some` the router never supplied. **A test that
constructs the thing under test cannot see a wiring bug**, which is why the
new guard is end to end.

**Fixed** at both call sites. The harness is fixed as well as the binary, and
that is the point of fixing both — `tests/support`'s own header says a harness
that boots the router differently from `main` stops proving anything about
`main`. `the_sign_in_rate_limit_is_per_source_address` burns one loopback
source's budget across ten distinct addresses (so the *email* budget can never
be what refuses the control), then makes a first-ever attempt from a second
source, then an eleventh from the first so a limiter that had simply stopped
working cannot pass either. Decisive: reverting the harness gives
`left: 429, right: 401`.

**Documented residual, new:** the address is the transport peer. vpay reads no
`X-Forwarded-For` and no `Forwarded` — both are caller-supplied on an
unauthenticated route, and honouring either without an authenticated
trusted-proxy list hands an attacker a fresh bucket per request. Behind an
Ingress the peer is the Ingress, so the per-IP budget bounds the deployment
rather than the caller. The per-email budget is unaffected.

## The two uncaught mutations, re-examined

**M14 (`verify_absent_account` → a no-op) is genuinely unobservable except by
timing, and the timing is equal.** The brief asked for this to be measured
rather than argued, and it was: ratio **1.064** over 20 samples per arm through
the real endpoint, **1.049** over 20 samples per arm in-process with no socket
and no rate limiter between the two. A 50 ms argon2id verification is what
either arm costs; the difference is noise. The implementer's reasoning —
that a test asserting a timing difference would be flaky by nature, and that
the real guard is `the_absent_account_hash_parses_and_never_matches` — stands,
and is now backed by a measurement instead of an argument. Left uncaught,
deliberately, and I agree.

**M17 (`Staff::create` → `upsert`) was catchable, and is now caught.** The
implementer got the analysis exactly right — a second `staff add` for one
address conflicts on `staff_members_email_key` whichever builder is used, so
the duplicate-address test cannot see the difference — corrected the doc
comment, and stopped there. But the doc then names the case the builder choice
*does* refuse: a caller supplying an id already in the table. That case is
expressible at the repository layer, and nothing asserted it.
`a_second_create_for_one_staff_id_is_refused_rather_than_overwriting` writes
twice with one id and two different addresses, so the email index cannot be
what refuses the second write. It asserts the `Conflict` classification and —
the half `expect_err` alone would not prove — that the first row still carries
its own address and password hash. Decisive: `.create(...)` → `.upsert(...)`
fails it.

Nothing supplies its own id today. That is why this is worth pinning rather
than why it is not: the day something does (an import, a restore), the
silent-overwrite failure is one staff member's password hash and second factor
replaced by another account's.

## What the F1/F6 fix cost the exp23 suite, and why that is the finding again

Adding the staff-row read broke **eight of the fifteen** tests in
`dashboard_read_surface.rs`, all with `403` where they expected `200`. The
cause is worth writing down because it is F1 restated:

`dashboard_read_surface.rs` mints its own tokens — that is the whole point of
that suite, which exists to test the resource server rather than the grant —
with `sub = "stf_0000000000000000000dash"`, a staff member **nobody ever
created**. Fifteen tests presented a credential for a person who did not
exist, and nothing noticed, because until this review nothing on that path
read `staff_members`. The suite's own header says its tokens are "exactly what
`vpay_api::staff::oauth::token` mints", and they were not: the shipping mint's
`sub` is `oauth_authorization_codes.staff_id`, a column with a foreign key to
that table.

Fixed by seeding the row rather than by relaxing the check — the suite now
presents what the grant can actually produce. And the case it used to
represent by accident now has a test of its own,
`a_token_whose_subject_names_no_staff_member_is_refused`, which only this
suite can express (`staff_sign_in.rs` mints nothing, so its staff row exists
by construction). It is what a **deleted** account's credential looks like.
Decisive: adding an `Ok(None) => {}` arm makes it `200`.

## Storage and secrets

- **The pepper and the TOTP key are never logged.** Every `tracing` call on the
  staff path was read, not grepped: fifteen of them, carrying `staff_id`,
  `error` and nothing else. No email, no session token, no code, no secret.
  `StaffCredentials`' `Debug` prints byte lengths; `vpay_config::StaffAuth`'s
  prints `[redacted]`; `Totp`'s prints `Totp([redacted])` and is hand-written
  precisely so a future `#[derive(Debug)]` is visible.
- **The request span records `path`, not the URI**, so the `302`'s `code`
  cannot reach a log through it. (The code is in a `Location` **response**
  header, which a reverse proxy's access log may record — an operator
  consideration rather than a vpay defect, and the code is single-use,
  PKCE-bound and 60 s.)
- **The one-time password appears once on stdout and nowhere else**, with a
  subcommand's tracing output redirected to stderr. The implementer found this
  the hard way and the test asserts stdout is exactly one line.
- **Migration `0035` on a populated database**, run by hand for this review:
  0001–0034 applied to a scratch Postgres, a `payment_intents` row inserted,
  then `0035` applied — exit 0, seventeen statements, the existing row intact.
  It is create-only and touches no existing relation.
- **Every constraint fires**, proven against that same database rather than
  inferred: `email_is_lower_case`, `status_is_known`, `totp_is_paired`,
  `totp_step_is_not_negative`, `staff_members_email_key`,
  `staff_sessions_state_is_known`, `staff_sessions_id_length` and
  `oauth_authorization_codes_method_is_s256` each rejected a row built to
  violate exactly it.
- **`_sqlx_migrations` count 35** — asserted by `postgres_smoke`, and it passed.
- **Drift 113 → 130 / 17 → 20 / 18 → 18**, re-derived against a freshly
  migrated database by `postgres_smoke`'s exact `assert_eq!` (not a floor), and
  it passed. The +17 is accounted line by line in the constant's own doc and
  the accounting checks out: 7 single-column CHECKs + 2 indexes on
  `staff_members`, 2 + 2 on `staff_sessions`, 2 + 2 on
  `oauth_authorization_codes`. `totp_is_paired` is multi-column and contributes
  nothing, which is why it is asserted against a real Postgres instead.
- **`@@allow` arms**: all three new modules carry
  `every_action_this_module_calls_has_an_allow_arm`, the S2a-style test, and it
  runs with no container.

## Deviation 3 — the app follows the `302` server-side

Nothing here exposes a code or a token where it should not be. The session
token travels in a request header, never a URL; the request span records
`path` and not the query; the `Location` carries `code` and `state` only; there
is no browser navigation, so no `Referer`. A13 confirmed the session token is
not in the redirect URL.

The residual worth an operator's attention is that an authorization code
appears in a `302` `Location` header, which reverse-proxy access logs commonly
record. It is single-use, PKCE-bound and lives 60 seconds, and the alternative
shapes are worse; recorded here rather than treated as a defect.

## Docs

ADR-0017 is `Accepted`, dated, and honest about what was not built.
`dashboard-auth.md`'s Status section, `dashboard.md` and the `status.md` rows
all state plainly that there are **no pages**, and `test-web` reporting "No
test files found" for `apps/dashboard` agrees with them. The
`client_credentials` tightening is written down as a behaviour change rather
than slipped in.

Two things were wrong and are corrected:

- **F4.** ADR-0017 decision 2 said deleting the session row "is what makes
  deleting the row a **revocation**". Its own Consequences, the flow document
  and `signing_out_deletes_the_session_and_with_it_the_access_token` all say
  the JWT stays valid for its TTL. It now reads as an *unobtainability* and
  points at the Consequences.
- **F5.** Decision 1 said `model Staff`. The model is `StaffMember`, and the
  distinction is load-bearing — `model Staff` reads and writes a table called
  `staffs`, which is the failure migration `0035`'s header spends a page on.

## Maintainer decisions

1. **Should a `/dash/v1` read be bound to a live `staff_sessions` row?** That
   would make signing out *invalidate* the token rather than merely make it
   unobtainable, and would close the residual ADR-0017's Consequences accepts.
   It also makes the surface stateful — a session read on every dashboard
   request. ADR-0017 accepted the residual explicitly, so reversing it is not a
   review's call. **Surfaced, not taken.** (F1's fix reads the *staff* row,
   which the ADR's own text already required; it does not read the session.)
2. **A trusted-proxy allow-list for the sign-in rate limit.** With F2 fixed,
   the per-IP budget is per *transport peer*, so behind an Ingress every staff
   member shares one bucket. Honouring `X-Forwarded-For` needs a list of proxies
   whose header may be believed; without one it is worse than not honouring it.
3. **A durable rate limiter.** Still per replica, still stated. Unchanged by
   this review.
4. **The payer mask.** `charges.payer_ref_masked` is never written, and the
   implementer's brief asked "derive it in the API from `payer_ref`, or fix the
   write — say which". Neither was done and the question is still open. It is a
   decision about what a staff reader may see of a payer's phone number.
5. **`change_password` requires no current password**, only an authenticated
   session. A stolen session inside its 30-minute idle window can therefore
   change the password and lock the owner out, and no other session is
   invalidated by the change. Both are conventional to require and neither was
   specified. Not changed here.

## What this review did NOT check

- **The pages.** There are none, so there was nothing to attack in a browser.
  `just test-e2e` was not run, and `dashboard.cy.ts` still asserts the scaffold
  notice, which is still what the app renders.
- **Concurrency at the HTTP layer.** The three compare-and-swaps are tested for
  races at the repository layer by the implementer's own
  `the_staff_guards_are_compare_and_swaps_and_only_one_caller_wins`; I did not
  add a concurrent-request attack on top of it.
- **The `compose.e2e.yml` stack and the Helm chart.** No compose service or
  chart value changed, and `helm-check` is not part of `just ci`.
- **Cryptographic review of the argon2/AES/HMAC crates themselves.** Parameters,
  modes, nonce handling and constant-time comparisons were read and are right;
  the implementations are upstream's.
- **A load test.** argon2id at 19 MiB runs on the async runtime rather than a
  blocking pool; at the rate limit's ten-per-five-minutes-per-key this is not
  reachable as a DoS, and it is not otherwise measured.

## Verdict

**Safe as delivered: no.** F2 was exploitable by any unauthenticated caller
on the internet; F1 and F6 were exploitable by an insider an operator had just
revoked. All three were claimed as working in ADR-0017, in the flow document
and in `docs/status.md`.

**Safe as reviewed: yes**, for the surface that exists — with the residuals
above written down where an operator will find them, and decision 1 left where
it belongs.

The pattern worth carrying forward: **every gate-hole here was a property that
was correctly implemented and never connected to the request path**, and in
each case a passing test is what made it invisible. The limiter's unit tests
passed a peer address the router never supplied; the disabled-account test
threw away the credential it was meant to check. Neither was a bad test of the
thing it tested — which is the point. A test that constructs the thing under
test cannot see a wiring bug, and the three cheapest questions that find this
class are: *what supplies this argument in production?*, *which credential does
this test actually exercise?*, and *is this a statement about the token or
about the person?*

## Gate on the review head

See the bottom of this file's commit for `just ci`, recipe by recipe, on the
final head.
