# The status archive

[docs/status.md](../status.md) was 6 151 lines on 2026-09-11. It is 275 now.
**The other 5 996 lines are here, verbatim.** Nothing was summarised away,
nothing was tidied, and in particular nothing that recorded a mistake was
dropped: this repository's habit of writing "this said X until date Y and was
wrong" is the reason the page is worth trusting, and a compression that lost one
of those would have cost more than the length did.

Only the relative links changed, because the files moved down a directory. The
32 lines that are neither here nor on `docs/status.md` are blank lines and `---`
rules that used to separate two sections which now live in two files.

## What answers what

| If you want to know                                                                                   | Read                                                           |
| ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| what each `just verify` gate checks, what it used to miss, and the mutation that proved the hole shut | [gates.md](gates.md)                                           |
| which backend capability is real, and what its test actually asserts                                  | [backend.md](backend.md)                                       |
| which payment provider can do what today, cell by cell, across every rail shipped or designed         | [providers.md](providers.md)                                   |
| why the Orange stub's hosted page grew a payer's window                                               | [backend-orange-hosted-page.md](backend-orange-hosted-page.md) |
| what the checkout page, the dashboard and the demo shop do                                            | [frontend.md](frontend.md)                                     |
| what boots, in compose and in CI, and what has never run in a pod                                     | [infrastructure.md](infrastructure.md)                         |
| how Docker, `cargo deny`, the toolchain pin and the migration manifest got fixed                      | [infrastructure-history.md](infrastructure-history.md)         |
| what the sqlx 0.8 → 0.9 bump cost, and which OP stores pinned 0.8                                     | [sqlx-and-op-stores.md](sqlx-and-op-stores.md)                 |
| how far CrateStack adoption has got, and what each step measured                                      | [cratestack.md](cratestack.md) and [cratestack/](cratestack/)  |
| what each merchant SDK ships, and which gaps are dated                                                | [merchant-sdks.md](merchant-sdks.md)                           |
| what the Flutter checkout plugin's gate/recipes/pin actually do, and what is still unbuilt            | [mobile-flutter-plugin.md](mobile-flutter-plugin.md)           |
| what the Tauri v2 checkout plugin actually compiles, what no gate touches, and what has never run     | [mobile-tauri-plugin.md](mobile-tauri-plugin.md)               |
| what would have to be true before any of this is an MVP                                               | [mvp.md](mvp.md)                                               |
| how the "vpay is a scaffold" banner was narrowed, step by step                                        | [overall-history.md](overall-history.md)                       |
| what `just ci` actually printed on a given day                                                        | [verification/](verification/)                                 |

## The verification log, newest first

Named by the newest date in each block, not by a single date: entries were
appended to `docs/status.md` in the order branches landed, which is not always
the order they were measured.

**This list is not the whole of [verification/](verification/), and does not
try to be.** Re-measured 2026-09-20, again 2026-09-22 when the Tauri
plugin's page landed, and ten times on 2026-09-23 (twice more on 2026-10-08, for the chart-global-values and jti-purge-horizon pages, twice on 2026-10-09, for the npm-advisories page and when the protected-diagnostics page merged, once on 2026-10-17, for the origin-refund-core page, once on 2026-10-18, for the numeric-user page, five times on 2026-10-20, for the staff-oidc-sso, notchpay-client, notchpay-adapter, notchpay-sandbox and path-segment-dot-segments pages, fourteen times on 2026-10-21, for the notchpay-sandbox-run, notchpay-trx-reference, sdk-dot-segment-ids, notchpay-step5, payer-dot-segment-ids, flutter-example-lockfile, provider-requests-refused-answers, declined-resubmit-fails-the-charge, cli-spawn-diagnostics, node-invalid-params-error, rail-callback-retries, dashboard-hydration, confirm-decline-order and storybook-vitest-advisories pages, and four times on 2026-10-22, for the rfc-0013-step-0-rest, self-contradictions, notchpay-refunds and rfc-0009-phases pages), when the super-linter
page, the stale-claims page, the customer-filters page, the manual-payments
page, the migrations-manifest page, the test-clock-skew page, the
session-customer page, the erasure-through-checkout-sessions page, the skills-reverification page and the single-clock-scheduling page did: the directory holds 101 <!-- count:files-with-suffix docs/status/verification .md --> files today;
this list names 78 of them. This is a hand-maintained list, not a generated one,
and the 23 it omits were simply never appended to it. (It said 59 files and 36
named until 2026-09-22, 60/37, 61/38, 62/39, 63/40, 64/41, 65/42, 66/43, 67/44, 68/45, 69/46 on 2026-09-23 and then 70/47 until 2026-10-08, when the chart-global-values page made it 71/48 and the jti-purge-horizon page 72/49, then 73/50 on 2026-10-09 with the npm-advisories page, 74/51 when the protected-diagnostics page merged, 75/52 on 2026-10-17 with the origin-refund-core page, 76/53 on 2026-10-18 with the numeric-user page, 77/54 on 2026-10-20 with the staff-oidc-sso page, 78/55 the same day with the notchpay-client page, 79/56 the same day again with the notchpay-adapter page, 80/57 the same day with the notchpay-sandbox page, 81/58 with the path-segment-dot-segments page, 82/59 on 2026-10-21 with the notchpay-sandbox-run page, 83/60 the same day with the notchpay-trx-reference page, 84/61 the same day again with the sdk-dot-segment-ids page, 85/62 the same day with the notchpay-step5 page, 86/63 the same day with the payer-dot-segment-ids page, 87/64 the same day with the flutter-example-lockfile page, then 89/66 once the dashboard-md3e and cli-tests-observability-port pages had landed too (master's version of this sentence was not extended for those two; both are in the list below), 90/67 and 91/68 with the provider-requests-refused-answers and declined-resubmit-fails-the-charge pages, 92/69 with the cli-spawn-diagnostics page, 93/70 the same day with the node-invalid-params-error page, 94/71 with the rail-callback-retries page, 95/72 with the dashboard-hydration page, 96/73 the same day with the confirm-decline-order page, 97/74 with the storybook-vitest-advisories page, 98/75 on 2026-10-22 with the rfc-0013-step-0-rest page, 99/76 the same day with the self-contradictions page, 100/77 the same day with the notchpay-refunds page, and 101/78 the same day with the rfc-0009-phases page; each time both
moved by one and one page was added, so the 23 is unchanged — the arithmetic
is stated because the "23" is the only one of the three numbers no gate
measures.) Most of the 23 are
still findable: they are linked individually from the area page whose
section they verify (for example
`verification/2026-09-13-adr-0018-admin-role-review.md` from
[backend.md](backend.md), or `verification/2026-09-14-flutter-review.md` from
[mobile-flutter-plugin.md](mobile-flutter-plugin.md)) — but at least one,
`verification/2026-09-13-credential-split.md`, is linked from nowhere else in
this repository and is only reachable by browsing the directory. A
hand-maintained list is exactly how this gap happened; browse
[verification/](verification/) directly for the complete, current file set
rather than trusting this list to be exhaustive.

- [verification/2026-10-22-rfc-0009-phases.md](verification/2026-10-22-rfc-0009-phases.md) —
  RFC-0009 revised on the maintainer's decisions: phase A (projects of one
  company share the company's account on each rail; a boot refusal for a
  second company; `charges.provider_account_id`) first, phase B (other
  companies, on Stripe Connect's shape) designed. "One deployment per
  merchant" becomes one per legal entity; nothing built, and counsel not
  asked.
- [verification/2026-10-22-notchpay-refunds.md](verification/2026-10-22-notchpay-refunds.md) —
  Notch Pay's sandbox refuses `POST /refunds`: `403` "Forbidden", with no
  message, to all four requests the probe's new `refunds` phase sent
  against a `complete` payment. `GET /refunds` answers `200` with an empty
  list. Nothing was refunded, so replay safety (RFC-0012 § 10 item 6) is
  still unmeasured. Also corrects adapter-notchpay.md's stale step-5 bullet:
  #314 already records refused reads as answered.
- [verification/2026-10-22-self-contradictions.md](verification/2026-10-22-self-contradictions.md) —
  fourteen places where vpay contradicted its own code or itself at v0.10.2,
  found by vaam-apps/vpay-docs' re-read and corrected as dated notes:
  `notify-docs` has run on every tag since v0.5.0 (its infrastructure row
  moves to ✅), the dashboard's SSO landing, chips, password field and BFF read
  path, the Overview's twelve counts, settlements observed against real
  sandboxes, and the poll ladder reading rather than settling every charge;
  four listed items that were not contradictions, and why.
- [verification/2026-10-22-rfc-0013-step-0-rest.md](verification/2026-10-22-rfc-0013-step-0-rest.md) —
  RFC-0013 Delivery step 0 completed: `scan:live` runs every ten minutes, not
  hourly (three pages corrected), and no MTN page says MTN requires or has an
  IP-allowlisted callback host (`vpay-config` and seven more places corrected;
  MTN's real rule is the API user's registered `providerCallbackHost`); what
  ran, and what was not.
- [verification/2026-10-21-storybook-vitest-advisories.md](verification/2026-10-21-storybook-vitest-advisories.md) —
  `web` red on `master` at `b7cc6c9`: `audit-web` failed on a critical
  Storybook advisory, and two moderate `@vitest/mocker` advisories appeared
  within the hour. Cleared by bumps (Storybook 10.6.0 → 10.6.3, vitest
  4.1.11 → 4.1.12), with nothing ignored. Audit counts before and after, the
  `web` steps re-run against a `master` baseline, and what was not run.
- [verification/2026-10-21-confirm-decline-order.md](verification/2026-10-21-confirm-decline-order.md) —
  `vpay-api`'s confirm now fails a declined charge before it records the attempt;
  the window the worker change's review found, measured as reachable without a
  crash (a refused settlement: `503`, an answered row beside a `submitting`
  charge, advanced to `submitted` on the first poll), the new order and why the
  accepted arm keeps the old one, the mutations, the counts, and what was not
  run.
- [verification/2026-10-21-rail-callback-retries.md](verification/2026-10-21-rail-callback-retries.md) —
  no file now says a rail retries a callback that is not answered with a 2xx:
  MTN documents one callback and no retry, Orange documents nothing either
  way, and the `202` for an unknown reference stands on the oracle argument
  alone (RFC-0013 § 11, item 1); MTN's `PUT`-or-`POST` callback method and
  whether Orange's notification carries `order_id` (items 2 and 3) are
  recorded as open "To confirm" items with their sources; what ran, and that
  no rail callback did.
- [verification/2026-10-21-node-invalid-params-error.md](verification/2026-10-21-node-invalid-params-error.md) —
  every local refusal in `sdks/nodejs` is a `VpayInvalidParamsError`, not a
  bare `TypeError`, closing the parity gap dated 2026-09-03; why it cannot
  also be a `TypeError`, the renamed tests, three caught mutations, the
  gates, and what was not run.
- [verification/2026-10-21-cli-spawn-diagnostics.md](verification/2026-10-21-cli-spawn-diagnostics.md) —
  a `vpay-server` that fails to boot in `tests/cli.rs` now says why: the exit
  status and its own stderr and stdout tail, at once, instead of `None` after 5 s;
  three regression tests, each caught by a mutation; what it was used for — the
  stolen-port candidate not supported in 30 widened runs, and every remaining red
  in the path to Postgres (an alive-but-silent boot, an exit 69, `PortNotExposed`);
  the one decision left, the 5 s budget.
- [verification/2026-10-21-declined-resubmit-fails-the-charge.md](verification/2026-10-21-declined-resubmit-fails-the-charge.md) —
  a resubmit the rail declines now fails the charge; the measured loop it ended
  (a declined charge resent after every third poll, four submits and eleven
  status reads in sixteen steps, the intent never told why), the settle-then-record
  order and what it assumes, the mutations (including the answered-row-but-live-
  charge pair), the counts, and what was not run.
- [verification/2026-10-21-provider-requests-refused-answers.md](verification/2026-10-21-provider-requests-refused-answers.md) —
  a status read the rail answered and the adapter refused is recorded as
  answered, not as "no answer" (11 such rows in RFC-0012 step 5's run); a
  refused submit stays unresolved, and why; the decision table, three new
  Postgres-and-WireMock cases and what each of four mutations did to them, the
  stale "nothing reads this table" claim, the gate output, and what was not
  run.
- [verification/2026-10-21-dashboard-md3e.md](verification/2026-10-21-dashboard-md3e.md) —
  the dashboard on `@vaam-apps/ui` 0.5.0 and Material 3 Expressive, an
  Overview of exact procedure counts, and navigation without reloads (6 → 1
  document loads over one click-through); the gate exit codes, the guards
  seen failing, and what was not run (Cypress, a real vpay).
- [verification/2026-10-21-dashboard-hydration.md](verification/2026-10-21-dashboard-hydration.md) —
  `dashboard.cy.ts` 17 of 18 red on `master` at `0706cd5`: the theme script
  #318 put in `<head>` failed hydration (React #418) under Cypress. It moves
  to the start of `<body>`. Reproduced under Cypress on a production build
  before, and passing after; what was not run.
- [verification/2026-10-21-cli-tests-observability-port.md](verification/2026-10-21-cli-tests-observability-port.md) —
  `vpay-server`'s `tests/cli.rs` lost a race for the observability default
  `0.0.0.0:9090` under plain `cargo test` (`master` failed 1 whole-file run of
  3; nextest serialises the file and hid it); the mechanism proven against the
  real binary, the one-line fix in `bin()`, 10 consecutive green runs and nextest
  after it, and one unexplained red run, named as such, with what was run to
  catch it.
- [verification/2026-10-21-flutter-example-lockfile.md](verification/2026-10-21-flutter-example-lockfile.md) —
  the Flutter example's `pubspec.lock` recorded the plugin at `0.1.1` while it
  was `0.9.1`; now synced, and `release-please.yml` rewrites that line on
  every release branch. The step was run from the workflow file against a
  simulated `0.9.2` bump and matched `flutter pub get` byte for byte.
  ~~No real release has run it yet.~~ _(2026-10-22: `v0.10.0`, `v0.10.1` and
  `v0.10.2` each carried the sync commit; the page's dated section says so.)_
- [verification/2026-10-21-payer-dot-segment-ids.md](verification/2026-10-21-payer-dot-segment-ids.md) —
  tests only: the payer surfaces' client-secret parsers, in `sdks/stripe-js`
  and the Flutter plugin, are shown to refuse an empty or all-dots id on
  every route that builds a path, sending nothing, and an id carrying
  `/../` stays one segment; why the Tauri plugin needs no test; the counts,
  the five mutations and what each turned red, and the gate output.
- [verification/2026-10-21-notchpay-step5.md](verification/2026-10-21-notchpay-step5.md) —
  RFC-0012 step 5, run: three test-mode payments through vpay itself, against
  Notch Pay's real sandbox. The success and the failure settled on the
  worker's second read; the short `complete` is refused and stays `pending`.
- [verification/2026-10-21-sdk-dot-segment-ids.md](verification/2026-10-21-sdk-dot-segment-ids.md) —
  both merchant SDKs refuse an empty or all-dots `id` before any request;
  what vpay answered for all 21 id-taking methods before the fix (nothing
  succeeded, `.` reached other handlers, a pinned `Idempotency-Key` locked
  out the corrected retry for 24 hours), the same sweep after it, the test
  counts, the caught mutations, the gate output, and what was not run.
- [verification/2026-10-21-notchpay-trx-reference.md](verification/2026-10-21-notchpay-trx-reference.md) —
  the `notchpay` adapter addresses process and read by Notch Pay's `trx.…`,
  kept as `ref_extra` key material; an unresolvable `409` fails closed; the
  conformance suite's per-rail key-material table; the residual crash
  window; the tests and the gates. Not yet met Notch Pay (step 5).
- [verification/2026-10-21-notchpay-sandbox-run.md](verification/2026-10-21-notchpay-sandbox-run.md) —
  RFC-0012 step 1, run: Notch Pay's real sandbox, through the probe and not
  the adapter. Item 2 contradicted (the path takes Notch Pay's `trx.…`
  only, so the adapter's `submit` fails closed, not yet fixed); the
  "insufficient funds" number read `complete` for less than asked, now
  refused; `canceled` is `payer_declined`; three webhook endpoints the probe
  could not delete; every § 10 (S) item, the redacted report, and the gates.
- [verification/2026-10-20-path-segment-dot-segments.md](verification/2026-10-20-path-segment-dot-segments.md) —
  `vpay_provider::http::path_segment` returns a `Result` and refuses an empty
  or all-dots segment as `Malformed`, because no escape keeps a dot segment
  in place; `mtn_momo`'s lookup migrated; the test counts, four caught
  mutations, the gate output, and what was not run.
- [verification/2026-10-20-notchpay-sandbox.md](verification/2026-10-20-notchpay-sandbox.md) —
  the first Notch Pay sandbox session, which made **no** request: the
  maintainer's real test keys carry `test.` and not the documented `test_`,
  so the livemode boot rule was wrong both ways and is corrected; every (S)
  item of RFC-0012 § 10 is "could not be measured", and why.
- [verification/2026-10-20-notchpay-adapter.md](verification/2026-10-20-notchpay-adapter.md) —
  the `notchpay` adapter, admitted under ADR-0031 (rule 3 amended): every
  gate's output, the test counts, the conformance row against WireMock, the
  two mutations that row was checked against, and what was **not** run —
  no sandbox session and no test-mode call, because no test-mode key was
  available.
- [verification/2026-10-20-notchpay-client.md](verification/2026-10-20-notchpay-client.md) —
  the Notch Pay client crate (linked by nothing), RFC-0012, RFC-0013 and the
  provider matrix: the independent review's two blocking findings and how
  they were fixed, every gate's output, and what was not run.
- [verification/2026-10-20-staff-oidc-sso.md](verification/2026-10-20-staff-oidc-sso.md) —
  ADR-0030: staff sign in through an OpenID Connect provider — the gate output,
  the test counts, the mutation checks, what a real Keycloak 26.6.1 issued (and
  that its `amr` vanishes from an SSO-cookie token after about a second unless
  the realm sets `default.reference.maxAge`), and what was not run.
- [verification/2026-10-18-numeric-user.md](verification/2026-10-18-numeric-user.md) —
  `frontends/Dockerfile`'s `runner` and `checkout` stages declare the numeric
  `USER 1000:1000` instead of `USER node`, because the kubelet cannot verify
  `runAsNonRoot` against a named user (observed on a cluster with chart 0.6.0);
  the status rows that cited `docker run` as non-root evidence are corrected;
  which commands ran in a build of both targets, and that no pod did.
- [verification/2026-10-17-origin-refund-core.md](verification/2026-10-17-origin-refund-core.md) —
  RFC-0010 § 7, delivery step 1: `refund_at_rail` hands an adapter the
  charge's `provider_txn_id` in the refund `ChargeRef`'s `ref_extra`; the
  `Origin` arm gains two integration cases on a test-only `Origin` rail and
  the conformance premise guard becomes a rule; which commands ran in a
  sandbox without Docker, with their counts, and which did not.
- [verification/2026-10-09-npm-advisories.md](verification/2026-10-09-npm-advisories.md) —
  the `next`, `brace-expansion`, `sharp` and `source-map-js` advisories that
  failed `trivy` and `web` on every PR: which version each moved to, Trivy's
  exit 1 → 0 and `pnpm audit`'s 18 → 1, the rest of the `web` job's results,
  and the one advisory (`braces`) with no published fix, ignored by id on the maintainer's decision.
- [verification/2026-10-08-jti-purge-horizon.md](verification/2026-10-08-jti-purge-horizon.md) —
  ADR-0028: the housekeeping sweep deleted a spent client-assertion `jti` at
  its `exp`, while the validator accepts the assertion for 61 s longer
  (`jsonwebtoken`'s leeway), so a captured, spent assertion could be replayed
  once. The sweep now keeps the row five minutes past `exp`. Carries the
  reproduction (the replay answered `200` before the fix), the counts after,
  and what was not run.
- [verification/2026-10-08-chart-global-values.md](verification/2026-10-08-chart-global-values.md) —
  [#269](https://github.com/vaam-apps/vpay/issues/269): the Helm chart's schema
  refused `global`, the key Helm passes to every subchart, so no chart could
  depend on it. The reproduction on Helm 3.16.1, 3.18.6, 3.19.0 and 4.0.0, the
  one-key fix, the new wrapper-chart step in `just helm-check` with its before
  and after output, three mutations and what each turned red, and what a
  `file://` dependency does not prove about the published OCI chart.
- [verification/2026-10-01-protected-diagnostics.md](verification/2026-10-01-protected-diagnostics.md) —
  RFC-0002 PR 3 (issue #147): the four protected diagnostic representations
  in `vpay-core` (`Secret`, `Masked`, `Pseudonymous`, `SafeUrl`), 24
  personal-data/secret types across `vpay-db`, `vpay-api` and `vpay-provider`
  swapped from derived to hand-written redacting `Debug` (17 in the first
  version, seven more after the maintainer's review of 2026-10-09), and the
  `debug_protections` registration list the privacy gate now enforces in both
  directions. What was run, what was not, what the gate cannot see, and what is
  recorded as open (the staff request/token types, the `Display` path of a
  transport failure).
- [verification/2026-09-23-session-customer-onto-intent.md](verification/2026-09-23-session-customer-onto-intent.md) —
  ADR-0025, answering ADR-0024's question 3: a checkout session's customer is
  written onto a customer-less intent in the insert's own transaction. What
  the old code allowed, the SQL, the four container-backed cases (one forcing
  the race with a held row lock), both mutations, every affected suite's
  passed and ignored counts, and the gates.
- [verification/2026-09-23-customer-filters.md](verification/2026-09-23-customer-filters.md) —
  RFC-0004 § 5's first bullet: `customer=` on the intent, session and refund
  lists, in both SDKs. Which column each list compares and why (a session's
  own, a refund's intent's), the container-backed cases and the mutations that
  turned them red, the `/dash/v1` refusal, and the `EXPLAIN` measurements. Its
  first section records what could not run while the host disk and then Docker
  were down; its second records the full suites once they could.
- [verification/2026-09-23-erasure-through-checkout-sessions.md](verification/2026-09-23-erasure-through-checkout-sessions.md) —
  ADR-0027: customer erasure also reaches a customer-less intent that one of
  the customer's checkout sessions names, and never an intent that names
  another customer. It has the copies extended, the guard, the three new
  cases and the extended whole-database scan, three mutations and what each
  turned red, every suite's passed and ignored counts, and the gates.
- [verification/2026-09-23-manual-payments.md](verification/2026-09-23-manual-payments.md) —
  RFC-0004 § 6, invoices paid out of band (migration `0049`): the gate output,
  every suite's passed and ignored counts, the drift constants derived and then
  measured, two mutations, and the lock-order and fixture checks asked for on
  review.
- [verification/2026-09-23-migrations-manifest-macos.md](verification/2026-09-23-migrations-manifest-macos.md) —
  `just migrations-manifest` made portable (`find … -exec basename` in place
  of GNU `-printf`, `shasum -a 256` when there is no `sha256sum`): macOS and
  Debian output byte-identical to the committed manifest and to each other on
  an appended line, the old recipe's output matched under GNU tools, and both
  refusals re-checked.
- [verification/2026-09-23-skills-reverification.md](verification/2026-09-23-skills-reverification.md) —
  fourteen claims a re-verification of the agent skills found stale, thirteen
  of them real: the route-probe test that had missed `manual_payments` (now
  checked against the schema's own model list, with the mutation that turns it
  red), a theme test title from before the `light` theme, a nine-method port
  documented as six, a thirteen-key object called twelve, "the only `DELETE`",
  a `draft → void` edge that never existed, "FIVE models" of fourteen, a
  two-leaf exit-code chain of four, and "both binaries" sixteen days after
  there was one. And the one that was not real, and why.
- [verification/2026-09-23-stale-claims.md](verification/2026-09-23-stale-claims.md) —
  fourteen claims found stale while writing the vpay-docs site against
  `v0.4.1`, each re-checked on `master` and corrected in place with its date:
  the API server "never calls a payment rail", a lifecycle edge the code never
  takes, three "MTN's real sandbox has never been called", a `200 OK` the
  callback route has never answered, "no job loop" and "no fan-out" a
  fortnight after both landed, event, invoice-key and `501` counts,
  `"expires_in": 300` against a 900-second constant, and READMEs that still
  said the dashboard had no login and the SDKs were not on npm. Documents and
  one doc comment only; no behaviour changed.
- [verification/2026-09-23-super-linter.md](verification/2026-09-23-super-linter.md) —
  the `lint / Super-linter` check that has been red since #238, which failed
  it and was merged during a GitHub Actions outage before checks reported.
  Why #240 and #241 were green in between (super-linter lints only a pull
  request's changed files, and neither touched markdown), the ten findings
  verbatim, and the one that is a real rendering bug rather than a nit: a
  `docs/sdks/parity.md` gap-ledger row that has supplied one cell where the
  table has four since **#176, 2026-09-14**. Carries the local
  `ghcr.io/super-linter/super-linter:slim-v8.7.0` reproduction, before and
  after, and the reason the two exclusions are exclusions — a file
  `cargo build` regenerates on every run, and TypeScript's own
  comment-bearing config format.
- [verification/2026-09-23-single-clock-scheduling.md](verification/2026-09-23-single-clock-scheduling.md) —
  ADR-0026: every job's `run_at`, a delivery's `next_attempt_at`, the
  live-charge backstop's staleness window and the queue-age gauge computed on
  Postgres' clock rather than the application host's. Every production site
  and what it does now (the ten the test-clock-skew page listed, plus the
  backstop cutoff it did not), what deliberately stays on the application's
  clock and why, the new exact-equality test and the two mutations that turn
  it red — and the one thing no test here can show, a genuinely skewed host.
- [verification/2026-09-23-test-clock-skew.md](verification/2026-09-23-test-clock-skew.md) —
  the `webhooks.rs` "the fan-out job is claimable" failures after a Docker
  Desktop restart, and every other test fixture that stamped a job with this
  host's clock and then claimed it on Postgres' `now()`, or held a
  database-written instant against this host's clock. Each now reads the
  database's clock. The failure mode is demonstrated with a simulated 200 ms
  skew; the flake itself was **not** reproduced, and the page says so. Ten
  production sites with the same two clocks are listed and deliberately not
  changed: in production they cost a delay equal to the skew, not a failure.
  Test code only.
- [verification/2026-09-22-tauri-plugin.md](verification/2026-09-22-tauri-plugin.md) —
  the Tauri v2 checkout plugin, built in parallel lanes: what each lane
  measured (the Rust crate's 19 tests + 1 doctest and its clippy/`cargo doc`
  runs; the guest-JS package's 71 vitest cases; the Swift's two clean
  compiles) and what the docs lane re-ran through the four new
  `just *-tauri-*` recipes. Carries the `cargo check --target
aarch64-apple-ios` **exit 101** that swift-rs's iOS 13.0 default caused and
  the `if #available(iOS 15.0, *)` guard that closed it, the finding that
  **iOS cannot produce `stopUrlReached` at all** on tauri-v2.11.6, and the
  gate limitations this pass hit rather than worked around —
  `verify-npm-scope` and `verify-links` both walking `git ls-files` on an
  untracked tree, and `verify-sdk-parity`'s table reader splitting a row on
  every raw `|`, which makes two live test titles uncitable. Says plainly
  that no gate compiles the Rust, the Kotlin or the Swift, that the Kotlin
  was compiled by none of the lanes that wrote it, and that nothing has run
  against a vpay, a rail, a device or a simulator.
- [verification/2026-09-20-doc-counts-gate.md](verification/2026-09-20-doc-counts-gate.md) —
  `verify-doc-counts`, the **fifteenth** gate, and the survey that argued for
  it: 35 checkably-false claims in the live docs, **thirteen** of them numbers
  that were right when measured and drifted after, and **not one** of them in a
  claim any gate reads. Carries the gate's output on this tree and all seven
  of its failure modes driven as mutations of the real files, including the one
  that matters most — an unknown marker kind is a hard error, never a skip. The
  `verify-gates` measurer reads the `justfile`'s own `verify:` line, so the
  gate checks the number its own arrival moved from fourteen to fifteen. One
  re-measurement contradicted the page it was added to and is recorded rather
  than smoothed over: the Rust SDK exposes **35** resource methods, not the
  fourteen two pages still built arithmetic on, and the routed-method count
  that arithmetic produced is left un-re-derived rather than guessed.
- [verification/2026-09-18-macos-loopback-and-two-load-flakes.md](verification/2026-09-18-macos-loopback-and-two-load-flakes.md) —
  three ways `just test-rust` fails on a macOS developer machine and in no CI
  job. Four **deterministic** failures in `staff_sign_in.rs`, because its
  per-source-address rate-limit cases bind loopback addresses macOS does not
  assign to `lo0` and Linux does to `lo`: fixed with a preflighting helper that
  names `just loopback-aliases`, explicitly **not** by gating or ignoring them.
  Two **load** flakes, each seen once in three runs: the housekeeping sweep's
  bounded `run_once` retry, which loses because a job's `run_at` comes from the
  test process's clock and `Jobs::claim`'s `now()` from the database's, now a
  deadline poll; and a conformance 404 from a live WireMock, because
  `/__admin/health` at the pinned 3.9.2 tag returns a hardcoded 200 and proves
  nothing about the stub tree — `start_wiremock` now waits for a positive
  `meta.total` on the mapped host port, and `compose.yml`'s claim to the
  contrary is struck through. No assertion weakened, nothing ignored. With
  the aliases in place `just test-rust` then reached **2002 passed, 0
  skipped, exit 0** — the recipe verbatim, which it could not do before. A
  **fourth** defect surfaced on the way and is named rather than half-fixed:
  testcontainers' `PortNotExposed`, about once per full run, whose Postgres
  half lives at fourteen-plus call sites and would need the sweep AGENTS.md
  forbids.
- [verification/2026-09-16-privacy-inventory-gate.md](verification/2026-09-16-privacy-inventory-gate.md) —
  the personal-data inventory of issue #144 and `verify-privacy-inventory`, the
  fourteenth gate; amended 2026-09-17 with the `DROP TABLE` hole that let both
  directions agree about a table migration `0009` deletes, and 2026-09-18 with
  the full `just ci` run this branch owed, four more parser holes that all
  failed open (a keyword match of one literal space among them), and six
  classifications the migrations' own comments contradict — recorded for a
  maintainer, not re-decided.
- [verification/2026-09-17-redirect-leg-review.md](verification/2026-09-17-redirect-leg-review.md)
  — issue #195: the review of the native sheet's redirect leg, and the **three**
  defects it found. The leg URL was built on the **API**'s origin, which serves
  no `/c/` route, and the `/c/{id}/redirect` page read `next_action` off the
  checkout-session route, which never renders one — either alone means no payer
  reaches the rail, and both shipped green because one test used a single string
  for two origins and the other hand-wrote a `fetch` answer the server does not
  send. Both fixed, both mutations re-run and observed failing;
  `flutter test` 302 / 0, `just test-web` checkout 538 / 1,
  `just test-storybook` 24 stories, four `cargo xtask` gates green. `just ci`
  was not run. Carries the throwaway-merge result with PR #197, which needs one
  hand edit `git merge` does not report. **§3, added 2026-09-18 when this
  branch met `origin/master` `84143e1d`:** PR #199's `resume_redirect` state on
  the hosted page reads `next_action` off the same route, for the same reason,
  and so never offered the payer their abandoned rail back — the third instance
  of one root cause, and the first found by the honest stub §2 left behind
  rather than by reading source. Fixed in `CheckoutController.start`;
  `just test-web` checkout **556 / 1**, mutation-checked in both directions.
- [verification/2026-09-17-flutter-remember-msisdn-read.md](verification/2026-09-17-flutter-remember-msisdn-read.md) —
  issue #194: the Flutter sheet's "remember this number" **read** — prefilling
  the MSISDN field and reflecting the saved state in the checkbox on relaunch
  — was broken (write, existence and "forget" all worked). Two wiring defects
  in the widget/controller, not the store: the prefill was gated on a screen
  transition while `defaultMsisdn` is populated asynchronously, and
  `rememberChecked` was never seeded from the stored record. Fixed and pinned
  by two widget + controller tests; `flutter test` 305 passed / 0
  skipped, `dart analyze --fatal-infos` clean. A review pass then found the
  new unticked-submit clear could destroy a record the payer never unticked,
  fixed it, and re-measured at 306 passed / 0 skipped.
- [verification/2026-09-17-release-please-yaml-rewrite.md](verification/2026-09-17-release-please-yaml-rewrite.md) —
  the first release release-please ever cut (#203) destroyed two of the files
  it was told to bump. A **bare-string** `extra-files` entry does not get the
  annotation-only updater; release-please infers one from the file extension,
  and `.yaml` gets `GenericYaml('$.version')`, which reparses and re-serialises
  the document. `deploy/helm/vpay/Chart.yaml` went from 48 lines to 13, its
  hand-managed chart `version:` was **downgraded** `0.2.0` → `0.1.1` — the one
  field the config deliberately excluded — and its annotated `appVersion` was
  left behind. #204 restored both files and made every entry
  `{"type": "generic"}`. **This page is what that left**: the gate had moved
  twice with no test at all, so nine now pin it — including the config from
  #203 itself, which every gate here was green on; a hole is named rather than
  closed, because the parser is kept identical to `vsms`' copy; and three
  pieces of fallout were still on `master` ten hours later — `CHANGELOG.md` and
  `AGENTS.md` still failing `prettier --check` in CI's `web` job, the chart
  version left behind by its own `appVersion`, and a gate count that said
  twelve in two files while the recipe printed thirteen. Says plainly what it
  does not prove: nothing here runs release-please, so the next release pull
  request's diff is the first real confirmation.
- [verification/2026-09-16-adr-0022.md](verification/2026-09-16-adr-0022.md) —
  ADR-0022 (surface isolation and independent scaling): `deployment.surfaces`
  and conditional `/v1`/`/dash/v1` mounting, the dashboard runner stage's real
  `docker run --read-only --tmpfs /tmp --user 1000:1000` observation and the
  `HOSTNAME`-binding defect it found, and the Helm chart's `-management`
  Deployment, HPA and `connection-budget` guard. Carries the review pass's own
  section — five mutations of the surface gate, four of which the new tests
  caught and one (`validate_all`'s boot-refusal wiring) which nothing did —
  **and then the remediation of two independent adversarial reviews**, whose
  first finding was a security boundary: `/v1/oauth/token`, the **merchant**
  credential-minting endpoint, was mounted on a `surfaces: [management]` pod,
  justified by an ADR section that did not exist. It is business-only now,
  the discovery pair stays on both tiers, and the mutation that proves the
  new test is worth having is quoted. Second: the chart could publish
  `/dash/v1` on a Gateway **and** deny that Gateway at the pod, green the
  whole way — closed with a `namespaceSelector` an operator could not
  previously express and a guard that forces the choice, **without** deciding
  the maintainer's open question of whether the tier faces the public gateway
  at all. Also: the management tier's metrics were scraped by nothing, the
  status page's guard count was wrong in three places, `NOTES.txt` printed a
  topology the release may not have, and `connection-budget` was gated in one
  direction of two (25 fixtures, 24 guards, kubeconform validates 68
  resources in 4 renders). `just ci`'s final `deny` step failed on
  `RUSTSEC-2026-0285` for most of this branch's life — pre-existing, and
  **not** dev-only as the page first said: `rustls 0.23.43` was in
  `vpay-server`'s normal graph via `cratestack-pg` and via `reqwest`. **No
  `deny.toml` allow was ever added**, and it no longer fires: the rebase onto
  `master` brought #178's bump to `rustls 0.23.45`, so `just deny` on the
  rebased head is `rc=0`, `advisories ok`. The fix came from somebody else's
  branch, not this one.
- [verification/2026-09-16-confirm-poll-job-latency.md](verification/2026-09-16-confirm-poll-job-latency.md) —
  the first CI run that compiled `--test live_refunds` went red, and what it
  had found was **not** in the SDK: a charge confirmed while the worker's
  claim loop was busy waited up to sixty-one seconds for its first status
  query, on every rail, in every deployment, since Step 4. Reproduced against
  a real stack (`attempts = 1`, `run_at = created_at + 61 s`, `state =
submitted` from six seconds in, no status query on the rail's journal),
  fixed in `vpay-api` with a grace on the enqueue and a pull-forward in the
  confirm's own compare-and-swap, and mutation-proven on two existing
  `confirm_rails` cases. With settlement fixed, both live refund suites then
  failed on a `cancel` that `4bcf6491` had deliberately made impossible;
  the tests were wrong and were corrected, not the guard
- [verification/2026-09-16-w3-seam.md](verification/2026-09-16-w3-seam.md) —
  RFC-0003 wave 3 / arm H, the seam pass, run after every arm and every review
  had merged. Eleven findings, of which one was not a documentation bug: **CI
  ran `--test live_invoices` and not `--test live_refunds`**, so three
  `sdks/parity.md` rows were ✅ citing live refund cases no job compiled —
  `verify-sdk-parity` proves a test **name** exists, never that anything runs
  it. Also: a merge had pasted seventeen lines of prose into
  `docs/status.md`'s gate table and deleted its `verify-status` row, and
  prettier had aligned the wreckage into a plausible-looking table; two
  paragraphs on that page contradicted each other about how many
  `NotImplemented` tokens there are; `docs/flows/ledger.md`'s heading claimed
  four invariants are "asserted nightly" when nothing schedules any of them;
  and three flow pages had no Status entry for the day their subject changed.
  Twelve gates, each invoked on its own; **no banner addendum was earned and
  none was added**
- [verification/2026-09-16-w3-merge.md](verification/2026-09-16-w3-merge.md) —
  wave 3 merged into the refunds feature branch: arm F with its money review,
  arm F with its contract review, and arm G with its review. One conflict (the
  boot banner, rewritten by two reviews at once) and the resolution that names
  all five refund routes; migration `0048`'s "no deployment holds" wording
  corrected **before** it landed, because a migration's bytes are immutable
  once they ship; one case red on `review/w3f-money` before the merge ever
  happened, repaired by building its premise rather than weakening it; and the
  stale claims that only became false once both arms were in one tree — among
  them `docs/flows/errors.md`'s "no `/v1` caller can provoke a `501` at all",
  which a test has contradicted since arm F (806 crate + 73 integration + 67
  conformance, **0 skipped**)
- [verification/2026-09-16-w3-sdk-refunds.md](verification/2026-09-16-w3-sdk-refunds.md) —
  RFC-0003 § 2 wave 3 / arm G and its review: the refund surface in both
  merchant SDKs, driven against a real `vpay-server` over HTTP, plus the
  `gen-demo-keys` fix without which every refund on the demo stack was a 500.
  The `202`s in that run came from a `wiremock/wiremock` container, and no
  refund it created ever left `pending`
- [verification/2026-09-16-w3-refund-routes.md](verification/2026-09-16-w3-refund-routes.md) —
  RFC-0003 § 2 wave 3: the four `/v1` refund routes and the first
  `charge.refunded` / `charge.refund.updated` this repository has ever emitted
  (384 `vpay-api` + 243 `vpay-db` + 260 port/adapter/core + 40 integration
  incl. eleven new WireMock-backed refund cases + 168 `vpay-sdk`, **0
  skipped**). The decisive mutation — deleting the `RefundDestination::Origin`
  refusal — run, and the finding recorded: it fails **one** test in the
  workspace, because vpay carries no `Origin` rail for any integration suite
  to reach. Three things still stand between these routes and a refund a payer
  receives, and the page names all three
- [verification/2026-09-15-refunds-write-path-money-review.md](verification/2026-09-15-refunds-write-path-money-review.md) —
  the adversarial money/concurrency review of the entry below: six mutations
  with their numbers, eight claims confirmed TRUE, and one finding — the
  narrowing that followed the over-refund guard moving to `Refunds::create`
  dropped the only assertion that `apply_refund_succeeded` is one transaction,
  measured by splitting it in two and watching all 25 refund and ledger cases
  still pass (328 `vpay-db`/`vpay-ledger`/`vpay-core` + 52 `postgres_smoke`,
  **0 skipped**; `EXPECTED_DRIFT_CHANGES` re-measured off-pin at cratestack
  0.11.1 and therefore **not** verified here — CI is the arbiter)
- [verification/2026-09-15-refunds-write-path.md](verification/2026-09-15-refunds-write-path.md) —
  RFC-0003 §§ 3-4 wave 2: the first `refunds` INSERT this repository has ever
  issued from Rust, the intent counters `apply_refund_succeeded` never moved,
  and the first ledger postings a call site ever made (241 `vpay-db` + 51
  `postgres_smoke` + 86 `vpay-core`/`vpay-ledger` tests, **0 skipped**; the
  decisive mutation — deleting the `amount_refund_pending` increment — run,
  failed the race test as it must, and reverted)
- [verification/2026-09-13-flutter-lane-b-gate.md](verification/2026-09-13-flutter-lane-b-gate.md) —
  Lane B of the Flutter plugin brief: `verify-sdk-parity` learns Dart
  (245 xtask tests, 0 ignored; clippy `--all-targets -D warnings` clean; the
  real `docs/sdks/parity.md` unaffected, still 469 proving tests), and the
  decisive `skip: true` mutation run against `verify_sdk_parity` itself on a
  synthetic fixture, with the literal error text it returned
- [verification/2026-09-13-dash-cratestack-transport.md](verification/2026-09-13-dash-cratestack-transport.md) —
  mounting CrateStack's read-only procedure transport (nav plan Lane C):
  `POST /dash/v1/$procs/searchPaymentIntents` answers over a real Postgres, an
  unauthenticated caller is refused, the tenant-mismatch mutation reddens the
  procedure body's own container test, and a routing-table walk proves no
  generated model CRUD route is reachable
- [verification/2026-09-13-dashboard-storybook.md](verification/2026-09-13-dashboard-storybook.md) —
  a Storybook for `frontends/apps/dashboard`, mirroring the checkout's: 25
  stories, both `@vaam-apps/ui` themes (21 `dark`, 4 `light`), a measured
  router/`next/link` fix, one real `@vaam-apps/ui` accessibility defect found
  and suppressed by rule id rather than hidden, and the checkout's own
  theme.css alias measured as unnecessary here — plus the same day's
  adversarial review, which found three stories rendering the wrong theme,
  an `AppShell` with no axe coverage at all, and a built-stylesheet gate that
  never ran in CI
- [verification/2026-09-13-dash-shell-drawer.md](verification/2026-09-13-dash-shell-drawer.md) —
  the dashboard shell's "More" drawer (nav plan Lane A): the theme switcher
  reachable below the ≥1280px sidebar, the signed-in identity kept visible in
  `<main>` rather than moved behind it, the bottom-sheet-below-`md` /
  right-panel-above split, and the review that reversed the first
  implementation's "a bottom sheet cannot pass `verify-ui`" finding
- [verification/2026-09-13-storybook-reverified.md](verification/2026-09-13-storybook-reverified.md) —
  the 2026-09-12 Storybook restoration re-run on `origin/master`, the six
  places that still said the gap was open, and the measured answer to the
  question `justfile` left about the new `.storybook`'s tsconfig
- [verification/2026-09-12-erasure-idempotency-window.md](verification/2026-09-12-erasure-idempotency-window.md) —
  issue #111: the 24-hour window in which an update that lost a race to a
  `DELETE` put the payer back into `idempotency_keys.response_body` —
  reproduced, then closed — and the merchant's own copy, which is a contract
  question and is **not** decided
- [verification/2026-09-12-worker-claim-latency.md](verification/2026-09-12-worker-claim-latency.md) —
  what a single worker's claim latency actually is (issue #100): one
  `IDLE_SLEEP` plus milliseconds, the same at two workers, the bound now
  pinned, and the loop left alone
- [verification/2026-09-12-customer-debug-redaction.md](verification/2026-09-12-customer-debug-redaction.md) —
  the customer **write** path's `Debug`, which printed the payer's name, email,
  phone, street and GPS point until this date; issue #113's two questions about
  the coordinate are prepared and **not taken**
- [verification/2026-09-12-issue-88.md](verification/2026-09-12-issue-88.md) —
  issue #88 item by item: the proactive re-mint, the outage-tolerant cookie,
  the paging test — which a same-day adversarial review found was green under
  the very mutation this log calls decisive, and which gained the two
  assertions that fix that — the CSRF forwarded-host guard, and why item 5 is
  moot. Amended in place the same day; one residual is named and not closed
- [verification/2026-09-12-dashboard-refine.md](verification/2026-09-12-dashboard-refine.md) —
  the dashboard on Refine (plan lanes 3 and 4): one array for the rail and the
  router, and the `401`-only sign-out rule carried into `authProvider`
- [verification/2026-09-12-instrument-register.md](verification/2026-09-12-instrument-register.md) —
  `InstrumentPanel` on the payment detail and `Card glow` on the payer's
  amount, and the aggregate panel that could not be built honestly
- [verification/2026-09-12-vaam-apps-ui-012.md](verification/2026-09-12-vaam-apps-ui-012.md) —
  `@vaam-apps/ui` 0.1.1 → 0.1.2: a second theme, and the three places this
  repository assumed there was only one
- [verification/2026-09-12-storybook-restored.md](verification/2026-09-12-storybook-restored.md) —
  Storybook restored in `frontends/apps/checkout`, and the dropped `@import`
  that had both apps' stylesheets silently losing their entire theme
- [verification/2026-09-12.md](verification/2026-09-12.md) —
  the `@vaam-apps/ui` cutover: both apps move off `@vpay/ui`, and it is
  deleted along with its Storybook
- [verification/2026-09-12-browser-a11y.md](verification/2026-09-12-browser-a11y.md) —
  the browser a11y gate PR #133 built against `@vpay/ui`, the four contrast
  violations it found, and the deletion that superseded it hours later
- [verification/2026-09-11.md](verification/2026-09-11.md) —
  `claude/exp51-demo-tenant`, the header of `claude/exp45-worker-pool-bound`,
  and `claude/exp46-customer-address`
- [verification/2026-09-10.md](verification/2026-09-10.md) —
  `claude/exp39-readme-gaps`, `claude/exp38-sigterm-scenario`
- [verification/2026-09-07.md](verification/2026-09-07.md) —
  `claude/exp30-single-binary`, `claude/exp29-migration-manifest`,
  `claude/exp21-checkout-page`
- [verification/2026-09-04.md](verification/2026-09-04.md) — Steps 9 and 8
- [verification/2026-09-03-steps-5c-to-7.md](verification/2026-09-03-steps-5c-to-7.md) — Steps 7, 5c and 6
- [verification/2026-09-03-steps-4-to-6.md](verification/2026-09-03-steps-4-to-6.md) — Steps 6, 5 and 4
- [verification/2026-09-02-steps-0-to-3.md](verification/2026-09-02-steps-0-to-3.md) — Step 3 and,
  chained beneath it, every note before it

## The CrateStack adoption log, oldest first

- [cratestack/drift.md](cratestack/drift.md) — the measured migration/model drift
- [cratestack/2026-09-06-first-read.md](cratestack/2026-09-06-first-read.md)
- [cratestack/2026-09-06-outbox.md](cratestack/2026-09-06-outbox.md) — the transaction seam
- [cratestack/2026-09-06-first-writes.md](cratestack/2026-09-06-first-writes.md)
- [cratestack/2026-09-06-currencies-and-providers.md](cratestack/2026-09-06-currencies-and-providers.md) — migration 0032
- [cratestack/2026-09-06-providers-and-d7.md](cratestack/2026-09-06-providers-and-d7.md) — migration 0033, decision D7
- [cratestack/2026-09-06-customers.md](cratestack/2026-09-06-customers.md)
- [cratestack/2026-09-07-money-tables.md](cratestack/2026-09-07-money-tables.md) — migration 0037
- [cratestack/2026-09-11-search-payment-intents.md](cratestack/2026-09-11-search-payment-intents.md) — the first `procedure`
- [cratestack/2026-09-13-credentials.md](cratestack/2026-09-13-credentials.md) — migration 0044, `model Credential`
- [cratestack/2026-09-13-dashboard-procedure-transport.md](cratestack/2026-09-13-dashboard-procedure-transport.md) — the first transport, mounting it

## What is _not_ here

The declaration `cargo xtask verify-status` reads — the list of every
`ProviderError::NotImplemented` token in shipping code — is still on
[docs/status.md](../status.md), under the heading the gate looks up by name. So
are the banner, the table of areas and the gate table. Everything else on that
page came from here.
