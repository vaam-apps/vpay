# Step 9, lane r2 — remediating the second review round

**Date:** 2026-09-04 · **Branch:** `claude/step9-remediate-r2` · **Base:**
`6abbaa0` (the Step 9 gate).

Nine findings from the r2 review of the Step 9 gate. Seven were remediated
here, one is informational and recorded rather than changed, and two are
deferred to owners who are not this lane. **Every evidence line was
re-verified against the tree before anything was edited**; two of them were
not quite right as filed, and this note says so.

One commit per finding, each naming it.

---

## What landed

| # | Commit | What it was | What it is now |
|---|---|---|---|
| 2 | `cfdc610` | `README.md:211-215` said the MTN intents are EUR and the Orange ones XAF | Mirrors `docs/runbooks/demo.md` §"One currency, and what it is not saying": both rails settle XAF **in the demo overlay only**, MTN's real sandbox rejects XAF, `config/application.yml` keeps `mtn_momo` on EUR |
| 3 | `ec0b348` | `examples/shop/src/server/store/types.ts:13-14`, `prisma-store.ts:5-7` and `examples/shop/README.md:144` said `PrismaShopStore` "is exercised by `just demo` and lane 6's Cypress specs" | All three say what is true: verified by hand on 2026-09-04 against a real Postgres, **no automated test covers it**, and the Cypress proof is lane 6's and gets recorded in `docs/status.md` when it lands |
| 4 | `a5a5f76` | `frontends/apps/checkout/src/testing/fixtures.ts:5-7` claimed nothing under `src/testing` is imported from `app/` or a component, while `src/components/screen-states.ts:12` imported `../testing/fixtures` | `screen-states.ts` **moved** to `src/testing/`, and a real guard ported from the shop: `src/testing/no-runtime-imports.test.ts` |
| 5 | `da6ee9b` | root `package.json:95` said nothing else in the workspace resolves lodash | Names the three that do, and gives the measured reason the overrides stay scoped |
| 6 | `be41fc4` | `gen-demo-keys`' XAF staleness check was `grep -q '^\s*- code: mtn_momo$'` — a presence proxy | `mtn_settles_xaf`, an awk range over the `mtn_momo` sequence item asserting `currency: XAF` inside it |
| 8 | `4100986` | `demo-up`'s comment said six services; `demo_services` is eight | Says eight and names them (plus two more miscounts in the same recipes) |
| 9 | `f8b65ff` | `compose.demo.yml` published five services on `0.0.0.0` | All five bound to `127.0.0.1:`, with the reason in the file |
| 10 | `9f2d986` | `examples/shop/.env.example:33,44` disagreed with the compose stack | Matches it, and says which values must match and which are local-dev defaults |

---

## The two evidence lines that were not quite as filed

Neither changes a verdict; both changed what got written.

**Finding 5.** The review said `@testing-library/jest-dom@6.5.0`,
`cypress@15.21.1` and `chevrotain-allstar@0.1.7` all declare `^4.17.21`.
jest-dom and chevrotain-allstar do; **cypress declares `^4.17.23`** (checked
against the installed `node_modules/.pnpm/cypress@15.21.1/.../package.json`
and the registry). The note in `package.json` gives the real ranges.

**Finding 9.** The review named the checkout, the shop and the Orange stub.
`compose.demo.yml` publishes **five** services, and `vpay-server` (8080) and
`wiremock-webhook` (8083) were on `0.0.0.0` too. All five are bound.

---

## Finding 6 — the before/after, in full

`gen-demo-keys` regenerates the demo overlay when it detects a stale shape.
The XAF check was keyed on the *presence* of `- code: mtn_momo`, which every
overlay with a `providers:` block carries — including one edited back to
`currency: EUR`, the one state the check exists to catch.

Measured on a real `.e2e/application-demo.yml`, backed up and restored
byte-identically (`sha256sum -c`):

```console
$ sha256sum .e2e/application-demo.yml
0721a5a1…  .e2e/application-demo.yml

# BEFORE — grep -q '^\s*- code: mtn_momo$'
$ sed -i '52s/currency: XAF/currency: EUR/' .e2e/application-demo.yml
$ just gen-demo-keys | tail -1
gen-demo-keys: … and .e2e/application-demo.yml already exist, keeping them
$ sed -n '52p' .e2e/application-demo.yml
    currency: EUR                 # ← the mutation SURVIVED
$ cp …/overlay.bak .e2e/application-demo.yml && sha256sum -c …/overlay-before.sha
.e2e/application-demo.yml: OK

# AFTER — mtn_settles_xaf
$ just gen-demo-keys | tail -1        # unmutated: no false regenerate
gen-demo-keys: … already exist, keeping them
$ sha256sum -c …/overlay-before.sha
.e2e/application-demo.yml: OK
$ sed -i '52s/currency: XAF/currency: EUR/' .e2e/application-demo.yml
$ just gen-demo-keys | head -1
gen-demo-keys: .e2e/application-demo.yml does not settle `mtn_momo` in XAF — regenerating the pair
$ sed -n '52p' .e2e/application-demo.yml
    currency: XAF                 # ← rewritten
```

The regenerated overlay diffs against the backup only on the `n:`/`kid:`
lines (fresh key pairs), and a second run is idempotent again. The awk range
is terminated by the next `- code:` **or** the next top-level key, so
`orange_money`'s own `currency: XAF` cannot satisfy the `mtn_momo` check —
which the mutation above proves, since orange still carried XAF when mtn was
flipped and the recipe still regenerated.

## Finding 4 — why the file moved instead of being allowlisted

The review offered both. `screen-states.ts`'s only importers are
`checkout-view.test.tsx` and `checkout-screens.stories.tsx`; nothing in
`app/` or in a shipping component names it. Moving it to `src/testing/`
leaves the invariant with **no exception to carry**, which an allowlist entry
would not.

The guard walks `app/`, `src/components`, `src/lib`, `src/i18n` and the root
`middleware.ts` — the checkout's one shipping file outside `src/`, which the
shop's version has no counterpart for and which sets the page's CSP. It
asserts the list is non-empty (`> 25`; 30 files today) and self-checks both
its import regex and its exclusion regex against literal lines.

Tests and stories are excluded **by suffix, not by path**: `*.test.ts(x)` is
vitest's, and `*.stories.tsx` is read only by
`pnpm --filter @vpay/ui build-storybook` (its glob lives in
`frontends/packages/ui/.storybook/main.ts`) and never by `next build`. A new
one is covered by the same rule rather than by an allowlist somebody has to
remember.

Decisive check, run and reverted: adding
`import { makeSession } from '../testing/fixtures';` to `src/lib/controller.ts`
fails the guard and names that file.

## Finding 9 — what "bound to 127.0.0.1" was measured against

On this machine (LAN address `10.10.0.227`), a container published
`-p 19099:6379` answered on both `localhost` and `10.10.0.227`; the same
container on `-p 127.0.0.1:19099:6379` answered on `localhost` and was
connection-refused on `10.10.0.227`. `getent ahosts localhost` here is
`127.0.0.1` only.

`docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml config`
reports `host_ip: 127.0.0.1` for `vpay-checkout`, `vpay-server`, `vpay-shop`,
`wiremock-orange` and `wiremock-webhook`. The same two files **without**
`compose.demo.yml` still report `0.0.0.0` for all eight — `compose.e2e.yml`
is what CI's `e2e` job runs and was deliberately not touched.

`dashboard` remains on `0.0.0.0:3000` and `compose.demo.yml` now says so out
loud: it is not in `demo_services`, `just demo` never starts it, and what it
serves is the static scaffold notice, which reads no database and calls no
API.

---

## Finding 11 — informational, recorded and not changed

`verify-docs` (a report, never a gate) lists production functions of 80 lines
or more. Step 9 added one:

```
    92  backends/crates/vpay-api/src/v1/checkout_sessions.rs:418  fn validate_create
```

It is nine entries in the list now, up from eight. The code is **left as it
is**, and the reason is in the function's own shape: it is a flat sequence of
request-level rules — `payment_intent` present, `pi_` well-formed, `ui_mode`
parsed, blank-is-absent normalisation of the three URLs, then one `match` on
`ui_mode` whose two arms each say which URLs are required, which are refused,
and which end up on the `ValidCreate`. There is no nesting to flatten and no
branch that repeats another. Splitting the `match` arms into two helpers
would move the two lists of URL rules away from the one place a reader
compares them, which is the thing this function exists to make comparable.

Recorded here so the count is a decision rather than drift. If it grows a
third `ui_mode`, split it then.

---

## Deferred, with owners

**Finding 1 — `docs/runbooks/demo.md` §4's transcript still shows EUR.**
Nine `5000 EUR` lines between :209 and :482. Correctly *not* fixed here: §4
opens by stating the block is "Real output of `just demo` … captured
2026-09-04 in the `vpay-ci` VM … **Verbatim and complete** from the program's
first line to its last; nothing below was written by hand." Hand-editing EUR
to XAF in it would make that sentence false — which is the identical defect
to the eight above, introduced deliberately. **Owner: the integrator**, who
re-pastes §4 from the final VM demo run. That run's output will be XAF,
because `examples/merchant-demo` sends `xaf` for all six outcomes and the
generated overlay settles both rails in XAF.

**Finding 7 — the flow docs.** `docs/flows/money.md` §"Why EUR is here at
all", `docs/flows/adapter-mtn-momo.md` and `docs/flows/stripe-sdk-compat.md`
carry currency statements that the demo overlay's XAF change bears on.
**Owner: lane E**, which owns `docs/flows/*` for Step 9; this lane touched no
file under `docs/flows/`.

---

## Gate

Run on this branch, in this worktree:

| Command | Result |
|---|---|
| `just verify` | the four gates pass; `verify-docs` is the report above |
| `just --summary` | parses |
| `pnpm --filter @vpay/checkout test` | 302 passed / 17 files, 0 skipped (was 299 / 16 — the new guard is 3 cases) |
| `pnpm --filter @vpay-examples/shop test` | 49 passed / 6 files, 0 skipped |
| `just lint-web` | exit 0 across all fourteen TypeScript projects |
| `docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml config` | exit 0 |

Additionally, because they were the evidence for two findings:
`pnpm --filter @vpay/ui build-storybook` still emits 23 checkout entries in
`storybook-static/index.json` (the count lane 3b recorded), and
`pnpm --filter @vpay/checkout typecheck` is clean.

## What this lane did not do

- Did not touch `docs/flows/*` (lane E's) or `docs/runbooks/demo.md` §4
  (the integrator's) — see **Deferred** above.
- Did not touch `compose.e2e.yml`. CI's `e2e` job runs against it and a bind
  address is a thing to get wrong in a runner.
- Did not run `just demo` or `just ci`. The host is shared; the demo stack
  was never brought up. `gen-demo-keys` was run (it starts no container) and
  one throwaway `redis:7-alpine` container was published, probed and removed
  for the finding 9 measurement.
- Did not refactor `validate_create` — finding 11 is informational.
- Did not update `docs/status.md`: nothing here changes what is built. The
  one behavioural change is a host bind address, which is recorded in
  `compose.demo.yml` and in this note.
