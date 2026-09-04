<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. Everything in §6 and §7 below is written so it can be applied verbatim. -->

# Step 9, lane 1b — the integration seams, and the review's server-side findings

Branch `claude/step9-lane-1b-integration`, on top of `74a2be8` (the gate with
lanes 5, 2, 3, 2b and 1 merged). Six commits, each its own subject and each
with a measured guard-failure proof:

| # | Commit | Subject |
|---|---|---|
| 1 | `25734d6` | wire the return trip to the checkout session |
| 2 | `04e98a7` | expire checkout sessions past their horizon |
| 3 | `0b7e86f` | refuse a checkout origin a browser would not recognise (F5) |
| 4 | `8ab0201` | render the merchant's display name on both browser reads (F2) |
| 5 | `a0818e0` | a confirm under an open checkout session needs no `return_url` (F3) |
| 6 | `125be53` | the browser reads end at the horizon, and the secret at `open` (F4) |

## 1. What landed

| # | Thing | Where |
|---|---|---|
| 1 | `SessionReturnPage` — the shipping `ReturnUrlSource`, holding the repositories **and** `checkout.public_base_url`; replaced lane 2's blanket `impl … for dyn Repositories` that answered `None` for every intent | `backends/crates/vpay-api/src/v1/return_trip.rs` |
| 2 | `ApiError::CheckoutNotConfigured(CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP)` — a third sentence under lane 1's code, for a session-driven confirm on a deployment that serves no checkout page | `backends/crates/vpay-api/src/error.rs:216` (the constant), `:455` (the variant's "three gaps, one code") |
| 3 | The resolution point **moved**: the session's return page is written into `charges.return_url` before the charge is committed, and `submit_to_rail` reads the committed row. `return_url_for_charge` is gone; the precedence lives in `payer_instrument` | `backends/crates/vpay-api/src/v1/payment_intents.rs` (`confirm_once`, `resolve_rail`, `payer_instrument`, `submit_to_rail`) |
| 4 | A redirect confirm under an **open** session needs no `return_url`, and ignores one that is sent (debug log, no error). No session → today's rule, unchanged | `backends/crates/vpay-api/src/v1/payment_intents.rs` (`payer_instrument`) |
| 5 | `CheckoutSessions::expire_due(now) -> u64` — open sessions past `expires_at` with no live charge → `expired`, `payment_status` untouched | `backends/crates/vpay-db/src/checkout_sessions.rs` (trait + `PgRepositories` impl) |
| 6 | The worker's hourly housekeeping sweep runs it as a fourth statement, with `checkout_sessions = <count>` on the same log line. **No new `jobs.kind`, no migration** | `backends/crates/vpay-worker/src/handlers.rs` (`sweep_expired`) |
| 7 | `merchant_clients[].display_name` — optional, non-blank, ≤ 80 characters, validated at boot, sample in `config/application.yml` | `backends/crates/vpay-config/src/oauth.rs` (the field), `config.rs` (`validate_display_name`, `DISPLAY_NAME_MAX_CHARS`), `lib.rs` (`ConfigError::MalformedDisplayName`) |
| 8 | `ResourceConfig::merchant_display_name`, `model::CheckoutMerchantObject`, `model::CheckoutSessionForPayer`, and `merchant: { name }` on **both** browser session reads | `vpay-api/src/v1/mod.rs`, `src/model.rs`, `src/browser/checkout_sessions.rs` (`for_payer`) |
| 9 | Both browser reads answer the uniform 404 once `now() >= expires_at`, whatever the `status`; the session read renders the intent's `client_secret` only while `status = 'open'` | `backends/crates/vpay-api/src/browser/checkout_sessions.rs` (`authenticate`, `retrieve`), `v1/checkout_sessions.rs` (`OPEN`) |
| 10 | `ConfigError::NonCanonicalCheckoutOrigin` — the raw text must equal `parsed.origin().ascii_serialization()`, and the message names what to write instead | `backends/crates/vpay-config/src/config.rs` (`validate_checkout_origins`), `lib.rs` (the variant) |
| 11 | Two nits: the caller-supplied publishable key in both browser log lines bounded to 40 characters (`bounded`); the origins route's module doc states that a key **with** origins is distinguishable from an unknown one, and why that is accepted | `backends/crates/vpay-api/src/browser/checkout_sessions.rs` |
| 12 | Two config fixtures | `backends/crates/vpay-config/tests/fixtures/{checkout-origin-non-canonical,merchant-display-name-too-long}.yml` |
| 13 | Eleven new tests, three retired | `confirm_rails.rs` (+3), `checkout_sessions.rs` (+4), `vpay-config` (+2), `vpay-api` (+2, −3) |
| 14 | Reference pages | `docs/reference/vpay-api.md` (rewritten §"Where the payer comes back to", new §§"What ends a browser read" and "`merchant.name`, and why there is a fallback"), `vpay-db.md` (new §"`expire_due` is the same guard on a clock"), `vpay-worker.md` (new §"The housekeeping sweep retires a fourth thing"), `vpay-config.md` (new §§"An origin must be spelled the way a browser spells it" and "`merchant_clients[].display_name`") |
| 15 | Counter | `justfile`: `min_tests` 1050 → 1080. `expected_suites` unchanged at 42 — every case landed in a binary that already existed |

## 2. Decisions taken in this lane, and why

- **A session-driven confirm with no `checkout.public_base_url` is refused,
  not fallen back from.** It is unreachable by any merchant request —
  `POST /v1/checkout/sessions` answers `checkout_not_configured` before a row
  can exist — so the only way to stand in that branch is an operator deleting
  the key while sessions are open. The alternative, quietly using the
  merchant's `charges.return_url`, is exactly the failure lane 2's note
  warned about: the payer is forwarded one step too early, the session never
  reaches `complete`, and nothing reports it. The refusal is loud and, since
  the lookup now runs *before* `open_attempt`, costs no charge row. It fires
  for a **push** rail too, which would have ignored the URL: the deployment's
  checkout page is gone either way, and an outage that depended on which rail
  a payer picked would be worse to debug than one that does not.
- **The return URL is resolved *before* the charge is written, not between the
  write and the rail call.** Lane 2 put it after `open_attempt` so the value
  would be "what would survive a crash"; writing it into the row satisfies
  that more strongly and fixes three things at once — the worker's
  `charge_ref` (which reads `charges.return_url` and would have resubmitted a
  different URL than the confirm sent), `next_action.redirect_to_url.return_url`
  (rendered from that column, so a merchant polling their own intent was shown
  a URL no payer was sent to), and the crash window itself. One column, one
  URL.
- **`return_url_for_charge` was retired rather than kept beside the new
  rule.** Two places deciding the same precedence is how they come to
  disagree. Its three unit tests went with it; the replacement
  (`a_session_driven_redirect_confirm_needs_no_return_url_and_ignores_one`)
  tests the shipping `payer_instrument` directly rather than a `Fixed`
  stand-in, which is a better test of the same property.
- **A `return_url` sent alongside a session is ignored, not refused.** A
  merchant's server integrating both surfaces may send its own on every
  confirm; refusing that would make the two paths differ for no gain. Logged
  at debug so an operator can see the value was dropped.
- **The expiry sweep is a fourth statement in the existing `sweep_expired`
  job, not a fifth `jobs.kind`.** It is the same shape as the other three —
  one unconditional statement, hourly, whose healthy answer is zero — and a
  new kind would have needed a migration to say nothing this one does not.
  Lane 1's ⛔ row assumed a new kind; it turned out not to be needed.
- **`expire_due` takes `now` rather than comparing against Postgres's
  `now()`**, unlike the other two sweeps. The horizon on the other side of
  that comparison was computed in Rust at create (D10's 24 hours belongs to
  the API, not to a migration), and both sides of a comparison belonging to
  the same layer is what keeps it one rule. It also lets a test sweep a future
  instant instead of rewriting a stored horizon.
- **The browser reads check `expires_at` themselves; the sweep is not what
  enforces it.** The sweep leaves a session with a live charge `open` on
  purpose, it runs at most once an hour, and a deployment whose worker was
  down would keep answering those reads for the length of the outage. The
  sweep makes `status` honest to a *merchant*; the read is what refuses a
  payer credential. The integration test runs **no worker** and asserts the
  row still says `open`, so it cannot pass for the sweep's reason.
- **The horizon ends both reads whatever the `status`, and `status` gates only
  the intent's `client_secret`.** A `complete` session's return page is the
  screen the whole redirect leg exists to reach, so refusing it would break
  the successful case; what ends the reads is the clock. The credential is a
  different question — it exists to drive `confirm`, and after settlement
  there is nothing to confirm.
- **The wire field is `merchant: { name }`, not `merchant.display_name`.**
  The lane brief said `display_name`; `frontends/apps/checkout`'s
  `isSessionEnvelope` (`src/lib/api.ts:69-87`) requires `merchant.name`, and
  lane 3's own note (§4a) states the shape. Rendering `display_name` would
  have made every session read `error.unexpected` on the page. **This is a
  deliberate departure from the brief's wording**, taken because the page is
  the consumer and its guard is the contract; the *configuration* key is
  `display_name`, as the brief asked.
- **The fallback for a merchant with no configured name is the tenant id, and
  that is a real trade-off.** Lane 1 declined to render `merchant_id` under a
  display-name key, calling it the plausible-looking fabrication AGENTS.md's
  second rule forbids (§3b), and that concern is right about *inventing* a
  name. What settles it here is that the page's contract makes the member
  required: the choice is not between a name and nothing, it is between the
  operator's own label for the tenant and a page that refuses to paint.
  `acme-cameroon-tenant` is a poor name to show a payer and it is not a
  fabricated one. The honest fix is configuration, and the field's doc, the
  YAML sample and `docs/reference/vpay-api.md` all say so. **If lane E
  disagrees, the change is one line in `ResourceConfig::merchant_display_name`
  and a corresponding `Option` on the wire — and it needs lane 3 to make
  `merchant` optional in the same PR.**
- **A non-canonical checkout origin is refused, not normalised.** Normalising
  would make the configuration file and the running policy two different
  documents. The message names the canonical spelling, which is why it is its
  own variant: the useful part of it is a *value*, and
  `MalformedCheckoutOrigin`'s reason is a `&'static str`.
- **The bounded publishable key in the log lines is a bound, not a
  redaction.** A publishable key is public by design and the whole value of
  logging it is that an operator can compare it against the merchant's page.
  40 characters is past the longest key `vpay_config` accepts, and the value
  arrives from an unauthenticated caller who may send any length.

## 3. Where this deviates, and what is not done

- **`merchant.name`, not `merchant.display_name`** — §2, and the reason is
  lane 3's shipping guard.
- **`charges.return_url` now stores vpay's return page for a session-driven
  redirect confirm**, where it used to store the merchant's. That is visible
  to a merchant: `next_action.redirect_to_url.return_url` on such a confirm is
  now vpay's URL. It is the honest value — it is where the payer actually
  goes — and it is what makes the row, the rail and every later read agree.
  Worth calling out because a merchant integrating both surfaces will see it
  change.
- **`checkout_not_configured` still answers 500, not 503.** Lane 1's §3a
  argument is unchanged and this lane adds a third message under the same
  code. Still a maintainer's decision.
- **Nothing in `frontends/`, `sdks/`, `docs/status.md` or `docs/flows/*` was
  touched.** §6 and §7 below are lane E's to apply.
- **No Cypress, no `just demo`, no `just ci` end to end.** The plan puts the
  full gate in the `vpay-ci` VM on the merged branch.
- **The expiry sweep does not notify anyone.** A session that expires emits no
  event and no webhook; a merchant learns by reading it. D10 names no event
  and this lane added none.
- **`verify-docs` reports one new production function of 80 lines or more**:
  `browser/checkout_sessions.rs:206 fn authenticate`, 106 lines. It grew by
  the horizon check and its comment. Recorded rather than suppressed — the
  five refusal steps and the clock are one ordered argument, and splitting
  them would put "what may read a session" in two places. The report is
  advisory and fails nothing.

## 4. Measured, on this branch

Host: the lane worktree, `CARGO_BUILD_JOBS=4`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, container suites with
`--retries 2 -j 1`.

| Gate | Result |
|---|---|
| `cargo +nightly fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features --locked` | clean, no new `#[allow]`/`#[expect]` |
| `cargo nextest run -p vpay-db -p vpay-api -p vpay-config -p vpay-worker --retries 2 -j 1` | **496 run, 496 passed, 0 skipped** (119.7 s) |
| `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions) \| binary(confirm_rails) \| binary(worker_recovery) \| binary(worker_e2e)' --retries 2 -j 1` | **55 run, 55 passed, 0 skipped** (200.0 s) — `checkout_sessions` 17 (was 13), `confirm_rails` 12 (was 9), `worker_recovery` 23, `worker_e2e` 3 |
| `just verify` | ok — `verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity` pass; `verify-docs` is the advisory report |
| `just verify-ignored` | **0 ignored (expected 0), 42 test binaries (expected 42), 1129 total (minimum 1080)** |
| `just test-doc` | **82 doctests passed, 1 ignored** (the ignored one is `vpay_sdk`'s and pre-existing) |

`cargo xtask verify-errors` is green with the new `ConfigError` variants: both
are on an enum that already `impl Classify`.

**Flakes, reported as flakes.** Two, neither in code this lane changed:

* `checkout_sessions::a_declined_payment_leaves_the_session_expired_and_failed`
  failed twice and passed on the third try (`FLAKY 3/3`) in one intermediate
  run, and passed first time in every run since, including the final gate. Its
  subject is lane 1's settlement flip.
* One run of a single-test filter failed with `container is not ready:
  container startup timeout` after 185 s, with no assertion reached. The host
  had 87 leftover `Created` testcontainers at the time. It passed on the next
  attempt. This is the environment, not the suite.

## 5. Guard-failure proofs

Each mutation was applied, the named test run, the file restored from a copy
taken first, and `git status --porcelain` confirmed empty afterwards.

| # | Mutation | Test | Observed |
|---|---|---|---|
| 1 | `SessionReturnPage::session_return_url` answers `Ok(None)` again (lane 2's body) | `a_session_driven_confirm_sends_vpays_return_page_to_the_rail` | **FAIL** — the rail was told `https://shop.example/order/1234/return` instead of `…/c/cs_…/return?t=…&key=…`; a payer it redirected would land on the merchant's site with the session still open. `a_direct_confirm_sends_the_merchants_return_url_to_the_rail` still passed, which is correct |
| 2 | Delete the `NOT EXISTS` live-charge clause from `CheckoutSessions::expire_due` | `the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one` | **FAIL** — the paying session came back `("expired", "unpaid")` while its charge was still live and the rail could still take the payment |
| 3 | Remove the session branch from `payer_instrument` | `a_browser_confirm_under_a_session_needs_no_return_url` | **FAIL** — `400 invalid_request`, `param: return_url`, "This payment method redirects the payer, so a `return_url` is required." — i.e. vpay's own checkout page refused |
| 4 | Disable the `expires_at` check in `browser::checkout_sessions::authenticate` | `both_browser_reads_stop_at_the_horizon_whatever_the_status` | **FAIL** — `200` for a session an hour past its horizon, rendering it in full |
| 5 | Render `ExpandableIntent::ExpandedWithSecret` unconditionally on the session read | `the_session_read_stops_handing_out_the_intents_secret_once_it_is_settled` | **FAIL** — a `complete` session re-issued `pi_…_secret_…` |
| 6 | Blank the name in `browser::checkout_sessions::for_payer` | `both_browser_reads_carry_the_merchants_display_name` | **FAIL** — `Some("")` against `Some("Boutique Acme Cameroun")` |
| 7 | Disable the canonical comparison in `validate_checkout_origins` | `a_checkout_origin_must_be_spelled_the_way_a_browser_spells_it` and `every_checkout_rule_refuses_its_own_fixture` | **FAIL, both** — `https://Shop.example:443` accepted, and `checkout-origin-non-canonical.yml` loaded |

Proofs 1, 4 and 5 are the security-relevant ones: 1 is a payer forwarded one
step too early with nothing reporting it, 4 is a written-down credential that
never expires, 5 is a live intent credential re-issued for a finished
checkout.

## 6. `docs/status.md` — rows and amendments for lane E

### 6a. Amendment to lane 2's row ("The payer's return trip through the port")

Replace, verbatim:

```markdown
**Not done, plainly:** vpay serves no page at a return URL of its own — `vpay_api::v1::return_trip`'s session branch answers `None` for every intent because there is no `checkout_sessions` table, which is the truth today and not a stub; the impl's doc comment names the repository method that must replace it.
```

with:

```markdown
**Closed 2026-09-04 (Step 9, lane 1b):** the session branch is wired. `SessionReturnPage` holds the repositories *and* `checkout.public_base_url`, and a charge driven by an open session is submitted with `{checkout.public_base_url}/c/{cs_id}/return?t={return_token}&key={pk}` — the URL `CheckoutSessionRow::return_page_url` builds, byte for byte, proven over the Orange stub's request journal by `a_session_driven_confirm_sends_vpays_return_page_to_the_rail` (which also asserts the merchant's own `return_url`, sent on the same confirm, is **not** what reached the rail). The value is written into `charges.return_url` **before** the charge is committed and read back from that row at submit, so what the rail is told, what `next_action.redirect_to_url.return_url` renders on every later read, and what the worker would resubmit under are one column. A session on a deployment with no `checkout.public_base_url` is refused (`checkout_not_configured`) rather than silently sent to the merchant's URL — unreachable by any merchant request, since `create` refuses first, and proven by `a_session_driven_confirm_is_refused_when_the_checkout_app_is_gone` against a second server over the same database.
```

### 6b. Amendment to lane 1's Checkout Sessions row

Replace, verbatim:

```markdown
Still 🟡 and not ✅: **no page exists yet** (lane 3), the return trip through the port is not wired (lane 2), no SDK models the resource (lane 5), and no expiry sweep runs — see the two rows below.
```

with:

```markdown
Since **2026-09-04 (lane 1b)** the return trip is wired, sessions expire on their own, and both browser reads carry `merchant: { name }` — the member vpay's own checkout page requires before it can paint. Two rules that were missing from those reads are now on the **read** itself and not on any sweep: past `expires_at` both answer the uniform 404 whatever the `status` (the `return_token` travels in a query string and therefore lands in a rail's logs, so the 24-hour horizon has to bound it), and the intent's `client_secret` is rendered only while `status = 'open'` (after settlement there is nothing left to confirm). A redirect confirm on an intent an open session drives no longer requires a `return_url` and ignores one that is sent — the page has none to send, and until this it answered `400`, so the hosted Orange flow could not complete at all. Still 🟡 and not ✅: no payer has ever driven this in a browser, no SDK models the resource (lane 5), and the page is 🟡 for its own reasons (lane 3).
```

### 6c. Replacement for lane 1's expiry-sweep ⛔ row

**Retire the row headed** `| Checkout session expiry sweep | ⛔ |` **entirely**
and replace it with:

```markdown
| Checkout session expiry sweep (`vpay_db::CheckoutSessions::expire_due`, `vpay_worker::handlers::sweep_expired`) | ✅ | **New 2026-09-04 (Step 9, lane 1b).** `checkout_sessions.expires_at` was written at create (24 h, D10) and read by nothing; a session past its horizon reported `status: open` until a merchant expired it by hand or the intent settled. The worker's existing hourly housekeeping job now runs a fourth statement — `open` and past `expires_at` and **no live charge** → `expired`, `payment_status` untouched — and logs its count as `checkout_sessions` beside `idempotency_keys`, `client_assertion_jtis` and `expired_leases`. **No new `jobs.kind` and no migration**: it is the same shape as the other three, and a fifth kind would have said nothing this one does not. The live-charge guard is a `NOT EXISTS` inside the `UPDATE`, over the same `LIVE_CHARGE_STATES` `expire` and `cancel` use, because a session whose payer confirmed seconds before the horizon has a rail holding a live payment and a background job that expired it would be contradicted by the settlement transaction minutes later. Proven by `the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one` (`backends/tests/integration/tests/checkout_sessions.rs`): two sessions created through `POST /v1/checkout/sessions`, one with a live charge opened through the browser confirm the page uses, both moved past their horizon, swept by the shipping `vpay_worker::run_once` over the shipping `seed_singletons` job. Measured 2026-09-04: deleting the `NOT EXISTS` clause expires the paying session. **What it is not:** the sweep is not what refuses an expired session's payer credential — the browser reads check `expires_at` themselves, because the sweep leaves live-charge sessions `open` on purpose and a deployment whose worker is down must not keep answering. And nothing is notified: an expired session emits no event and no webhook. |
```

### 6d. Replacement for lane 1's §3b gap (the merchant display name)

**Retire** any row or sentence saying the merchant's display name is not
built. Add to the Configuration table:

```markdown
| `merchant_clients[].display_name` (`vpay_config`) | ✅ | **New 2026-09-04 (Step 9, lane 1b).** What a payer is told they are paying, on vpay's own checkout page. Optional, non-blank, at most 80 **characters** — a rendering bound, not a storage one: it is painted into "Pay {merchant}" in a heading on a phone-sized page, and it is refused at boot (`ConfigError::MalformedDisplayName`) rather than truncated at render time. Not secret: it is rendered to every payer of this merchant by construction, so it prints in `MerchantClient`'s `Debug`. Rendered as `merchant: { name }` on both `/v1/browser/checkout/sessions/{id}` and `…/return` — the member name `frontends/apps/checkout`'s own envelope guard requires, so a server that rendered `display_name` there would make every session read `error.unexpected`. A merchant with none falls back to its **tenant id**, which is stated rather than hidden: the page's contract makes the member required, so the choice is not between a name and nothing but between the operator's own label and a page that refuses to paint — a deployment serving hosted checkout should set this for every merchant. Sample in `config/application.yml`, one fixture, and `both_browser_reads_carry_the_merchants_display_name` exercises both branches in one deployment. |
```

### 6e. Amendment to lane 1's `checkout_origins` configuration row

Append to that row:

```markdown
**Tightened 2026-09-04 (lane 1b):** an entry must also be the *canonical* spelling a browser compares against — `parsed.origin().ascii_serialization()`, i.e. lower-cased host, IDNA-encoded to ASCII, default port elided. `https://Shop.example`, `https://shop.example:443` and `https://shöp.example` all passed every earlier rule and were all dropped **silently** by the checkout app's own filter (`frontends/apps/checkout/src/lib/origins.ts`), leaving the merchant with `frame-ancestors 'none'` and no diagnostic anywhere. `ConfigError::NonCanonicalCheckoutOrigin` names what to write instead rather than which rule was broken, because the useful part of that message is a value. Refused rather than normalised, so the file and the running policy stay the same document. One fixture, and a unit case that also accepts the three canonical spellings so it cannot pass by refusing everything.
```

### 6f. Amendments to existing counters

- **Test counts.** `cargo nextest list --workspace`: **1129 total, 42 test
  binaries, 0 ignored**. `min_tests` moved 1050 → 1080 in this lane's commit
  with the reasoning in the recipe's own comment; `expected_suites` is
  unchanged, because every case here landed in a binary that already existed.
- **Doctest count.** `just test-doc`: **82 passed, 1 ignored** — unchanged.
  This lane added no doctest and removed none.
- **Migrations.** Still **28**. This lane added none.

## 7. Flow-doc lines for lane E

### 7.1 `docs/flows/browser-checkout.md` — append to the "The routes" table's Step 9 rows

After the three rows lane 1 supplied, append this paragraph beneath the table:

```markdown
Both checkout session reads stop at the session's `expires_at` — 24 hours from
create (D10) — **whatever its `status`**, and answer the uniform 404 past it.
The `return_token` is the reason: it travels in a query string because a
fragment does not survive a rail's redirect, so a copy of it is in the rail's
storage, in whatever the rail logs and in the checkout app's access logs, and
the horizon is the bound on how long that copy is worth anything. It is
deliberately not conditioned on `status`, because a `complete` session's return
page is the screen the whole redirect leg exists to reach. The check is on the
**read** and not on the hourly expiry sweep, which leaves a session with a live
charge `open` on purpose and would keep answering for the length of a worker
outage.

The session read hands over the *intent's* `client_secret` only while
`status = 'open'`. That credential exists so vpay's page can drive
`POST /v1/browser/payment_intents/{id}/confirm`; once the session is finished
there is nothing left to confirm, and the page has already read it.

Both reads also carry `merchant: { name }` — the merchant's configured
`display_name`, or its tenant id when it has none. It is the one fact about
the merchant a payer is shown.
```

### 7.2 `docs/flows/browser-checkout.md` — replace lane 2's D4 paragraph "What is still open"

Replace, verbatim:

```markdown
**What is still open.** vpay serves no page at a return URL of its own: the
page that receives the payer is `frontends/apps/checkout` (Step 9, lane 3).
```

with:

```markdown
**Closed at the server end 2026-09-04 (lane 1b).** A charge driven by an open
checkout session is submitted with vpay's own return page —
`{checkout.public_base_url}/c/{cs_id}/return?t={return_token}&key={pk}` —
written into `charges.return_url` before the charge is committed and read back
from that row at submit, so what the rail is told and what a later read renders
are one column. A confirm under an open session needs no `return_url` at all,
which is what vpay's own page sends; a confirm with no session still requires
one. The page that receives the payer is `frontends/apps/checkout` (Step 9,
lane 3), and no browser has driven the round trip yet.
```

### 7.3 `docs/flows/payment-lifecycle.md` — append to lane 1's Step 9 paragraph

```markdown
A checkout session also ends on its own: the worker's hourly housekeeping
sweep moves an `open` session past its 24-hour horizon to `expired`, unless
its intent has a charge the rail may still be acting on — the clock never
overrules a live payment. `payment_status` is untouched by that, exactly as by
a merchant's own `expire`.
```

### 7.4 `docs/flows/configuration.md` — append to the checkout paragraph

```markdown
Two more rules on that block since 2026-09-04: a `checkout_origins` entry must
be the canonical spelling a browser compares against (lower-case host, IDNA to
ASCII, default port elided), because anything else is dropped silently by the
page and leaves the merchant unable to embed with nothing to read; and
`merchant_clients[].display_name` is what a payer is told they are paying —
optional, non-blank, at most 80 characters, falling back to the tenant id.
```

## 8. What I did not do

- Did not touch `frontends/`, `sdks/`, `docs/status.md`, `docs/roadmap.md` or
  `docs/flows/*`. §6 and §7 are lane E's to apply.
- Did not add a migration, a `jobs.kind`, an event or a webhook for an expired
  session.
- Did not change `checkout_not_configured`'s status from 500 to 503 — still
  lane 1's open maintainer decision, now with a third message under the code.
- Did not make `merchant` optional on the wire, which is the alternative to
  the tenant-id fallback and needs lane 3 in the same PR (§2).
- Did not run `just ci` end to end, `just demo`, the conformance suite or the
  Cypress specs; the plan puts those in the `vpay-ci` VM on the merged branch.
- Did not touch lane 2's `charge_ref` in the worker, though this lane removes
  the hazard its comment describes: `charges.return_url` is now the session's
  page for a session-driven charge, so a future redirect `Resubmit` arm would
  send the right URL. The comment is now conservative rather than wrong, and
  updating it is a one-line change lane E may prefer to fold in.

## Superseded by the integrator (2026-09-04)

§2's tenant-id fallback did not land: with lane 3b's page tolerating an absent
`merchant`, the integrator made `ResourceConfig::merchant_display_name` answer
`Option<&str>` and `CheckoutSessionForPayer.merchant` `Option<_>` (absent on the
wire when unconfigured). `both_browser_reads_carry_the_merchants_display_name`
now asserts the member is absent and the tenant id appears nowhere in the body.
