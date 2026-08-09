# The provider port

The core decides what a payment *means*. An adapter decides how to say it on the
wire.

> If `if provider == "mtn_momo"` appears anywhere outside `adapters/`, the port
> is wrong. Fix the port, not the caller.

## The interface

`backends/crates/vpay-provider/src/lib.rs`

| Method | Contract |
|---|---|
| `submit` | Idempotent on `reference_id`. A duplicate submission MUST report `Submitted`, never an error. Redirect rails also return `redirect_url` and `ref_extra` |
| `query_status` | The authoritative read. Takes the whole charge, because some rails need the amount and their own token. Must work indefinitely |
| `parse_callback` | Identifiers **only** — never a status |
| `refund` | Optional; gated by `supports_refunds` |
| `capabilities` | Static declaration the core reads instead of special-casing |

## Capabilities

`flow`, `supports_refunds`, `supports_partial_refunds`, `delivers_callbacks`,
`requires_ip_allowlist`.

`orange_money` declares `supports_refunds: false`, and that flag — not a
rail-specific branch — is what makes the core refuse a refund on that rail. The
capability system earns its keep on day one.

## Preconditions, per flow shape

**A push rail must satisfy both:** you can supply your own idempotent reference
on submit; and you can query final status by it, indefinitely. Both are
load-bearing because the payer's phone starts buzzing before you learn whether
your request succeeded.

**A redirect rail must satisfy:** the submit response is persistable before the
payer can act (guaranteed by construction); and status is queryable by material
you hold after that persist.

Ask these **during commercial negotiation**, not after signing.

## Adding a rail

1. Answer the preconditions above. If either fails for a push rail, **stop and
   renegotiate** before writing code.
2. `INSERT INTO providers` with capability flags. *No schema migration.*
3. `INSERT INTO provider_hosts` for sandbox, production and stub hosts.
4. New `backends/crates/vpay-adapter-<rail>/` implementing the trait.
5. A mapping table into the [failure taxonomy](failures.md).
6. WireMock scenarios, reusing the shared conformance suite unchanged.
7. Add the code to the documented `payment_method_types` values.
8. A flow doc recording its quirks.

**Nothing in the core changes.** If step 9 is "and also patch the reconciler",
the port leaked.

## The conformance suite

One suite, parameterised over every adapter
(`backends/tests/conformance/tests/adapter_conformance.rs`). **Adding a rail
means making this pass — not writing a new suite.** That is the real test of
whether this is a port or just a folder.
