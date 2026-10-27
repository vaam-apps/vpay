# 2026-10-26 — ADR-0033 step 2: the payment-intent and refund writes, split from axum

Branch `claude/dazzling-curie-g8sxn2-phase2`, stacked on
`claude/dazzling-curie-g8sxn2` (vaam-apps/vpay#374) at `6af7d7d`. A
refactor with no behaviour change, done ahead of the MCP tools that will call
it ([ADR-0033](../../adr/0033-mcp-is-the-primary-agent-surface.md) D3, D10
step 2).

## The change

In `backends/crates/vpay-api/src/v1/payment_intents.rs` and `refunds.rs`, six
writes are each split in two:

- **Which six:** payment intent `create`, `confirm` and `cancel`; refund
  `create`, `update` and `cancel`.
- **The handler** reads the request into a `PostRequest` and returns
  `<name>_with(…)`.
- **`<name>_with`** is the old handler body, word for word, over borrowed
  `&dyn Repositories`, `&ResourceConfig`, adapters, `&MerchantScope` and
  `&str` id.

So a caller that did not arrive over HTTP can run the same claim, the same
rules and the same write. The read handlers (`retrieve`, `list`) need no
split: their extractors are tuple structs a caller can build.

**Narrower than ADR-0033 D10 step 2 says.** Customers, checkout sessions,
invoices and invoice items are not split here. Invoices' three transitions go
through a shared `transition(…)` and need their own shape. They are split in
step 4, alongside their tools. The `MerchantOperations` port is not added
either: it would have no caller until step 3.

## Gates run here

The checkout was a git worktree sharing the main `target/`, built with
`CARGO_PROFILE_*_DEBUG=0`. Docker was available.

- `cargo clippy -p vpay-api --all-targets -- -D warnings`: clean.
- `cargo nextest run -p vpay-api -p vpay-tests-integration --lib --test
payment_intents --test refunds --test confirm_rails --test browser_checkout
--test worker_e2e`: **539 tests run: 539 passed, 0 skipped**.
- `just verify` and `cargo fmt --all -- --check`: see the PR's CI. The full
  `just ci` ran on the base branch
  ([2026-10-26-cratestack-0.16.md](2026-10-26-cratestack-0.16.md)), not on
  this one.
