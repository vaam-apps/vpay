# 2026-10-21 — The `notchpay` adapter addresses payments by Notch Pay's reference

## What changed, and why

Notch Pay's sandbox answered `404` "Payment Not Found" to every process and
read by vpay's own reference, and accepted only its own `trx.…`
([2026-10-21-notchpay-sandbox-run.md](2026-10-21-notchpay-sandbox-run.md),
RFC-0012 § 10 item 2). The adapter addressed both by ours, so its `submit`
failed closed against the real rail. On the maintainer's decision in the
running session ("Yes to both"), it now:

- keeps the `trx.…` from the initialize answer (`transaction.reference`) as
  `ref_extra` key material, under `notchpay_reference`
  (`vpay_adapter_notchpay::NOTCHPAY_REFERENCE`);
- sends the process call and the read to `/payments/{notchpay_reference}`;
- requires every payment answer to name the charge's own reference as its
  `trxref`, so an answer about another payment never settles this charge.
  The sandbox put ours there on every payment answer;
- refuses an initialize answer with no Notch Pay reference, or one that
  carries ours back in its place, as `Malformed`, before the prompt;
- answers `Config`, before any request, for a read of a charge that holds no
  `notchpay_reference`. That is key material vpay lost, not a payment Notch
  Pay never heard of, so it is never `NotFound` (Orange's `pay_token` rule);
- on a `409`, reads by the held `notchpay_reference` (`pending` → process;
  anything else → `Submitted`). With none held, it answers `Malformed`:
  fate unknown, never `Submitted`, never a decline. This was the
  maintainer's decision, because Notch Pay finds a payment by its own
  reference only;
- in `parse_callback`, takes `data.trxref` before `data.reference`. That
  follows the payment object's measured shape. No webhook was received, so
  the event's `data` shape is assumed, not measured.

**What this does not fix.** A crash between the initialize answer and
`submit` returning leaves a payment vpay cannot read. Before the process
call, nobody was prompted and the payment expires. After it, the payer may
pay, and only Notch Pay's dashboard shows it. The retry's `409` is
`Malformed`, so an operator must reconcile it. `GET /payments?search=` does
not close this: in the sandbox it matched neither our reference nor the
description.

**It has not met Notch Pay.** Every assertion below runs against a loopback
socket or a WireMock stub. Step 5, the first test-mode call through the
adapter, is what would prove it, and it has not run. `providers.md`'s cells
stay "stub-only; never called".

## The conformance suite: a per-rail hook

`Rail::charge` stands in for a charge after `submit`. Its `ref_extra` was
chosen by flow (push: empty, redirect: a token). Notch Pay is a push rail
addressed by its own token, which broke that rule. The maintainer approved a
per-rail table: `submitted_key_material(rail, reference_id)`, beside
`documented_declines`. MTN gets nothing, Orange its `pay_token`, Notch Pay
`notchpay_reference: trx.test_{reference_id}`. Two shared cases moved with
it, and neither branches on a rail name:

- `submit_returns_a_reference_and_a_flow_shaped_result` asserts that every
  key in the stand-in's `ref_extra` comes back from `submit`, on either flow.
- `duplicate_submit_reports_submitted_not_an_error`, on a push rail, asserts
  that the duplicate returns exactly the key material the charge holds. That
  is still empty for MTN.

One case is new and Notch Pay only:
`notchpay_a_duplicate_it_cannot_resolve_fails_closed` (a `409` with no held
reference is `Malformed`).

The stub (`wiremock/notchpay/mappings/`) now answers in the sandbox's shape.
Initialize mints `reference: trx.test_{ours}` and `trxref: {ours}`. Process
and read match `/payments/trx\.test_…` only, so an adapter that addressed by
our reference matches nothing. No answer carries an `id`. The `409` body is
the sandbox's.

## Tests

New in the adapter (`src/tests.rs`), each against a loopback socket that
answers real HTTP:

- `the_read_is_addressed_by_notch_pays_reference_and_carries_both_keys`
  (renamed from `…by_our_reference…`, and inverted);
- `a_read_without_notch_pays_reference_is_config_before_any_request`;
- `the_process_call_is_addressed_by_the_reference_initialize_returned`
  (initialize, then `POST /payments/trx.test_…`, then `Submitted` carries it);
- `an_initialize_answer_for_another_reference_never_prompts`;
- `a_duplicate_with_no_held_reference_is_malformed_and_sends_nothing_more`;
- `a_duplicate_with_a_held_reference_reads_it_and_does_not_prompt_again`;
- `a_read_naming_another_trxref_is_never_a_success`;
- `a_callback_in_the_sandboxs_payment_shape_names_our_charge_by_trxref`.

## Gates and tests

Run on 2026-10-21, on Linux, in two rounds.

**First, while stacked on vaam-apps/vpay#303** (head `2fcc56f`, base the
branch of vaam-apps/vpay#303 at `acb2028`):

- `cargo test -p vpay-notchpay --lib`: **35 passed, 0 ignored**.
  `cargo test -p vpay-adapter-notchpay --lib`: **39 passed, 0 ignored**.
  The probe's own tests: **5 passed**. Doctests: 2 passed, 0 ignored.
- `cargo nextest run -p vpay-tests-conformance`: **94 passed, 0 skipped**,
  against WireMock in Docker (27 `notchpay` rows, re-run after the last stub
  edit).
- `cargo clippy -p vpay-notchpay -p vpay-adapter-notchpay -p
vpay-tests-conformance --all-targets -- -D warnings`: clean.
  `cargo fmt --all -- --check`: clean.
- `just verify`: all fifteen gates passed.
- **`verify-ignored` was not run** (this container's disk). No target was
  added: the new tests are functions in existing binaries, so
  `expected_suites` (53) cannot move. CI runs it.
- **Not run here:** `just ci` as a whole. CI runs it.

**Then on the head merged with `master`** after #303 was squash-merged as
`f7c7ae3`. The head was `1b57199`, base `master` at `f7c7ae3`. The merge
conflicted only because of the squash: `git diff acb2028 f7c7ae3` is empty,
so every conflict took this branch's side. The merged tree is identical to
`2fcc56f`'s. Re-run on it:

- `cargo nextest run -p vpay-tests-conformance -E 'test(notchpay)'`: **27
  passed, 0 skipped**, against WireMock in Docker.
- `just verify`: all fifteen gates passed.
- CI on `1b57199`: `rust`, `web`, `e2e (compose)`, the self-checks and
  "status.md matches the code" green. The `quality` workflow (Super-linter,
  CodeQL, Trivy, link integrity) did not run on it, because it was pushed
  before the PR was retargeted to `master`. The commit adding this paragraph
  is what runs it.
