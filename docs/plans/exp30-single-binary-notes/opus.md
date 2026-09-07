# exp30 — one binary: `vpay-worker-bin` folded into `vpay-server worker` (issue #77)

Working notes for the change that made `backends/apps/vpay-worker-bin`
disappear. Written on the branch, 2026-09-07. The decisions are here; the
evidence is in [`docs/status.md`](../../status.md)'s dated entry and the
Status sections of [`docs/flows/deployment.md`](../../flows/deployment.md) and
[`docs/flows/crash-safety.md`](../../flows/crash-safety.md).

## What the shape had to be, and the two things that were not obvious

`serve` is the **absence** of a subcommand, not a `serve` variant. Every
existing entrypoint — `ENTRYPOINT ["/vpay-server"]`, `compose.e2e.yml`'s
`vpay-server` service with no `command:`, the chart's server Deployment with
no `args:` — keeps working with no edit, which is the whole reason the issue
asked for a default. A `serve` variant that had to be spelled would break all
three at once; a `serve` variant that were accepted-but-optional would be two
spellings of one thing, one of which nothing in this repository uses.

**`CommonArgs` is `global = true` on every field.** This is the mechanism that
replaced a guarantee, and it is the first non-obvious part. Before #77 the
server and the worker could not drift on a shared flag because `CommonArgs`
was `#[command(flatten)]`ed into two parsers and
`the_flattened_common_args_are_identical_on_both_binaries` read both
`clap::Command`s and compared them. With one parser there is nothing to
compare. What has to hold instead is that `--config` works on **either side**
of the word `worker`, so that every invocation `vpay-worker-bin` accepted
still works with only the subcommand word inserted. `global = true` is what
buys that, and it is per-field, so a `global` dropped from one field is
silent: the flag still parses *before* the subcommand, which is the position
every compose file and Helm chart happens to use.
`the_common_options_are_accepted_on_either_side_of_the_worker_subcommand`
exercises all seven fields in both positions for exactly that reason.

**The log stream is a match on the variant, not `command.is_some()`.** This is
the second non-obvious part and the one most likely to have shipped broken.
`install_process_defaults` sent a subcommand's logs to stderr, because
`staff add`'s whole output is a one-time password an operator pipes and a
password interleaved into a JSON log is a password in a log aggregator. The
predicate was `args.command.is_some()`. The moment `worker` became a
subcommand, that predicate would have moved **every worker log line to
stderr** — a container whose logs vanish from anything reading stdout, with
every exit-code test still green. It is now `logs_to_stderr`, a `const fn`
matching each variant with no `_ =>` arm, so adding a third subcommand is a
decision rather than an inheritance. The ten `worker::*` cases read stdout
(`spawn_and_capture_stdout` pipes stdout and discards stderr), so the whole
module is the guard.

## Duplicates deleted, and what each one's justification was

Five functions and one enum existed twice, each with a doc comment arguing the
duplication was deliberate. Every argument rested on a premise a single binary
does not have. They were deleted rather than kept:

| Was duplicated | Stated reason | Why it no longer holds |
|---|---|---|
| `adapters()` / `adapter_codes()` | "the worker's capabilities must not be a function of the API server's crate; the two deploy independently" | They are one crate and one image. `worker.rs` calls `vpay_server::adapters` |
| `exit_code_for` | "the two binaries are free to diverge as they grow"; a shared helper would need a crate depending on both `vpay-config` and `vpay-db`, and ADR-0011 keeps `anyhow` out of libraries | One binary. The `anyhow`-at-the-boundary half is untouched and still true — the function still lives in the binary |
| `StartupError` | "which inputs a process requires is a property of that process" | One process, two modes. Two variants in one enum, both `Category::Configuration`, so the merge cannot move an exit code |
| `install_recorder` | "the exporter's configuration is a property of what a process measures, and the two measure different things" | One process, one recorder, installed before the dispatch |
| `install_crypto_provider`, `init_tracing`, `env_filter` | byte-identical copies | One of each |
| `start_observability` / `join_observability` | took `&ServerArgs` / `&WorkerArgs` | Take a `SocketAddr`; one listener, one flag, one implementation |

`exit_code_for`'s two versions looked for their leaves in **different orders**
— the worker's was `ConfigError → StartupError → DbError`, the server's is
`StartupError → ConfigError → SigningKeyError → DbError`. Keeping the server's
order cannot change any number the worker produced: `ConfigError` classifies
as one `Category::Configuration` for every variant by construction
(`vpay_config`'s `Classify` impl is a single arm), the worker's `StartupError`
is `Configuration` too, and `DbError` is last in both. The two cases that
would catch it if that reasoning were wrong are
`worker::a_bad_config_is_exit_78_naming_the_problem` and
`worker::an_unreachable_database_is_exit_69_naming_postgres`, and both are in
the suite.

## The one thing that genuinely got weaker

`vpay-worker-bin` could not be handed the OAuth signing key **at all** — the
flag did not exist on that binary, and
`the_worker_is_not_handed_the_signing_key` asserted a parse error for every
spelling. `vpay-server` must accept `--oauth-signing-key-file`, so now:

* `vpay-server worker --oauth-signing-key-file …` → parse error (the flag is
  top-level and deliberately not `global`);
* `vpay-server --oauth-signing-key-file … worker` → **parses**, and the value
  is read by nothing.

The CLI therefore no longer refuses every spelling. What actually keeps the
Secret away from the worker is where it is *mounted*:
`deployment-worker.yaml` templates no `signingKey` volume (verified on the
rendered output — the worker Deployment's `volumes` list is empty while the
server's is `['signing-key']`), and `compose.e2e.yml`'s worker service mounts
no key file. That was always the load-bearing half; a flag naming a path that
is not in the container is not a leak. It is stated in four places rather than
one, because it is the kind of thing a reader will otherwise assume the
opposite of: the field's doc comment, the Helm chart comment, the chart README
and the `vpay-config` reference page.

## `command:` in compose is not `command:` in Kubernetes

Docker's `command:` is the **CMD**, appended to the image's `ENTRYPOINT`.
Kubernetes' `command:` is the **ENTRYPOINT**, and `args:` is the CMD. So the
same intent is spelled two ways:

```yaml
# compose.e2e.yml                    # deployment-worker.yaml
command: ["worker"]                  args: ["worker"]
```

The issue text said `command: [worker]` for both. Written that way in the
chart, the kubelet would try to exec a file called `worker`, which does not
exist in a `scratch` image, and the pod would `CrashLoopBackOff` with
`exec: "worker": executable file not found`. This is recorded because it is a
silent-in-review, loud-in-production difference, and because the spelling that
has actually been *exercised* (compose, by `just test-e2e` and `just demo`) is
not the spelling that will run in a cluster. `deployment.md`'s Status section
names that asymmetry as the largest untested edge this change introduces.

## `expected_suites` moved by two, not one

The issue and the brief both expected one. `vpay-worker-bin` contributed
**two** test binaries:

* `vpay-worker-bin::cli` — its ten subprocess cases;
* `vpay-worker-bin::bin/vpay-worker-bin` — the two unit tests in its
  `main.rs`.

The ten cases moved into `vpay-server/tests/cli.rs` under a `worker` module,
keeping their names (four of them collide with a `serve` case asserting the
same contract for the other mode, which is a pairing worth keeping visible);
that file already existed, so they add no binary. The two unit tests did
**not** move: they tested that binary's own copies of
`adapters`/`adapter_codes`/`install_crypto_provider`, the copies are gone, and
`vpay-server`'s tests of the same names cover the originals the worker mode
now calls. 46 → 44.

## Mutations, and one the brief got wrong

**Mutation A — `worker` does nothing.** `worker::run` returns `Ok(())` before
`boot`.

* `worker_kill9`: **both scenarios FAIL** — `worker-1 (to be killed) never
  reached 'job loop running' within 60s`.
* `vpay-server::cli worker::*`: **9 of 10 FAIL**. (The tenth,
  `an_invalid_log_format_env_var_is_read_and_rejected`, passes by design: it
  asserts a clap parse failure that happens before dispatch.)
* `worker_recovery`: **23 of 23 PASS.**

That last line is the correction. The brief specified "make `worker` a no-op →
`worker_recovery` FAILS". It does not, and cannot: `worker_recovery.rs` drives
`vpay_worker::run_once`/`run_loop` **in-process** and never spawns a binary,
so it is green against a `worker` subcommand that does nothing at all. It is a
test of the loop, not of the entrypoint. The suites that hold the *subcommand*
honest are `worker_kill9` — which spawns the shipping binary, and which this
change had to edit (`Command::new(shipping_binary("vpay-worker-bin"))` →
`shipping_binary("vpay-server")` plus `.arg("worker")`) — and the ten moved
CLI cases. Measured, not reasoned: all three suites were run under the
mutation.

**Mutation B — drop the default-to-`serve`.** Recorded in `docs/status.md`
with its measurement.

## Deliberately not done

* **The stage in `backends/Dockerfile` is still called `server`**, not
  `backend`. It is the name `release.yml`, `just release-dry-run` and both
  compose files pass as `target:`; the image is still `vpay-server` and the
  entrypoint is still `/vpay-server`. Renaming it edits four files to say the
  same thing.
* **`vpay-worker` as a tracing target or service name is not preserved**, and
  nothing needed it to be. The string was grepped across `deploy/` and `docs/`
  before deciding: every occurrence is a Kubernetes object name, a compose
  *service* name, a chart label (`app.kubernetes.io/component: worker`) or an
  image name — all of them set by the orchestrator, none emitted by the
  binary. No dashboard, alert or `PrometheusRule` keys on a log field or
  metric label carrying it; the `PrometheusRule` keys on metric names, and
  those are unchanged. Events from inside the job loop still carry the target
  `vpay_worker`, because that is the crate's module path and the crate was not
  touched. What did change is the target on the handful of events the *binary*
  emits at boot: `vpay_worker_bin` → `vpay_server`. Nothing filters on it —
  no `RUST_LOG` directive in any compose file, chart value or CI workflow
  names either.
* **The `vpay-worker` GHCR package is not deleted.** Nothing publishes to it;
  it is frozen at run `33929374661`'s digest. Deleting it needs a
  `read:packages`/`delete:packages` scope this repository has never had (the
  same 403 the "GHCR visibility" note in `status.md` records), so it is left
  and documented rather than half-attempted.
* **No ADR.** Issue #77 records the decision and its reasoning, and the
  reference pages carry the "what this replaced and why" narrative. An ADR
  that restated the issue would be a fourth copy.
