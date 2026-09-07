# exp31 — sabotage review of the README truth pass

**Date: 2026-09-07. Branch `claude/exp31-readme-truth`.** Reviewed at
`045db7e` (base `05ae51a`), then **rebased onto `e58862a`** — master with
`verify-migrations`, PR #84, merged 2026-09-07 15:39 UTC — and re-measured
there.

Reviewer's brief: make the README lie in neither direction. An overstated
capability and an understated one are the same defect, and the maintainer's
question ("is *vpay cannot take a payment yet* still true?") is answered
wrongly by both.

Method: read the new README as a sceptical merchant, and for every sentence
asserting a capability, a number, a path, a command or a gate, check it
against this tree by running the command, opening the file or counting the
thing. Nothing below is taken from `docs/status.md` where the code could be
read instead; where the two disagree, the code is the finding.

---

## 1. Findings

Severity: **misleading-claim** (the README says something a reader would act
on that the tree does not support, in either direction) · **correctness** (a
wrong fact that misleads less) · **nit**.

| # | Severity | Claim | What the tree says | Fixed in |
|---|---|---|---|---|
| F1 | misleading-claim | "**Four** table families — `disabled_clients`, `staff_members`, `staff_sessions`, `oauth_authorization_codes` — run their statements through it; the rest is a design sketch" | **Nine** of the thirteen models in `schemas/vpay.cstack` carry production statements. The omitted three are `customers`, `events` and `webhook_deliveries` — the personal-data object and both halves of the webhook outbox | `ace3ff9` |
| F2 | misleading-claim | "`--config`, `--database-url` and `--oauth-signing-key-file` are required … a missing one exits `78` before the port is bound" | Measured: `78` for `--config` and `--oauth-signing-key-file`, **`1`** for `--database-url`, on **both** binaries. And `vpay-worker-bin` does not accept `--oauth-signing-key-file` at all | `a0c8d71` |
| F3 | misleading-claim | "`just verify-ignored` … **1550 tests listed**; the recipe fails if **any of the three** moves" | The total is a **floor** (`-lt "{{min_tests}}"`, 1080), not a pin; only `expected_ignored` and `expected_suites` are exact. Demonstrated: the total moved to **1563** on the rebase and the recipe exited 0 | `d943f4c` |
| F4 | correctness | MTN's outcome is "selected by the payer's MSISDN (a documentation number in the `2376000000xx` block)" | The three MSISDNs are `237600000ce0`, `237600000f01`, `237600000f02` — hex steering codes, and the walkthrough's own step 6 shows one refused with a `400` for not being a Cameroon E.164 number | `27be744` |
| F5 | correctness | "eleven gates and one advisory report" | Eleven was true at `045db7e` and false four hours later. Rebased onto `e58862a`, `just verify` echoes **twelve**; the sentence now takes its count from the recipe and dates it | `da9ed64`, `7a85ca0` |
| F6 | correctness | "**Frontend** — Next.js 15" | `frontends/apps/*` are `^15.5.25`; `examples/shop`, the app `just demo` builds and a browser opens, is `16.3.4` | `bc3225c` |
| F7 | misleading-claim | `docs/status.md` MVP item 7: "**Nothing here has ever performed a login**", plus a paragraph of reasons it could not be closed by wiring | Seven `/dash/v1` sign-in routes are mounted and `staff_sign_in.rs` drives 13 cases in which every token came from `POST /dash/v1/oauth/token` after a password, a TOTP code and a PKCE exchange (ADR-0017, 2026-09-07) | `5e620f9` |
| F8 | misleading-claim | `docs/flows/dashboard.md` § Status: "**Not built:** login, of any kind" | Same. True of the *app*, false as written | `5e620f9` |
| F9 | correctness | `docs/runbooks/demo.md` §6: "`/dash/v1` does not exist (Phase 2b, not started)" | `DASH_ROUTES` has carried two reads since 2026-09-06 | `b270275` |
| F10 | correctness | `docs/flows/README.md`'s index | Omits `dashboard.md` and `customers.md` — 19 of 21 flow documents listed | `1d14278` |
| F11 | correctness | `AGENTS.md`: "Headless UI for behaviour, framer-motion for motion, vaul for sheets" | None of the three is a dependency of any `package.json` in the tree | `3a44c73` |
| F12 | correctness | `AGENTS.md` **on master after #84**: "`just verify` is **twelve** gates" followed by a parenthetical naming **eleven** | `verify-ui` is missing from that list. Fixed in the rebase resolution | `3a44c73` |
| F13 | correctness | `.xtask/src/main.rs:3` "ten gates"; `justfile:440` "the **eleven** gates passed"; `justfile:1532` "# Everything CI runs, in CI's order" | Twelve, twelve, and `just ci` covers four of CI's six jobs. The first two are literals #84 did not sweep — the justfile said "eleven" eight lines above a recipe echoing "twelve" | `3a44c73` |
| F14 | correctness | `docs/plans/exp17-notes/opus.md:161` cites `#228` with an `issue` cue | Resolved against `vaam-apps/vpay`, where it 404s. It is `cratestack/cratestack#228`. `just docs-check-citations` failed on this tree before this branch existed | `e323669` |

F13's `.xtask` and `CLAUDE.md` entries are claims the implementer's notes did
not list. They are the same defect as the four items that were listed, they
were wrong *before* #84 rather than after it, and one of them is in the file
every agent reads first.

### The three that matter most

**F1 is an understatement, and this pass exists partly because
understatement misleads too.** A reader who believes only four bookkeeping
tables run through CrateStack concludes the generated layer is a curiosity.
It is not: the outbox writes through it, and so does `customers`. That
changes what a schema edit costs. Measured by classifying every generated
builder call in `backends/crates/vpay-db/src/` against each file's first
`#[cfg(test)]`:

```
currencies                config_reconcile.rs:306,364
providers                 config_reconcile.rs:454
disabled_clients          disabled_clients.rs:160,216,257
customers                 customers.rs:803,829
events                    webhook_deliveries.rs:299
webhook_deliveries        webhook_deliveries.rs:230
staff_members             staff.rs:398,436,449,469,498,522,541
staff_sessions            staff_sessions.rs:277,299,318,336,360,378
oauth_authorization_codes authorization_codes.rs:162,192,206
```

The four with no production statement — `payment_intents`, `charges`,
`ledger_transactions`, `ledger_entries` — are now named in the README, which
is the direction that can decay silently.

`docs/status.md`'s `schemas/*.cstack` row still says "**one** of them —
`DisabledClient` — has queries running through it … The other six models are
still a design sketch". That sentence was written when the file had seven
models; it has thirteen. **Not corrected here** — see §5.

**F2 is the one an operator would be burned by.** Exit `78` is `EX_CONFIG` —
"fix your configuration". Exit `1` is `Category::Internal` — "this process is
broken". A restart policy or an alert routing rule written on the README's
sentence sends a forgotten `DATABASE_URL` to the wrong pager. Measured, each
run with the other two flags present and a valid config:

```
vpay-server      --config missing                exit 78
vpay-server      --oauth-signing-key-file missing exit 78
vpay-server      --database-url missing          exit  1
vpay-worker-bin  --config missing                exit 78
vpay-worker-bin  --database-url missing          exit  1
```

`backends/apps/vpay-server/src/main.rs:346` requires the flag with
`.context(…)`, producing a bare `anyhow::Error`; `exit_code_for` finds no
typed leaf in the chain and falls through to `Category::Internal`.
`docs/status.md`'s CLI row has recorded the gap since Step 1, so the README
was contradicting its own status page. **The code gap is not fixed here** —
that is a change to two `main`s plus subprocess tests, not a README pass.

**F3 was found by the rebase rather than by reading.** The README asserted
that all three of `verify-ignored`'s numbers are pinned. The rebase moved the
total from 1550 to 1563 and the recipe exited 0, printing "1563 total
(minimum 1080)" — so the assertion was falsified by the same event that made
the number stale. `ignored` and `suites` are `-ne` comparisons; the total is
`-lt` against a floor the justfile deliberately keeps under the count "so it
is not a number people bump reflexively".

---

## 2. What was checked and holds

Everything below was verified against this tree and needed no change. It is
listed because "the reviewer read it and it was fine" is otherwise
indistinguishable from "the reviewer did not read it".

| Claim | How it was checked |
|---|---|
| The `/v1` table | Transcribed `vpay_api::v1::V1_ROUTES` (`v1/mod.rs:188`) — 13 `V1Route` entries. Every one is in the table and the table invents none. `/v1/customers/{id}` really is `GET`/`POST`/`DELETE`, the only three-method path |
| `POST /v1/refunds` and `GET /v1/balance` routed nowhere | Neither is in `V1_ROUTES`; `v1/mod.rs:170` states the `/v1/balance` exclusion in as many words |
| "**nothing in this repository creates a `refunds` row**" | `grep -ni 'insert into refunds'` over the tree returns four hits, all under `backends/tests/` |
| `mtn_momo::refund` is the one `NotImplemented`, `orange_money` answers `supports_refunds: false` | `cargo xtask verify-status`: "1 unimplemented item(s)". `adapter-mtn-momo/src/lib.rs:692`, `adapter-orange-money/src/lib.rs:327`. MTN's `supports_refunds` stays `true`, and the README does not say otherwise |
| The boundary test walks the constant | `every_registered_v1_path_answers_401_without_a_token`, `payment_intents.rs:1037` |
| `/dash/v1`'s "two read routes" | `dash::DASH_ROUTES` has exactly two entries, both `GET` on payment intents |
| Eight demo services | `demo_services` names eight, and the measured run started exactly those eight |
| Six `just` variables for a second demo | `demo_project`, `demo_port`, `demo_receiver_port`, `demo_orange_port`, `demo_checkout_port`, `demo_shop_port` are the whole of the `demo_*` block. There is no `demo_mtn_port` and the README does not claim one — `compose.demo.yml` `!reset`s the MTN stub's publication |
| Six steps, the fourth a table | The measured run printed `[1/6]`…`[6/6]`, and `[4/6]` is the payments table |
| `just test-e2e`: four specs, 11 tests | `ls cypress/e2e` → 4 specs; `it(` counts 1 + 3 + 4 + 3 = 11. The two `cypress run`s (`e2e:default`, `e2e:framed`) are **disjoint** — `specPattern`/`excludeSpecPattern` split `shop-embedded.cy.ts` off — so 11 is a total, not a half |
| `just ci` covers four of CI's six jobs | `ci: fmt-check clippy verify test-rust test-doc verify-ignored lint-web test-web deny` against `.github/workflows/ci.yml`'s `verify` / `rust` / `deny` / `web` / `e2e (compose)` / `deploy (helm chart)` |
| No `headlessui`, `framer-motion` or `vaul` | `git grep` over every `package.json`: nothing. `@base-ui/react` 1.8.0, `tailwindcss` 4.3.3, `daisyui` 5.7.28, `storybook` 10.6.0, react 19, cva ^0.7.1 all confirmed. Theme `bumblebee` confirmed in four files |
| The layout block | `ls` on every directory it names. `backends/crates` (10), `backends/apps` (2), `backends/tests` (3), `frontends/packages` (4), `frontends/apps` (2), `sdks` (4), `examples` (7), `docs`, `deploy/helm/vpay`, `.xtask` — all present, none invented |
| Prerequisites (Docker, Rust, `just`, `jq`, `curl`, `openssl`) | The four recipes' own `command -v` lists: `gen-demo-keys` (cargo, jq), `gen-e2e-signing-key` (openssl), `demo-up` (docker, curl), `demo-walk` (cargo, curl). Exactly the set the README names |
| `just up`, `just install`, `just` with no argument | `compose.yml` has three services (postgres, wiremock-mtn, wiremock-orange); `install: install-rust install-node`; `default: @just --list` |
| The XAF paragraph | `config/application.yml:39` `currency: EUR` for `mtn_momo` with the "MTN's sandbox rejects XAF" comment; `application-sandbox.yml` declares no `currency`, so it inherits; the generated `.e2e/application-demo.yml` puts **both** rails on XAF and says why; `payment_intents.rs:1541` is the confirm-time comparison |
| `--public-base-url` removed 2026-09-03 | `git log -S` on `vpay-config/src/cli.rs` → `7d62751`, dated `2026-09-03` |
| The worker paragraph | `run_loop.rs:60` `GAUGE_INTERVAL = 60s`, the `job loop gauge` line at `:980`, lease reaping at boot and on a timer |
| `--observability-bind` default `0.0.0.0:9090`, `/livez` + `/metrics` off the traffic port | `cli.rs:140`; `observability.rs`'s module docs and `neither_livez_nor_metrics_is_reachable_on_the_traffic_router` |
| `Idempotency-Key` required on every `POST` | `idempotency.rs:117` |
| The `stripe-compat` sentence | `lifecycle.compat.test.ts:192` "settles to succeeded, because the worker polls the rail"; `webhooks.compat.test.ts:194` "verifies with stripe.webhooks.constructEvent, and rejects a tampered body"; both under CI's `e2e (compose)` job |
| "No test inside `sdks/nodejs` itself has ever spoken to a vpay" | Nine `*.test.ts` files; the only server any of them starts is `src/testing/test-server.ts`, `createServer` from `node:http` |
| SDK parity "machine-checked in both directions" | `docs/sdks/parity.md` § "The gate reads this file **and** the SDKs, since 2026-09-06"; the gate reported 407 proving tests over 23 rows |
| rustls only, mimalloc, musl → scratch | `deny.toml:150-152` bans `openssl`, `openssl-sys`, `native-tls`; `MiMalloc` in both `main.rs`; `verify-toolchain` confirms the Dockerfile pin |
| The banner's "what has never happened" | `docs/flows/deployment.md:415` "No cluster has ever run this — not a real one, not kind"; `docs/flows/dashboard.md` § Status for the pages; no `/metrics` scrape |

---

## 3. The demo, measured

Run at `045db7e`, on an isolated project and free ports, with the user's own
`vpay-demo` stack left untouched:

```
just demo_project=exp31-review demo_port=18080 demo_receiver_port=18083 \
     demo_orange_port=18082 demo_checkout_port=13080 demo_shop_port=13001 demo
```

**Exit 0.** Eight containers started and reported healthy —
`postgres`, `wiremock-mtn`, `wiremock-orange`, `wiremock-webhook`,
`vpay-server`, `vpay-worker`, `vpay-checkout`, `vpay-shop` — which is
`demo_services` exactly, and the dashboard was not among them.

Six steps printed, `[4/6]` the payments table. Every row of the README's
table reproduced:

| # | Rail | Outcome | `last_payment_error.code` | Event |
|---|---|---|---|---|
| 1 | `mtn_momo` | `succeeded` after **7 polls** | — | `payment_intent.succeeded` |
| 2 | `mtn_momo` | `requires_payment_method` | `insufficient_funds` | `payment_intent.payment_failed` |
| 3 | `mtn_momo` | `requires_payment_method` | `payer_timeout` | `payment_intent.payment_failed` |
| 4 | `orange_money` | `requires_action` → `succeeded` | — | `payment_intent.succeeded` |
| 5 | `orange_money` | `requires_payment_method` | `payer_timeout` | `payment_intent.payment_failed` |
| 6 | `orange_money` | `requires_payment_method` | `provider_error` | `payment_intent.payment_failed` |

Closing line: *"all six steps behaved as expected — 6 payments on 2 rails,
every one settled by the worker asking the rail and evidenced by a signed
webhook"*. Steps 5 and 6 (two Checkout Sessions, both left `open` and
unpaid; three account-holder lookups) also matched the README.

So **"six payments on both rails" is measured, not transcribed** — and F4 is
the one detail of that paragraph the run contradicted.

`just demo_project=exp31-review demo-down` afterwards: containers, volume and
network removed; `docker compose ls` shows `vpay-demo` still `running(8)`.

The demo was not re-run after the rebase: PR #84 touches migrations, the
manifest gate and `postgres_smoke.rs`, none of which is on the walkthrough's
path, and `verify-migrations` passes on the rebased head.

---

## 4. Gates, on the final head

`.nvmrc` Node, `CARGO_BUILD_JOBS=4`, rootless `DOCKER_HOST`, on the rebased
branch.

| Gate | Result |
|---|---|
| `just verify` | **exit 0** — "the **twelve** gates above passed; the verify-docs report is advisory" |
| `cargo xtask verify-links` | ok — 961 repository link(s) in 178 tracked markdown file(s) resolve |
| `cargo xtask verify-status` | ok — 1 unimplemented item, declared and still in shipping code |
| `cargo xtask verify-no-mocks` | ok |
| `cargo xtask verify-sdk-parity` | ok — 407 proving tests, 35 dated gaps, 19 methods over 23 rows |
| `cargo xtask verify-migrations` | ok — 35 migration files all match `MANIFEST.sha256` |
| `just verify-ignored` | 0 ignored (expected 0), 46 test binaries (expected 46), **1563** total (minimum 1080) |
| `just docs-check-citations` | **ok — exit 0**, 55 unique ids over 178 files, 0 MISS. It exited 1 with 1 MISS before F14's fix, which is the mutation evidence that the gate still checks bare ids |
| `just --list`, `just --evaluate` | parse (the justfile edits are comments) |

One environment note, not a regression: `check-schema` prints
`WARNING — cratestack 0.11.1 on PATH, this repository pins 0.12.0` and then
passes on 0.11.1. That is the recipe's documented behaviour on an authoring
machine (exp9 review, F9); CI installs the pin.

`verify-ui` prints nothing on success — it is `exit $fail` — which is why the
README's new sentence anchors on the `verify` recipe's own echo rather than
on a per-gate line.

---

## 5. Left alone, and why

- **`docs/status.md`'s `schemas/*.cstack` row** ("**one** of them —
  `DisabledClient` … The other six models"). Wrong in the same direction as
  F1 and by more, but rewriting it means reconciling it with three later
  sections that narrate what superseded it ("The first CrateStack read",
  "The first CrateStack writes", "The outbox through CrateStack"). Whether
  that row is rewritten or struck with a dated correction is a maintainer
  call about the status page's own idiom, not a README pass's.
- **The `--database-url` exit-code gap itself.** A typed `StartupError`
  variant in both `main`s plus a subprocess test each. The README now states
  the behaviour instead of asserting the contract.
- **`examples/merchant-demo/src/main.rs`.** Two stale in-code claims found
  while checking F4: `Steering::Msisdn`'s doc comment says "a documentation
  number in the `2376000000xx` block" (the values below it are not), and
  `ACCOUNT_HOLDER_CASES`' says "`237600000100` is the same number step 4's
  *settling* MTN outcome uses" (step 4's settling outcome uses
  `237600000ce0`). Source changes in a file this brief did not open; the
  README no longer repeats either.
- **`just test-e2e`.** Not re-run. The claim it backs — a real browser has
  driven the hosted and the embedded page — rests on the four specs being
  present and on two independent records of a full green run on this tree
  (`docs/status.md` MVP item 6; `docs/flows/dashboard.md`'s Lane D entry,
  2026-09-07, "11/11 across all four specs"). Re-running it needs a second
  browser stack and `compose.e2e.yml`'s hard-coded `:3000` (issue #78).
- **`just ci` end to end.** `just verify`, `just verify-ignored` and
  `just docs-check-citations` were run individually on the final head; the
  full `ci` sweep (clippy, `test-rust`, `test-web`, `deny`) was not.

---

## 6. Verdict

**Is the README now exactly true? Yes, for every sentence this review could
check** — with three qualifications stated rather than buried:

1. One claim rests on records rather than on a run this review performed:
   "a real browser has driven the hosted and the embedded checkout page".
2. The README points at `docs/status.md` § CrateStack for a fuller account,
   and that section's `schemas/*.cstack` row understates the same thing F1
   corrected. The README does not repeat it, but a reader following the link
   lands on it.
3. Two of the README's numbers are dated measurements rather than enforced
   pins — the gate count and the test total — and both now say so in the
   sentence that carries them. That is the durable form: the next gate and
   the next test leave them stale rather than wrong, and a reader is told
   which of the three `verify-ignored` numbers a green run actually pins.

The banner's own answer to the maintainer's question is sound in both
directions: a payment does go end to end against stub rails and `just demo`
proves six of them on demand, and no HTTP call to MTN's or Orange's own
endpoints has ever been made, no cluster has run vpay, and there is no
dashboard a person can use.
