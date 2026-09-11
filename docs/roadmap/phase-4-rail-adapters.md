# Roadmap — Phase 4a and 4b: The rail adapters, and push-rail recovery

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

## Phase 4a — The rail adapters

**Done 2026-09-03** (branch `claude/step3-rails`; Step 3 of the
production-readiness plan). _This phase was "Phase 4 — The rails" until that
day, when it was split: the push-rail recovery table it used to contain moved
to Phase 4b/Phase 5, per Step 3's decision 5. The split is recorded rather
than silently applied because the old phase's Definition of Done was met by
the adapters alone, and anyone reading it afterwards would have believed
recovery had landed._

**Goal.** Seven of the eight `ProviderError::NotImplemented` tokens replaced
with real HTTP calls, passing the shared conformance suite. _(Eight, in the
original wording. `mtn_momo::refund` stays — MTN refunds are the
Disbursements product, with a subscription key and token scope no deployment
holds — and `orange_money::refund` left the list without being built,
because Orange documents no refund API and the adapter now inherits the
port's permanent `Unsupported` default. See `docs/status.md`.)_

**Status.** Done, against WireMock. Capabilities ✅; `submit`,
`query_status` and `parse_callback` ✅ on both rails against a real
`wiremock/wiremock` container; **the real sandboxes ⛔ — never called**.

**What landed.**

- The port became `#[async_trait]`, `ProviderConfig` gained per-rail
  timeouts, and the vendored-roots HTTP client moved into
  `vpay_provider::http` (redirects refused, proxies ignored, bodies capped
  at 256 KiB).
- `mtn_momo::{submit, query_status, parse_callback}` with a fingerprinted
  token cache and the failure table transcribed from its flow doc.
- `orange_money::{submit, query_status, parse_callback}`, returning
  `pay_token` + `notif_token` + `payment_url` together so a caller cannot
  hold a URL without the material to query it; `refund` is the port's
  `Unsupported`.
- Config gained `providers[].callback_url` / `currency`, `REQUIRED_RAIL_KEYS`
  ([ADR-0012](../adr/0012-rail-configuration-requirements-in-config.md)) and
  `ProviderHost::to_provider_config`; a livemode-secret rule that had made
  livemode unbootable was fixed.
- `POST …/confirm` moves the intent: `processing` / `requires_action` /
  `409 charge_declined` / `502`, with the redirect's `next_action` built
  only from the committed charge row.
- `verify-no-mocks` became a `cargo metadata` reachability walk;
  `verify-status` became two-directional and comment-aware.

**Definition of done — met, and stated exactly.** The shared conformance
suite (`backends/tests/conformance/tests/adapter_conformance.rs`) passes
parameterised over both adapters with **no `#[ignore]`s left**: 26 tests, 26
passed, 0 skipped, measured 2026-09-03; `just verify-ignored` pins
`expected_ignored := "0"`.

**What remains before this phase can be called done against the world.**

- **Neither rail's real sandbox has ever been called.** Every assertion is
  against a stub whose mappings were written from `docs/flows/adapter-*.md`,
  so a document that is wrong about the rail would still pass.
- The 401 → re-mint → retry path is unproven on both rails.
- ~~No callback route exists:~~ **the callback route exists (Step 8, lane C)**
  but nothing verifies Orange's `notif_token`, and MTN's callbacks are
  unsigned — so a callback is a hint on both rails and always will be. No rail
  has ever called the route: every body it has parsed was transcribed from
  `docs/flows/adapter-*.md` by this repository's own tests.
- `mtn_momo::refund` is still a `NotImplemented` token, and `POST /v1/refunds`
  is unrouted.
- Orange's duplicate-submit idempotency is an assumption about the rail.

**Unblocks.** A meaningful Phase 3 `confirm` (delivered); Phase 4b/Phase 5
(there is now something to poll); Phase 6 (something to sign a webhook
about).

---

## Phase 4b — Push-rail recovery _(moved into Phase 5, and delivered there)_

**Done 2026-09-03 (Step 4), as part of Phase 5.** This was the half of the old
Phase 4 that Step 3 deliberately did not ship. Its scope below is now
implemented by `vpay_worker::recovery::recovery_step` and proven by
`backends/tests/integration/tests/worker_recovery.rs` — with one item
outstanding and named at the end of this section. The heading stays because
Phase 4a's Definition of Done never covered recovery, and deleting the split
would make that look retroactively fine.

**Scope.**

- The push-rail recovery table
  ([`docs/flows/crash-safety.md`](../flows/crash-safety.md)): disambiguating a
  `submitting` charge via `provider_requests` (no row → resubmit; row with
  `status_code IS NULL` → poll, 3 consecutive `NotFound` over ≥60 s before
  treating the request as never received).
- Crash tests that kill the process at each of the three documented points
  and assert no double charge.

**What Step 4 delivered against that scope.** The `submitting` charges and
status-less `provider_requests` rows a lost submit leaves behind are now
_read_: no row → resubmit under the same reference; row with
`status_code IS NULL` → poll, and 3 consecutive `NotFound` over ≥60 s before
treating the request as never received; row with a status → advance the
bookkeeping. A redirect charge stuck in `submitting` is failed instead, keyed
on `Capabilities::flow`. Each has a test named in
[`docs/flows/crash-safety.md`](../flows/crash-safety.md).

**What is still outstanding from this phase's scope:** ~~the crash tests do not
kill a process.~~ **Corrected 2026-09-04 (Step 8, lane D):** two of the three
kill points are now proven by a real `SIGKILL` to a real shipping process
(`worker_kill9.rs`); **kill point 1 still writes the state rather than causing
it**, which proves the recovery table but not that moment's behaviour under a
signal. That distinction is stated in
[`crash-safety.md`](../flows/crash-safety.md) rather than smoothed over.

_The redirect-rail half of the old scope — "`ref_extra` must commit before
`redirect_to_url` is ever emitted" — **did** land in Phase 4a: the commit and
the `next_action` are one transaction and a re-read, proven by
`redirect_confirm_commits_the_rails_material_before_it_answers`._

**Risks carried by this phase.** Unchanged: rail testing depends on WireMock
hosts, and on this machine that meant a rootless Docker daemon that could
not start containers. It now can, and the conformance and integration suites
run against real containers locally — but no CI run has exercised them on
this branch.

---
