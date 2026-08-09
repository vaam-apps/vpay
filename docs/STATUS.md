# STATUS

**What actually works today.** This page is the contract behind the repo's second
rule: *never advertise a feature as done when it clearly is not.*

It is machine-checked. `cargo xtask verify-status` scans the workspace for every
`ProviderError::NotImplemented("…")` token and fails the build if one is missing
from this file. You cannot quietly ship an unimplemented path.

Last verified: `just verify` on the commit that introduced this file.

---

## Legend

| Marker | Meaning |
|---|---|
| ✅ **Done** | Implemented, tested, and the tests actually assert behaviour |
| 🟡 **Partial** | Some of it is real. The rest is listed explicitly below |
| ⛔ **Not started** | No implementation. Calls return `NotImplemented`; tests are `#[ignore]`d with a reason |

Nothing in this repo is ✅ unless a test would fail if it broke.

---

## Overall

> **vpay is a scaffold.** It compiles, lints clean, and its tests pass — but it
> cannot take a payment. No HTTP call to any rail has ever been made by this
> code. Do not deploy it.

---

## Backend

| Area | Status | Notes |
|---|---|---|
| Workspace, edition 2024, resolver 3 | ✅ | `cargo check --workspace --all-targets` clean |
| Lint policy (no `unwrap`/`expect`/`panic`/float in prod) | ✅ | `cargo clippy -- -D warnings` clean; tests exempted via `clippy.toml` |
| `Money` — integer minor units, XAF zero-decimal | ✅ | 6 tests incl. cross-currency and over-refund rejection |
| Canonical failure taxonomy | ✅ | 3 tests |
| Charge / intent state + `ProviderFlow` | ✅ | 3 tests incl. live-xor-terminal exhaustiveness |
| Ledger balancing invariant | 🟡 | Types and `validate()` done + 3 tests. **Persistence not started** |
| Config guard rails (stub host, literal secret) | 🟡 | Rules done + 5 tests. **YAML loading and DB reconciliation not started** |
| Provider port trait | ✅ | Interface defined; both adapters implement it |
| Poll ladder | 🟡 | `poll_delay` done + 3 tests. **Job loop not started** |
| HTTP surface | 🟡 | Only `/healthz` and the Stripe-shaped 404. **No `/v1/*` route exists** |
| Database schema / migrations | ⛔ | Designed in the design doc; no SQL in this repo yet |
| Webhooks (signing, outbox, delivery) | ⛔ | |
| Idempotency | ⛔ | |
| Reconciler | ⛔ | |

### Unimplemented items tracked by `verify-status`

Every token below appears verbatim in the source. Removing an item here without
removing it from the code fails CI, and vice versa.

- `mtn_momo::submit`
- `mtn_momo::query_status`
- `mtn_momo::parse_callback`
- `mtn_momo::refund`
- `orange_money::submit`
- `orange_money::query_status`
- `orange_money::parse_callback`
- `orange_money::refund`

### Adapters

| Rail | Capabilities | Wire calls |
|---|---|---|
| `mtn_momo` (push) | ✅ declared and tested | ⛔ not started |
| `orange_money` (redirect) | ✅ declared and tested | ⛔ not started |

Capabilities being real matters more than it sounds: `orange_money` declares
`supports_refunds: false`, and that flag — not a rail-specific branch — is what
makes the core refuse a refund on that rail.

---

## Frontend

| Area | Status | Notes |
|---|---|---|
| pnpm workspace, TS strict | ✅ | `pnpm -r typecheck` clean |
| `@vpay/tokens` status tokens | ✅ | 3 tests incl. "success tone belongs to `succeeded` alone" |
| `@vpay/ui` `StatusBadge` (cva + daisyUI) | ✅ | 3 tests |
| `@vpay/ui` `PayerSheet` (vaul + framer-motion) | 🟡 | Renders; **no test** — needs interaction coverage |
| Storybook | 🟡 | Configured with a11y addon; **only `StatusBadge` has stories** |
| `@vpay/api-client` | 🟡 | `formatAmount` done + 4 tests. **Every network call throws `NotImplementedError`** |
| Dashboard app | 🟡 | Renders a scaffold notice and a design-system smoke test. **No data, no auth, no routes** |
| Cypress e2e | 🟡 | 3 specs written against the compose stack. **Never executed in CI here** — the Cypress binary CDN is unreachable from the authoring sandbox; run `pnpm exec cypress install` locally first |

---

## Infrastructure

| Area | Status | Notes |
|---|---|---|
| `compose.yml` (Postgres + 2 WireMock rails) | 🟡 | Written; **not started in this sandbox — Docker unavailable** |
| `compose.e2e.yml` (full stack) | 🟡 | Written; **never run** |
| `backends/Dockerfile` (musl → scratch) | 🟡 | Written; **never built** |
| `frontends/Dockerfile` | 🟡 | Written; **never built** |
| `deny.toml` | 🟡 | Policy written incl. an openssl ban; **`cargo deny` not run here** |
| GitHub Actions | 🟡 | Workflow written; **never executed** |
| `schemas/*.cstack` | ⛔ | **Syntax unverified** — see below |

### CrateStack

`schemas/vpay.cstack` is written from the framework's public overview, which
describes `.cstack` as the single source of truth for models, procedures, auth
and policies. **The actual grammar is not publicly documented** — `cratestack.dev/docs`
returns 404 and no authoritative reference was found.

Consequently the file is:

- marked `STATUS: UNVERIFIED` in its own header,
- **excluded from the build graph** — no crate depends on it, no macro consumes it,
- treated as a design sketch to be rewritten once the real grammar is known.

It is here because the repo layout asked for it, not because it works.

---

## What would have to be true to call this "an MVP"

1. Database schema + migrations, with the `one_charge_per_intent` unique index.
2. Both adapters making real HTTP calls, passing the shared conformance suite
   with the `#[ignore]`s removed.
3. The worker's job loop, poll ladder and reconciler, with crash tests.
4. `/v1/payment_intents` create + confirm, form-encoded, with idempotency.
5. Signed webhooks with the two-step outbox.
6. `just test-e2e` green against the compose stack.

Until every one of those is ✅, this README's own claim is: **it does not take payments.**
