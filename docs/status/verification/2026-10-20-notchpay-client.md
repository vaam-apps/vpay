# 2026-10-20 — The Notch Pay client crate, RFC-0012, RFC-0013 and the provider matrix

## What this is, and what it is not

One branch, `claude/happy-hopper-r3h6rz`, cut from `master` at `0a86e83` and
merged with `master` at `dbe8300` (the 0.8.0 release) the same day; the gates
below are the post-merge run. It adds:

- **`backends/crates/vpay-notchpay`**, a typed client for the Notch Pay REST
  API transcribed from Notch Pay's OpenAPI specification 2.1.0, at the
  maintainer's direction ("From their official api-ref we can create a new
  crate and use it internally"). Initialize, process and read a payment;
  list and create webhooks; parse an inbound event.
  [reference](../../reference/vpay-notchpay.md).
- **[RFC-0012](../../rfc/0012-notchpay-adapter.md)**, the Notch Pay adapter
  design (Draft, not buildable as written), and
  **[RFC-0013](../../rfc/0013-vpay-registers-its-own-callbacks.md)**, vpay
  registering its own callback endpoints (Draft).
- **[providers.md](../providers.md)**, one matrix of every provider vpay ships
  or has designed.
- Twelve `debug_protections` in `schemas/privacy-inventory.yaml`, and a dated
  re-measurement of Notch Pay and Flutterwave on the 2026-09-23 evaluation.

**It is not an adapter.** No crate depends on `vpay-notchpay`, no binary links
it, `adapters()` does not know a `notchpay` rail, and **nothing in this
repository has called Notch Pay**. Every shape in the crate is the
specification's. The adapter waits on Notch Pay's written answer to whether
its status read can require the private key (ADR-0029 D1's third rule); the
maintainer chose to ask, and emailed Notch Pay on 2026-10-20.

## The independent review, and what it changed

An agent that had not written the crate reviewed it on 2026-10-20 against the
specification, the port and this repository's rules. It found two blocking
defects and eight minor ones. All ten are fixed on this branch; the
[reference page](../../reference/vpay-notchpay.md) § "The guards, and the
review that added them" has each one. The two that mattered:

- **A reference could choose the endpoint.** `vpay_provider::http::path_segment`
  passes `""`, `.` and `..` through, and URL parsing treats `%2E%2E` as `..`.
  `retrieve_payment("")` would have read the payments list, and
  `process_payment("..")` would have posted the payer's number to the API
  root. An empty or dot-only reference is now `Malformed` before any request;
  `an_empty_or_dot_only_reference_is_refused_before_any_request` and
  `the_public_payment_methods_refuse_a_dot_reference_without_sending` pin
  it. The shared helper still passes dots; MTN's only use of it takes a
  digits-only number. A suggested task records the port-level fix.
  _(True when measured. Later on 2026-10-20 #294 made `path_segment` itself
  refuse an empty or all-dots segment and deleted this crate's private
  guard; see
  [2026-10-20-path-segment-dot-segments.md](2026-10-20-path-segment-dot-segments.md).)_
- **An empty body read as success.** Every envelope field was `Option`, so
  `{}` decoded as a payment answer with no payment, and as a webhook list with
  no endpoints — which would make a caller register a duplicate endpoint on
  every run. The payloads are required now;
  `a_success_missing_its_payload_is_malformed_not_an_empty_success` pins it.

## What ran, measured

`just` is not installed where this was written, so each gate ran as
`cargo xtask <gate>`, which is what the recipe runs.

| Command                                                              | Result                                                                                     |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `cargo test -p vpay-notchpay`                                        | **34 passed, 0 failed, 0 ignored**; doctests **1 passed**                                  |
| `cargo clippy -p vpay-notchpay --all-targets -- -D warnings`         | clean                                                                                      |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p vpay-notchpay --no-deps`    | clean                                                                                      |
| `cargo check --workspace --all-targets`                              | finished, no warnings                                                                      |
| `cargo fmt --all -- --check`                                         | clean                                                                                      |
| `cargo xtask verify-no-mocks`                                        | ok                                                                                         |
| `cargo xtask verify-status`                                          | ok — 1 unimplemented item, unchanged: this branch adds no `NotImplemented` token           |
| `cargo xtask verify-errors`                                          | ok — 20 error types; the crate defines none                                                |
| `cargo xtask verify-sdk-parity`                                      | ok                                                                                         |
| `cargo xtask verify-links`                                           | ok — 2 308 links in 452 tracked Markdown files                                             |
| `cargo xtask verify-npm-scope`                                       | ok                                                                                         |
| `cargo xtask verify-serde`                                           | ok — 123 types; the crate's wire types rename every field to Notch Pay's own spelling      |
| `cargo xtask verify-repositories`                                    | ok                                                                                         |
| `cargo xtask verify-toolchain`                                       | ok — 1.98.0                                                                                |
| `cargo xtask verify-migrations`                                      | ok — 50 migrations; this branch adds none                                                  |
| `cargo xtask verify-versions`                                        | ok — 25 version references at 0.8.0, one more than `master` (the new workspace dependency) |
| `cargo xtask verify-privacy-inventory`                               | ok — `debug_protections` checked in both directions, twelve of them new                    |
| `cargo xtask verify-doc-counts`                                      | ok                                                                                         |
| `npx prettier@3.9.6 --check` on every changed Markdown and YAML file | clean                                                                                      |

**Not run:** `verify-ui` and `check-schema` (bash recipes of the `justfile`
with no `cargo xtask` form; neither reads anything this branch touches),
`just ci` and `cargo nextest run --workspace` (no Docker here; the crate's own
tests start no container, and no integration suite touches it). CI's `rust`
job is the workspace-wide run.

**What CI caught that the gates above could not.** `just verify-ignored` is a
bash recipe too, and it pins the number of test binaries. The new crate is one
more (`vpay-notchpay`'s library tests), and the pin was not moved with it, so
CI's `rust` job failed on `f219b0d` reporting **51** binaries, **2232** total,
**0** ignored — 2198 at ADR-0030 plus the crate's 34. `expected_suites` is now 51. `cargo metadata --no-deps` against `master` differs by that one target and
no other. A local `cargo nextest list --workspace` was attempted and did not
fit the disk, so the count is CI's measurement, not this page's.

## How the work was split

At the maintainer's request the work fanned out to sub-agents: two research
agents (Stripe and PayPal webhook registration; MTN and Orange callback
registration), one for Flutterwave's re-measurement, one to draft RFC-0013,
one to review the crate, and one for skills parity. Every fact an agent
reported that this branch relies on was either checked against the
repository here (three of RFC-0013's file-and-line citations, the
reviewer's dot-segment and envelope findings) or kept with its source and
marked "To confirm". The research fact sheets cite every vendor page they
read, and they read every one with a generic User-Agent.

## What is left

- **The Notch Pay adapter** (RFC-0012 steps 0 to 5), behind Notch Pay's answer.
- **RFC-0013**, all of it. No `callback_url_mode`, no
  `ensure_callback_endpoint`, no registration job exists.
- **Flutterwave**: no RFC. Its blocker is Flutterwave's written word on XAF
  withdrawals in Cameroon.
- **The two suggested follow-ups**: correcting vpay's claim that MTN retries
  callbacks, and making `path_segment` refuse dot segments. _(The second was
  done the same day, in #294.)_
