# 2026-10-25 — a failed callback check releases its lock before it returns

Branch `claude/happy-hopper-r3h6rz-callback-lock-release`, from `master` at
`bcbbf6fe`. A race in `vpay-worker`'s callback-registration pass, found by a
red build and fixed in the worker. **The integration case could not run in
this environment (no Docker); CI is its first run.**

## What failed

The `rust` job of the v0.14.1 release PR (vaam-apps/vpay#368, run
`40110843337`, job `120922427290`). That PR changes version numbers and
lockfiles only. 2097 of 2489 tests had passed when this one failed and
nextest stopped:

```text
FAIL [3.835s] vpay-tests-integration::callback_registration
     the_job_reschedules_after_a_refusal_and_sooner_after_silence
panicked at backends/tests/integration/tests/callback_registration.rs:439:5:
a 401 is the account's keys refused: Locked
```

The same test passed on `master` at `bcbbf6fe` (run `40110777979`) and on
vaam-apps/vpay#367. Each test starts its own Postgres container, so no other
test can hold the lock: the holder is this test's own previous call.

## Why

- `check_one` (`backends/crates/vpay-worker/src/callbacks.rs`) took
  `pg_try_advisory_xact_lock(vpaycbk)` in a transaction and returned a rail's
  failure as the closure's `Err`.
- `UnitOfWork::transaction` (`backends/crates/vpay-db/src/repository.rs`)
  drops an erring transaction instead of awaiting its rollback, on purpose:
  an awaited rollback could fail and mask the real error.
- sqlx 0.9.0 only queues a dropped transaction's `ROLLBACK`
  (`sqlx-core/src/transaction.rs`, `impl Drop for Transaction`: "we queue a
  rollback operation that will happen on the next asynchronous invocation of
  the underlying connection"). `PoolConnection::drop` spawns
  `return_to_pool`, whose `ping()` "flush[es] anything time-sensitive like
  transaction rollbacks" (`sqlx-core/src/pool/connection.rs`).
- So between `check_one` returning and that spawned task running, the old
  connection still holds the lock. The test's third call, `run_pass`, began
  a transaction on another connection in that window and was told the lock
  was held.

## The change

- `check_one` returns the rail's error inside `TxOutcome::Abandon`, whose
  rollback `transaction` awaits, so the lock is released before `check_one`
  returns. `PassFailure` is gone: the only error left to the closure is a
  `DbError`. `UnitOfWork::transaction` is unchanged.
- `callback_registration.rs` gains `lock_is_free`, which takes the lock in a
  fresh transaction. The case asserts it after the refused call and after the
  unanswered one, so a regression is named as the lock outliving the call.
- `docs/reference/vpay-worker.md` and the module docs of `callbacks.rs` say
  why the error travels in `Abandon`.

## Gates run here

- Clippy on `vpay-worker` and `vpay-tests-integration`, `--all-targets`,
  warnings denied: exit 0. This compiles `callback_registration.rs` with the
  new helper and assertions.
- `cargo test -p vpay-worker`: 82 unit tests passed, 0 failed, 0 ignored, and
  5 doctests passed. `cargo-nextest` is not installed in this environment;
  CI runs the suite under it.
- `cargo fmt --all --check`: clean.
- `just verify`: exit 0, all fifteen gates.
- `prettier@3.9.6 --check` on the four changed Markdown files: clean.

## What was not run

- `callback_registration.rs`, and every other integration binary: they need
  Docker, which this environment does not have. CI runs them.
- No rail was called.
