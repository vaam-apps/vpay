# 2026-10-23 — rail callbacks are checked where the rail signs (RFC-0014)

_**Updated 2026-10-24:** the check has now met real Notch Pay webhooks. Four
delivered to a dashboard-registered capture endpoint were signed over the raw
body, and this adapter's `verify_callback`, run offline over the recorded
requests, answered `Verified` for each. The same deliveries found that
`parse_callback` could not read them: they name the event under `event`, not
`type`. That was fixed the same day. See
[2026-10-24-notchpay-dashboard-webhooks.md](2026-10-24-notchpay-dashboard-webhooks.md).
"Stub-proven; never verified against a real Notch Pay delivery" below is true
of this page's own date._

Branch `claude/happy-hopper-r3h6rz-webhook-signatures`, from `master` at
`82a68f4e`. **Stub-proven; never verified against a real Notch Pay delivery.** Every proof
below is against WireMock's Notch Pay, a real Postgres, and a signing scheme
transcribed from Notch Pay's "Verify webhooks" guide. No webhook Notch Pay
sent has been received or checked, and nothing was sent to Notch Pay.

## Whose decision

The maintainer was asked, on 2026-10-23: "Webhook signature verification
(RFC-0014) needs Notch Pay's hash key held by vpay as a secret. Lift the
earlier 'hash key is evidence only, never vpay configuration' rule for this?"
The answer was "Yes, build RFC-0014". That lifts RFC-0012 § 2's refusal for
this feature only: the Hash Key is a vpay credential like the API keys, never
printed, logged or committed, real values only through the environment, and
every test fixture is an obviously fake value.

Every choice RFC-0014 left open is the implementing agent's, taken on the
safer side and recorded in
[RFC-0014 § "Built"](../../rfc/0014-verifying-rail-callbacks.md#built-2026-10-23):
the secret as a port-named credential rather than a new list; the
acknowledgement as `callbacks.verify: off`; four boot refusals where the RFC
named one; the `provider` label; `Unavailable` for what the route cannot
build; Orange's `BodyToken` not built.

## What was built

| Piece                                                                                                                                                                                                                                                                                                           | Where                                                                                                      |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `CallbackRequest` (hand-written `Debug`: header names, body length), `CallbackVerdict` (`admits_pull_forward`, `label`, `reason`), `CallbackCheck`, the port-named `CALLBACK_SIGNING_SECRET` and `_PREVIOUS`, `signing_secrets_in`, and the one MAC comparison, `hmac_sha256_hex_matches` (`Mac::verify_slice`) | `vpay-provider/src/callback.rs` (new)                                                                      |
| `ProviderAdapter::verify_callback`, defaulting to `NotChecked`; `Capabilities::callback_check` and `callback_check_is_coherent`                                                                                                                                                                                 | `vpay-provider/src/lib.rs`                                                                                 |
| `Measured` forwards `verify_callback` and counts nothing                                                                                                                                                                                                                                                        | `vpay-provider/src/measured.rs`                                                                            |
| `vpay_provider_callbacks_total{provider,verdict}` and `metrics::callback_verdict`                                                                                                                                                                                                                               | `vpay-core/src/metrics.rs`                                                                                 |
| The route's step 4: `verdict_for`, `charge_ref_of`, the counter, `warn` on a refusal                                                                                                                                                                                                                            | `vpay-api/src/provider_callback.rs`                                                                        |
| `IncoherentCallbackCheck`, `CallbackSigningSecretMissing`, `CallbackSigningSettingUnread`, `CallbackVerifyOffWithSecret` (exit 78); `unverified_callback_rails` and `warn_about_unverified_callbacks`, called by `serve`                                                                                        | `vpay-config/src/lib.rs`, `vpay-api/src/v1/boot.rs`, `vpay-server/src/main.rs`                             |
| `providers[].callbacks.verify: on \| off`                                                                                                                                                                                                                                                                       | `vpay-config/src/config.rs`                                                                                |
| `notchpay`: `CallbackCheck::Signature`, and `verify_callback` over `x-notch-signature`                                                                                                                                                                                                                          | `vpay-adapter-notchpay/src/lib.rs`                                                                         |
| `callback_check: None` on `mtn_momo` and `orange_money`, and on every test fixture                                                                                                                                                                                                                              | the two adapters, 19 literals                                                                              |
| `NOTCHPAY_HASH_KEY` in `.env.example`, the commented `notchpay` block, and the Helm Secret's comment; `CallbackRequest` in `debug_protections`                                                                                                                                                                  | `.env.example`, `config/application.yml`, `deploy/helm/vpay/values.yaml`, `schemas/privacy-inventory.yaml` |

## How it is proven

**Unit.** `vpay-provider`: the HMAC known answer; the raw body byte for byte
(re-serialised, one byte longer, one shorter); malformed, truncated and
whitespace-wrapped signatures; an empty secret verifies nothing; secrets
current first, blanks skipped; which verdicts admit the pull-forward; the
labels are the metric vocabulary; the `Debug` canary; and a structural test
that the comparison is `verify_slice` with no `==`, `!=` or `.eq(` (constant
time is not measurable on a shared runner, so the structure is what is
pinned). `Measured`: `the_callback_check_is_forwarded_and_not_counted`.
`vpay-adapter-notchpay`: `Signature` declared and coherent; a signed webhook
verifies, upper-case hex too; no signature, the wrong secret, the public or
private key as the secret, zeros, non-hex and empty are `Invalid`; the raw
body byte for byte; the previous secret during a rotation, and a retired one
refused; a replayed genuine webhook verifies (the accepted exposure, pinned);
no secret or a blank one is `NoSecret` whatever the header; a refusal's reason
quotes no secret, signature or body. `vpay-config`: `verify` defaults to `on`
and refuses an unknown value. `vpay-api` boot: every row of the rule's table
in both modes, with no secret in any message; a signing setting on a rail that
checks none; the coherence refusal; the warning, louder in livemode, never a
key, silent once the secret is set.

**Conformance.** `a_forged_callback_is_refused_by_a_rail_that_checks`, over
all three rails, branched on `callback_check`: a `Signature` rail refuses a
forged signature, a missing one and a body altered after signing, verifies its
own, and answers `NoSecret` without a secret; a `None` rail answers
`NotChecked` to each. `documented_callback_signature` is the per-rail data
row, signed with the same helper the check is proven against.
`every_adapter_declares_coherent_capabilities` also holds
`callback_check_is_coherent`.

**The real binary.** `vpay-server`'s `cli.rs`: a livemode Notch Pay
configuration without `callback_signing_secret` exits `78` naming the setting
and `verify: off`, and never prints the fake secret; a sandbox one logs
`serve`'s warning that its webhooks are unverified.

**Integration, real Postgres and WireMock's Notch Pay.**
`provider_callback.rs`'s
`a_webhook_the_rail_did_not_sign_moves_nothing_and_a_signed_one_settles`
confirms a Notch Pay charge through the SDK, puts it on the stub's
pending-then-complete scenario, parks its poll, and POSTs four forgeries (no
signature; a signature under another secret; Notch Pay's genuine signature of
a different webhook; the genuine webhook re-serialised after signing, with its
genuine signature). After each: the poll's `run_at` is unchanged, nothing is
claimable, and the stub's journal holds no new read. A genuinely signed
webhook for an unknown reference is answered too. Then the genuine webhook,
byte for byte as sent (spaced, keys out of order, a trailing newline): the
poll is claimable at once, the read settles the charge `succeeded`, and the
stub has recorded exactly one more read. All six answers are the same `202`,
byte for byte. The scrape says
`vpay_provider_callbacks_total{provider="notchpay",verdict="invalid"} 4` and
`verdict="verified"} 1`, and holds no secret. _(Three forgeries, a compact
genuine body and `invalid"} 3` until the security review below: a compact
body survives a re-serialisation byte for byte, so the route's use of the
raw bytes was not pinned.)_

## Mutation checks

Each mutation was applied alone, the named tests run, and the file restored
(a hash-checked copy).

| #   | Mutation                                                | Fails                                                                                                                                                      |
| --- | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1  | Notch Pay's `verify_callback` always answers `Verified` | 6 adapter unit tests; the conformance case's `notchpay` row; the integration case ("no signature: a webhook the rail did not sign must not move the poll") |
| M2  | the route's `admits_pull_forward` gate removed          | the integration case, at the same assertion                                                                                                                |
| M3  | `Measured` answers `NotChecked` instead of forwarding   | `the_callback_check_is_forwarded_and_not_counted`                                                                                                          |
| M4  | `verify_slice` replaced by a hex `==`                   | `the_comparison_is_the_constant_time_one`                                                                                                                  |
| M5  | boot's livemode refusal deleted                         | `a_signing_rail_without_a_secret_refuses_livemode_unless_it_says_verify_off`                                                                               |
| M6  | its `livemode` condition dropped                        | the same, at the sandbox row                                                                                                                               |
| M7  | its `verify: off` acknowledgement ignored               | the same                                                                                                                                                   |
| M8  | the off-with-secret refusal deleted                     | the same                                                                                                                                                   |
| M9  | the unread-setting refusal deleted                      | `a_signing_setting_on_a_rail_that_checks_nothing_is_refused`                                                                                               |
| M10 | the coherence refusal deleted                           | `a_callback_check_on_a_rail_that_never_calls_back_is_refused`                                                                                              |
| M11 | a blank secret counted as a secret                      | `the_signing_secrets_are_current_first_and_skip_blanks`                                                                                                    |
| M12 | the counter incremented by 0                            | the integration case, at the scrape                                                                                                                        |

## Gates

Run on this branch at its head, 2026-10-23, Rust 1.98.0, Docker 29.8.2.

| Gate                                                                                                       | Result                                                                                                  |
| ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all -- --check`                                                                               | clean                                                                                                   |
| `cargo clippy --workspace --all-targets -- -D warnings`                                                    | 0 warnings                                                                                              |
| `cargo nextest run`, every workspace package, and `vpay-tests-integration` one binary at a time (for disk) | **2 456 run, 2 456 passed, 0 failed, 0 skipped**, over 41 runs: 17 packages and 24 integration binaries |
| `cargo test --doc --workspace`                                                                             | **138 passed, 0 failed, 1 ignored**                                                                     |
| `just verify`                                                                                              | the fifteen gates passed; `verify-docs` is advisory                                                     |
| prettier 3.9.6, every changed Markdown and YAML file                                                       | clean                                                                                                   |

The first full run went red once, and the change fixed it:
`vpay-server`'s `a_livemode_notchpay_rail_boots_only_acknowledged_and_still_warns`
booted a livemode Notch Pay fixture with no signing secret, which
`CallbackSigningSecretMissing` now refuses. Its fixture holds a fake
`callback_signing_secret` (`${CLI_NOTCHPAY_HASH_KEY}`), and a third run
removes it and expects `78` naming the setting, so the refusal is also proven
on the real binary; the sandbox case asserts `serve`'s warning. `vpay-server`
was re-run alone after the fix: 60 passed. The figures above use that run.

## Security review (2026-10-23)

An independent reviewer read `82a68f4e..5b1dad71` adversarially, ran the
touched crates' tests (the integration case against real containers), and
diffed `vpay-adapter-notchpay/src/lib.rs` against the base line by line,
because a disk-full incident had truncated it mid-work and it was rebuilt by
hand. **Verdict: no security defect.** The comparison is constant-time over
the raw bytes; rotation cannot be made to accept every signature; a forgery
gets the same `202`, moves nothing and causes no read; no secret reaches a
log, a `Debug`, an error or a metric; and the adapter file lost nothing (its
eleven removed lines are a reflowed import and a rewritten doc section about
the limitation this branch removes).

| #   | Finding                                                                                                                                                                | Severity   | Done                                                                                                                                                                                                                                                 | Pinned by                                                                                                                                                                                       |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | The skills say the opposite of the code                                                                                                                                | blocker    | the vpay-skills branch of the same name, updated with this change                                                                                                                                                                                    | `tools/verify-coverage.mjs`                                                                                                                                                                     |
| 2   | The port's default fails open: an adapter that declares `Signature` and keeps the default `verify_callback` has its secret demanded at boot, then admits every forgery | should-fix | `verdict_for` answers `Unavailable` and logs `error` when a rail that declares a check answers `NotChecked`                                                                                                                                          | `a_declared_check_that_was_not_performed_fails_closed`                                                                                                                                          |
| 3   | A rotation has no end signal; the previous key is accepted silently for ever; `_previous` alone boots                                                                  | should-fix | `CallbackVerdict::VerifiedPrevious`, label `verified_previous`; a `WARN` at every `serve` boot while it is set (`warn_about_rotations_in_flight`); `CallbackSigningPreviousWithoutCurrent` (exit 78, both modes)                                     | `the_previous_secret_verifies_during_a_rotation_and_a_retired_one_does_not`, `a_rotation_in_flight_is_warned_about_at_every_boot`, `a_previous_signing_secret_without_a_current_one_is_refused` |
| 4   | A secret with surrounding whitespace (`"hsk_…\n"` from a file) boots and refuses every genuine webhook, with only a counter to say so                                  | should-fix | `CallbackSigningSecretWhitespace` (exit 78, both modes, current or previous, naming the setting); a blank value is still no secret                                                                                                                   | `a_signing_secret_with_surrounding_whitespace_is_refused`                                                                                                                                       |
| 5   | The unauthenticated route cloned every rail credential per request                                                                                                     | nit        | `RailConfig::provider_config_ref`, borrowed                                                                                                                                                                                                          | (no behaviour to test)                                                                                                                                                                          |
| 6   | `verdict_for`'s two `Unavailable` branches had no test                                                                                                                 | nit        | a unit test for each                                                                                                                                                                                                                                 | `a_check_the_route_cannot_build_admits_no_pull_forward`                                                                                                                                         |
| 7   | Raw-body handling was not pinned at the route: the compact fixture survives a re-serialisation                                                                         | nit        | the integration case's genuine webhook is non-canonical, and its re-serialisation is a fourth forgery. Folded into the existing case rather than added beside it: that case asserts exact `notchpay` counter values, and the recorder is per process | `a_webhook_the_rail_did_not_sign_moves_nothing_and_a_signed_one_settles`                                                                                                                        |
| 8   | One `warn!` per anonymous forged request, and `verdict="invalid"` is attacker-inflatable                                                                               | nit        | **recorded, not changed**: it costs no more than before (an `info` and a transaction), and the runbook's "is `verified` also non-zero?" tells forgeries from a stale secret. A caller who knows one live reference can still drive the alert to have | —                                                                                                                                                                                               |
| 9   | Two docs contradicted the code (`vpay-api/src/lib.rs`, "neither MTN nor Orange signs"; the threat model's flood row)                                                   | nit        | both corrected, with dated notes                                                                                                                                                                                                                     | —                                                                                                                                                                                               |
| 10  | `the_comparison_is_the_constant_time_one` is a source-text tripwire, not a proof: a `.cmp(` or a hand-written loop with `.verify_slice(` left in a comment would pass  | nit        | **recorded, not changed**: constant time cannot be measured on a shared runner, and the tripwire catches the obvious `==` regression                                                                                                                 | —                                                                                                                                                                                               |

Not done from the review: an assertion that the conformance suite's rail
list equals `vpay_server::adapters()`. `vpay-tests-conformance` would need
`vpay-server` as a dev-dependency, which links the whole server stack
(`vpay-api`, `vpay-db`, `vpay-worker`, `sqlx`) into a suite that today needs
only the port and the adapters. The fail-closed rule at the route (finding 2)
is the guard instead.

**Mutation checks, the review's.** M13 to M16 were applied **together** in
one build (each named test fails on its own rule; none depends on another)
and the files restored; M17 and M18 alone.

| #   | Mutation                                                            | Fails                                                                         |
| --- | ------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| M13 | the route's fail-closed rule disabled                               | `a_declared_check_that_was_not_performed_fails_closed`                        |
| M14 | "no configuration for the rail" answers `NotChecked`                | `a_check_the_route_cannot_build_admits_no_pull_forward`                       |
| M15 | the previous-without-current refusal disabled                       | `a_previous_signing_secret_without_a_current_one_is_refused`                  |
| M16 | the whitespace refusal disabled                                     | `a_signing_secret_with_surrounding_whitespace_is_refused`                     |
| M17 | Notch Pay answers `Verified` for a match under either key           | `the_previous_secret_verifies_during_a_rotation_and_a_retired_one_does_not`   |
| M18 | the route verifies the body re-serialised (`serde_json` round trip) | the integration case, at "a genuine webhook must make the poll claimable now" |

**Gates for the review's changes**, 2026-10-23, Rust 1.98.0, `cargo test`
(`cargo-nextest` is not installed in this environment), each package or
filter alone:

| Run                                                                                                                            | Result                                |
| ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------- |
| `cargo fmt --all -- --check`                                                                                                   | clean                                 |
| `cargo clippy` on the eight touched packages, `--all-targets -- -D warnings`                                                   | 0 warnings                            |
| `vpay-provider` unit / doc                                                                                                     | 59 / 14 passed, 0 failed, 0 ignored   |
| `vpay-adapter-notchpay` unit / doc                                                                                             | 64 / 1 passed, 0 failed, 0 ignored    |
| `vpay-config` unit / doc                                                                                                       | 148 / 5 passed, 0 failed, 0 ignored   |
| `vpay-core` unit / doc                                                                                                         | 94 / 68 passed, 0 failed, 0 ignored   |
| `vpay-api --lib`                                                                                                               | 445 passed, 0 failed, 0 ignored       |
| `vpay-server` (lib 9, main 8, `cli.rs` 43)                                                                                     | 60 passed, 0 failed, 0 ignored        |
| conformance: `a_forged_callback_is_refused_by_a_rail_that_checks` (3 rails) and `every_adapter_declares_coherent_capabilities` | 4 passed, 0 failed (101 filtered out) |
| integration: `a_webhook_the_rail_did_not_sign_moves_nothing_and_a_signed_one_settles`, real Postgres and WireMock              | 1 passed, 0 failed (11 filtered out)  |
| `just verify`                                                                                                                  | the fifteen gates passed              |

Not run for the review's changes: the whole workspace, the other integration
binaries, and `just ci`.

## Merged with `master` at `71a351df` (2026-10-23)

`master` gained vaam-apps/vpay#353 the same day: `ProviderAdapter`'s
`default_callback_registration` (`notchpay` answers `Off`), an optional
`providers[].callbacks.register`, and a process `422` on the payer's phone
alone as `invalid_payer`. Both changes are kept. `ProviderCallbacks` holds an
`Option<CallbackRegistration>` and RFC-0014's `verify`; `Measured` forwards
both new methods; the port has **fourteen** methods
([provider-port.md](../../flows/provider-port.md)). With `register` unset,
vpay asks a Notch Pay account nothing, so a signature check needs the
endpoint registered by hand in Notch Pay's dashboard and its Hash Key in
`credentials.callback_signing_secret`.

Measured on the merged tree, `cargo test` (nextest is not installed here):

| Check                                                   | Result            |
| ------------------------------------------------------- | ----------------- |
| `cargo fmt --check`                                     | clean             |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings        |
| `vpay-provider` unit / doc                              | 60 / 14 passed    |
| `vpay-adapter-notchpay` unit / doc                      | 66 / 1 passed     |
| `vpay-config` unit / doc                                | 148 / 5 passed    |
| `vpay-core` unit / doc                                  | 94 / 68 passed    |
| `vpay-api --lib`                                        | 445 passed        |
| `vpay-server` lib / main / `cli.rs`                     | 9 / 8 / 43 passed |
| conformance, the whole suite                            | 106 passed        |
| `provider_callback.rs`, real Postgres and WireMock      | 12 passed         |
| `callback_registration.rs`, real Postgres and WireMock  | 6 passed          |
| `just verify`                                           | ok, all fifteen   |

Every row: 0 failed, 0 ignored. The rest of the integration binaries were
not run here; CI runs them.

## What is stub-proven only, and what is not done

- **No webhook from Notch Pay has ever been delivered** to any vpay
  endpoint, so the signature's format, header name, encoding and Notch Pay's
  retries are all unmeasured. The sandbox fork's run of 2026-10-23 (branch
  `claude/happy-hopper-r3h6rz-notchpay-sandbox`, landing first) found that an
  endpoint created through `POST /webhooks` with a public URL is answered
  `201` and stored with `"url": null`, that nothing was delivered to it in 21
  minutes, and that reading, updating or deleting it by id answers `404`. It
  advises registering vpay's URL in Notch Pay's dashboard with
  `callbacks.register: off`, which is untested. So verifying a real webhook
  needs the Hash Key **and** a dashboard-registered endpoint, and until a
  delivery is seen an operator should expect no Notch Pay callbacks: polling
  settles every charge.

- **What Notch Pay signs.** The check signs the raw body. Notch Pay's guide
  shows that in its PHP and Python examples and `JSON.stringify(req.body)` in
  its Node one. RFC-0012 § 10 item 7 is open: if the real scheme differs,
  every genuine webhook reads `invalid`, and each charge pays its callback
  latency while the poll ladder still settles it.
- **The header's case and encoding** as Notch Pay really sends them (the check
  accepts either case of hex; header names are case-insensitive).
- **Not built:** Orange's `BodyToken` check (RFC-0014 open question 2);
  Stripe's and PayPal's (no adapter); per-account secrets (RFC-0009);
  RFC-0013's sealed store; an alert rule on `verdict="invalid"`; a secret in
  the `notchpay-sandbox` profile, which still answers `NoSecret`.
- **Replay** of a captured genuine webhook is accepted (no timestamp), and
  pinned by a test as a decision.
