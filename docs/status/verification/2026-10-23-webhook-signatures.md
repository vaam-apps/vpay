# 2026-10-23 — rail callbacks are checked where the rail signs (RFC-0014)

Branch `claude/happy-hopper-r3h6rz-webhook-signatures`, from `master` at
`82a68f4e`. **Stub-proven; the real scheme measured on: not yet.** Every proof
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

**Integration, real Postgres and WireMock's Notch Pay.**
`provider_callback.rs`'s
`a_webhook_the_rail_did_not_sign_moves_nothing_and_a_signed_one_settles`
confirms a Notch Pay charge through the SDK, puts it on the stub's
pending-then-complete scenario, parks its poll, and POSTs three forgeries (no
signature; a signature under another secret; Notch Pay's genuine signature of
a different webhook). After each: the poll's `run_at` is unchanged, nothing is
claimable, and the stub's journal holds no new read. A genuinely signed
webhook for an unknown reference is answered too. Then the genuine webhook:
the poll is claimable at once, the read settles the charge `succeeded`, and
the stub has recorded exactly one more read. All five answers are the same
`202`, byte for byte. The scrape says
`vpay_provider_callbacks_total{provider="notchpay",verdict="invalid"} 3` and
`verdict="verified"} 1`, and holds no secret.

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

GATE_COUNTS_PENDING

## What is stub-proven only, and what is not done

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
