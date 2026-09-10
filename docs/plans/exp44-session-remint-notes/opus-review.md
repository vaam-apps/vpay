# exp44 review — the proactive re-mint and the TOTP page, attacked

**Branch:** `claude/exp44-session-remint` · **Reviewed head as delivered:**
`b9de532` (four commits on `231f51e`) · **Rebased onto** `origin/master`
`44e0c80` (which carries #102's `ConfigError` variant and #104) — clean, no
conflicts · **Date:** 2026-09-10

The brief called this a security surface and said to break in. What follows is
what was attacked, what held, and the five things that did not.

## Verdict

**Safe as delivered: no — but not because anything it built is wrong.**

Every authorisation decision on the new surface is right, and I could not find
a way through it. The new unauthenticated route publishes one word and no
identity, it is refused for every session `load_session` refuses, and it
unlocks nothing: the two routes reachable by a `pending_totp` caller are the
code form and this one, and every other staff route calls
`authenticated_session`. The re-mint is the authorization-code leg, which is
strictly *more* checking than carrying a token, and it is refused for a
disabled account, a moved staff member, an idle session and a signed-out one —
each proven with a control. The CHECK pairing the token and its expiry fires
against a real Postgres. The migration clears tokens rather than inventing
expiries and signs nobody out.

What was not safe is that **two of the security properties the branch states
most emphatically were enforced by nothing**, and both are one line away from
being gone with the whole gate still green. Those are F1 and F2 below; both are
now measured. F3 is a real (small) defect in the demo's freshness check; F4 and
F5 are claims that had gone stale.

## Findings

| # | Severity | Finding |
|---|---|---|
| F1 | gate-hole | `staff_auth.access_token_ttl_seconds`' `10..=3600` bound was asserted by **nothing**. Measured: `#[garde(range(...))]` -> `#[garde(skip)]`, `cargo nextest run -p vpay-config` **112 passed, 0 failed**, and the whole Rust gate green as far as it ran |
| F2 | gate-hole | "`session_stage` does not touch `last_seen_at`" — stated in four places — was measured by **nothing**. Measured: one `touch` line added to the handler, `staff_sign_in` + `boot_coherence` + `dashboard_read_surface` **43 of 43 green** |
| F3 | correctness | `gen-demo-keys`' new TTL freshness check is a `grep -F` on a line ending in a bare number, so a value that is a *prefix* of the one on disk reads as fresh. Measured against the real overlay (`30`): `demo_staff_token_ttl=3` -> KEEP, i.e. a stack serving ten times the requested TTL while the recipe says it kept the file |
| F4 | misleading-claim | `docs/reference/vpay-api.md` was corrected in one place and not the other — line 522 still said "the seven staff routes" — while the branch's notes claim the file was corrected |
| F5 | misleading-claim | `dash-read.ts`'s header still attributed the dashboard token's 900 s to `vpay_api::op::ACCESS_TOKEN_TTL_SECS`, which stopped being the dashboard's TTL on this branch, and still described its own reactive retry as the only re-mint there is. `docs/flows/dashboard-auth.md` said "six `gateFor` cases"; counted, nine |

Each is fixed in its own commit with the measurement in the message.

### F1 — the TTL bound was decoration

`10..=3600` is stated in the field's doc comment, in the ADR-0017 amendment, in
`docs/flows/dashboard-auth.md` and in the `justfile`, and until this review no
test named the field at all — a workspace-wide grep for
`access_token_ttl_seconds` found it only in production code and in two
integration-test helpers that pass legal values.

Both ends carry weight. **The floor** is what stops the re-mint margin — a
fifth of the TTL — collapsing under two seconds, at which point every render
runs an authorization-code leg: a credential operation per page view, which
presents as latency and not as an error. **The ceiling** is the only thing
bounding how long a *signed-out* session's already-minted JWT stays
cryptographically valid, which ADR-0017's Consequences records as an open
residual. A deployment writing `access_token_ttl_seconds: 86400` with the
attribute gone would quietly widen that residual by a factor of ninety-six.

Fixed with two fixtures (9 and 3601, each stating its only defect), the
refusal asserted to be `ConfigError::Validation` **naming the field**, three
accepted values as controls so a rule that refused everything cannot pass, zero
in the refused list (it is what a `#[derive(Default)]` would hand the field —
the reason `StaffAuth`'s `Default` is hand-written), and a second case over
`config/application.yml`, which writes no `staff_auth` at all, so the *serde*
default is proven rather than only the `Default` impl.

### F2 — "it does not touch `last_seen_at`" was an unenforced comment

This is the property that keeps the new route from being a way to hold a
half-authenticated session open. A `pending_totp` session has proved a password
and no second factor; the route is unauthenticated and unlimited, exactly as
`/staff/session` is; and if it moved the idle bound, anything that could reach
it with a stolen session token could keep that session alive to the twelve-hour
absolute bound instead of thirty minutes. The handler is right. Nothing said so.

Fixed with `the_stage_route_does_not_move_the_idle_bound`, which ages the row
five minutes before each read — `StaffSessions::touch` filters on
`last_seen_at < now`, so a stamp inside the same instant writes nothing and a
test that did not age the row could not tell a deliberate no-op from an
accidental one — covers both stages, and ends with a `/staff/session` control
so the case cannot pass because touching broke everywhere.

## The attack table

| Attacked | Result |
|---|---|
| `GET /staff/session/stage` with a forged token | `401`, one sentence, same as every other refusal. Nothing distinguishes "no such session" from any other cause in the body |
| …as a **session-id oracle** | Infeasible, and the number is worth stating: session tokens are `TOKEN_BYTES = 32` from `OsRng` — **256 bits** — stored as their SHA-256, and looked up by primary key. There is a *timing* difference between "no row" (one query) and "row, then staff row, then active check" (three), but it is a distinguisher over a value nobody can reach by guessing |
| …does it leak identity | No. `SessionStageResponse` has one field, pinned by a unit test over the serialised key set **and** by an assertion over a booted server. Adding an `email`/`merchant_id` fails both |
| …does it move `last_seen_at` | No — **and now measured** (F2) |
| …is it rate limited | **No**, and neither is `GET /staff/session` or `POST /staff/logout`. Consistent with what shipped; the cost is one indexed primary-key lookup on a miss, against `/staff/login`'s argon2id, which is limited. Recorded rather than fixed |
| …what can a `password`-stage caller reach | `POST /staff/totp` (by design), `GET /staff/session/stage`, `POST /staff/logout` (which takes no session gate at all and deletes by digest). Everything else — `/staff/session`, `/staff/password`, `GET /oauth/authorize`, `POST /oauth/token` — goes through `authenticated_session`; read on all eight, and `a_session_that_has_not_presented_a_second_factor_cannot_authorize` proves the one that matters over a booted server |
| Wrong code keeps the cookie **and** the enrolment blob | Yes, in a real browser: `dashboard.cy.ts` leg 3a asserts the path, the alert, the surviving cookie and the surviving QR — the QR being the assertion that catches a page that stayed at the path but lost the panel |
| Replaying the sealed enrolment blob on another session | Refused where it matters: `totp_step` uses the **stored** secret for an enrolled account and ignores a caller-supplied blob, and `enrol_totp` is a compare-and-swap on `totp_enrolled_at IS NULL`. The blob is not bound to a session id, and does not need to be — it is `httpOnly` on the app's origin and only ever enrols the account the presented session names. Pre-existing, unchanged by this branch |
| Dead session -> `/login` | Yes, and it is the **only** branch that ends a session on that page. An outage keeps the cookie |
| Re-mint at 80 % before any `401` | Proven the only way it can be: `access_token_expires_at` read back from vpay, asserted to have **moved while the old one had not yet passed** |
| Re-mint for a **disabled** staff member | `401` at `/authorize`, with the same session restored afterwards as a control |
| Re-mint for a staff member **moved to another merchant** | `401`, same control |
| Re-mint for an idle or signed-out session | `401`, same control |
| vpay unreachable at 80 % | The render succeeds on the token in hand and **no cookie is cleared** — the `stale-token` arm's whole reason. A `401` is deliberately *not* fallen back on |
| Token without an expiry, or expiry without a token | Refused by `staff_sessions_token_expiry_is_paired` against a real Postgres, both directions, with both legal rows (neither, and both) inserted as controls |
| TTL bounds refuse 9 and 3601 at boot | **Now** (F1). Not before |
| Migration 0040 on a populated table | Tokens cleared, rows kept; the applied count moves 39 -> 40, the manifest line is present and its digest matches, the multi-column CHECK list moves to 18, and the drift count is unchanged |
| The eighth route listed where the seven were | `dash/mod.rs`, `docs/flows/dashboard-auth.md`, `docs/status.md`, `docs/reference/vpay-api.md` §442 — and **not** §522 until F4 |

## Mutations

| Mutation | Gate | Measured |
|---|---|---|
| `#[garde(range(min = 10, max = 3600))]` -> `#[garde(skip)]` | `-p vpay-config` | **PASSED 112/112** as delivered -> after F1, **FAILS** on `the_dashboard_token_ttl_is_refused_outside_its_bounds` |
| `session_stage` gains `touch(&session.id, now)` | `staff_sign_in`, `boot_coherence`, `dashboard_read_surface` | **PASSED 43/43** as delivered -> after F2, **FAILS** with the two instants five minutes apart |
| `grep -F` freshness check, `demo_staff_token_ttl=3` against an overlay written for 30 | `gen-demo-keys` | **KEEP** (wrong) as delivered -> after F3, **REGENERATE**; and 30 against 30 still **KEEP**, so no stack regenerates its secrets spuriously |
| `session_stage` calls `authenticated_session` | `staff_sign_in` | FAILS (the implementer's, re-read not re-run) |
| the OP's TTL hard-coded back to `ACCESS_TOKEN_TTL_SECS` | `staff_sign_in` | FAILS (the implementer's, re-read not re-run) |
| `staleTokenFor` returns `false` for an absent expiry | dashboard vitest | covered by `re-mints when vpay sent no expiry at all` |
| `totpGateFor` maps any failure to `'dead'` | dashboard vitest | covered by `keeps the cookie when vpay could not be reached` |
| the margin dropped (`marginMs = 0`) | `dashboard.cy.ts` | the implementer's, 8/9; not re-run here |

## What was NOT checked

* **The three browser mutations were not re-run.** `dashboard.cy.ts` was run as
  delivered; re-running each mutation is three more full stack builds.
* **Two counts in `postgres_smoke.rs`'s CHECK block are still stale** and were
  left alone: "Measured 2026-09-05: ten multi-column CHECKs" and "The
  eleventh" on `customers` (it is second in an eighteen-row list). Both read as
  history rather than as counts of the list beside them, and rewriting somebody
  else's dated measurement is worse than leaving it.
* **`/staff/password` at the `password` stage has no test of its own.** It
  calls `authenticated_session`, read directly, so the conclusion is from the
  source rather than from a case.
* **The re-mint storm is bounded by nothing.** A dashboard clock ahead of
  vpay's makes every render a re-mint. It fails in the safe direction and costs
  a signature and two writes per page view; recorded, not changed.

## Maintainer decisions, surfaced not taken

1. **The ADR-0017 residual is still open** — a minted token stays
   cryptographically valid for its TTL after sign-out. A configurable TTL
   narrows the window and does not close it; closing it means binding every
   `/dash/v1` read to a live session row. The exp24 review surfaced this and
   nobody has taken it.
2. **`deploy/helm` carries no `access_token_ttl_seconds`.** A chart deployment
   gets 900. The chart writes no vpay-server config document at all today, so
   this would be the first.
3. **`demo_staff_token_ttl := "30"` applies to `just demo`, not only to
   `just test-e2e`.** That is deliberate and documented in
   `docs/runbooks/demo.md`, and it means the walkthrough stack re-mints its
   token every twenty-four seconds. Worth a maintainer's eye because it is the
   stack a person clicks through.
