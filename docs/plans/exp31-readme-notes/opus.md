# exp31 — README.md rewritten against what the repository can prove today

**Date: 2026-09-07. Branch `claude/exp31-readme-truth`, base `05ae51a`.**

The maintainer asked whether the README's banner — *"vpay cannot take a payment
yet … no HTTP call to any payment rail has ever been made by this code"* — was
still true. It was not, and it had not been since Step 3 (2026-09-03). This
page lists every claim that changed, what it changed to, and the artefact in
this tree that decided it. A claim removed with no replacement is listed too,
with the reason.

Nothing in `docs/status.md` moved except one dated row recording this pass.
`just verify` was green before and after.

---

## 1. The banner

| | |
|---|---|
| **Was** | "⚠️ **vpay cannot take a payment yet.** This repository is a **scaffold**. It compiles, lints clean and its tests pass, but **no HTTP call to any payment rail has ever been made by this code**." |
| **Is** | "⚠️ **vpay has never taken a real payment.**" — plus what works against stub rails, and what has never run. |
| **Evidence** | `docs/status.md` § Overall retired the "no HTTP call to any rail" sentence on 2026-09-03 (Step 3) and replaced it with the narrower **"no HTTP call to a *real* rail has ever been made"**, which every step since has restated unchanged. Step 4 made an intent reach `succeeded` with nobody touching it; Step 5 delivered a signed webhook; Step 9 put a real browser through the hosted and embedded page. |

The word **scaffold** is gone from the banner for the same reason: it now
understates the repository as badly as the old sentence overstated the rails.
It survives in one place only — the Layout block's note that the *dashboard app*
is a scaffold, which `docs/status.md`'s "Dashboard app" row still says verbatim
("Renders a scaffold notice and a status-badge reference. **No data, no auth,
no routes**", unchanged as of 2026-09-07).

**What the new banner claims, and what proves it:**

| Claim | Proof |
|---|---|
| a payment goes end to end against stub rails | `docs/status.md` § Overall (Step 4); `backends/tests/integration/tests/worker_e2e.rs`, `worker_recovery.rs` |
| `just demo` walks six payments on both rails | `examples/merchant-demo/src/main.rs` (`run_outcomes`, the `[4/6]` step); `docs/runbooks/demo.md` §4–5 |
| a real browser has driven the hosted and embedded page | `frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts` (3), `shop-embedded.cy.ts` (4); `docs/status.md` MVP item 6 — `just test-e2e` exit 0, 11 tests, 4 specs, 0 skipped |
| no HTTP call to MTN's or Orange's own endpoints | `docs/status.md` § Overall, every step's load-bearing sentence; `docs/runbooks/demo.md` "Status, stated before anything else" |
| no cluster has ever run vpay | `docs/flows/deployment.md` § Status — "**No cluster has ever run this — not a real one, not kind**" |
| no dashboard a person can use | `docs/status.md` "Dashboard app" row; `docs/flows/dashboard.md` § Status ("Not built: login, of any kind; … every page") |
| `/dash/v1` reads and staff sign-in answer over HTTP | `backends/tests/integration/tests/dashboard_read_surface.rs` (13 cases); `.../staff_sign_in.rs` (13 cases, ADR-0017); `vpay_api::dash::DASH_ROUTES` (two entries) |

---

## 2. Claims corrected, one by one

### 2.1 "nothing polls the charge, and no intent has ever reached `succeeded`"

**Removed.** False since Step 4 (2026-09-03). `vpay_worker::run_loop` claims
the `poll_charge` job the confirm committed in the same transaction as the
charge, asks the rail on the poll ladder, and commits charge, intent and one
event together (`docs/status.md` "Worker job loop" and "Settlement" rows;
`vpay_worker::handlers::poll_charge`). Replaced by the new banner's first
paragraph and by the "Both binaries call a payment rail" paragraph.

### 2.2 "`vpay-worker-bin` calls none, because it has no job loop" / "its job loop is not implemented, and it says so in a startup banner and a repeating heartbeat log line"

**Removed, both sentences.** `docs/status.md`'s "Worker job loop" row is
explicitly "the row that used to say *there is no job loop*". Replaced by a
paragraph naming `vpay_worker::run_loop`, the lease reaping at boot and on its
own timer, and the one-a-minute `job loop gauge` line — each of which that row
states.

### 2.3 The `/v1` route paragraph

**Was** a prose sentence naming four payment-intent paths plus `/v1/events`,
`/v1/account_holders` and `GET /v1/refunds/{id}`, with an embedded correction
about a claim that had been wrong since Step 5.

**Is** a table, transcribed from `vpay_api::v1::V1_ROUTES`
(`backends/crates/vpay-api/src/v1/mod.rs:188`), which the old prose did not
cover: **`/v1/checkout/sessions` (four methods) and `/v1/customers` (five) were
missing entirely.** The table names the boundary test that walks that same
constant — `every_registered_v1_path_answers_401_without_a_token`,
`backends/tests/integration/tests/payment_intents.rs:1037` — rather than
asserting the boundary in prose.

The self-correcting sentence ("This sentence named all three of `/v1/refunds`,
`/v1/events` and `/v1/balance` as unrouted, and had been wrong about
`/v1/events` since Step 5") is **dropped**: a README is not the place a
correction ledger lives, and `docs/status.md` and `vpay-api`'s own module docs
both carry it.

**Added, and it is the sharper claim:** `GET /v1/refunds/{id}` is a read with
no writer — *nothing in this repository creates a `refunds` row*. Source: the
`GET /v1/refunds/{id}` row in `docs/status.md` ("🟡, and it will stay 🟡 until
a refund can exist … Every row the four cases read is `INSERT`ed by the suite
itself"). The old README said only that creating one 404s, which let a reader
infer the read had something to read.

### 2.4 The SDK sentences

| Was | Is | Why |
|---|---|---|
| the Rust SDK "confirmed one and watched the intent move to `processing`. **It has still never taken a payment:** the rail behind that confirm is a WireMock container, nothing polls the charge, and no intent has ever reached `succeeded`" | the Rust SDK "is what `examples/merchant-demo` and `just demo` drive against a running `vpay-server`" | the two trailing clauses are 2.1; the blow-by-blow of which method landed on which day belongs in `docs/status.md` § Merchant SDKs |
| "The Node SDK is still tested only against stubs of the contract" | "**No test inside `sdks/nodejs` itself has ever spoken to a vpay** — every server in that package's own tests is a `node:http` stub; what has driven a live stack from Node is `sdks/stripe-compat` and `examples/shop`" | `docs/status.md` § Merchant SDKs states exactly this distinction, and the old wording contradicted the README's own preceding paragraph about `sdks/stripe-compat` driving a real stack in CI |
| the SDK matrix is "checked on every `just verify`" | "machine-checked **in both directions** on every `just verify`" | `docs/status.md`, "Updated 2026-09-06: that gate was one-directional, and now is not" |
| — (absent) | `sdks/stripe-js` (`@vaam-apps/vpay-stripe-js`) named | it is a shipped SDK directory the old README's Layout and prose both omitted |

The parenthetical correcting "no Stripe SDK can authenticate against vpay" is
**dropped** — it corrected a claim this README has not made since 2026-09-03,
and ADR-0010's amendment is where that record belongs. What replaces it is the
positive claim with its proof: `sdks/stripe-compat` drives the real `stripe`
package against a live stack in CI's `e2e (compose)` job, through to a
confirmed intent reaching `succeeded` and a webhook verifying with
`stripe.webhooks.constructEvent` (`docs/status.md`, the
`@vaam-apps/vpay-sdk/stripe` row: 25 cases, 0 skipped).

### 2.5 `schemas/*.cstack` "excluded from the build"

**Was:** `schemas/  *.cstack (syntax verified, design sketch, excluded from the build — see docs/status.md)`.

**Is:** compiled by `vpay-db` and gated by `just check-schema`, with the four
table families whose statements run through it named, and the honest remainder
("the rest is a design sketch a compiler now type-checks, and
`backends/migrations` remains the authoritative schema").

**Evidence:** the `schemas/*.cstack` row in `docs/status.md` — *"~~Content
remains a design sketch, excluded from the build graph~~ — corrected
2026-09-06: `vpay-db` compiles this file now (`mod schema` →
`include_server_schema!(…)`)"* — plus `justfile:673` (`check-schema`, in
`verify` since 2026-09-05, pinned at `cratestack_version := "0.12.0"`) and
`docs/flows/dashboard-auth.md` § Status for `staff_members`,
`staff_sessions`, `oauth_authorization_codes`.

### 2.6 The dashboard sentence

**Was:** "The dashboard is deliberately not started … per `docs/status.md` it
renders a static scaffold notice and makes no call to `vpay-server`, so there is
no data source that could show the six payments."

Half of that is still exactly right and is kept. What was **stale by omission**
is the reason: `docs/runbooks/demo.md` §6 gives it as "`/dash/v1` does not
exist (Phase 2b, not started)", and `/dash/v1` has existed since 2026-09-06 and
staff sign-in since 2026-09-07. The README now says the app has no pages and no
login while the two `/dash/v1` reads and the sign-in routes *do* answer, naming
the two integration suites that prove them. It also says "the one service of
the demo file set that stays down" rather than "not started", because `just
demo-up` now brings up **eight** services and the dashboard is the single
exclusion (`justfile`'s `demo-up` comment; `demo_services`).

### 2.7 The demo walkthrough

| Was | Is | Source |
|---|---|---|
| "brings up Postgres + both WireMock rail stubs + the merchant webhook receiver + `vpay-server` + `vpay-worker`" (five) | eight services, `vpay-checkout` and `vpay-shop` added | `justfile` `demo_services` and `demo-up`'s own comment ("It said six until 2026-09-04") |
| "four steps, the last of which is a table" | six steps, the **fourth** of which is the table | `examples/merchant-demo/src/main.rs` prints `[1/6]`…`[6/6]`; steps 5 (checkout sessions) and 6 (`/v1/account_holders`) were absent from the README |
| three `just` variables | six — `demo_project`, `demo_port`, `demo_receiver_port`, `demo_orange_port`, `demo_checkout_port`, `demo_shop_port` | `justfile` variable block, 1761–1890 |
| "two demos can run on one machine at once" (three paragraphs) | one paragraph, same claim | length; `docs/runbooks/demo.md` §7 carries the measured detail |

### 2.8 The testing table

| Was | Is | Source |
|---|---|---|
| `just verify` runs "the three self-checks" and needs "nothing but Rust" | **eleven gates and one advisory report**, and it needs the pinned `cratestack` CLI on `PATH` as well as Rust | `justfile:502` (`verify:` lists eleven prerequisites and echoes "the eleven gates above passed"); AGENTS.md says ten and predates `verify-ui`; `justfile:673` `check-schema` **fails** rather than skips without the CLI |
| `just test` runs "`cargo nextest run --workspace` + `pnpm -r test`" | those two plus `cargo test --doc --workspace` | `justfile:89` — `test: test-rust test-doc test-web` |
| `just test-e2e` boots "`compose.yml` + `compose.e2e.yml`" | those two **plus `compose.demo.yml`** | `justfile:197` `test-e2e` — it depends on `gen-demo-keys` and uses `demo_compose`; MVP item 6 records why (without the demo overlay no merchant is registered and every spec answered `invalid_client`) |
| `just ci` "runs everything CI runs, in CI's order" | CI's self-checks, `rust`, `web` and supply-chain steps, in CI's order — **not** `e2e (compose)` or `deploy (helm chart)` | `justfile:1437` against `.github/workflows/ci.yml`'s six jobs. The justfile's own comment makes the same overstatement and was left alone; it is out of this pass's scope |
| — (absent) | `just verify-ignored`: **0 ignored, 46 test binaries, 1550 tests listed** | **measured on this tree**, `just verify-ignored`, 2026-09-07. The old README asserted the 0 without the other two numbers the recipe also pins |

### 2.9 The Stack section

**Was:** "Design system on Tailwind + daisyUI + `class-variance-authority` +
Headless UI, with framer-motion and vaul for motion and sheets. Storybook with
the a11y addon."

**Is:** "Tailwind 4 + daisyUI 5 (`bumblebee`) + `class-variance-authority` +
`@base-ui/react`. Storybook 10 with the a11y addon."

**Evidence:** `grep -rn 'headlessui\|framer-motion\|vaul' --include=package.json`
over the whole repository returns **nothing** — none of the three is a
dependency anywhere. `frontends/packages/ui/package.json` declares
`@base-ui/react` 1.8.0, `tailwindcss` 4.3.3, `daisyui` 5.7.28 and `storybook`
10.6.0. `docs/status.md`'s `@vpay/ui` and Storybook rows say the same.
(AGENTS.md's "TypeScript conventions" still names Headless UI; that is a
separate stale claim, recorded in §4 below and not fixed here.)

### 2.10 The Layout block

Corrected against `ls`:

- `backends/crates/` was missing **`vpay-db`**.
- `backends/tests/` was missing **`webhook-receiver`**.
- `frontends/apps/` was missing **`checkout`** — the hosted payment page, the
  most visible thing Step 9 built.
- `sdks/` was missing **`stripe-js`** and **`stripe-compat`**.
- `examples/` was missing **`shop`**, **`checkout-browser`** and
  **`merchant-stripe-node`**.
- `docs/` was missing **`reference/`**, **`sdks/`** and **`plans/`**.
- **`deploy/`** was absent altogether; it is now listed with the claim
  `docs/flows/deployment.md` § Status makes for it — rendered and
  schema-validated, never applied to a cluster.

### 2.11 Smaller corrections

- **"the two `/v1` methods an SDK can name and vpay does not serve"** — kept,
  and the reason sharpened from "creating a refund will keep doing so until a
  rail can refund" to name both halves: `mtn_momo::refund` is the one remaining
  `NotImplemented` token in the workspace (`just verify-status` on this tree:
  "1 unimplemented item(s)"), and `orange_money` answers `Unsupported`, a
  permanent capability answer rather than unbuilt work
  (`docs/flows/adapter-orange-money.md` § Status).
- **`--public-base-url`** — the four-sentence account of its removal is one
  sentence now. The fact is unchanged and still stated, because a deployment
  that sets the flag fails to start.
- **`/metrics`** — "Nothing has ever scraped `/metrics`" kept verbatim; it is
  what `docs/status.md`'s "`/metrics` scraped by a Prometheus" row says.
- **The runtime-image note** (the 2026-09-02 `FROM scratch` trust-store bug,
  fixed the same day with vendored `webpki-roots`) — **removed.** It is true
  (`vpay_api::http_client` is a re-export of `vpay_provider::http`, and
  `backends/crates/vpay-api/Cargo.toml:68` records the move) but it is a closed
  historical defect, and `docs/status.md`'s "Resource-server JWT validation"
  row is where it belongs. Removed for length, not because it is false.
- **`musl target` gotcha** — the sentence "the Dockerfiles themselves have not
  been built in this repo's own development environment" is **removed**: it is
  contradicted by `docs/status.md`'s "Docker / compose — made bootable, proven
  by CI run `33647189156`" section and by every `just demo` run, which builds
  them. Replaced by a link to ADR-0014.
- **A stale `pgdata` volume** — **added** as a fourth environment gotcha. It is
  the failure a reader is most likely to hit on a machine that ran the demo
  before Step 9 (`docs/runbooks/demo.md`, "Its database is created once, on a
  fresh volume").
- **Prerequisites** — "`pnpm` is needed only if you want to work on the
  dashboard; the demo does not start it" is now "`pnpm` is needed only to work
  on the web packages; `just demo` builds every image it needs in Docker".
  `gen-demo-keys` checks for `cargo` and `jq` only; `demo-up` for `docker` and
  `curl`; the checkout and shop images are built by `docker compose --build`.

---

## 3. What was left alone deliberately

- **`docs/status.md`'s substance.** One dated row was appended to the § Overall
  banner recording this README pass, as the brief permits. Nothing else moved —
  in particular the MVP list's item 7, which still reads "**Nothing here has
  ever performed a login**" and has been stale since ADR-0017 landed on
  2026-09-07. Correcting it is a status-page change, not a README one.
- **The demo's own module doc** (`examples/merchant-demo/src/main.rs`) says
  "Five steps" and prints six. Not this pass's file.
- **`docs/runbooks/demo.md` §6** gives "`/dash/v1` does not exist (Phase 2b,
  not started)" as the reason the dashboard stays down. Stale since 2026-09-06.
  The README no longer repeats it; the runbook is not fixed here.
- **`docs/flows/README.md`**'s table omits `dashboard.md` and `customers.md`.
- **AGENTS.md** says `just verify` is "**ten** gates" (it is eleven since
  `verify-ui`) and names Headless UI under TypeScript conventions (not a
  dependency anywhere).
- **`justfile`'s `ci` comment** — "Everything CI runs, in CI's order" — is the
  overstatement the README used to repeat.

Each of the six is a real stale claim in a file this brief did not open. None
was corrected, so that this pass's diff is one file plus its record.

---

## 4. Gates

Run on this branch, on the final head, with `CARGO_BUILD_JOBS=4` and the
`.nvmrc` Node (`v22.23.2`):

| Gate | Result |
|---|---|
| `just verify` | **ok** — "the eleven gates above passed; the verify-docs report is advisory" |
| `cargo xtask verify-links` | ok — 946 repository link(s) in 174 tracked markdown file(s) resolve to a tracked path |
| `cargo xtask verify-status` | ok — 1 unimplemented item(s), all declared and all still in shipping code |
| `cargo xtask verify-no-mocks` | ok — no test double reachable from a shipping binary |
| `just verify-ignored` | 0 ignored (expected 0), 46 test binaries (expected 46), 1550 total (minimum 1080) |
| `cargo xtask verify-citations` | **not run** — it needs the network and a GitHub token, and this pass added no CI-run, PR or issue citation to any tracked document |

`just verify` was green on the base commit before any edit, so its green here
is a *no regression* result and not evidence about the README's content. What
is evidence about the content is §2's table of sources, every row of which
names a file in this tree.
