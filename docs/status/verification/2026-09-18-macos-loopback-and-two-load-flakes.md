# 2026-09-18 — `just test-rust` on a Mac: four deterministic failures and two load flakes

**No capability moved, and no assertion was weakened.** Three separate defects
in how this repository's Rust suite runs, found by running
`cargo nextest run --workspace` three times on a macOS developer machine on
2026-09-18. The branch they were found on touches no file under `backends/`, so
none of them is caused by the code under review — they were always there, and
CI cannot see any of them.

Nothing here is `#[ignore]`d. `just verify-ignored`'s `expected_ignored` is
still `0` and this page does not move it.

## The three, and what they have in common

| #   | Shape                                 | Where                                                     | Visible in CI?   |
| --- | ------------------------------------- | --------------------------------------------------------- | ---------------- |
| 1   | four **deterministic** macOS failures | `backends/tests/integration/tests/staff_sign_in.rs`       | no — CI is Linux |
| 2   | one **load** flake, ~1 run in 3       | `backends/tests/integration/tests/checkout_sessions.rs`   | not yet seen     |
| 3   | one **load** flake, ~1 run in 3       | `backends/tests/conformance/tests/adapter_conformance.rs` | not yet seen     |

All three are the same species: **a test asked a question whose answer depended
on something it had not actually waited for, or had not actually got.** Two
waited on a clock or a container; one waited on a network interface that was
never going to appear.

---

## 1. macOS assigns one loopback address; Linux assigns sixteen million

### The failure

```text
error sending request for url (http://127.0.0.1:PORT/dash/v1/staff/login)
  1: client error (Connect)
  2: tcp bind local error
  3: Can't assign requested address (os error 49)
```

Four cases, every run, identically whether run in the full workspace or alone —
so not a load effect:

- `the_sign_in_rate_limit_is_per_source_address`
- `the_second_factor_is_rate_limited_and_not_only_the_password`
- `a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget`
- `two_replicas_share_one_sign_in_budget`

### The mechanism

Each simulates distinct client source addresses with
`reqwest::ClientBuilder::local_address` — `127.0.0.2` and `.3`, `.4`, `.20`, and
`.30`–`.35`. Linux assigns the whole `127.0.0.0/8` to `lo`, so those binds
succeed and CI is green. macOS assigns only `127.0.0.1` to `lo0`; `ifconfig lo0`
on the machine this was measured on shows exactly one `inet` line. `bind()`
therefore returns `EADDRNOTAVAIL`.

The part that made this cost an afternoon rather than a minute:
**`local_address` binds nothing at `build()` time.** It records the address for
the socket `connect()` opens on first use, so the client constructs happily and
the failure surfaces at the first `send()`, inside reqwest's transport, with
nothing at the call site naming the cause.

### What was done, and what was deliberately not

**Chosen: a helper that preflights the bind and fails with a message naming the
fix** (`support::client_bound_to`). It binds a throwaway `TcpListener` on the
address, synchronously, at client-construction time — the same question the
kernel would be asked lazily, so it can never change the verdict, only when and
how it is reported.

Two alternatives were on the table and are recorded here because rejecting them
is the substance of the decision:

- **Documentation alone.** Rejected as the _mechanism_, kept as its companion.
  A developer who has not read the right paragraph still gets `os error 49` and
  no pointer. The note is in [CLAUDE.md](../../../CLAUDE.md) § "Things that will
  waste your time", pointed at from [AGENTS.md](../../../AGENTS.md) § Testing
  and from the `test-rust` recipe — but the artefact that actually reaches the
  person who needs it is the error text.
- **Gating the four behind a condition.** Rejected outright. These are the only
  tests covering the per-source-address half of the sign-in rate limit, and two
  of them exist precisely to prove `X-Forwarded-For` is _not_ honoured from an
  untrusted peer. A gate would make a Mac go green while measuring none of it,
  which is this repository's cardinal sin. **A macOS developer seeing red here
  is correct.** What was missing was not a skip; it was a sentence.

The four cases are otherwise untouched: same addresses, same attempt counts,
same assertions, same assertion messages.

### The fix a developer runs

```bash
just loopback-aliases
```

`sudo ifconfig lo0 alias … up` for exactly the ten addresses the suite binds,
idempotent, a no-op on Linux (detected with `uname -s`, because the whole `/8`
already being on `lo` there is the real reason nothing needs adding). **The
aliases do not survive a reboot.**

### The new failure text, measured

```text
Error: 127.0.0.20 is not assigned to a local interface, so no client can send
from it (EADDRNOTAVAIL, os error 49). Linux assigns the whole 127.0.0.0/8 range
to `lo`, which is why CI is green; macOS assigns only 127.0.0.1 to `lo0`, so
every other 127.0.0.0/8 address this suite binds needs an explicit alias on this
machine — and the alias does not survive a reboot. One-time fix:
`sudo ifconfig lo0 alias 127.0.0.20 up`, or `just loopback-aliases` for every
address this suite needs at once. See CLAUDE.md § "Things that will waste your
time".

Caused by:
    Can't assign requested address (os error 49)
```

---

## 2. The housekeeping sweep raced two clocks, not two tests

`the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one`, panic
`the housekeeping sweep never ran`. Seen once in three full runs; passes alone
and in a clean run.

### The mechanism, and the hypothesis it replaced

The suspicion when this was raised was a **shared job queue**: other tests' jobs
consuming the `for _ in 0..8` budget before the sweep is reached. That is
**ruled out**, in two steps:

1. `support::migrated_postgres` starts a fresh `postgres:16-alpine`
   testcontainer per call, and `Harness` owns it. The `jobs` table is
   **per test**. No other test's jobs can be in it.
2. Restated as "this test's own other jobs", it is ruled out by arithmetic.
   `vpay_worker::seed_singletons` seeds exactly five singletons in one
   transaction; the only other row is the `poll_charge` job the test itself
   defers by an hour; and none of the other four singletons cascades here
   (`fan_out_events` has zero endpoints — the test passes
   `support::no_webhook_endpoints()`; `scan_live_charges` re-enqueues only for
   charges stale past `stale_after()`, and this charge was confirmed moments
   earlier). Five claimable jobs cannot exhaust eight iterations.

What is left is the loop's **other** exit, and it is the one that fires:

```rust
let Some(settled) = vpay_worker::run_once(…).await? else { break; };
```

`run_once` answering `None` means "nothing claimable **this instant**", and the
loop treated it as "nothing left to claim", breaking on iteration zero with
`swept` still false — zero jobs run, not partial progress.

**Why `None` can happen a moment after seeding: two clocks.**
`seed_singletons` stamps `run_at` from **this process's** clock
(`OffsetDateTime::now_utc()`, `backends/crates/vpay-worker/src/run_loop.rs:484`),
and binds it as a plain parameter. `Jobs::claim` selects
`WHERE run_at <= now()`, and that `now()` is evaluated by **the Postgres
server**, inside its own container
(`backends/crates/vpay-db/src/jobs.rs:437`). If the container's clock reads even
milliseconds behind the host's — ordinary VM-clock and scheduler jitter, far
likelier under a full-workspace run's container churn than on a quiet machine —
every freshly seeded row is still in the future from the database's point of
view, and nothing is claimable yet.

**The shipping worker never notices**, which is why this is a test defect and
not a production one: `run_loop`'s `Ok(None) => idle(…)` waits out `IDLE_SLEEP`
and asks again, for as long as the process lives. Only the test's hand-rolled,
sleepless, eight-try loop lacked that patience.

### The fix

The test now calls `run_until`, the helper this same file already had for the
`checkout.session.expired` cases — which carried the **identical** defect
(`for _ in 0..16`, bailing on the first `None`). Fixing it once fixes eight call
sites instead of patching two copies.

`run_until` now polls on a wall-clock deadline of five seconds, re-entering
`run_once` when it answers `None`, with a 20 ms wait between empty claims. **The
deadline, not the wait, is what bounds the loop** — the condition is real and
checked, and a `kind` that never runs still fails, now with a message saying how
many other jobs ran first. Five seconds is orders of magnitude past any
plausible skew, so a genuine "the sweep is broken" still fails fast rather than
hiding behind the same `None` a few milliseconds of skew produce.

Every assertion the case made, it still makes: the sweep reported no error, the
abandoned session is `expired`/`unpaid`, the paying session is `open`/`unpaid`,
and its charge is still live.

### Deliberately not changed

**The two clocks themselves.** A job whose `run_at` comes from the app and whose
claim predicate comes from the database is a real asymmetry, and it is named
here so the next person does not rediscover it — but it is invisible in
production for the reason above, and rewriting `enqueue_in_tx` to derive
`run_at` from `now()` server-side is a change to shipping persistence that this
piece of work did not need and did not verify.

---

## 3. WireMock's health endpoint answers "ok" without checking anything

`not_found_is_never_on_its_own_a_failure::case_2_orange_money`, panic
`a rail with no record of a reference must answer, not error:
Config("orange_money: the token endpoint answered HTTP 404; check
providers[].host.url")`. Seen once in three full runs.

So the adapter reached **a** WireMock host, and that host had no stub for the
token endpoint: a live server, serving nothing.

### The hypothesis, and why it is wrong

The suspicion was a shared-server or shared-port race **between conformance
cases**. It does not hold: `.config/nextest.toml`'s `postgres-containers` group
(`max-threads = 1`) already covers `package(vpay-tests-conformance)`, so no two
container-starting tests in this workspace are ever mid-execution at once. A
leaked container answering on a reused port is also out — testcontainers
0.27.3's `ContainerAsync::drop` blocks synchronously until Docker's `rm`
completes, so a test process cannot exit while its container is alive.

### What was actually wrong

`start_wiremock`'s readiness gate was `WaitFor::healthcheck()`, which waits for
the image's own `curl -f http://localhost:8080/__admin/health`. **That endpoint
is hollow.** Read from the pinned tag's own source —
`HealthCheckTask.execute()` at `wiremock/wiremock@3.9.2`, fetched and checked
rather than inferred — it never touches the `Admin` it is handed:

```java
public ResponseDefinition execute(Admin admin, ServeEvent serveEvent, PathParams pathParams) {
  return responseDefinition()
      .withStatus(HTTP_OK)
      .withStatusMessage("Wiremock is ok")
      …
}
```

Every call is a hardcoded 200. A passing healthcheck proves the JVM started and
`/__admin/*` is routed, and nothing whatever about the stub tree.

**This repository believed otherwise, in writing.** `compose.yml`'s comment on
this image said `/__admin/health` means "the admin API is up AND the mappings
under /home/wiremock have been loaded". That sentence is now struck through in
place, with the source citation and a dated correction.

### The honest residual

WireMock 3.9.2 loads its bind-mounted `mappings/` synchronously in
`WireMockApp`'s constructor and binds the Jetty socket only afterwards, so a
WireMock process cannot answer _any_ request before its mappings are loaded.
That ordering is an undocumented implementation detail, and it says nothing
about the concern outside the container: whether the ephemeral **host** port
Docker just handed back is, at that instant, routing end to end to this
container. **Which of those two the observed 404 came from was not pinned**, and
this page does not claim it was. The fix closes both, because it asks the
question over the same path and the same resource a real rail request uses.

### The fix

`start_wiremock` now runs a second gate after the healthcheck:
`wait_for_mappings_loaded` polls `GET /__admin/mappings` **on the mapped host
port** until WireMock's own `meta.total` is positive, every 100 ms, bounded by a
five-second deadline that fails with a diagnostic rather than hanging.

Not a sleep: it polls a real, checkable condition and returns as soon as it is
true, usually within a poll or two of the healthcheck already passing.

The count is read by hand (`mappings_total`) rather than with `serde_json`,
which is a test-only dependency of this crate — the same reason the conformance
suite hand-parses `/__admin/requests/count`. It reads the **last** `"total"` in
the body rather than the first, because every mapping is serialised before
`meta` and a rail's own stubbed payload is free to contain a `total` field of
its own; reading the first would silently answer about a stub. Four unit tests
pin the parser, including that shadowing case. None needs Docker.

This gate now runs for **every** `start_wiremock` caller — nineteen call sites
across the integration and conformance suites — not only the flaky one. All
three mappings directories it is ever pointed at (`mtn`: 8 files, `orange`: 5,
the webhook receiver: 3) are non-empty, so the `total > 0` condition is
satisfiable everywhere it is used.

### Deliberately not changed

- **`.config/nextest.toml`.** No cross-test race was found, so no test-group
  change is warranted. Every existing group and comment is exactly as found —
  that file's comments record measurements that cost real time.
- **`compose.yml`'s healthcheck itself**, as opposed to its comment. It is still
  the hollow `/__admin/health`, so `docker compose up --wait` (`just demo-up`)
  can still return before WireMock serves a stub. The Rust suites no longer rely
  on it; the demo stack still does. Left visible and named in the file rather
  than quietly patched, because changing the demo stack's readiness is its own
  change with its own verification.

---

## Evidence

All on this branch, macOS (Darwin 25.6.0), Docker Desktop, 2026-09-18.

### The gates

| Command                                                      | Result                                                                              |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| `cargo fmt --all -- --check`                                 | clean                                                                               |
| `just clippy`                                                | `--workspace --all-targets -- -D warnings`, no warnings                             |
| `just verify`                                                | `ok — the thirteen gates above passed; the verify-docs report is advisory`          |
| `cargo xtask verify-status`                                  | `ok — 1 unimplemented item(s)` — **unchanged**; this retires no token               |
| `cargo xtask verify-links`                                   | `ok — 1720 repository link(s) in 400 tracked markdown file(s)`                      |
| `just verify-ignored`                                        | `0 ignored (expected 0), 48 test binaries (expected 48), 2002 total (minimum 1080)` |
| `just test-doc`                                              | **121 passed, 0 failed, 1 ignored** — a separate runner from nextest                |
| `node tools/verify-coverage.mjs` (vpay-skills, companion PR) | exit 0 — `20 skills, 205 vpay paths claimed, 45 feature pages, 0 exempt`            |

### The runs

`just test-rust` is `cargo nextest run --workspace`, which **fails fast**. With
four cases red by design on an unaliased Mac it therefore cannot reach a total,
so runs 2 and 3 add `--no-fail-fast` — the only difference from the recipe.

| Run                                   | Summary                                                     |
| ------------------------------------- | ----------------------------------------------------------- |
| 1 — `just test-rust` verbatim         | `1925/2002 tests run: 1924 passed, 1 failed, 0 skipped`     |
| 2 — `--no-fail-fast`, 1276 s          | `2002 tests run: 1995 passed (1 slow), 7 failed, 0 skipped` |
| 3 — `--no-fail-fast`, 1200 s          | `2002 tests run: 1997 passed, 5 failed, 0 skipped`          |
| `--test staff_sign_in --no-fail-fast` | `27 tests run: 23 passed, 4 failed, 0 skipped`              |

Run 1 was cancelled at 1925 by the first macOS bind failure, which is what
fail-fast does; both cases this page fixes had already run and passed by then.

### Every failure, attributed

| Failure                                                                                     | Runs | What it is                                                                                                                 |
| ------------------------------------------------------------------------------------------- | ---- | -------------------------------------------------------------------------------------------------------------------------- |
| the four `staff_sign_in` source-address cases                                               | all  | **intended.** No `lo0` alias on this machine; `sudo` was unavailable non-interactively. § 1                                |
| `webhooks::the_delivered_signature_verifies_with_the_shipping_node_sdk`                     | 2, 3 | `sh: tsc: command not found` — the worktree had no `node_modules`. Passed after `pnpm install`, re-run alone               |
| `vpay-db::a_cancel_racing_a_settlement_leaves_one_terminal_state_and_one_event`             | 2    | `unexpected response from SSLRequest: 0x00` — a Postgres container not yet serving. Passes alone                           |
| `worker_recovery::an_answered_submit_advances_the_bookkeeping_rather_than_submitting_again` | 2    | `container … does not expose port 8080/tcp` — the container-start flake `.config/nextest.toml` already names. Passes alone |

The last three are **pre-existing environment flakes, not regressions**, and
none recurred in run 3. The third is the same family this repository has been
bounding since `.config/nextest.toml` was written; it is recorded here because
it was seen, not because this change touched it.

### The two cases this page is about

| Case                                                                                        | Run 1 | Run 2 | Run 3 |
| ------------------------------------------------------------------------------------------- | ----- | ----- | ----- |
| `checkout_sessions::the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one` | PASS  | PASS  | PASS  |
| `adapter_conformance::not_found_is_never_on_its_own_a_failure::case_2_orange_money`         | PASS  | PASS  | PASS  |
| `…::case_1_mtn_momo`                                                                        | PASS  | PASS  | PASS  |

### The new macOS diagnostic, as measured

```text
Error: 127.0.0.20 is not assigned to a local interface, so no client can send
from it (EADDRNOTAVAIL, os error 49). …
One-time fix: `sudo ifconfig lo0 alias 127.0.0.20 up`, or `just
loopback-aliases` for every address this suite needs at once.

Caused by:
    Can't assign requested address (os error 49)
```

### Not run

`just ci` as a whole, and therefore `lint-web`, `test-web`, `audit-web` and
`deny`. `just ci` needs the network, and nothing in this change is reachable
from a shipping binary or from any web package. `just test-storybook` and
`cargo xtask verify-citations` were not run either; neither has an input this
change touches.

## What this does not do

- **It retires no `NotImplemented` token**, mounts no route, and moves no
  capability. `cargo xtask verify-status` reports the same count as before.
- **It adds no `#[ignore]`.** `expected_ignored` is still `0`.
- **It does not prove the two flakes are gone.** A flake seen once in three runs
  is not disproved by a handful of green runs; what is proved is that the
  mechanism each fix addresses is real, cited in the code, and no longer
  reachable by the path that produced the failure. The sweep case can no longer
  exit on a first empty claim at all, and no rail request can now be issued
  against a WireMock that reports zero mappings.
- **It does not verify the four sign-in cases passing on this machine.** Adding
  a `lo0` alias needs root, and this session had no non-interactive `sudo`. What
  is measured is the new diagnostic, not the green run behind it. See § Evidence.
