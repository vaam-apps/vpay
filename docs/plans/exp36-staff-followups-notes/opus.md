# exp36 — staff sign-in follow-ups (issue #79 items 1–3, #88 items 2 and 4)

**Date:** 2026-09-10 · **Branch:** `claude/exp36-staff-followups` · **Base:**
`d5a93df` (S5 / PR #93, migration `0037`) · **Commits:** three.

Five of the six items in the brief are built and gated. One is not, and one
gate could not be run at all. Both are stated here before anything else.

## What is NOT done

| Item | Why |
|---|---|
| **#88 item 1 — proactive token re-mint** | The *reactive* re-mint already exists (`server/dash-read.ts` runs the authorization-code leg once on a `401`, so a render at TTL+1 works today). Making it happen **before** the 900 s TTL needs a `staff_sessions.access_token_expires_at` column (a second migration), a field on `GET /staff/session`, a `gateFor` that treats a nearly-expired token as `needs-token`, and a configurable TTL so an e2e can use a short one. That is a second pass, and half of it would have been unprovable without the e2e run below |
| **#79 item 4 — the payer mask** | Out of scope by the brief: it touches the `charges` write S5 owns |
| **#88 item 3 — the pager's `has_more` Cypress case** | Not attempted; it needs the e2e run below |
| **#88 item 5 — the `@vpay/ui` select flake** | Not attempted |
| **`just test-e2e`** | **Could not be run.** See below |

### `just test-e2e` could not be run, and this is not a skip

The recipe's dashboard is on a **hard-coded** host port:
`compose.demo.yml:301` publishes `127.0.0.1:3000:3000` with `!override`, the
`test-e2e` recipe polls `http://localhost:3000/` as a literal, and
`compose.e2e.yml:263` registers the dashboard client's `redirect_uri` as
`http://localhost:3000/dash/v1/callback` — which is matched **byte for byte**
at `/authorize` and again at `/token`, so the port cannot be moved by an
overlay alone. Measured at the time of writing, the maintainer's own
`vpay-demo` stack was up and holding 8080, 3000, 3001, 3080, 8082 and 8083.

So the two available moves were to take the maintainer's ports (the brief
forbids it) or to parameterise the dashboard port in `compose.demo.yml` and
the `justfile` — which is the change **exp35 owns and is making concurrently**,
and which the brief forbids this branch from touching. Neither was taken.

What that leaves unverified: **no browser has exercised the current-password
field.** `dashboard.cy.ts`'s leg 4 is updated for it — including a wrong
current password refused in the browser — and it typechecks and lints, but it
has not run. Anyone landing this should run `just test-e2e` once exp35's port
work is in.

## What is done

### 1. `staff_auth.trusted_proxies` (#79 item 1)

`X-Forwarded-For` is read **exactly** when the transport peer is in the
configured list, and the client address is then the first untrusted hop
**from the right**. Empty by default, and empty means the peer — which is
what ADR-0017 shipped, so an unconfigured deployment is unchanged.

The design note worth keeping: taking the **leftmost** entry is what most
"get the real IP" snippets do, and it is not a smaller version of this
feature — it is the hole the feature exists to not open, because the leftmost
entry is whatever the caller wrote and a caller writes a new one per request.
Four fallbacks end the walk at the peer (untrusted peer, unparseable hop,
all-trusted header, absent header) and each is a hole if dropped.

`Forwarded` (RFC 7239) is deliberately not read: two parsers over one
caller-supplied string means the **more permissive** answer wins.

**A bug the tests caught while being written**, recorded because it is the
kind that ships: the absent-header branch was `let header = forwarded_for?;`,
which compiles, reads identically to the right thing, and returns `None`
*from the function* — so every trusted-proxy deployment would have counted
every request with no `X-Forwarded-For` under the limiter's one shared
unknown-address key. `an_all_trusted_header_falls_back_to_the_peer` found it.

### 2. `rate_limit_windows` — one budget for every replica (#79 item 2)

Migration `0038`, modelled in `schemas/vpay.cstack`. ADR-0017 had refused a
durable counter as "a denial-of-service amplifier of a different kind"; the
objection is **answered** by three properties of one statement rather than
overruled:

- the key is `SHA-256("<action>:<dimension>:<value>")`, so an unauthenticated
  caller cannot size a row, and the address they are guessing at — which for
  the interesting attempts has no account — is never written down;
- the same statement sweeps up to 32 elapsed rows `FOR UPDATE SKIP LOCKED`
  and never the row it is about, sixteen times the two an attempt adds;
- it is one `INSERT … ON CONFLICT DO UPDATE … RETURNING attempts`, so two
  replicas serialise on the row. A read-then-write would have kept the
  over-admission and added a race.

`model RateLimitWindow` is **the first model in the schema with a live table
and no `@@allow` arm**, and the absence is the point: nothing queries it
through the generated layer, because a `Update…Input` carries values and
`attempts = attempts + 1` is an expression. `docs/reference/vpay-db.md`
§ "Migration 0038" has the full account.

**Two judgement calls, both surfaced rather than taken:**

1. **The brief asked for "429 on the sixth wrong attempt", and ADR-0017's
   documented budget is ten.** Rather than silently change a security policy
   whose rationale is written down at length ("ten … so a person who mistypes
   a generated one-time password twice and then fetches it from a terminal is
   not locked out"), the limits became **configuration** with the shipped
   numbers as the defaults, and the decisive test configures five so the
   sixth attempt is the `429` the brief names. **Whether the shared default
   should now be below ten is a maintainer decision** — the budget is
   strictly stricter than it was, since it is no longer multiplied by the
   replica count — and it is recorded as open in `docs/status.md` rather than
   decided here.
2. **"Policy per action" is two actions**, `sign_in` and `change_password`.
   The second falls out of item 3: the current-password check is an argon2id
   verification an attacker holding a stolen cookie can drive, and it is
   keyed by the **session** rather than the email so that a thief cannot lock
   the owner out of their own login by guessing there. The two legs of a
   sign-in still share one budget, which is ADR-0017 decision 2 unchanged.

### 3. `POST /staff/password` (#79 item 3)

Requires the password in force; deletes every **other** session of that staff
member. A **breaking change** to that endpoint and to the dashboard's form.

The argument for the current password being absent was written down in three
places — the handler, the repository method and the React component — and it
missed one word: *when*. A session lives twelve hours and its two factors
were presented once, at its start, so the credential protecting an
irreversible takeover (the change also clears `password_change_required`) was
the session cookie alone.

The order of the four checks is deliberate and documented at the handler:
authenticated session, then the **new** password's bounds, then the limiter,
then the **current** password. (2) before (3) so that fumbling one's own new
password costs no budget; (3) before (4) because an attempt over budget must
not cost an argon2id verification.

### 5. Outage tolerance (#88 item 2)

`refusalFor` in `server/gate.ts` — a pure function, so "a `503` signs
everybody out" is a red test rather than something noticed during an
incident. `401` is the only status that ends a session, which is exact rather
than conservative because vpay answers `401` for *every* session refusal by
design.

`server/api.ts` turns a rejected `fetch` into an `ApiFailure` with
`status: 0` rather than throwing, which is why the old code could not tell
"vpay is restarting" from "your session is over": a rolling restart signed
every staff member out.

### 6. The CSRF origin check (#88 item 4)

Next's own check compares `Origin` against `x-forwarded-host` (falling back
to `Host`) — **two caller-supplied values, which agree whenever the caller
wants them to.** `server/csrf.ts` compares against
`VPAY_DASHBOARD_PUBLIC_ORIGIN`, and `originIsAllowed`'s **signature** is the
guard: there is no parameter a forwarding header could arrive through, so the
decisive mutation is a signature change.

**The variable is optional, and that is a compromise rather than a design.**
Making it required would have refused every action on every deployment not
yet reconfigured — including this repository's compose stacks, which exp35 is
editing. Unset, the check compares `Host` (never `X-Forwarded-Host`), which
is strictly stronger than Next's and **wrong behind a proxy that rewrites
`Host`**, where every action is refused with one sentence and a container-log
line naming the variable. **The follow-up is to set it in
`compose.demo.yml` and `deploy/helm` and make it required.** No compose file
and no helm value was touched by this branch.

## The mutations, each actually run

| Mutation | Result |
|---|---|
| Drop `proxies.contains(peer)` in `client_address` | `a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget` reads `401`, demands `429` |
| Give each `SignInLimiter` instance its own counters (the in-process limiter this replaces) | `two_replicas_share_one_sign_in_budget` reads `[401, 401, 401, 401, 401, 401]` |
| Delete the `verify_password` of `current_password` | `changing_a_password_…` reads `200` where it demands `401` |
| Delete the `delete_others` call | the other browser's next render reads `200` where it demands `401` |
| Widen `refusalFor` to `status >= 400` | four of five `gate.test.ts` cases red |
| Prefer the caller-supplied host over the configured origin (what Next does) | the forwarded-host case in `csrf.test.ts` red |

**One mutation was wrong and is recorded as such.** The first attempt at the
two-replica mutation salted the key with `std::process::id()` — and it
*passed*, because both "replicas" are two `axum::serve` tasks in one test
process. That is a fact about the harness, not about the code, and it is now
written into the test's own doc comment: what two OS processes would
additionally catch is a counter in a process-global `static`, of which there
is none.

## Gates

`just ci` on the final head of this branch, end to end, exit code read from a
file rather than from a harness banner. (No SHA here on purpose: the commit
that carries this document is the head, so a SHA written into it would name
its own parent and read as a claim about a tree one commit older.)

| Recipe | Result |
|---|---|
| `fmt-check` | clean (only the touched files were formatted, with `rustfmt` rather than `just fmt`) |
| `clippy` | clean |
| `verify` (12 gates) | all ok — `verify-migrations` 38 files, `check-schema` 26 declarations under cratestack **0.12.0** |
| `test-rust` | **1619 passed, 0 skipped, 0 ignored** |
| `test-doc` | 109 doctests passed |
| `verify-ignored` | 0 ignored (expected 0), 45 binaries (expected 45), 1619 total (min 1080) |
| `lint-web`, `test-web` | clean; dashboard **172** (from 151), checkout 507, shop 102 |
| `deny` | advisories ok, bans ok, licenses ok, sources ok |

Drift: **167 → 172** changes over **23 → 24** relations, unmappable columns
**unmoved at 19** — five lines, all of them a hand-named CHECK or an index,
and not one column-level line.

`just test-e2e`: **not run.** See the top of this document.
