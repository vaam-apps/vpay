# exp24 — staff sign-in, implementation notes

Branch `claude/exp24-staff-auth`, base `9edec7c`. Written 2026-09-07.

Working notes: what the brief asked for, what was built, what was measured
rather than reasoned out, and — the part that matters most — what was **not**
built.

## The sentence

**A staff member can sign in to `/dash/v1`.** That has never been true in this
repository before, and every other paragraph here is subordinate to it.

`docs/flows/dashboard-auth.md` opened its Status section with "No login has
ever been performed" from the day it was written. `dashboard_read_surface.rs`
opens its own header with "it does not prove that a staff member can sign in,
because nobody can". `backends/tests/integration/tests/staff_sign_in.rs` is
thirteen cases against a real `vpay_api::router` on a real socket over a real
Postgres, and it **mints no token at all**: every token it presents came out
of `POST /dash/v1/oauth/token` after a password, a TOTP code and a PKCE
exchange, and the server verified it through its own published JWKS over the
same socket.

## What was built

Three commits, in the order the layers stack.

1. **ADR-0017, migration `0035`, three `.cstack` models, `vpay-db::{staff,
staff_sessions, authorization_codes}`, and `vpay_api::staff_auth`** — the
   decisions, the schema, the persistence and the cryptography. No route
   served any of it.
2. **`vpay_api::staff` and `vpay_api::op::dashboard`** — seven unauthenticated
   routes, the authorization-code grant, `vpay-server staff add`, and the
   audience change that `require_dashboard_token` had been waiting for.
3. **`staff_sign_in.rs`** — the proof, plus four subprocess cases for the CLI.

## What was measured rather than reasoned out

These are the four things this pass learned from a failing run, not from
reading. Each is written into the source where it is paid.

### 1. The table name is decided by the model name, and no gate says so

CrateStack 0.11.1 derives a table name with
`cratestack_core::route_naming::pluralize(to_snake_case(model))` and has no
`@@map`. `model Staff` therefore reads and writes a table called **`staffs`**.

The first draft of migration `0035` created `staff`. `cargo build` passed,
`just check-schema` said `schema OK`, `just clippy` passed and all ten
`just verify` gates passed. The first thing to say anything was a
container-backed test:

```text
Error: creating the suite's staff member
Caused by: database error: database: relation "staffs" does not exist
```

The model is `StaffMember` and the table is `staff_members`. The Rust trait is
still `vpay_db::Staff`, because it is a trait about staff and not about a
table.

### 2. `.eq(None)` is not "is null", and that is a compile error

`FieldRef::eq` takes `V: IntoSqlValue` and `Option<T>` does not implement it.
The method is `is_null()` (`cratestack-sql-0.11.1/src/filter/field_ref.rs:163`).

This is recorded because the _first_ design worked around a limit that does
not exist: `oauth_authorization_codes` was going to carry a `consumed BOOLEAN`
beside `consumed_at` so the compare-and-swap could filter on `.eq(false)`, and
the schema comment said in so many words that "is null" was not a predicate
0.11.1 offers. It is. The boolean is gone, the cross-column CHECK that would
have kept the pair in step is gone with it, and the comment now says the true
thing.

### 3. `NULL < step` is NULL, so the replay guard needs a seeded column

`Staff::record_totp_step` is a compare-and-swap:
`update_many().where_(id).where_(last_totp_step().lt(step))`. With
`last_totp_step` nullable, the **first** code a staff member ever presents
matches zero rows and is refused — forever, because nothing else writes the
column. Migration `0035` declares it `NOT NULL` and `Staff::create` seeds it
to 0.

### 4. The tracing subscriber writes to stdout, and so did the password

`vpay-server staff add` printed the one-time password with `println!`, which
is right; `tracing_subscriber::fmt()` defaults to stdout too, which meant the
password arrived as the **fifth line of a JSON log**, and `> password.txt`
would have written the log to the file and left the password on the terminal.

Found by `staff_add_creates_a_staff_member_and_prints_a_one_time_password_on_stdout`,
which asserts stdout is exactly one line. A subcommand's logs now go to
**stderr**; the server's stay on stdout, where a container log collector reads
them. The split is by _what this invocation is_, not by log level.

## Three security properties are compare-and-swaps in SQL

Not checks in Rust, because in each of the three the thing being prevented is
a race:

| Property                      | Guard                                        | What a read-then-write would lose                           |
| ----------------------------- | -------------------------------------------- | ----------------------------------------------------------- |
| TOTP replay                   | `record_totp_step`'s `last_totp_step < step` | two concurrent presentations of one code both win           |
| Authorization code single use | `consume_code`'s `consumed_at IS NULL`       | the TOCTOU `AuthorizationCodeStore`'s own doc names         |
| Enrolment happens once        | `enrol_totp`'s `totp_enrolled_at IS NULL`    | a second-factor reset with no authentication in front of it |

`model StaffMember`'s `@@allow("update", …)` is therefore the single most
dangerous line in `schemas/vpay.cstack`: `update_many`'s policy is compiled
into the statement's own `WHERE`, so an empty allow list renders `FALSE`, the
statement matches zero rows and every one of the three answers `Ok(false)`
**with no error anywhere**. All three fail closed in that state — nobody can
sign in — and the danger is the fix somebody reaches for when every sign-in
starts failing. `every_action_this_module_calls_has_an_allow_arm` in each of
the three modules names the slot rather than the symptom, and runs in
milliseconds with no container.

## The audience change, and what it cost

exp23's review recorded finding F7: `require_dashboard_token` compared the
token's `sub` to the registered dashboard client id, which is right under
`client_credentials` and **wrong for every token a real login produces**,
where `sub` is the staff member and the client id is the audience. It was left
as a maintainer decision.

ADR-0017 takes it, and takes it in the direction that changes the _validator_
rather than the grant: the audience is the registered
`dashboard_client.client_id`, because that is what
`default_handle_authorization_code` mints and it has no requested-audience
path at all. Consequences:

- `Surface::audience()` is gone; `JwtValidator::new` takes the audience as a
  value, because it is configuration now and not a constant.
- `vpay_config::DASHBOARD_AUDIENCE` is retired and replaced by
  `DASHBOARD_MERCHANT_CLAIM` — a different constant answering a different
  question ("which tenant", not "which surface").
- `ResourceClaims::client_id` is renamed to `subject`. A field named
  `client_id` was only ever right for one of the two grants, and the rename is
  what stops it being read as a client id again.
- `MerchantClaimsDashboardAudience` moves to whole-document scope, because
  its forbidden value is no longer a constant one registration could compare
  itself against.
- **A `client_credentials` token is now refused on `/dash/v1`.** That is a
  tightening over a surface that already worked, and it is the one behaviour
  change this pass makes to something that was not broken. It is deliberate:
  `/dash/v1` is a staff surface, and the refusal is a property of the mint
  (nothing but this grant stamps the merchant claim) rather than of a list.

## Design decisions a reader will want argued

**The dashboard app follows the `/authorize` redirect itself.** ADR-0017
decision 4. In a browser-driven flow the user agent follows it and a callback
_page_ completes the exchange; here the app's own server does, and a browser
never sees a code, a verifier or a token. Everything the grant checks is
unchanged — exact redirect-URI match, mandatory PKCE, single-use code — and
what changes is only who follows the `302`. The alternative was for vpay to
set the session cookie on its own origin so a top-level navigation would carry
it, which needs a credentialed CORS allow-list for the login POST and puts a
second cookie domain in play.

**The enrolment secret travels back to the caller, sealed.** A secret written
to `staff_members` before the person has proved they can generate a code from
it locks out anybody whose scan failed — permanently, because `enrol_totp`'s
guard is `totp_enrolled_at IS NULL`. It cannot live in the session row either,
which has no column for it, and giving it one would mean a table of
unconfirmed secrets. So it goes back sealed under the deployment key: the
caller learns nothing from holding it, and replaying it buys nothing because
`enrol_totp` is a compare-and-swap.

**The `Identity` stored with a code is not the `Identity` in the token.**
`authkestra_engine::token::Claims` serialises `identity` **whole**, so an
attribute left on would publish a session digest inside a bearer token.
`identity_for` carries the session and the tenant (the only fields
`handle_authorize` passes through to the store); `token_identity_for` carries
the staff id and nothing else — no email, no display name.

**`/token` is vpay's own handler.** `authkestra-op` offers exactly the seam
this needs (`OpStore::handle_authorization_code_grant`, whose own doc says it
exists because "`issue_user_token_with_extra` exists precisely for this, but
the built-in handler had no way to reach it"), and taking it would still mean
re-writing every check the default performs, because the default's last step
_is_ the mint. So the checks are written in `vpay_api::staff::oauth::token`,
in the same order, each with its own test — and the PKCE check is pinned
against RFC 7636 Appendix B's own vector rather than against a second function
in the same file.

`/authorize` is **not** rewritten: `handle_authorize` does the client lookup,
the exact redirect-URI match, the per-scope check, the `response_type` check,
the unconditional PKCE requirement and the error-redirect encoding, and every
one of those is a check vpay would otherwise own a second copy of.

## Mutations

Every one applied to a **clean** tree by a harness that asserts the branch
name and refuses a dirty tree, run, and reverted. `+` = caught, `-` = not.

| #   | Mutation                                                       | Result                                                                                                                        |
| --- | -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| M1  | delete the PKCE verifier check at `/token`                     | + `a_pkce_verifier_mismatch_is_refused`                                                                                       |
| M2  | delete `last_totp_step < step` from `record_totp_step`         | + `a_replayed_totp_code_is_refused`                                                                                           |
| M2b | widen the same filter from `lt` to `lte`                       | + `the_staff_guards_are_compare_and_swaps_and_only_one_caller_wins`                                                           |
| M3  | delete the merchant-claim check from `require_dashboard_token` | + 2 fail: `a_token_whose_merchant_claim_is_not_the_binding_is_refused` and `a_client_credentials_token_is_refused_on_dash_v1` |
| M4  | delete `consumed_at IS NULL` from the code's compare-and-swap  | + 2 fail: `an_authorization_code_cannot_be_exchanged_twice`, `a_code_redeemed_against_another_redirect_uri_is_refused`        |
| M5  | delete `totp_enrolled_at IS NULL` from `enrol_totp`            | **-** then **+** — see below                                                                                                  |
| M6  | `model StaffMember` loses `@@allow("create", …)`               | + `every_action_this_module_calls_has_an_allow_arm`, in 4 ms with no container                                                |
| M7  | `/authorize` accepts a password-only session                   | **-** then **+** — see below                                                                                                  |
| M8  | `/authorize` accepts a staff member of another merchant        | + `a_staff_member_of_another_merchant_cannot_obtain_a_dashboard_token`                                                        |
| M9  | drop the **absolute** session bound                            | + `an_idle_session_and_an_expired_one_are_both_refused`                                                                       |
| M10 | sign-out stops deleting the session row                        | + `signing_out_deletes_the_session_and_with_it_the_access_token`                                                              |
| M11 | drop the **idle** session bound                                | + the same test — both halves are separately caught                                                                           |
| M12 | delete the printed-password gate from `/authorize`             | + `the_printed_password_cannot_reach_dash_v1`                                                                                 |
| M13 | delete the disabled-account check from `load_session`          | + `disabling_a_staff_member_refuses_their_live_session`                                                                       |
| M14 | `verify_absent_account` replaced by a bare `false`             | **-** deliberately, see below                                                                                                 |
| M15 | the rate limiter is consulted and ignored                      | + `vpay-api` unit suite                                                                                                       |
| M16 | `find_by_email` made case-insensitive                          | + `a_staff_address_is_unique_and_looked_up_exactly`                                                                           |
| M17 | `Staff::create` becomes an `upsert`                            | **-** and the _doc_ was wrong, not the test — see below                                                                       |

### M7, and the test that was not decisive

`a_session_that_has_not_presented_a_second_factor_cannot_authorize` **passed
under its own mutation**. It signed in from scratch, so the session it built
was `pending_totp` _and_ belonged to a staff member who had never replaced the
printed one-time password — and `/authorize` refuses that too. With
`authenticated_session` swapped for `load_session`, the test went on passing,
refusing for the second reason while the first was gone.

Fixed: it now signs in fully first (clearing `password_change_required`),
starts a second login, stops after the password, and ends with a control — the
same session, once it has presented a code, gets its `302`. **Caught after the
fix.**

### M5, and a guard no HTTP test can reach

Deleting `enrol_totp`'s guard changed nothing observable at the HTTP layer,
and the reason is structural rather than a missing case:
`vpay_api::staff::totp_step` computes `enrolling` from the staff row it has
just read, so a _sequential_ second caller never reaches `enrol_totp` at all —
by then the row says enrolled and the handler takes the stored-secret path.
The second sign-in was refused, by a secret mismatch.

A test written at the HTTP layer to close it (two logins, the second
presenting a code from its own secret one step ahead) was written, and it
**still** did not catch the mutation, for the same reason. It is kept anyway,
because it pins the observable property.

The guard closes a real TOCTOU that only the repository layer can express: two
requests both read the row as unenrolled _before_ either writes, both compute
"I am enrolling", both call the method, and the swap makes exactly one win.
`the_staff_guards_are_compare_and_swaps_and_only_one_caller_wins` in
`vpay-db/tests/repositories.rs` is where that lives. **Caught after the fix.**

### M14, not caught, and deliberately

Replacing `verify_absent_account(&password)?` with a no-op leaves every test
green, and it must: the two answers are **identical by construction** — same
status, same body — and `a_wrong_password_and_an_unknown_address_are_the_same_refusal`
asserts exactly that. What the mutation removes is the _timing_ equality, and
a test that measured argon2id wall-clock would be flaky by nature.

The partial guard is
`staff_auth::password::tests::the_absent_account_hash_parses_and_never_matches`,
which asserts the dummy hash **parses** — because an unparseable one makes
`verify_password` return early and restores the very difference it exists to
remove. It is recorded here rather than smoothed over, in the shape exp23's
own M6 is.

### M17, where the _documentation_ was wrong

Swapping `Staff::create` for an `upsert` left every test green, and the test
was right. `staff add` mints a fresh `stf_…` every time, so a second account
for one address conflicts on the **email index**, not on the primary key — and
that index refuses it whichever builder is used. The doc comment claimed
`create`-not-`upsert` was what made the duplicate-address case fail. It is
not. Both doc comments now say what the builder choice actually guards (a
caller supplying an id already in the table, which nothing does today) and
name the index as what refuses the case that matters.

## Gate

`just ci` run end to end on `1a29ec7` (the head before this file's own
commit), `CARGO_BUILD_JOBS=4`, Node 22.23.2 (`.nvmrc`), `pnpm install
--frozen-lockfile`, rootless Docker.

| recipe           | exit | note                                                                                                                                             |
| ---------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `fmt-check`      | 0    |                                                                                                                                                  |
| `clippy`         | 0    |                                                                                                                                                  |
| `verify`         | 0    | ten gates; `verify-docs` advisory. `verify-status` still 1 unimplemented item, all declared. `verify-links`: 894 links over 159 files            |
| `test-rust`      | 0    | **1546 run, 1546 passed, 0 skipped, 0 ignored** (1106 s)                                                                                         |
| `test-doc`       | 0    |                                                                                                                                                  |
| `verify-ignored` | 0    | 0 ignored (expected 0), **46 binaries (expected 46)**, 1546 total (floor 1080)                                                                   |
| `lint-web`       | 0    |                                                                                                                                                  |
| `test-web`       | 0    | 807 vitest cases across eight packages, unchanged                                                                                                |
| `deny`           | 0    | advisories, bans, licenses, sources all ok — `sha1` 0.10 beside the graph's existing 0.11 is a `multiple-versions` **warn**, which is the policy |

The numbers that moved:

| Constant                      | Before | After | Why                                                                                    |
| ----------------------------- | ------ | ----- | -------------------------------------------------------------------------------------- |
| `expected_suites`             | 45     | 46    | `staff_sign_in.rs` is a new binary                                                     |
| test count                    | 1421   | 1546  | +125, none ignored                                                                     |
| migration count               | 34     | 35    | `0035`                                                                                 |
| `EXPECTED_DRIFT_CHANGES`      | 113    | 130   | 17 lines over three new relations, every one a hand-named CHECK or an undeclared index |
| `EXPECTED_DRIFTED_RELATIONS`  | 17     | 20    | the three new tables                                                                   |
| `EXPECTED_UNMAPPABLE_COLUMNS` | 18     | 18    | **unchanged** — no `bytea`, which is the point                                         |

## What was NOT built

- **The pages.** `frontends/apps/dashboard` is byte-identical to the base
  commit: still the scaffold, still saying so on screen, still zero tests, and
  `dashboard.cy.ts` still asserts the scaffold notice. `/login`, the OIDC
  callback, `/payments`, `/payments/{id}` and sign-out were in ADR-0017's own
  decision 4 and none was built. The brief anticipated this and said to
  deliver the grant end to end first and report the pages as not done rather
  than stubbing them; that is what happened. **Everything above is reachable
  over HTTP and by nothing a person can click.**
- **Screenshots.** There is nothing to screenshot. This directory contains no
  images, deliberately.
- **`dashboard.cy.ts`.** Unchanged. It still asserts the scaffold notice,
  which is still what the app renders. `just test-e2e` was not run.
- **vitest for the app.** Nothing to test: session cookie handling, masking
  and cursors are all frontend code that was not written.
- **The payer mask.** `charges.payer_ref_masked` is still never written, and
  the brief's "mask in the API by deriving from `payer_ref`, or fix the write;
  say which" was **not answered**. Neither was done. It is a decision about
  what a staff reader may see of a payer's phone number, and it belongs with
  the pages that would render it.
- **A sweep** of expired `staff_sessions` or `oauth_authorization_codes`.
  Expired rows are refused on read and removed by the sign-out cascade; the
  indexes a sweep would need exist and the sweep does not.
- **An `audit_log`.** ADR-0008 wants one row per dashboard _write_ and this
  surface mounts none.
- **A kill switch for the dashboard client.** `disabled_clients` revokes a
  _merchant_ credential; the dashboard registration can only be removed from
  YAML and the process restarted. What can be disabled per person is
  `staff_members.status`.
- **Key rotation.** ADR-0009's fourth blocker, untouched.
- **The `compose.e2e.yml` stack.** No compose service was changed, and no
  staff member exists in any compose deployment.
