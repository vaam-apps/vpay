<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. Everything in §5 and §6 below is written so it can be applied verbatim. -->

# Step 9, lane 2 — the return trip through the provider port

Branch `claude/step9-lane-2-return-trip`, on top of `f186d67` (master
`10793e4` + the Step 9 plan).

Four commits: `9234a51` (the port and both adapters), `38e347e` (conformance
and the stub's hosted page), `6b12f21` (the two integration cases), `97b1eb3`
(compose and the justfile) — plus this file and the reference pages.

## 1. What landed

| # | Thing | Where |
|---|---|---|
| 1 | `ChargeRef::return_url: Option<String>`, with a doc comment naming who fills it and what it replaced | `backends/crates/vpay-provider/src/lib.rs:86` |
| 2 | `vpay_api::v1::return_trip` — the `ReturnUrlSource` trait, its shipping impl over `dyn Repositories`, and `return_url_for_charge` | `backends/crates/vpay-api/src/v1/return_trip.rs` (new), registered at `v1/mod.rs` |
| 3 | The confirm path resolves it between `open_attempt` and the rail call, from the **committed** charge row | `backends/crates/vpay-api/src/v1/payment_intents.rs` (`confirm_once`, `submit_to_rail`) |
| 4 | Orange sends it as `return_url` **and** `cancel_url`; the deployment `settings.return_url`/`settings.cancel_url` fallback is gone; a redirect charge without one is `ProviderError::Config` | `backends/crates/vpay-adapter-orange-money/src/lib.rs` (`submit`, new `return_url` helper) |
| 5 | MTN ignores it, asserted | `backends/crates/vpay-adapter-mtn-momo/src/wire.rs` (`a_return_url_is_not_carried_on_a_push_rails_body`) |
| 6 | The worker fills it from `charges.return_url` | `backends/crates/vpay-worker/src/handlers.rs` (`charge_ref`) |
| 7 | `webpayment.json` **requires** an `http(s)` `return_url` and `cancel_url` on every accepted submit (catch-all and both duplicate-scenario mappings), and templates them into `payment_url` | `backends/tests/conformance/wiremock/orange/mappings/webpayment.json` |
| 8 | `stub-hosted-page.json` — `GET /stub-hosted-page/{token}` as HTML with a Pay link and a Cancel link | `backends/tests/conformance/wiremock/orange/mappings/stub-hosted-page.json` (new) |
| 9 | Conformance case `the_submit_tells_the_rail_where_to_send_the_payer_back` (×2 rails), plus `return_url_pattern` and `recorded_requests` | `backends/tests/conformance/tests/adapter_conformance.rs` |
| 10 | Two `confirm_rails` cases and `Harness::orange_origin` | `backends/tests/integration/tests/confirm_rails.rs` |
| 11 | `wiremock-orange` published on `${VPAY_DEMO_ORANGE_PORT:-8082}` in the demo stack, restated in the e2e stack, with a port-agreement guard | `compose.demo.yml`, `compose.e2e.yml`, `justfile` (`demo_orange_port`, `gen-demo-keys`) |
| 12 | Reference docs | `docs/reference/rails.md` (new §"The payer's return URL is the *core's* answer, not a rail's" and §"The stub's hosted page, and where it is not the rail"), `docs/reference/vpay-api.md` (new §"Where the payer comes back to") |

**What this lane closes of D4, and what it does not.** The *rail* half:
Orange is now told, per charge, where to send the payer, and the stub's page
links there. The *page* that receives the payer is lane 3's
(`frontends/apps/checkout`); `vpay-server` still serves no HTML and no return
route, and this lane added none.

## 2. Decisions taken in this lane, and why

- **Orange refuses a redirect charge with no `return_url`** rather than
  falling back to deployment settings or to the callback URL. D2 says the
  adapter must stop inventing a deployment-wide answer to a per-charge
  question, and a fallback that is never exercised is a fallback nobody knows
  is wrong. It is the exact twin of MTN's "payer_ref required on a push rail".
  Blast radius checked before taking it: `vpay_api::payer_instrument` already
  *requires* a `return_url` on a redirect confirm, and the worker never
  resubmits a redirect charge (`recovery_step` answers `FailDeadOrder` for
  `ProviderFlow::Redirect` before looking at anything else), so no path in
  this repository can reach the refusal without a merchant having omitted a
  field the API refuses first. `config/application.yml` set neither settings
  key, so no deployment configuration changes.
- **`return_url` and `cancel_url` get the same value.** Orange's page
  distinguishes them; vpay cannot. The outcome comes from the authenticated
  status query, and a charge the payer abandoned is `Pending` until it
  expires, so two different URLs would encode a distinction nothing checks.
  When lane 3's return page can tell the two apart, this is one line.
- **The value is read from the committed charge row, not from the request.**
  `confirm_once` resolves it *after* `open_attempt`. What the rail is told is
  therefore what would survive a crash, and not a second read that could
  differ from what was made durable.
- **`ReturnUrlSource`'s shipping impl answers `None`, and that is the truth
  rather than a stub.** There is no `checkout_sessions` table in this tree, so
  no charge is session-driven. It is not `NotImplemented` (nothing is being
  refused) and not a fabricated URL. The doc comment on the impl names lane
  1's method to replace it with **and the failure if lane 1 forgets**: a
  session-driven payer forwarded one step too early, with nothing reporting
  it. See §3.
- **The conformance charge carries a `return_url` on the *push* rail too.**
  Not what production does — and deliberate. "MTN sent no return URL" proves
  nothing when there was none to send.

## 3. Deviations from the lane brief, and one thing lane 1 must not miss

- **The Orange submit mapping requires a URL, not a URL under the
  deployment's `public_base_url`/`checkout.public_base_url`.** The brief asked
  for the second. It cannot be: for a direct `/v1` or `/v1/browser` confirm
  the correct value is the **merchant's own site** — that is precisely the
  case this lane exists to close — and a matcher demanding vpay's origin would
  refuse it, breaking `confirm_rails.rs` and every merchant-driven Orange
  confirm. The mapping therefore requires `^https?://.+` and the *exact* value
  is pinned twice: by the conformance case over the request journal, and end
  to end by `a_direct_confirm_sends_the_merchants_return_url_to_the_rail`. The
  mapping's own `metadata` says all of this.
- **`demo_orange_port` is a variable with a guard, not a free choice.**
  `webpayment.json` templates `payment_url` on a committed
  `http://localhost:8082`, and WireMock renders a response from the current
  request alone — vpay's submit arrives over the compose network as
  `wiremock-orange:8080`, so the stub cannot learn what the host published it
  on. `gen-demo-keys` reads the port back out of the mapping and fails naming
  both numbers when they disagree. That is the same stale-artefact discipline
  it applies to `demo_port`, with a **check** instead of a regeneration
  because the mapping is shared with `compose.yml`, CI's e2e stack and both
  Rust suites and is not this recipe's to rewrite. The consequence, stated
  plainly: two concurrent demos now collide on 8082 unless one of them edits
  that mapping. Before this lane they did not collide, and the demo's redirect
  URL also could not be opened.
- **For lane 1.** `vpay_api::v1::return_trip`'s `impl ReturnUrlSource for dyn
  Repositories` returns `Ok(None)` unconditionally. Landing migration `0028`
  and the `CheckoutSessions` repository **without changing that body** breaks
  nothing loudly: every session-driven payer goes to the merchant's URL
  instead of vpay's return page, the session never reaches `complete`, and the
  only symptom is a checkout that forwards one step too early. There is no
  test that can catch it from this side, because from this side there are no
  sessions.
- **For a future rail.** `vpay_worker::handlers::charge_ref` fills
  `return_url` from `charges.return_url`, which is the merchant's and not a
  session's. That is safe only because no redirect charge is ever resubmitted.
  If `recovery_step` ever grows a redirect `Resubmit` arm, the session's URL
  has to become readable from that row or the worker will silently send a
  different URL than the confirm did. The comment on `charge_ref` says so.

## 4. Proofs, with numbers

Run on the authoring host (rootless Docker, `DOCKER_HOST=unix:///run/user/1000/docker.sock`,
`CARGO_BUILD_JOBS=4`), 2026-09-04.

| Gate | Result |
|---|---|
| `cargo +nightly fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features --locked` | clean, no new `#[allow]`/`#[expect]` |
| `cargo nextest run -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money -p vpay-api -p vpay-tests-conformance --retries 2 -j 1` | **373 tests run, 373 passed, 0 skipped**, 1 flaky |
| `cargo nextest run -p vpay-tests-conformance --retries 2 -j 1` (alone) | **30 tests run, 30 passed, 0 skipped** (was 28) |
| `cargo nextest run -p vpay-tests-integration -E 'binary(confirm_rails) \| binary(worker_e2e)' --retries 2 -j 1` | **12 tests run, 12 passed, 0 skipped** — `confirm_rails` 9 (was 7), `worker_e2e` 3 |
| `just verify` | the four gates pass; `verify-docs` is advisory |
| `just verify-ignored` | **0 ignored (expected 0), 41 test binaries (expected 41), 1068 total (minimum 1000)** — was 1059 |
| `just test-doc` | **77 doctests passed, 1 ignored** (the ignored one is `vpay_sdk`'s and pre-existing) |

**The flaky, reported as a flake and not as a pass.**
`a_declined_charge_maps_to_the_documented_failure_code::case_1_mtn_momo`
failed once and passed on retry (2/3) in the five-package run; it passed
first time in the conformance-only run. It touches nothing this lane changed
— its subject is MTN's decline vocabulary — and the signature is a container
that was slow to start on a loaded host.

**The guard-failure proof, and it is decisive.** `#[serde(skip)]` on
`WebPaymentRequest`'s `return_url` and `cancel_url` (so the adapter sends the
body without them) makes `webpayment.json` match nothing, WireMock answer
`404`, and the adapter raise `ProviderError::Config`:

```text
FAIL the_submit_tells_the_rail_where_to_send_the_payer_back::case_2_orange_money
  an accepted submit must be Ok: Config("orange_money: no webpayment endpoint
  under the configured base_url (HTTP 404): Request was not matched …")
```

The MTN case still passed, which is correct — it asserts an absence. Restored
immediately; `git diff` on `wire.rs` is empty.

**The stub page, from a browser's position.** `just demo_orange_port=18082
gen-demo-keys` fails with the named message; the default passes. With the
three compose files layered and only `wiremock-orange` up,
`GET localhost:8082/stub-hosted-page/pay-abc?return=…&cancel=…` answers `200
text/html` with `<a id="pay" href="https://shop.example/order/1234/return">`
and the matching `cancel`. `docker compose … config` confirms
`demo_orange_port=18082` moves the publication and does not append a second
one.

## 5. Verbatim status-row text for lane E

### 5.1 New row, in the same table as "Charge submission (`confirm` → rail)"

```markdown
| The payer's return trip through the port — `vpay_provider::ChargeRef::return_url`, `vpay_api::v1::return_trip` | 🟡 | **New 2026-09-04 (Step 9, lane 2).** The provider port carries a **per-charge** `return_url` and `vpay-adapter-orange-money` sends it as both `return_url` and `cancel_url`. Until this landed the adapter answered that question out of **deployment** settings, falling back to the notification endpoint — so the merchant's own `return_url` was validated, written to `charges.return_url`, echoed back as `next_action.redirect_to_url.return_url`, and **never sent to the rail that would act on it**, while every conformance and integration case passed. `vpay_api::v1::return_trip::return_url_for_charge` fills it after the charge row is committed and before the rail is called, so the value the rail is told is the value that would survive a crash. `mtn_momo` ignores it: a push rail has no browser. **Proven:** `the_submit_tells_the_rail_where_to_send_the_payer_back` (conformance, ×2 rails — Orange's exact value pinned over WireMock's request journal, MTN's *absence* asserted against the stub's whole journal, body, headers and query alike, with the push charge deliberately carrying a URL so the case is not vacuous); `a_direct_confirm_sends_the_merchants_return_url_to_the_rail` and `the_stub_hosted_page_links_to_the_return_url_the_submit_carried` (`backends/tests/integration/tests/confirm_rails.rs`, real Postgres + real WireMock, through the shipping confirm path). `webpayment.json` **requires** an `http(s)` `return_url` and `cancel_url` on every accepted submit — measured 2026-09-04: `#[serde(skip)]` on those two fields makes the stub answer 404 and the Orange conformance case fail on `ProviderError::Config`; restored. The matcher is a URL and deliberately **not** a prefix under vpay's own origin, because for a direct confirm the correct value is the merchant's own site. **Not done, plainly:** vpay serves no page at a return URL of its own — `vpay_api::v1::return_trip`'s session branch answers `None` for every intent because there is no `checkout_sessions` table, which is the truth today and not a stub; the impl's doc comment names the repository method that must replace it. And no payer has been redirected anywhere by this repository: every assertion above is against a `wiremock/wiremock` container, never Orange |
```

### 5.2 Strike-and-replace inside the "Browser checkout surface" row

Replace, verbatim:

```markdown
the Orange redirect *return trip* has no route (`/provider/{code}/callback` does not exist — see the Reconciler row above), so this surface ships push-only (D4).
```

with:

```markdown
~~the Orange redirect *return trip* has no route (`/provider/{code}/callback` does not exist — see the Reconciler row above), so this surface ships push-only (D4)~~ — **half-retired 2026-09-04 (Step 9, lane 2): the rail is now told the merchant's own `return_url` per charge and sends the payer there, which is what D4 was about. The parenthesis was already stale — `POST /provider/{code}/callback` has existed since Step 8 lane C and was never the return trip. What is still missing is a vpay-served *page*, which is Step 9 lane 3's; a merchant integrating `@vpay/stripe-js` against a redirect rail must land the payer on their own `return_url` and poll from there.**
```

### 5.3 Strike-and-replace inside the `examples/checkout-browser` row

Replace, verbatim:

```markdown
Ships push-only (D4); the redirect half is untested here (see the Reconciler row)
```

with:

```markdown
Ships push-only (D4); the redirect half is untested here (see the Reconciler row). **Unchanged by Step 9 lane 2:** the rail is now told where to send the payer, but this example has no page to receive one and was not modified
```

## 6. Verbatim flow-doc replacement text for lane E

### 6.1 `docs/flows/browser-checkout.md` — replace the whole "The redirect gap (D4)" section

```markdown
## The redirect gap (D4) — the rail half closed 2026-09-04

Step 5c shipped **push-only**, and the reason was that vpay never told a
redirect rail where to send the payer. `confirmPayment` returned the rail's
real `next_action.redirect_to_url` and `@vpay/stripe-js` navigated to it, but
`vpay-adapter-orange-money` read `return_url` from a **deployment** setting
(`setting(config, "return_url").unwrap_or(callback_url)`), so with
`config/application.yml` setting none, Orange returned every payer to
`{public_base_url}/provider/orange_money/callback` — a `POST`-only route for
the rail's own backend, where a browser gets an empty `405` (measured:
`a_get_on_the_callback_path_is_a_405_and_not_the_404_envelope`). The
merchant's own `return_url` was stored on `charges` and echoed back in
`next_action` as a label, and nothing redirected to it.

**Step 9's D2 closed the rail half** (`docs/plans/2026-09-04-step9-hosted-checkout.md`,
`docs/reference/rails.md`). `vpay_provider::ChargeRef` carries a per-charge
`return_url`, `vpay_api::v1::return_trip` fills it from the committed charge
row — the merchant's own URL for a direct `/v1` or `/v1/browser` confirm,
vpay's session return page when a checkout session drives the charge — and
Orange sends it as both `return_url` and `cancel_url`. A redirect charge with
none is refused before the rail is called. Proven by
`the_submit_tells_the_rail_where_to_send_the_payer_back` (conformance, once
per rail) and by two cases in
`backends/tests/integration/tests/confirm_rails.rs`.

**What is still open.** vpay serves no page at a return URL of its own: the
page that receives the payer is `frontends/apps/checkout` (Step 9, lane 3).
Until it ships, a `@vpay/stripe-js` integration against a redirect rail lands
the payer on the **merchant's** `return_url` and must poll
`GET /v1/browser/payment_intents/{id}` from there — the outcome comes from
vpay's authenticated status query and never from the fact that a payer came
back. Tracked in `docs/status.md`.
```

### 6.2 `docs/flows/adapter-orange-money.md` — insert after the "The calls" block, before "## Status mapping"

```markdown
### Where the payer comes back to

`return_url` and `cancel_url` are **per charge**, filled by the core and
carried on `vpay_provider::ChargeRef::return_url` (Step 9, D2). Both fields
get the same value: Orange's page distinguishes "paid" from "cancelled" and
vpay cannot — the outcome comes from the authenticated `transactionstatus`
read, and a charge the payer abandoned is `Pending` until it expires — so two
URLs would encode a distinction nothing checks. A charge with no `return_url`
is `ProviderError::Config` before the call; this adapter will not invent one.

Until 2026-09-04 both fields came from **deployment** settings
(`settings.return_url` / `settings.cancel_url`, falling back to `notif_url`),
which was one answer per deployment to a per-charge question. Those two
settings keys are gone; nothing shipped set them.

`lang` is unchanged and is still the one defaulted field in the request body.
```

### 6.3 `docs/flows/adapter-orange-money.md` — replace the Status section's conformance sentence

Replace, verbatim:

```markdown
**All 11 conformance port cases now pass for this rail** — 26 tests across
both rails, 26 passed, 0 skipped, measured 2026-09-03 with `cargo nextest run
-p vpay-tests-conformance`.
```

with:

```markdown
**All 13 conformance port cases now pass for this rail** — 30 tests across
both rails, 30 passed, 0 skipped, measured 2026-09-04 with `cargo nextest run
-p vpay-tests-conformance` (26 on 2026-09-03; `the_submit_tells_the_rail_where_to_call_back`
was added by Step 8 lane C and `the_submit_tells_the_rail_where_to_send_the_payer_back`
by Step 9 lane 2).
```

### 6.4 `docs/flows/adapter-orange-money.md` — add to the "Still unverified against the real rail" list

```markdown
- **The stub's hosted page is not Orange's.** `wiremock/orange/mappings/stub-hosted-page.json`
  serves `/stub-hosted-page/{pay_token}` with a Pay link and a Cancel link so
  a browser can finish the redirect leg. The real rail *stores* `return_url`
  and `cancel_url` against the `pay_token` at submit and renders them from its
  own state; WireMock can only template from the current request, so the
  submit's `payment_url` carries the two URLs as query parameters and the page
  templates them back. The pairing is real — those are the bytes that submit
  sent — but nothing here shows Orange would accept a `return_url` it had not
  been told about, and nothing claims it would.
```

### 6.5 `docs/flows/adapter-mtn-momo.md` — insert after the paragraph ending "refused as `Malformed` rather than guessed at.", before "### What the transport refuses, and why"

```markdown
`ChargeRef::return_url` is ignored, and that is asserted rather than assumed
(`a_return_url_is_not_carried_on_a_push_rails_body` in `wire.rs`, and the
push half of `the_submit_tells_the_rail_where_to_send_the_payer_back` in the
conformance suite, which checks the stub's whole request journal). The core
fills the field for any charge whose merchant sent a URL and leaves it to
each adapter to decide whether its rail has a use for one; `requesttopay`
has no browser step, and a field MTN does not document would at best be
dropped.
```

### 6.6 `docs/flows/provider-port.md` — insert after the `ProviderConfig` paragraph ("…only place a `ProviderConfig` is built from YAML."), before "## Capabilities"

```markdown
`ChargeRef` carries `reference_id`, `amount`, `payer_ref`, `ref_extra` and —
since Step 9 — `return_url`: where a redirect rail must send the payer when
its own page is done with them. It is **the core's** answer, never an
adapter's (`docs/reference/rails.md`), and an adapter on a push rail must
ignore it.
```

### 6.7 `docs/flows/provider-port.md` — replace the conformance count in the Status section

Replace, verbatim:

```markdown
26 tests — 4 capability cases plus 11 port cases parameterised over both
rails — run live against a real `wiremock/wiremock` container started by
`vpay_testkit::containers::start_wiremock`. **26 passed, 0 skipped,
0 ignored**, measured on 2026-09-03.
```

with:

```markdown
30 tests — 4 capability cases plus 13 port cases parameterised over both
rails — run live against a real `wiremock/wiremock` container started by
`vpay_testkit::containers::start_wiremock`. **30 passed, 0 skipped,
0 ignored**, measured on 2026-09-04 (26 on 2026-09-03; Step 8 lane C added
`the_submit_tells_the_rail_where_to_call_back` and Step 9 lane 2 added
`the_submit_tells_the_rail_where_to_send_the_payer_back`).
```

## 7. What this lane did **not** do

- No route was added to `vpay-server`, and no HTML is served by it.
  `provider_callback.rs`'s behaviour is untouched.
- No migration, no `docs/status.md` edit, no `docs/flows/*` edit, no frontend
  change. §5 and §6 above are lane E's to apply.
- No checkout session was stubbed, faked or invented. The session branch of
  the return-URL lookup answers `None` because there are no sessions.
- `lang` was not touched: it is still `settings.lang` falling back to `fr`.
  Passing the payer's locale through is lane 3's, and there is no field on the
  port for it.
- Nothing called a real rail. Every wire assertion in this lane is against a
  `wiremock/wiremock` container.
- The stub hosted page was not driven by a browser here. Cypress is lane 6's;
  what this lane proves is that the page exists, is published on a port the
  demo stack maps, and carries the two links the submit paid for.
