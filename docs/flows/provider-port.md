# The provider port

The core decides what a payment _means_. An adapter decides how to say it on the
wire.

> If `if provider == "mtn_momo"` appears anywhere outside `adapters/`, the port
> is wrong. Fix the port, not the caller.

## The interface

`backends/crates/vpay-provider/src/lib.rs`

| Method               | Contract                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `async submit`       | Idempotent on `reference_id`. A duplicate submission MUST report `Submitted`, never an error. Redirect rails also return `redirect_url` and `ref_extra` — **in the same value**, so a caller physically cannot hold a URL without the key material it will need to query the charge                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `async query_status` | The authoritative read. Takes the whole charge, because some rails need the amount and their own token. Must work indefinitely                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `parse_callback`     | Identifiers **only** — never a status. **Stays synchronous on purpose:** it parses bytes that already arrived and must not be able to make a network call, so an adapter cannot smuggle a status out of an unauthenticated request                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `parse_destination`  | Optional; the adapter's own `destination[<rail_code>][…]` sub-map → `RefundTarget`. Symmetric with `parse_callback`, and synchronous for the same reason — it reads a merchant's parameters and must not be able to call a rail. The core strips the rail code (its own envelope) and interprets nothing inside. The trait's **default** is `ProviderError::Unsupported`, which is the honest permanent answer for a rail declaring `RefundDestination::Origin`: it has no destination to parse. A `Required` rail overrides it, and `a_required_rail_parses_its_own_destination` in the conformance suite fails if one does not. A missing key, a non-string value or a blank one is `Malformed`, and the message names the parameter and never the number in it (RFC-0003 open question 4, decided 2026-09-15) |
| `async refund`       | Optional; gated by `supports_refunds`. Answers `Refunded`, **not** `Submitted`: a refund has no payer's browser, so `redirect_url` was a question no adapter could ever answer, and `Refunded` carries instead the one thing a refund has that a charge does not — `fee: Option<Money>`, what the rail charged us to move the money. The trait's **default** is `ProviderError::Unsupported` — a permanent capability answer. An adapter whose rail _does_ refund but whose refund is unbuilt overrides it with its own `NotImplemented` token so `verify-status` can see it                                                                                                                                                                                                                                     |
| `capabilities`       | Static declaration the core reads instead of special-casing                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |

The three network methods are `async`, via `#[async_trait]` rather than a
native `async fn`: a trait with a native `async fn` is not dyn-safe, and this
port is only ever held as `Box<dyn ProviderAdapter>` — which is what keeps
`if provider == "mtn_momo"` structurally impossible outside an adapter crate
(the HTTP layer holds trait objects whose concrete types it cannot name).
The cost is one boxed future per rail call, against a network round trip.
Implementors write `#[async_trait]` too.

`ProviderConfig` carries `base_url`, `callback_url`, `currency`, `settings`,
`credentials` and the two deadlines (`connect_timeout`, `request_timeout`).
The deadlines are on the _config_, not on the client, because one
`reqwest::Client` is shared by every rail — a client-level timeout could only
ever be one rail's. `vpay_config::ProviderHost::to_provider_config` is the
only place a `ProviderConfig` is built from YAML.

`ChargeRef` carries `reference_id`, `amount`, `payer_ref`, `ref_extra` and —
since Step 9 — `return_url`: where a redirect rail must send the payer when
its own page is done with them. It is **the core's** answer, never an
adapter's (`docs/reference/rails.md`), and an adapter on a push rail must
ignore it.

## Capabilities

`flow`, `supports_refunds`, `supports_partial_refunds`, `delivers_callbacks`,
`requires_ip_allowlist`, `supports_account_holder_lookup`,
`refund_destination`.

| Capability                       | `mtn_momo` | `orange_money` | What the core does with it                                                                                                                                                                                                                                                                                                                                          |
| -------------------------------- | ---------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `flow`                           | `Push`     | `Redirect`     | decides whether a confirm needs a `payer_ref` or a `return_url`, and whether `submit` may answer a `redirect_url`                                                                                                                                                                                                                                                   |
| `supports_refunds`               | `true`     | `false`        | refuses a refund on a rail with no refund API, with no rail-specific branch                                                                                                                                                                                                                                                                                         |
| `supports_partial_refunds`       | `true`     | `false`        | implies `supports_refunds`; a CHECK constraint in migration `0002` says so too                                                                                                                                                                                                                                                                                      |
| `delivers_callbacks`             | `true`     | `true`         | whether to expect a notification at all. Callbacks are hints either way                                                                                                                                                                                                                                                                                             |
| `requires_ip_allowlist`          | `true`     | `false`        | an operational fact for a deployment, not a code path                                                                                                                                                                                                                                                                                                               |
| `supports_account_holder_lookup` | `true`     | `false`        | refuses `GET /v1/account_holders` on a rail with no such API, with a `400` naming the parameter ([account-holder-lookup.md](account-holder-lookup.md), issue #47)                                                                                                                                                                                                   |
| `refund_destination`             | `Required` | `Required`     | whether a refund needs an explicit payee. **Declared only; no core code reads it yet** — the `POST /v1/refunds` that will is Wave 3 of [RFC-0003](../rfc/0003-refunds-destinations-and-the-first-ledger-postings.md) and is not built. Since 2026-09-15 both adapters implement `parse_destination`, so the raw map that handler will hand over has somewhere to go |

`orange_money` declares `supports_refunds: false`, and that flag — not a
rail-specific branch — is what makes the core refuse a refund on that rail. The
capability system earns its keep on day one.

**The last two rows are the newest and the only ones that are not
persisted.** The five above them are columns on `providers` (migration
`0002`), seeded at boot from the adapter's own declaration;
`supports_account_holder_lookup` and `refund_destination` have no column and
no `ProviderSeed` field, because nothing reads a capability _out of_ that
table — `vpay_api` resolves an adapter in-process and asks it — so a column
would be a second copy of an answer the linked code already owns. The flow
doc records the decision.

`refund_destination` is also the only capability with **no coherence rule**,
and that is deliberate rather than an omission. `orange_money` declares
`Required` while `supports_refunds` is still `false` — the rail's refunds are
outbound transfers, and it is vpay that has not built them (RFC-0003 § 5) — so
the obvious rule "a rail that cannot refund declares `Origin`" would force a
live declaration to lie. And there is nothing to mirror a Rust-only rule with:
`Capabilities::is_coherent` is one half of a pair whose other half is a CHECK
on `providers`, which this field has no column in.
`a_refund_destination_is_inert_to_coherence` in `vpay-provider` pins the
decision and says what would have to change first.

**A rail that _has_ the API but has not written the call declares `true`
anyway**, and overrides the port method with its own
`ProviderError::NotImplemented` token, exactly as `mtn_momo::refund` does.
`Unsupported` is a claim about the _rail_; a token is an admission about
_us_, and `verify-status` only sees the second one.

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
2. `INSERT INTO providers` with capability flags. _No schema migration._
   **Corrected 2026-09-06:** in a running deployment nobody writes this table
   by hand — `providers` is reconciled from `config.yaml` at boot step 4 by
   `vpay_db::ConfigReconcile::reconcile`, which is the only writer, so the
   real step 2 is a `providers[]` entry in the deployment's YAML plus the
   adapter of step 4. The sentence still holds where it matters (adding a
   rail needs no migration), and a hand-written `INSERT` at a psql prompt is
   still possible — but since migration 0033 it must name **all eight**
   columns: the five capability booleans have no column default, so an
   omitted one is a `23502` rather than an invented capability.
3. ~~`INSERT INTO provider_hosts` for sandbox, production and stub hosts.~~
   **Corrected 2026-09-06: there is no `provider_hosts` table and there never
   has been** — no migration under `backends/migrations` creates one, and the
   only other mention in the tree is a `vpay-testkit` doc comment that
   inherited the error from here. A rail's hosts are _configuration_:
   `providers[].host.{url,label}` in the deployment's YAML
   (`vpay_config::ProviderHost`), one entry per deployment, which is what
   makes a sandbox, a production and a WireMock stub three profiles rather
   than three rows. See [configuration.md](configuration.md).
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

**Updated 2026-09-15 (RFC-0003 wave 1b): the adapter parses the destination,
not the core.**

- `ProviderAdapter::parse_destination` — a `&serde_json::Map<String, Value>`
  in, a `Result<RefundTarget, ProviderError>` out — implements RFC-0003 open
  question 4, which the maintainer decided on 2026-09-15. Wave 1 had assumed the core
  would build a `RefundTarget` out of `destination[<rail_code>][msisdn]` the
  way the confirm path reads `payment_method_data[<code>]["msisdn"]`; the
  decision went the other way, so that a bank-account upstream costs **no**
  core change rather than a core that has learned a second set of wire keys.
  Both shapes satisfy ADR-0002; neither adds a rail-code branch.
- The default body is `ProviderError::Unsupported` — the honest permanent
  answer for an `Origin` rail, which has no destination to parse at all, and
  the same shape `refund` and `account_holder_name` already have. Both rails
  vpay carries declare `Required` and both override it.
- **`Measured` forwards it.** That is the dangerous half of a defaulted
  method: a missing forward compiles, leaves every adapter's own tests green
  because they hold the adapter unwrapped, and answers "this rail has no such
  API" for every refund in production, because
  `vpay_api::v1::boot::adapters_by_code` wraps every shipping adapter.
  `a_defaulted_method_is_not_silently_answered_by_the_wrapper` fails if the
  forward is deleted.
- **MSISDN validation did not move and was not copied — a named gap.** The
  rule the adapters apply is the _confirm path's_ (`payer_instrument`:
  present, a JSON string, not whitespace-only), so a number vpay accepts as a
  payer it accepts as a payee. It is **not** `vpay_api`'s stricter E.164
  `canonical_msisdn`, which is `pub(crate)` there and which an adapter crate
  cannot reach; writing a second spelling of it per adapter is the drift this
  refused. `not a phone number` is therefore accepted here and refused by the
  rail, exactly as on confirm today. Closing it means giving one
  canonicaliser a home both layers can see, which changes where the confirm
  path validates — [status/backend.md](../status/backend.md) records it as a
  maintainer decision rather than guessing.
- **Not built by this change:** still no `POST /v1/refunds` (wave 3), no core
  code reading `refund_destination`, no MTN Disbursements call, no Orange
  transfer, no ledger posting. Nothing calls `parse_destination` outside
  tests.
- Evidence:
  [verification/2026-09-15-refunds-w1b-parse-destination.md](../status/verification/2026-09-15-refunds-w1b-parse-destination.md).

**Updated 2026-09-15 (RFC-0003 wave 1): the port can express a refund
destination, and nothing calls it yet.**

- `Capabilities` gained `refund_destination: RefundDestination`
  (`Origin` | `Required`), and `ProviderAdapter::refund` gained
  `destination: Option<&RefundTarget>` between `amount` and `config`. The
  default body is still `ProviderError::Unsupported` and `mtn_momo::refund`
  is still its `NotImplemented` token — **no refund got closer to working**;
  what changed is that "does this rail need a payee?" is now a capability the
  core will branch on instead of a rail code (ADR-0002).
- Both rails declare `Required`. On `orange_money` that sits beside
  `supports_refunds: false` on purpose — see the Capabilities section above.
- `RefundTarget` carries a canonical MSISDN, keeps it private behind
  `msisdn()`, and its `Debug` renders `[redacted]`: a payee's number is the
  data RFC-0003 refused to put in `metadata`, and its retention is that
  RFC's open question 2, undecided. _(Superseded in part by wave 1b above:
  "canonical" is weaker than it sounds — see the MSISDN gap there. Wave 1
  also expected `vpay_api` to construct one; the adapter does.)_
- **Not built by this change:** `POST /v1/refunds` (wave 3), any MTN
  Disbursements call (RFC-0003 § 5), any Orange transfer call, any ledger
  posting. The core does not yet check a destination against this capability,
  because there is no core path that creates a refund at all.
- **Three claims on this page are time-limited, and only one of them has a
  test.** `orange_money`'s `false` in the `supports_refunds` row, and the
  paragraph under the table that reads that `false` as the capability system
  earning its keep, both stop being true when RFC-0003 § 5 flips the flag; the
  `refund_destination` row's "no core code reads it yet" stops being true when
  § 2's `POST /v1/refunds` lands. The one guard is
  `a_refund_on_this_rail_would_need_a_payee` in `vpay-adapter-orange-money`,
  whose failure message names this page. Nothing guards the third — the arm
  that builds the route has to come back here.
- **The destination is proven to survive the decorator.** `Measured` is what
  `vpay_api::v1::boot::adapters_by_code` wraps every shipping adapter in, and
  until the correctness review of the same day nothing observed what it
  forwarded: dropping the destination there left 199 tests green, because no
  adapter reads the argument yet.
  `the_destination_reaches_the_inner_adapter_and_never_a_metric` is what fails
  now, and it also asserts the payee's number reaches no metric label. The
  conformance suite asserts the other half — a payee is supplied exactly when
  the rail declares `Required`, read off the adapter's own capability. See
  [verification/2026-09-15-refunds-w1-provider-port.md](../status/verification/2026-09-15-refunds-w1-provider-port.md).

**Updated 2026-09-06 (review of exp20): "Adding a rail" steps 2 and 3 above
were both wrong about where a rail is written down.** Step 2 said `INSERT INTO
providers`; `ConfigReconcile::reconcile` is the only writer of that table and
takes its rows from `config.yaml` at boot step 4, and since migration 0033 a
hand-written `INSERT` must name all eight columns. Step 3 named a
`provider_hosts` table that does not exist and never has — hosts are
`providers[].host` in the YAML. Both are corrected in place rather than
deleted, because the wrong version is what a reader of an older checkout has.

**Updated 2026-09-03 (Step 3). The port is implemented, and two rails now
speak over it to a real HTTP host.**

- The trait is `#[async_trait]`; `submit`, `query_status` and `refund` are
  `async`, `parse_callback` is not. `ProviderConfig` gained
  `connect_timeout`/`request_timeout` (`DEFAULT_CONNECT_TIMEOUT` 5 s,
  `DEFAULT_REQUEST_TIMEOUT` 20 s).
- **Updated 2026-09-05 (issue #46): `refund` answers `Refunded`, not
  `Submitted`.** The new type carries `ref_extra` exactly as before plus
  `fee: Option<Money>` — what the rail charged us to move the money, in the
  refund's own currency. **No adapter populates it**, and none can: Orange
  has no refund API and `mtn_momo::refund` is still `NotImplemented`. `None`
  means "the rail did not report a fee" and `Some(zero)` means "the movement
  was free"; an adapter that collapsed the two would put an invented number
  in a merchant's settlement statement. See [../status.md](../status.md).
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
33 tests — 4 capability cases plus 13 port cases parameterised over both
rails, and 3 MTN-only cases — run live against a real `wiremock/wiremock`
container started by `vpay_testkit::containers::start_wiremock`. **33 passed,
0 skipped, 0 ignored**, measured on 2026-09-04 (26 on 2026-09-03; Step 8 lane C
added `the_submit_tells_the_rail_where_to_call_back`, Step 9 lane 2 added
`the_submit_tells_the_rail_where_to_send_the_payer_back`, and Step 9 lane 2b
added `a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin` with three
cases on the push rail alone). The 11 port cases are
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
suite and is unproven on both rails. ~~No callback route exists, so
`parse_callback`'s output is verified by tests and by nothing in production.~~
**Corrected 2026-09-04 (Step 8, lane C): since Step 8 `parse_callback`'s output
_is_ consumed in production**, by `vpay_api::provider_callback` — but only to
name a charge and pull its poll job forward, and only from a body no rail has
ever actually sent to this deployment. **The route is still a hint that never
moves state**, and since 2026-09-04 (Step 8, lane H) what it costs is measured
rather than asserted. It runs **two statements in one transaction** —
`TxRepositories::enqueue_in_tx` (`ON CONFLICT DO NOTHING`, so nothing in the
ordinary case) and `TxRepositories::pull_forward_in_tx` — and the pull-forward
now takes a **floor**: `vpay_api::provider_callback::PULL_FORWARD_FLOOR`, ten
seconds, the poll ladder's own fastest rung, below which a job already due is
left exactly where the ladder put it. **The true bound, stated because the
module used to overstate it:** a charge the queue was about to ask about anyway
costs a caller nothing, but the ladder's rungs grow (20 s, 30 s, 45 s …) while
the floor stays at ten, so a charge parked further out is still brought forward
by every callback and a caller repeating against one live charge can hold it at
roughly one authenticated `query_status` per worker claim. **There is no rate
limit**, per charge or per source. **And the floor has a behavioural cost:** a
rail calling back while the charge sits on the ladder's _first_ rung no longer
settles it early — it settles at that rung, up to ten seconds later than before
(`a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run`).
See [../status.md](../status.md).
