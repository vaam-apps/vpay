# exp30 — sabotage review of the one-binary change (issue #77)

The review pass over [`opus.md`](opus.md)'s change, on the same branch,
2026-09-07. It is a separate file because the two answer different questions:
that one says what was built and why, this one says what was _checked_, what
the check measured, and what it did not reach.

Base `30fb8f1`, implementation head `9ded03d` (three commits), review commits
on top.

## The four proofs, as run

Everything below was run on this host, with `DOCKER_HOST=unix:///run/user/1000/docker.sock`
and a compose project of its own (`exp30rev`, ports 18290-18294). The
maintainer's `vpay-demo` stack was up on 8080/8082/8083/3001/3080 throughout
and was not touched.

| Proof                             | Result on `9ded03d`                                                                                                                                                                                                                                                                                                                |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just ci`                         | **exit 0.** Ten gates; `test-rust` **1548 run, 1548 passed, 0 skipped** across **44** binaries in 949.8 s; `test-doc` 99 passed, 1 ignored (`sdks/rust`'s README block, pre-existing); `verify-ignored` **0 ignored (expected 0), 44 binaries (expected 44), 1548 total (floor 1080)**; `deny` advisories/bans/licenses/sources ok |
| `just helm-check`                 | **exit 0.** 17 guards, all fired by name; kubeconform **23 resources, 23 valid, 0 invalid**                                                                                                                                                                                                                                        |
| `just test-e2e`                   | **exit 0.** **11 Cypress tests, 11 passing, 0 failing** — `checkout.cy.ts` 1, `dashboard.cy.ts` 3, `shop-hosted.cy.ts` 3, `shop-embedded.cy.ts` 4 — through a stack whose `vpay-worker` container is the server image with `command: ["worker"]`                                                                                   |
| `just demo-up` + `just demo-walk` | **exit 0.** `✔ all six steps behaved as expected — 6 payments on 2 rails, every one settled by the worker asking the rail and evidenced by a signed webhook`, plus a hosted and an embedded Checkout Session and three account-holder lookups                                                                                      |

Every count matches [`opus.md`](opus.md) and [`../../status.md`](../../status.md)
exactly. Nothing in the implementer's report was found to be overstated.

## Behaviour equivalence: the old binary against the new subcommand

`git show 30fb8f1:backends/apps/vpay-worker-bin/src/main.rs` (693 lines) read
against `backends/apps/vpay-server/src/worker.rs` (335) plus the parts of
`main.rs` it now calls. What was checked, line by line, and what was found:

| Property                   | Old `vpay-worker-bin`                                                                                                                                         | `vpay-server worker`                                                              | Same? |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- | ----- |
| Boot order                 | signals → `install_process_defaults` → `boot` → observability → `run_loop`                                                                                    | identical; the first two are `main`'s `run`, before the dispatch                  | yes   |
| `boot` internals           | `load_config` → concurrency → adapter log → HTTP client → `boot_seeds` → `database_url` → migrate → reconcile → `ResourceConfig` → rails → endpoints → egress | same order, same `.context(..)` strings, same log messages                        | yes   |
| Exit codes                 | 78 config, 69 database, 1 unclassified, 1 on `Drain::TimedOut`                                                                                                | same; `exit_code_for`'s lookup order changed and cannot move a number (see below) | yes   |
| Signal handling            | `ShutdownSignals::install()` first, before tracing                                                                                                            | same call, now _before the subcommand dispatch_ — stronger, not weaker            | yes   |
| `Drain::TimedOut`          | `warn!` naming grace + released, then `exit(1)`                                                                                                               | byte-identical branch                                                             | yes   |
| Observability listener     | bound last, `--observability-bind`, `/livez` + `/metrics`, stops with the loop                                                                                | same; `start_observability` takes a `SocketAddr` instead of `&WorkerArgs`         | yes   |
| `vpay_build_info{version}` | `env!("CARGO_PKG_VERSION")` of `vpay-worker-bin`                                                                                                              | of `vpay-server`; both are `version.workspace = true`                             | yes   |
| Flags and env vars         | `CommonArgs` + `--worker-concurrency`                                                                                                                         | same seven + one, `global = true` so either side of `worker` works                | yes   |
| Log stream                 | stdout                                                                                                                                                        | stdout — `logs_to_stderr` matches the variant                                     | yes   |

**`exit_code_for`'s reordering was checked rather than accepted.** The old
worker looked for `ConfigError` → `StartupError` → `DbError`; the merged one
looks for `StartupError` → `ConfigError` → `SigningKeyError` → `DbError`. It
cannot move a worker number, for the reason `opus.md` gives (both leaves are
`Category::Configuration`, `ConfigError` is one category for every variant,
`DbError` is last in both) and for one it does not: the two leaves cannot
co-occur in a worker chain at all, because `load_config` runs to completion
before `concurrency()` is called. `worker::a_bad_config_is_exit_78_naming_the_problem`
and `worker::an_unreachable_database_is_exit_69_naming_postgres` are in the
suite and green.

**The tracing target did change** (`vpay_worker_bin` → `vpay_server` on the
handful of events the binary itself emits at boot; the loop's own events still
carry `vpay_worker`, the crate's module path, untouched). The implementer's
claim that nothing filters on it was re-checked independently rather than
taken: every `RUST_LOG` in the tree is the bare `info` (`compose.e2e.yml` ×2,
`_helpers.tpl`, `values.yaml`, `.env.example`), no workflow sets a directive,
and `prometheusrule.yaml`'s five alerts key on metric _names_
(`vpay_charge_transitions_total`, `vpay_jobs_oldest_claimable_age_seconds`,
`vpay_alert_events_total`, `vpay_jobs_completed_total`) with no label naming a
binary. Confirmed: nothing keyed on it.

**The two dropped unit tests were read at `30fb8f1` before accepting that they
were self-referential.** They were: `the_codes_match_the_adapters_that_are_linked`
and `installing_the_crypto_provider_leaves_a_process_default_and_is_idempotent`
tested that binary's own copies of functions that no longer exist, and
`vpay-server` carries both by the same names over the originals the worker
mode now calls (`src/lib.rs:62`, `src/main.rs:1266`). Neither asserted a CLI
property. Nothing was lost.

## The graceful stop, measured on a running stack

`worker_kill9` covers `kill -9`. A **SIGTERM with work outstanding** is a
different property and the suite does not reach it: `stop_worker_cleanly` says
so in its own assertion string ("a worker with nothing in flight"). That gap
predates #77 — the same helper did the same thing when it spawned
`vpay-worker-bin` — but the subcommand's shutdown path is new code around old
code, so it was run rather than reasoned about.

On the `exp30rev` stack, mid-`demo-walk`, `docker kill -s TERM` on the worker
container:

1. **The stop is graceful.** `docker events` reports `exitCode=0`; the
   container logged `received SIGTERM, starting graceful shutdown` and
   `graceful shutdown complete, exiting`. That is `Drain::Clean` — not
   `Drain::TimedOut`, which is the branch that would have exited `1`.
2. **The work outstanding at that moment was not lost, it was waiting.**
   `pi_ajm4789bbs4wd5129jke364n` had already settled through the API, and its
   outbox event had not been delivered. `docker kill` suppresses compose's
   `restart: unless-stopped`, so no worker ran for the next several minutes —
   and the receiver's journal held **zero** POSTs naming that intent for the
   whole 30 s the walk waited before failing on it. `demo-walk` exiting **1**
   there is the correct observation, not a defect: the producer had committed
   and there was nothing to run the fan-out.
3. **A restart completes it, and nothing else was touched.** `docker start`
   on the same container, no other action: the queued webhook arrived within
   ~6 s, carrying `vpay-signature`, `stripe-signature` and `vpay-event-id`,
   `type: payment_intent.payment_failed`, `data.object.status:
requires_payment_method`.

All 14 of the worker container's log lines were on **stdout** and none on
stderr, checked by splitting the two streams. That is the regression
[`opus.md`](opus.md) identifies as "most likely to have shipped broken" —
`args.command.is_some()` would have moved every one of them to stderr — and it
is now measured in a container rather than only by the ten `worker::*` cases
that read a pipe.

`docker inspect` on the running containers, as the last word on which argv
each mode got from one image: worker `Entrypoint=["/vpay-server"]
Cmd=["worker"]`, server `Entrypoint=["/vpay-server"] Cmd=null`.

## Findings

### 1 — correctness: `worker` accepted serve-only flags and read nothing

Fixed in `20429a7`. `vpay-worker-bin` declared neither `--bind` nor
`--oauth-signing-key-file`, so it refused both in every position. The merged
binary refused them only _after_ the subcommand word;
`vpay-server --oauth-signing-key-file … worker` parsed and the value was read
by nothing — the exact `--public-base-url` trap `vpay_config::cli`'s own
header describes, re-created by #77.

`ServerArgs::parse_checked` now refuses it, scoped to
`ValueSource::CommandLine` so `VPAY_OAUTH_SIGNING_KEY_FILE` in the
environment stays _ignored_ rather than refused — which is what
`vpay-worker-bin` did, and what keeps a shared env block from
`CrashLoopBackOff`ing a worker. Five documents that stated the weakening now
state the refusal and the deliberate env exception. See the commit for the
three mutations that pin it, including the one that matters most: reverting
`main` to clap's `parse()` leaves both `vpay-config` unit tests green and
fails only the subprocess case.

### 2 — misleading claim: five present-tense references to a deleted package

Fixed in `4dbd236`. Most `vpay-worker-bin` references in the tree are
historical and correct. Five were not, and one of them was a security
sentence whose _meaning_ changed: `vpay-api/src/staff_auth/mod.rs` kept the
password pepper and the TOTP key "out of `vpay-worker-bin`, which links
`vpay-db`" — a claim about the link graph, which one binary does not have.
The surviving claim is weaker (the worker mode constructs no `StaffLogin`)
and now says so.

### 3 — closed: the largest untested edge, measured

`opus.md` and `deployment.md` both named `args: ["worker"]` as the change's
biggest untested edge, correctly: the spelling exercised by `just test-e2e`
is compose's `command:` (the CMD), and the spelling that will run in a cluster
is Kubernetes' `args:` (also the CMD, but reached through a different key,
with `command:` meaning the _entrypoint_). The kubelet half is still untested.
The container-runtime half is not, as of this pass — the table is in
[`../../flows/deployment.md`](../../flows/deployment.md)'s Status section. On
the real `FROM scratch` image, one env block, three runs: `docker run IMG
worker` exits **69** through the job loop's own boot lines, `docker run IMG`
exits **78** through the serve path's missing signing key, and `docker run
--entrypoint worker IMG` exits **127** with `exec: "worker": executable file
not found in $PATH` — which is the CrashLoopBackOff the chart comment
predicts, now observed.

### 4 — maintainer action recorded: the retired GHCR package

`deployment.md` §1 stated that `ghcr.io/vaam-apps/vpay-worker` had not been
deleted as a _fact_. It is now a task with an owner and a named failure mode:
a package nothing publishes to still answers `docker pull`, so an un-upgraded
deployment keeps running 2026-09-04's worker against a newer server image and
a newer schema. Deleting it needs a scope this repository has never held, so
the choice between deleting, deprecating and accepting is the maintainer's.

### 5 — correctness: the release runbook told an operator to write a values file the chart now rejects

`docs/runbooks/release.md` §4's copy-pasteable block still read

```yaml
images:
  server: { digest: "sha256:<64 hex>" }
  worker: { digest: "sha256:<64 hex>" }
```

and `values.schema.json` is `additionalProperties: false` with `images.worker`
removed, so pasting it gives `helm lint`: `images: Additional property worker
is not allowed`. Measured, both ways: the old block fails `helm lint`, the new
one passes. `just helm-check` cannot catch this — it lints `values.yaml` and
`ci/values-full.yaml`, not a YAML fence in a runbook — which is why it
survived a green gate.

The same section's bold sentence had also become self-contradicting: "the two
workloads are pinned independently and must be pinned together", above a body
saying they are one image. Both fixed. Three other places name `images.worker`
and all three are prose correctly saying it no longer exists.

### 6 — nits, not fixed and listed rather than swept

- `opus.md` says the weakening "is stated in four places rather than one". It
  was stated in six (`cli.rs`'s field doc, the chart comment, the chart
  README, the `vpay-config` reference page, `docs/runbooks/rotate-signing-key.md`
  and `README.md`). All six now describe the refusal instead; the count is
  recorded here rather than corrected in a superseded notes file.
- `backends/Dockerfile`'s new comment says compose passes `command:
["worker"]` in "`compose.e2e.yml` and `compose.demo.yml`". Only
  `compose.e2e.yml` spells it; `compose.demo.yml` is an overlay layered on
  top and inherits it. True in effect, loose in wording.
- The task brief this review was written from says `release.yml` goes "down
  to two images". It goes to **three** (`vpay-server`, `vpay-dashboard`,
  `vpay-checkout`); the brief was counting the backend. The delivered state
  is right and says three everywhere.
- Two waits in the moved `worker::*` cases widened from 20 s to 30 s
  (`the_worker_serves_livez_and_metrics_on_the_observability_port`). Checked
  rather than assumed: it is a _wait_ bound, not an assertion — the
  `.expect(..)` still fires if the line never appears — so it trades
  flake-resistance for latency and weakens nothing. The binary it waits on is
  larger than the one it used to.
- The startup failure prefix on stderr moved from `vpay-worker-bin: ` to
  `vpay-server: `. Nothing in `docs/`, the runbooks or any workflow greps the
  old string (checked), and `docker compose logs vpay-worker` still works
  because the compose _service_ name is unchanged.

## What is still not proven, after this pass

- **A kubelet has never applied this chart**, and the review did not change
  that. `securityContext`, the probes, the `Recreate` strategy and the
  `ServiceAccount` are rendered and schema-valid and nothing else.
- **`release.yml`'s three-image matrix has not run.** `actionlint` is clean
  over the whole `.github/workflows` tree and both matrices were read; no
  push to `master` happened on this branch, so the evidence is shape and
  lint, not a workflow run.
- **`cosign verify` has still never been run** against any published digest,
  and the retired package has not been deleted (finding 4).
- **A SIGTERM that lands while a job is genuinely in flight** is exercised at
  the loop level (`vpay_worker::run_loop`'s drain, `worker_recovery`) and at
  the process level only with _nothing_ in flight — `worker_kill9`'s
  `stop_worker_cleanly` says so in its own assertion message. That gap
  predates #77 by the same helper and is not a regression from it; what the
  review added is the empirical check recorded below.
