# The provider port

The core decides what a payment *means*. An adapter decides how to say it on the
wire.

> If `if provider == "mtn_momo"` appears anywhere outside `adapters/`, the port
> is wrong. Fix the port, not the caller.

## The interface

`backends/crates/vpay-provider/src/lib.rs`

| Method | Contract |
|---|---|
| `async submit` | Idempotent on `reference_id`. A duplicate submission MUST report `Submitted`, never an error. Redirect rails also return `redirect_url` and `ref_extra` — **in the same value**, so a caller physically cannot hold a URL without the key material it will need to query the charge |
| `async query_status` | The authoritative read. Takes the whole charge, because some rails need the amount and their own token. Must work indefinitely |
| `parse_callback` | Identifiers **only** — never a status. **Stays synchronous on purpose:** it parses bytes that already arrived and must not be able to make a network call, so an adapter cannot smuggle a status out of an unauthenticated request |
| `async refund` | Optional; gated by `supports_refunds`. The trait's **default** is `ProviderError::Unsupported` — a permanent capability answer. An adapter whose rail *does* refund but whose refund is unbuilt overrides it with its own `NotImplemented` token so `verify-status` can see it |
| `capabilities` | Static declaration the core reads instead of special-casing |

The three network methods are `async`, via `#[async_trait]` rather than a
native `async fn`: a trait with a native `async fn` is not dyn-safe, and this
port is only ever held as `Box<dyn ProviderAdapter>` — which is what keeps
`if provider == "mtn_momo"` structurally impossible outside an adapter crate
(the HTTP layer holds trait objects whose concrete types it cannot name).
The cost is one boxed future per rail call, against a network round trip.
Implementors write `#[async_trait]` too.

`ProviderConfig` carries `base_url`, `callback_url`, `currency`, `settings`,
`credentials` and the two deadlines (`connect_timeout`, `request_timeout`).
The deadlines are on the *config*, not on the client, because one
`reqwest::Client` is shared by every rail — a client-level timeout could only
ever be one rail's. `vpay_config::ProviderHost::to_provider_config` is the
only place a `ProviderConfig` is built from YAML.

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
6. WireMock mappings under `backends/tests/conformance/wiremock/<rail>/mappings/`,
   reusing the shared conformance suite unchanged. (Since 2026-09-03 this step
   has a referent: the suite starts a real `wiremock/wiremock` container per
   rail and drives it over HTTP.)
7. Add the code to the documented `payment_method_types` values.
8. A flow doc recording its quirks.

**Nothing in the core changes.** If step 9 is "and also patch the reconciler",
the port leaked.

## The conformance suite

One suite, parameterised over every adapter
(`backends/tests/conformance/tests/adapter_conformance.rs`). **Adding a rail
means making this pass — not writing a new suite.** That is the real test of
whether this is a port or just a folder.

## Status

**Updated 2026-09-03 (Step 3). The port is implemented, and two rails now
speak over it to a real HTTP host.**

- The trait is `#[async_trait]`; `submit`, `query_status` and `refund` are
  `async`, `parse_callback` is not. `ProviderConfig` gained
  `connect_timeout`/`request_timeout` (`DEFAULT_CONNECT_TIMEOUT` 5 s,
  `DEFAULT_REQUEST_TIMEOUT` 20 s).
- The outbound HTTP client moved here from `vpay-api` as
  `vpay_provider::http` — vendored Mozilla roots, redirects refused, proxies
  ignored, and `bounded_body` capping any rail response at
  `MAX_RAIL_BODY_BYTES` (256 KiB). `vpay_api::http_client` is now a
  re-export, so no call site changed. **The cost, stated plainly: this crate
  is no longer a pure interface** — it links reqwest, rustls and
  webpki-roots, so a future non-HTTP rail (a USSD gateway, a file drop)
  compiles a TLS stack it never uses. No binary grew; both already resolved
  all three (Step 3 design, decision 2).
- 11 unit tests in `vpay-provider` (`cargo nextest run -p vpay-provider`,
  measured 2026-09-03), including
  `a_redirect_is_returned_rather_than_followed` and
  `a_request_timeout_actually_fires_against_a_silent_peer`.

**The conformance suite is the proof that this is a port and not a folder.**
26 tests — 4 capability cases plus 11 port cases parameterised over both
rails — run live against a real `wiremock/wiremock` container started by
`vpay_testkit::containers::start_wiremock`. **26 passed, 0 skipped,
0 ignored**, measured on 2026-09-03. The 11 port cases are
`submit_returns_a_reference_and_a_flow_shaped_result`,
`duplicate_submit_reports_submitted_not_an_error`,
`not_found_is_never_on_its_own_a_failure`,
`a_declined_charge_maps_to_the_documented_failure_code`,
`an_unavailable_rail_is_a_transport_error_never_a_decline`,
`bad_credentials_are_not_reported_as_a_payer_problem`,
`a_callback_body_round_trips_to_identifiers_only`,
`a_rail_without_the_refund_capability_answers_unsupported`,
`pending_then_successful_walks_the_scenario`,
`redirects_are_refused_and_never_followed` and
`an_oversized_rail_body_is_refused_at_the_cap`.

**What the suite does not prove.** Every one of those cases talks to
WireMock. **Neither adapter has ever called MTN's or Orange's real
sandbox**, so a mapping that is faithful to the flow doc but not to the rail
would pass. The 401-after-a-good-token re-mint path has no mapping in the
suite and is unproven on both rails. No callback route exists, so
`parse_callback`'s output is verified by tests and by nothing in production.
See [../status.md](../status.md).
