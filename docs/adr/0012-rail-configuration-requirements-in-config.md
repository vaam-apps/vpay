# 0012 — Rail configuration requirements are validated in `vpay-config`, keyed by rail code, until the port grows a hook

Date: 2026-09-03. Status: accepted (interim; supersedes nothing, narrows ADR-0002's reading).

## Context

[ADR-0002](0002-provider-port.md) says rails live behind the provider port and
that `if provider == "mtn_momo"` outside `backends/crates/vpay-adapter-*` is a
defect. Step 3 gave each rail real credentials and settings (`api_user`,
`target_environment`, `merchant_key`, `client_id`, `client_secret`, …) and a
boot-time rule that a deployment naming a rail without its required keys must
refuse to start (exit 78) rather than fail at the first live charge.

The natural home for "which keys does this rail need" is the adapter — a
`ProviderAdapter::required_settings()` hook. That hook does not exist, and
`vpay-config` cannot call it anyway: `vpay-config` is a leaf crate that the
adapters do not depend on and that must not depend on the adapters (the
binaries link the adapters and hand `vpay-config` plain data). The join
between YAML and linked adapters happens in `vpay_api::v1::boot`, at boot,
after `Config::load` has already validated the document.

## Decision

`vpay_config::config::REQUIRED_RAIL_KEYS` is a small table keyed by rail
code, consulted by `Config::validate_all`, listing the `settings` and
`credentials` keys each known rail requires. It is the **one** sanctioned
place outside an adapter crate where a provider code is matched on, and it
carries a comment saying so and pointing here.

This is an interim exception to ADR-0002's letter, not to its spirit: the
table decides nothing about payment behaviour (no flow, no capability, no
amount), only whether a deployment document is complete — the same kind of
rule as "every currency's exponent matches the canonical table".

## Consequences

- A new rail must add a row here in the same commit as its adapter; the
  conformance suite does not check this, so the reviewer must.
- When the port grows `required_settings()` (or the adapters expose a
  `const` the binaries can hand to `Config`), this table moves behind it and
  this ADR is superseded by the one that does it.
- `ProviderHost::Debug` prints `settings` in full; keys that are secrets
  belong in `credentials`, which is redacted. Orange's `merchant_key` stays
  in `credentials` for that reason (Step 3 design, decision 4).
