# SDK parity matrix

The record [ADR-0015](../adr/0015-sdk-parity.md) requires: one row per
capability, one column per merchant SDK, and a cell that is either

- **✅** followed by the test name(s) that prove the capability **in that
  SDK** — nothing else, each name in a code span; or
- **⛔** followed by a `YYYY-MM-DD` date, the reason, and who owns closing it.

`cargo xtask verify-sdk-parity` reads this file on every `just verify`. Every
name in a ✅ cell must exist under that column's directory as a live Rust
`#[test]`/`#[tokio::test]` function or a TypeScript `it("…")`/`test("…")`; an
`#[ignore]`d or `it.skip`ped test does not count. Every ⛔ must carry a date.
No cell may be blank. Rename a test without editing this file and `just
verify` fails naming the cell.

## The gate reads this file **and** the SDKs, since 2026-09-06

Until that date it only ever read this file, and could therefore only check
whether what this file *said* was true. Deleting a whole capability row was
measured to pass — 350 proving tests dropped to 347 and `just verify` stayed
green — and an SDK method with no row at all was invisible. ADR-0015's rule
is a claim about the SDKs ("every SDK ships every feature or a dated gap"),
so the gate now runs in both directions:

- **code → doc.** Every `<resource>.<method>` either SDK declares must have a
  row here. A method with no row fails, naming the `file:line` it is declared
  on.
- **doc → code.** Every `<resource>.<method>` row must name a method at least
  one SDK declares — *unless* every one of its cells is a dated ⛔, which is
  how a capability written down before it exists is recorded (the
  `events.retrieve` row below has been ⛔/⛔ since 2026-09-03). A stale row
  fails, naming its own line.

### How a capability row is named

A row is a **capability row** when its first cell *opens* with a code span
holding `<resource>.<method>`, in the SDKs' own spelling:

| Source | Read as |
|---|---|
| Rust `impl <Resource>Resource { pub async fn <method>(` in `sdks/rust/src/resources.rs` | `<resource_snake>.<method>` |
| Node exported class methods in `sdks/nodejs/src/resources/<resource>.ts` | `<resource_snake>.<method>` |

Resource names map by snake_case in both languages, so
`PaymentIntentsResource` and `client.paymentIntents` are both
`payment_intents`, and `AccountHoldersResource` / `client.accountHolders` are
both `account_holders`. The one nested resource keeps the spelling a merchant
reads — `checkout.sessions`, not `checkout_sessions` — because that is what
`client.checkout().sessions()` and `client.checkout.sessions` say and what
every row here has said since 2026-09-04. Private helpers, constructors and
namespace accessors (Rust `CheckoutResource::sessions`, Node
`CheckoutResource.sessions`) are not capabilities.

**Opening with the span is load-bearing, not tidiness.** Rows that describe a
behaviour spanning several methods carry no leading span and are checked by
the cell rules alone, and rows that *mention* a dotted code span mid-sentence
— the `checkout.session.expired` event-type rows below — must not be read as
naming a method, because there is no such method and there must not be one.

**Measured by reading both SDKs, 2026-09-03**, again for the Checkout Session
rows on **2026-09-04** (Step 9), again on **2026-09-04** for the
assertion-audience row (Step 9, lane 5b), and again on **2026-09-04** for the
two `checkout.session.expired` rows, and again on **2026-09-05** for the
`refunds.retrieve` row (issue #45, the change that made
`GET /v1/refunds/{id}` a served route). Nothing here is inferred from a file
name or a doc comment.

Again on **2026-09-05** for the five `account_holders` rows (issue #47). Two
things about those are worth stating rather than leaving to be discovered:
the Node accessor is `client.accountHolders` (camelCase, like
`client.paymentIntents`) while its **request field** is
`payment_method_type` (snake_case, like every other params type in that
package and like the wire) — the issue's own sketch spelled the field
`paymentMethodType`, and following it would have made this the only
camelCase request field in the SDK. And neither SDK validates the MSISDN
locally, deliberately and identically: a phone-number rule is a *market*
rule vpay owns and may widen, so an SDK copy would refuse offline a number a
later server version accepts.

A note on the first of those two, because the two SDKs are at parity on the
*capability* and not on the shape: `@vaam-apps/vpay-sdk` has always carried a
`KnownEventType` string union, and `sdks/rust` had no event-type vocabulary at
all until 2026-09-04. Adding the type meant adding
`vpay_sdk::KnownEventType` — a `#[non_exhaustive]` enum with `as_wire_str` /
`from_wire`, mirroring `PaymentMethodType`'s shape in the same file — so the
Rust column names an enum where the Node column names a union. Both keep
`Event::kind` / `Event.type` a plain string, so an event type either SDK
version predates is still deliverable rather than a decode failure; that is
the property the proving tests assert, and it is what makes the two cells
comparable at all (ADR-0015's decision 1: parity is per capability, not per
method name).

> A capability absent from *both* SDKs is still recorded ⛔/⛔ — the SDKs are
> at parity with each other and both short of the server. That is a different
> statement from "done", and the rule that keeps this table honest is that
> neither shape may be silently omitted.

---

## Authentication — the `private_key_jwt` handshake

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| RS256 assertion with the six claims the OP verifier reads, and no others | ✅ `mints_an_assertion_with_the_expected_claim_shape` | ✅ `produces a header with alg RS256, typ JWT, and no kid when none is configured`, `carries exactly the six claims the OP verifier expects and no nbf` |
| `iss` and `sub` are the `client_id` | ✅ `mints_an_assertion_with_the_expected_claim_shape` | ✅ `sets iss and sub to the client_id, and aud to the token endpoint`, `sets iss and sub to the configured client_id` |
| `kid` stamped when configured, omitted when not | ✅ `omits_kid_from_the_header_when_none_was_configured`, `stamps_a_configured_kid_onto_the_header` | ✅ `includes kid in the header exactly when configured`, `forwards a configured kid into the assertion header`, `omits kid from the assertion header when none is configured` |
| A fresh UUIDv4 `jti` per mint | ✅ `mints_a_fresh_jti_on_every_call` | ✅ `mints a UUIDv4 jti that differs across two mints` |
| `exp` is `iat` plus exactly the configured lifetime | ✅ `exp_is_iat_plus_exactly_the_configured_lifetime` | ✅ `sets exp exactly lifetimeSeconds after iat, and within 300s of now`, `honours a configured assertionLifetimeSeconds` |
| Lifetime bound `1..=300` refused at construction, never clamped | ✅ `rejects_a_lifetime_below_one_second`, `rejects_a_lifetime_above_three_hundred_seconds`, `accepts_the_boundary_values`, `build_rejects_an_assertion_lifetime_outside_one_to_three_hundred_seconds` | ✅ `throws VpayConfigError when lifetimeSeconds exceeds 300`, `throws VpayConfigError when lifetimeSeconds is 0`, `throws VpayConfigError when lifetimeSeconds is not an integer`, `accepts the boundary values 1 and 300`, `rejects an assertionLifetimeSeconds outside 1..=300 at construction` |
| The assertion `aud` is an endpoint URL, never the `audience` parameter | ✅ `the_assertion_audience_follows_an_overridden_token_endpoint` | ✅ `sets aud to the token endpoint URL, never to the audience parameter`, `keeps aud on the token endpoint even when audience is overridden`, `follows a custom tokenEndpoint into aud` |
| The assertion `aud` is settable independently of the URL the token request is POSTed to | ✅ `the_assertion_audience_defaults_to_the_token_endpoint`, `an_explicit_assertion_audience_does_not_move_the_token_endpoint`, `an_overridden_token_endpoint_still_supplies_the_default_assertion_audience`, `an_explicit_assertion_audience_is_signed_without_moving_the_request`, `an_unset_assertion_audience_signs_the_url_the_request_went_to`, `the_real_verifier_refuses_a_client_that_reaches_vpay_internally_and_sets_no_audience`, `the_real_verifier_accepts_the_same_client_once_assertion_audience_is_set`, `the_issuer_works_as_an_assertion_audience_too` | ✅ `leaves aud on the token endpoint when assertionAudience is not set`, `signs aud as the configured assertionAudience while still POSTing to the token endpoint`, `accepts the issuer as an assertionAudience, which the OP also allows`, `is refused by the OP audience check when assertionAudience is left unset`, `authenticates once assertionAudience names the OP's own token endpoint` |
| A private key the SDK cannot read is refused before any request | ✅ `a_rejected_private_key_does_not_echo_the_key_into_the_error`, `build_fails_without_credentials` | ✅ `throws VpayConfigError from the constructor, before any request`, `requires baseUrl, clientId and privateKey` |
| The signature verifies against the matching public key | ✅ `an_assertion_without_a_kid_is_accepted_when_one_key_is_registered` | ✅ `produces a signature verifiable with the matching public key`, `fails verification against a different public key` |
| The assertion is accepted by the **real** pinned `authkestra-op` verifier, in CI | ✅ `an_assertion_without_a_kid_is_accepted_when_one_key_is_registered`, `a_kid_selects_the_matching_key_out_of_two_registered_keys`, `an_assertion_naming_a_kid_it_did_not_sign_with_is_refused`, `an_assertion_signed_by_an_unregistered_keypair_is_refused`, `an_assertion_minted_for_another_audience_is_refused`, `an_assertion_for_a_different_client_id_is_refused` | ⛔ 2026-09-03 — Node cannot link the Rust verifier. `just sdk-conformance-node` pipes a Node-minted assertion into `sdks/rust/examples/verify_assertion.rs`, but it is a manual recipe outside `just ci`, so no test in this package proves it. Owner: SDK maintainers |

## Token lifecycle

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| Token request carries exactly the documented form fields, and no client secret | ✅ `the_token_request_carries_exactly_the_documented_form_fields` | ✅ `sends the exact form fields and content type, and no client_secret` |
| `scope` sent when configured, omitted entirely when not | ✅ `a_configured_scope_is_sent_and_an_unconfigured_one_is_omitted` | ✅ `omits scope from the token request when not configured, and includes it when configured` |
| The token is presented as `Authorization: Bearer` on resource calls | ✅ `the_access_token_is_presented_as_a_bearer_header_on_the_next_resource_call` | ✅ `carries the access token as Authorization: Bearer on the following resource call` |
| Token cached, and refreshed at `expires_in` minus the margin | ✅ `a_cached_token_is_reused_across_calls`, `an_expired_token_is_refreshed` | ✅ `reuses one token across two resource calls`, `re-authenticates once the cached token has passed expires_in minus the safety margin` |
| The margin is 30 s, or half of `expires_in` for a short TTL, integer arithmetic | ✅ `cached_token_margin_is_thirty_seconds_or_half_expires_in_whichever_is_smaller` | ✅ `halves the margin instead of using 30s when expires_in is short` |
| Concurrent callers share one in-flight token request | ✅ `concurrent_first_calls_share_one_token_request` | ✅ `shares one in-flight token request across concurrent callers` |
| A token-endpoint rejection is its own error and is never retried | ✅ `a_token_endpoint_rejection_surfaces_as_a_token_error_and_is_never_retried` | ✅ `maps a token-endpoint 401 to VpayAuthError and never retries it` |
| A 200 whose `token_type` is not `Bearer` is refused | ⛔ 2026-09-03 — the Rust `TokenResponse` decodes `access_token` and `expires_in` only. `token_type` is never read, so a `DPoP` or `MAC` response is accepted and then presented as a Bearer. Node refuses it. Owner: SDK maintainers | ✅ `rejects a 200 whose token_type is not Bearer`, `rejects a 200 with no token_type at all`, `accepts a lowercase bearer, which RFC 6749 §7.1 makes case-insensitive` |

## Re-authentication

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| Exactly one re-auth-and-retry on a 401, and a second 401 is returned | ✅ `a_401_from_a_resource_route_triggers_exactly_one_reauth_and_retry`, `a_second_consecutive_401_is_returned_to_the_caller` | ✅ `discards the token, re-authenticates, and retries once on a single 401`, `throws VpayApiError after two consecutive 401s, having hit the token endpoint exactly twice` |
| The retry replays the identical body and `Idempotency-Key` | ✅ `a_reauthed_post_replays_the_callers_own_idempotency_key_and_body`, `a_reauthed_post_replays_the_generated_idempotency_key_too`, `a_reauthed_confirm_replays_its_nested_body_byte_for_byte` | ✅ `resends the caller-supplied Idempotency-Key and an identical body`, `resends the same generated Idempotency-Key when the caller supplied none`, `resends an identical body for a confirm, whose body is nested` |
| A second concurrent 401 does not discard the token the first one just fetched | ✅ `a_second_concurrent_401_does_not_discard_the_token_the_first_one_just_fetched` | ⛔ 2026-09-03 — `TokenManager.invalidate()` clears unconditionally, with no compare-and-swap against the token that was actually refused, so the losing caller of a concurrent 401 pair discards a valid token and spends a second assertion. Not a correctness failure, but a divergence in behaviour under load, and untested. Owner: SDK maintainers |
| Automatic retry of anything other than the single 401 (5xx, timeout, backoff) | ⛔ 2026-09-03 — no retry policy exists; a 5xx or a timeout is returned to the caller. Deliberate today, and recorded so the two SDKs stay equal rather than one growing a policy quietly. Owner: SDK maintainers | ⛔ 2026-09-03 — same: `HttpClient.request` loops only for the 401 re-auth. Owner: SDK maintainers |

## Transport, headers and URLs

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| `User-Agent` names this SDK and its own version | ✅ `the_user_agent_names_this_sdk_and_its_version` | ✅ `sends Accept and User-Agent on a GET resource call`, `sends Accept and User-Agent on a POST resource call`, `sends the same User-Agent on the token call and the resource call`, `matches package.json, so the User-Agent never lies about the version` |
| Documented endpoint defaults derived from one `base_url` | ✅ `derives_the_documented_defaults`, `overriding_the_issuer_moves_the_default_token_endpoint_but_not_the_resource_base` | ✅ `strips one trailing slash so paths and the assertion aud never double a slash` |
| One trailing slash on the base URL is normalised | ✅ `strips_a_trailing_slash_from_base_url` | ✅ `strips one trailing slash so paths and the assertion aud never double a slash` |
| A **repeated** trailing slash is normalised | ⛔ 2026-09-03 — `trim_end_matches` does strip a run, but no test covers it, so the two SDKs' answers to `https://api.vpay.example//` are unproven and known to differ. Owner: SDK maintainers | ⛔ 2026-09-03 — `stripTrailingSlash` removes exactly one, so `https://api.vpay.example//` yields `//v1`. `@vaam-apps/vpay-stripe-js` already fixed this class of bug for the browser surface. Owner: SDK maintainers |
| A merchant-supplied id is percent-encoded and cannot escape `/v1` | ✅ `an_id_with_url_metacharacters_is_percent_encoded_into_the_path`, `confirm_and_cancel_encode_the_id_too` | ✅ `percent-encodes a path id so it can never escape the /v1 namespace`, `percent-encodes a hostile id on confirm too` |
| The two SDKs encode byte-identical form bodies | ✅ `create_payment_intent_body_matches_the_node_sdk_byte_for_byte`, `confirm_body_matches_the_node_sdk_byte_for_byte`, `leaves_exactly_the_characters_encodeuricomponent_leaves` | ✅ `encodes the pinned payment_intents.create example exactly`, `encodes nested objects with bracket notation`, `encodes arrays with numeric indices, in order`, `percent-encodes brackets a merchant put inside a key, rather than reading them as structure`, `keeps a merchant bracket key distinct from the structure it imitates` |
| A request timeout fires and surfaces as this SDK's transport error | ⛔ 2026-09-03 — `ClientBuilder::timeout` is applied to the reqwest client and defaults to 30 s, but no test makes one fire, so nothing proves it maps to `Error::Transport` rather than escaping as something else. Owner: SDK maintainers | ✅ `maps a server that accepts the connection and never answers to VpayTransportError`, `maps a stall part-way through the response body to VpayTransportError`, `maps a stall part-way through the token response body to VpayTransportError` |
| TLS trust roots are the SDK's own, and no process-wide crypto state is installed | ✅ `a_client_builds_in_a_process_that_never_installed_a_crypto_provider`, `the_tls_config_carries_vendored_roots_and_advertises_http2`, `an_https_url_actually_attempts_a_tls_handshake` | ⛔ 2026-09-03 — the package calls the global `fetch` and configures no trust store, so its roots are whatever the host Node was built with. Nothing here tests TLS at all. Owner: SDK maintainers |

## `/v1` resource operations

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| `payment_intents.create` — path, body, response object | ✅ `create_payment_intent_sends_the_documented_body_and_decodes_the_object`, `create_omits_absent_optional_fields_rather_than_sending_them_empty` | ✅ `payment_intents.create: exact path, method, Idempotency-Key, and body` |
| `payment_intents.retrieve` | ✅ `retrieve_payment_intent_is_a_get_with_no_body_and_decodes_next_action` | ✅ `payment_intents.retrieve: exact GET path` |
| `payment_intents.confirm`, push rail and redirect rail | ✅ `confirm_sends_the_push_rail_instrument`, `confirm_sends_the_redirect_rail_return_url` | ✅ `payment_intents.confirm on a push rail: exact path and body`, `payment_intents.confirm on a redirect rail: includes return_url` |
| `payment_intents.cancel` | ✅ `cancel_posts_an_empty_body_and_still_carries_an_idempotency_key` | ✅ `payment_intents.cancel: exact path and method` |
| `payment_intents.list`, with `limit` and both cursors | ✅ `list_payment_intents_encodes_its_pagination_into_the_query_string`, `a_list_call_with_no_parameters_sends_no_query_string_at_all` | ✅ `payment_intents.list: exact query string` |
| `refunds.create`, and a full refund omitting `amount` | ✅ `create_refund_sends_the_documented_body_and_decodes_the_object`, `a_full_refund_omits_the_amount_entirely` | ✅ `refunds.create: exact path and body, amount omitted for a full refund` |
| `refunds.retrieve` — `GET /v1/refunds/{id}`, no body, no `Idempotency-Key` | ✅ `retrieve_refund_is_a_get_with_no_body_and_decodes_the_object`, `a_refund_id_with_url_metacharacters_is_percent_encoded_into_the_path` | ✅ `refunds.retrieve: exact GET path, no body, no Idempotency-Key`, `refunds percent-encodes a hostile id so it cannot escape /v1` |
| The `checkout.session.expired` event type is in this SDK's vocabulary, and its payload decodes as a session | ✅ `a_checkout_session_expired_event_is_a_known_type_and_decodes_as_a_session`, `an_unknown_event_type_is_none_rather_than_a_failure_and_the_wrong_accessor_errs` | ✅ `is a member of KnownEventType and narrows with isCheckoutSessionEvent`, `is not narrowed by the payment-intent or refund guards`, `leaves an unknown checkout.session.* type deliverable rather than a failure` |
| A delivered `checkout.session.expired` carries no `client_secret` and a null `url`, and a null `url` is still distinguishable from an embedded session | ✅ `a_checkout_session_expired_event_is_a_known_type_and_decodes_as_a_session` | ✅ `carries no client_secret and a null url, so a webhook body holds no payer credential` |
| `events.list`, with cursors and the `type` filter | ✅ `list_events_filters_by_type_and_keeps_data_object_as_raw_json` | ✅ `events.list: exact query string including type` |
| `events.retrieve` — `GET /v1/events/{id}` | ⛔ 2026-09-03 — the route is mounted and served (`vpay_api::v1::V1_ROUTES`, `events::retrieve`) and this SDK has no method for it. A merchant who missed a webhook is told to re-read the event, and cannot. Owner: SDK maintainers | ⛔ 2026-09-03 — same: `EventsResource` exposes `list` only. Owner: SDK maintainers |
| `balance.retrieve` | ✅ `retrieve_balance_is_a_bare_get_and_decodes_both_buckets` | ✅ `balance.retrieve: exact path, no body` |
| `account_holders.retrieve` — path, query string, response object (issue #47) | ✅ `retrieve_account_holder_sends_the_documented_query_and_decodes_the_name` | ✅ `accountHolders.retrieve: sends the documented query and decodes the name` |
| An account holder the rail has no record of decodes as a **present null** `name`, not as an absence and not as an error | ✅ `a_holder_the_rail_does_not_know_decodes_as_a_present_null_name` | ✅ `accountHolders.retrieve: a holder the rail does not know decodes as a present null name` |
| A rail that could not be *asked* raises this SDK's API error rather than answering a null `name` — the distinction the resource exists for | ✅ `a_rail_that_could_not_be_asked_is_an_error_and_not_a_null_name` | ✅ `accountHolders.retrieve: a rail that could not be asked throws rather than answering a null name` |
| A `payment_method_type` whose rail has no account-holder API is surfaced as the server's `400` naming the parameter, and is **not** pre-empted locally | ✅ `a_rail_with_no_account_holder_api_surfaces_the_servers_named_parameter` | ✅ `accountHolders.retrieve: a rail with no such API surfaces the server's named parameter` |
| `account_holders.retrieve` exercised against a running vpay | ⛔ 2026-09-05 — every server in these cases is `wiremock`. The route is real and `backends/tests/integration/tests/account_holders.rs` drives it over a socket against the shipping adapter, but **not through this SDK**, so "the stub answers the way this SDK expects" is the whole of the evidence here. Recorded ⛔/⛔ because both SDKs are equally short of the server. Owner: SDK maintainers | ⛔ 2026-09-05 — same: every server in these cases is `src/testing/test-server.ts`. Owner: SDK maintainers |
| `checkout.sessions.create` — path, body, and every unset field omitted rather than sent empty | ✅ `create_checkout_session_sends_the_documented_body_and_decodes_the_object`, `create_checkout_session_omits_absent_optional_fields_rather_than_sending_them_empty` | ✅ `checkout.sessions.create: exact path, method, Idempotency-Key, and body`, `checkout.sessions.create omits every field the caller left unset`, `checkout.sessions.create generates an Idempotency-Key when the caller supplies none` |
| `checkout.sessions.create` puts byte-identical bodies on the wire from both SDKs | ✅ `create_checkout_session_body_matches_the_node_sdk_byte_for_byte` | ✅ `checkout.sessions.create sends an embedded session's return_url` |
| `checkout.sessions.retrieve`, and the `client_secret` it carries | ✅ `retrieve_checkout_session_is_a_get_with_no_body_and_surfaces_client_secret` | ✅ `checkout.sessions.retrieve: exact GET path, and the client_secret it carries` |
| `checkout.sessions.list`, with the `payment_intent` filter, and no secret on an item | ✅ `list_checkout_sessions_encodes_its_pagination_and_intent_filter` | ✅ `checkout.sessions.list: exact query string including the payment_intent filter` |
| `checkout.sessions.expire` — empty-bodied POST, still idempotency-keyed | ✅ `expire_checkout_session_posts_an_empty_body_and_still_carries_an_idempotency_key` | ✅ `checkout.sessions.expire: exact path, method and empty body` |
| A hostile checkout-session id is percent-encoded and cannot escape `/v1` | ✅ `a_checkout_session_id_with_url_metacharacters_is_percent_encoded_into_the_path` | ✅ `checkout.sessions percent-encodes a hostile id so it cannot escape /v1` |
| The session `404` and the expire `409` map to this SDK's own API error | ✅ `a_404_for_an_unknown_checkout_session_maps_to_an_api_error`, `a_409_on_expiring_a_session_with_a_live_charge_maps_to_an_api_error` | ✅ `checkout.sessions maps the 404 envelope for an unknown session`, `checkout.sessions maps a 409 on expiring a session with a live charge` |
| Checkout Sessions exercised against a running vpay | ⛔ 2026-09-04 — every server in these cases is `wiremock`. `/v1/checkout/sessions` is built by lane 1 of the same step and `backends/tests/integration/tests/checkout_sessions.rs` is its proof; until that is green, "the stub answers the way this SDK expects" is the whole of the evidence. Recorded ⛔/⛔ because both SDKs are equally short of the server, which is a different statement from "done". Owner: SDK maintainers | ⛔ 2026-09-04 — same: every server in these cases is `src/testing/test-server.ts`. Owner: SDK maintainers |
| An `Idempotency-Key` on every POST, caller-supplied or a generated UUIDv4 | ✅ `a_post_without_a_caller_supplied_key_generates_a_uuid_v4_idempotency_key`, `cancel_posts_an_empty_body_and_still_carries_an_idempotency_key` | ✅ `payment_intents.create: exact path, method, Idempotency-Key, and body`, `payment_intents.create generates an Idempotency-Key when the caller supplies none` |

## Request validation

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| An amount outside `0..=2^53-1` is refused before anything is sent | ✅ `an_amount_outside_the_cross_sdk_safe_range_is_refused_before_any_request`, `the_largest_safe_amount_is_still_sent`, `refuses_a_negative_amount`, `refuses_an_amount_past_the_safe_integer_ceiling`, `accepts_zero_and_the_maximum_safe_integer` | ✅ `payment_intents.create refuses a negative amount before sending`, `payment_intents.create refuses 1e21, which Number.isInteger accepts`, `refunds.create refuses a negative amount before sending`, `throws TypeError on an integer past MAX_SAFE_INTEGER`, `accepts zero and the largest safe integer` |
| The refusal names the offending field | ✅ `the_message_names_the_field_it_was_given` | ✅ `names the field it was given` |
| The refusal is one of **this SDK's own** error types, catchable with the rest | ✅ `refuses_a_negative_amount` | ⛔ 2026-09-03 — `assertIntegerAmount` throws a bare `TypeError`, not a `VpayError`, so `catch (err) { if (err instanceof VpayError) … }` — the one narrowing the package documents — misses it. Rust returns `Error::InvalidParams`. Owner: SDK maintainers |

## Error mapping

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| A Stripe-shaped envelope maps to a typed API error carrying all four fields | ✅ `a_400_error_envelope_maps_to_an_api_error_carrying_all_four_fields`, `an_envelope_without_the_optional_fields_still_maps_to_an_api_error` | ✅ `maps a 400 with the Stripe envelope to VpayApiError with all fields` |
| A non-envelope body maps to an unexpected-response error with a bounded prefix | ✅ `a_proxy_html_502_maps_to_an_unexpected_response_with_the_status_and_body`, `an_oversized_error_body_is_truncated_to_a_bounded_prefix`, `a_success_body_that_is_not_the_expected_object_is_an_unexpected_response` | ✅ `maps a 502 HTML body to VpayUnexpectedResponseError carrying a bounded prefix`, `carries the status and the bounded prefix, and names both in its message` |
| The bound is on bytes, and a character straddling the cut is dropped, not mangled | ✅ `an_oversized_multibyte_error_body_is_cut_on_a_character_boundary`, `bounded_prefix_cuts_on_a_character_boundary_and_keeps_real_replacements` | ✅ `bounds a multi-byte body at 500 bytes, not 500 code units`, `drops a character straddling the cut rather than emitting U+FFFD`, `leaves a body of exactly 500 bytes untouched` |
| A transport failure is its own error, distinct from an HTTP one | ✅ `a_refused_connection_to_the_token_endpoint_is_a_transport_error`, `a_refused_connection_to_a_resource_route_is_a_transport_error` | ✅ `maps a connection failure to VpayTransportError` |
| A token endpoint answering HTML is an unexpected response, not a token error | ✅ `a_token_endpoint_returning_html_is_an_unexpected_response_not_a_token_error` | ✅ `maps a 502 HTML body to VpayUnexpectedResponseError carrying a bounded prefix` |
| The `request-id` header is surfaced on responses and on errors | ⛔ 2026-09-03 — vpay mirrors one request id under `request-id` and `x-request-id` on every response (`vpay_api::STRIPE_REQUEST_ID_HEADER`), and stripe-node reads it. This SDK reads no response header at all, so a merchant cannot quote the id in a support ticket. Owner: SDK maintainers | ⛔ 2026-09-03 — same: `HttpClient.#mapResponse` is handed the status and the body text and never sees the headers. Owner: SDK maintainers |
| `stripe-should-retry` is read and acted on | ⛔ 2026-09-03 — vpay derives the header from `Classify::retry` and sends it on every rendered error. Neither the value nor the header is read here. Owner: SDK maintainers | ⛔ 2026-09-03 — same. Owner: SDK maintainers |

## Diagnostics and redaction

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| The private key never reaches diagnostic output | ✅ `credentials_debug_shows_the_client_id_and_kid_but_no_key_material`, `client_and_builder_debug_carry_no_key_material`, `credentials_debug_output_never_contains_the_pem`, `builder_debug_output_never_contains_the_pem` | ✅ `never includes the PEM in util.inspect output`, `never includes the PEM in JSON.stringify output` |
| A rejected key is not echoed into the error that rejects it | ✅ `a_rejected_private_key_does_not_echo_the_key_into_the_error` | ✅ `throws VpayConfigError from the constructor, before any request` |
| A cached access token never reaches diagnostic output | ✅ `a_cached_access_token_never_appears_in_the_clients_debug_output`, `client_debug_output_never_contains_the_pem_or_a_cached_token` | ✅ `never appears in util.inspect of the client, even after a successful exchange` |
| A cached access token never reaches a **thrown error's** output | ⛔ 2026-09-03 — no error variant carries a header or a token, so there is nothing to leak; but nothing asserts that, and the Node suite does. Recorded rather than assumed. Owner: SDK maintainers | ✅ `never appears in util.inspect of a thrown VpayApiError`, `never appears in util.inspect of a thrown VpayTransportError, cause chain included`, `never appears in util.inspect of a thrown VpayUnexpectedResponseError` |
| `client_secret` decodes on `create`/`retrieve` and is absent on a list item | ✅ `create_surfaces_client_secret_when_the_server_sends_it`, `retrieve_surfaces_client_secret_when_the_server_sends_it`, `a_list_items_client_secret_is_none`, `a_create_or_retrieve_response_carrying_client_secret_decodes_it` | ✅ `payment_intents.create surfaces client_secret typed, when the server sends it`, `payment_intents.retrieve surfaces client_secret typed, when the server sends it` |
| `client_secret` is redacted from the payment intent's own diagnostic output | ✅ `a_payment_intents_debug_output_never_contains_its_client_secret`, `client_secret_never_appears_in_debug_output_but_its_absence_or_length_does` | ⛔ 2026-09-03 — `PaymentIntent` is a plain interface, so `console.log(intent)` and `JSON.stringify(intent)` print a live payer credential verbatim. `@vaam-apps/vpay-stripe-js` redacts its own. Owner: SDK maintainers |
| A checkout session's `client_secret` — **and the `url` fragment carrying the same value** — are redacted from diagnostic output | ✅ `a_checkout_sessions_debug_output_never_contains_its_client_secret_or_its_url_fragment`, `a_checkout_session_without_a_secret_or_a_fragment_renders_no_redaction_marker` | ✅ `redacts a checkout session's client_secret from util.inspect, and leaves JSON faithful`, `redacts a list item's url fragment too, though it has no client_secret to redact`, `leaves a session whose url has no fragment untouched` |
| Public option types are safe under `exactOptionalPropertyTypes` | ⛔ 2026-09-03 — not applicable to Rust and recorded as such rather than left blank: `Option<T>` has no equivalent hazard, so there is nothing here to build or to test. Owner: n/a | ✅ `accepts an explicitly undefined value for every optional property` |

## Webhook verification

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| A validly signed payload is accepted and the event decoded | ✅ `a_validly_signed_payload_is_accepted` | ✅ `accepts a validly signed payload and returns the parsed event` |
| The wrong secret, and a body altered by one byte, are both rejected | ✅ `a_signature_from_the_wrong_secret_is_rejected`, `a_body_altered_by_one_byte_is_rejected` | ✅ `rejects a signature computed with the wrong secret`, `rejects when the body changes by even one byte` |
| More than one `v1=` is accepted during a secret rotation | ✅ `a_second_v1_value_matching_is_accepted_during_a_secret_rotation` | ✅ `accepts a payload when the second of two v1 signatures matches (secret rotation)` |
| Tolerance window, including the exact boundary | ✅ `a_timestamp_outside_tolerance_is_rejected`, `a_timestamp_exactly_on_the_tolerance_boundary_is_accepted` | ✅ `rejects a timestamp outside the tolerance window`, `accepts a timestamp exactly at the tolerance boundary`, `respects a custom toleranceSeconds` |
| The HMAC covers the **literal** `t` text, not a re-rendered number | ✅ `the_hmac_covers_the_literal_t_text_not_a_re_rendered_number` | ✅ `signs over the literal t text, not its numeric re-rendering`, `rejects a signature computed over the numeric re-rendering of t` |
| `t` must be a bare run of decimal digits | ✅ `a_t_that_is_not_a_run_of_decimal_digits_is_malformed`, `a_t_too_wide_for_an_i64_is_a_tolerance_failure_not_a_malformed_header` | ✅ `rejects a fractional t as malformed, even when signed over that literal`, `rejects an empty t as malformed rather than as a 1970 timestamp`, `rejects a hexadecimal t as malformed`, `rejects other Number()-friendly t forms as malformed` |
| An unknown header element is ignored and an empty `v1=` never matches | ✅ `an_unparseable_part_and_an_unknown_key_are_both_ignored`, `an_empty_v1_is_never_treated_as_a_match` | ✅ `rejects a malformed header with no t=`, `rejects a malformed header with no v1=`, `rejects a completely garbled header`, `tolerates surrounding whitespace in t, which is header formatting and not part of the signed bytes` |
| A verified-but-undecodable body is a **distinct** failure from a bad signature | ✅ `a_verified_but_undecodable_body_is_a_distinct_error_from_a_bad_signature` | ⛔ 2026-09-03 — both cases throw `WebhookSignatureError`, so a caller cannot tell "somebody forged this" from "we changed the event shape", and no test covers the second. Rust has `WebhookError::InvalidBody`. Owner: SDK maintainers |
| Raw bytes verify identically to the string form | ✅ `a_validly_signed_payload_is_accepted` | ✅ `verifies a Buffer body identically to its string form` |

## Interoperability with the official Stripe SDK

| Capability | `sdks/rust` | `sdks/nodejs` |
|---|---|---|
| An authenticator that lets the **official** Stripe SDK talk to vpay | ⛔ 2026-09-03 — `async-stripe` has no per-request async hook, so the equivalent means wrapping its transport in custom middleware. Named as a follow-up by ADR-0010's 2026-09-03 amendment ("No Rust equivalent … Scoped as a follow-up"). Owner: SDK maintainers | ✅ `sets Authorization from a bearer minted by a real token exchange`, `is assignable to Stripe.StripeConfig['authenticator']`, ``authenticates a real `stripe` client end to end`` |
| That authenticator refuses to sign a request addressed anywhere but `baseUrl` | ⛔ 2026-09-03 — there is no authenticator, so there is nothing to bind. The hazard it exists to prevent (a live vpay bearer token sent to `api.stripe.com`) does not arise here. Owner: SDK maintainers | ✅ `refuses to sign a request addressed to Stripe, and mints nothing`, `refuses the right host on the wrong port`, `refuses the right host and port on the wrong protocol`, `treats an omitted port as the scheme's default on both sides` |
| That authenticator shares one token cache and one handshake with the core client | ⛔ 2026-09-03 — no authenticator exists to share with. Owner: SDK maintainers | ✅ `performs exactly one token fetch for N concurrent calls`, `reuses the cached token across sequential calls`, `re-mints once the token passes expires_in minus the safety margin`, `invalidate() forces the next call to re-mint` |

---

## The browser surface — `@vaam-apps/vpay-stripe-js`

A different surface, not a third merchant SDK: it authenticates a **payer's
browser** with a publishable key and a per-intent `client_secret`
(ADR-0010's Step 5c amendment), speaks `/v1/browser` rather than `/v1`, and
therefore shares no capability row with the tables above. It gets its own
table for the same reason they exist.

| Capability | `sdks/stripe-js` |
|---|---|
| `loadStripe` validates its inputs and normalises the base URL | ✅ `rejects a blank publishable key — an integration mistake, not a payer failure`, `rejects a blank base URL`, `strips trailing slashes so the path cannot become //v1/browser`, `strips a long run of trailing slashes in linear time, not just one`, `uses the injected fetch rather than the global one` |
| `retrievePaymentIntent` — the polling endpoint, all thirteen keys | ✅ `GETs the browser route with key and client_secret in the query string`, `renders all thirteen keys of PaymentIntentWithSecret` |
| `confirmPayment` — the form body, byte for byte | ✅ `POSTs the form-encoded body the design specifies, byte for byte`, `omits return_url and payment_method_data when the caller sent neither`, `answers invalid_request_error rather than sending an unencodable payment_method_data` |
| `confirmMobileMoneyPayment` — the push-rail shorthand | ✅ `writes the rail code as both payment_method_data[type] and the nested key` |
| `handleNextAction` | ✅ `retrieves, and resolves unchanged when there is nothing to act on`, `navigates for an intent already in requires_action, and never settles` |
| `waitForPaymentIntent` — transitions, budget and interval | ✅ `polls through processing and resolves on succeeded`, `resolves on canceled`, `resolves on requires_payment_method once last_payment_error is populated`, `keeps polling through an unconfirmed requires_payment_method, which has no error`, `polls on the requested interval and gives up at the deadline`, `defaults to a three-minute budget polled every two seconds`, `clamps the last sleep to the remaining budget rather than overshooting it` |
| Error shapes — the Stripe error envelope, and the uniform 404 | ✅ `maps the uniform 404 every browser credential failure renders`, `reports a non-envelope failure body as unexpected_response, without quoting it`, `reports a 200 that is not a payment intent as unexpected_response`, `reports a refused connection as api_connection_error and never rejects`, `reads the four keys vpay_api::error_envelope_with_param writes`, ``leaves param absent when the server omitted it, as `'param' in error` callers expect`` |
| Redirect scheme allowlist | ✅ `refuses a javascript: URL rather than navigating to it`, `refuses a relative path rather than navigating to it`, `still navigates to an ordinary https URL`, `does not navigate to an empty URL`, `answers redirect_unavailable where there is no window, rather than inventing a resolution` |
| Redirect semantics — `always` never settles, `if_required` resolves | ✅ `navigates and never settles when the rail asks for a redirect`, `navigates under an explicit redirect: 'always' too`, `resolves with the intent, and does not navigate, under redirect: 'if_required'`, `resolves normally on a push rail, where there is no next_action`, `does not navigate when the confirm itself failed` |
| `credentials: 'omit'` and `mode: 'cors'` on every request | ✅ `sets credentials: 'omit' and mode: 'cors' on a GET`, `sets credentials: 'omit' and mode: 'cors' on a POST`, `sends no Idempotency-Key and no Authorization — a preflight is the cost of either` |
| The `client_secret` stays out of diagnostics, errors and logs | ✅ `never reveals a client secret through inspect or JSON of the client`, `keeps the client secret out of every error it builds`, `has no console call anywhere in the shipping source`, `refuses a malformed clientSecret without polling at all` |
| Assignability to `@stripe/stripe-js`'s own types | ✅ `pins every assignability claim the README makes`, `narrows the same way Stripe.js's result does`, `accepts a Stripe error object wherever a vpay one is expected` |
| `retrieveCheckoutSession` — the route, the object, and the second credential | ✅ `GETs the browser checkout-session route with key and client_secret in the query string`, `renders all fourteen keys of the checkout session`, `reads a hosted session's url and both forwarding URLs`, `refuses a payment-intent secret where a checkout-session secret belongs, without sending anything` |
| `payment_intent` is the **expanded** intent on the browser read, the `pi_…` id on `/v1` | ✅ `expands payment_intent into the whole intent, with the intent's own client_secret typed`, `keeps the expanded intent's secret out of the client's diagnostics, exactly as it keeps the session's` |
| The session's own uniform 404, and a 200 that is not a session | ✅ `maps the uniform 404 every checkout-session credential failure renders`, `reports a 200 that is not a checkout session as unexpected_response`, `reports a refused connection on the session route as api_connection_error and never rejects`, `keeps the session client secret out of a non-envelope failure it reports` |
| `initEmbeddedCheckout` — the frame's exact `src`, and what the frame may do | ✅ `mounts an iframe whose src carries the session id, the key in the query and the secret in the fragment`, `sandboxes the frame without allow-top-navigation, so only the parent can navigate`, `strips trailing slashes from checkoutBaseUrl so the frame src cannot become //e` |
| The `postMessage` origin check, and that this side posts nothing | ✅ `ignores a message from an origin that is not the checkout app`, `ignores a message that is not from this frame, even from the checkout origin`, `ignores a message whose type it does not know`, `posts no message into the frame, so there is no target origin to get wrong` |
| The three messages the framed page sends — resize, complete, redirect | ✅ `sets the frame height from a vpay:resize message`, `calls onComplete with the payload of a vpay:complete message`, `ignores a vpay:complete whose session or status is not a string`, `assigns window.top.location for a vpay:redirect message, with the exact URL`, `refuses a javascript: URL in a vpay:redirect rather than navigating to it` |
| The embedded handle's lifecycle — mount, unmount, destroy | ✅ `unmount() detaches the frame and mount() re-attaches the same one`, `destroy() removes the message listener`, `refuses a second mount() while already mounted`, `refuses a selector that matches nothing`, `mounts into an element as well as a selector` |
| An embedded-checkout integration mistake rejects rather than degrading | ✅ `rejects when loadStripe was given no checkoutBaseUrl`, `rejects a checkoutBaseUrl that is not an absolute http(s) URL, at loadStripe`, `rejects when fetchClientSecret does not return a checkout-session secret`, `propagates a rejection from fetchClientSecret unchanged` |
| Exercised against a running vpay | ⛔ 2026-09-03 — every server in this package's suite is `src/testing/browser-stub.ts`. The `/v1/browser` routes are proven server-side by `backends/tests/integration/tests/browser_checkout.rs`, and `examples/checkout-browser` vendors the built package, but no test drives this package against a live stack. Still true on 2026-09-04 for the Checkout rows added that day: `initEmbeddedCheckout` runs against a real `iframe` in jsdom and `retrieveCheckoutSession` against the `node:http` stub, and the checkout app and its browser route are lanes 3 and 1 of the same step. Owner: SDK maintainers |

`sdks/stripe-compat` is **evidence, not an SDK**, and gets no rows: it drives
the real `stripe@22.6.1` package through `@vaam-apps/vpay-sdk/stripe` against a live
compose stack. It is where the `request-id` mirror and the
`stripe-should-retry` advisory are actually observed — which is exactly why
the two ⛔ rows above are gaps in the *merchant SDKs* rather than in the
server.

---

## Gap ledger

Every ⛔ above, in one list. The cells are authoritative; this is an index.

| Gap | Where | Found | Owner |
|---|---|---|---|
| No CI-gated real-OP conformance for the Node assertion | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `token_type` is not validated on the token response | `sdks/rust` | 2026-09-03 | SDK maintainers |
| `invalidate()` has no compare-and-swap against the refused token | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| No retry policy beyond the single 401 re-auth | both | 2026-09-03 | SDK maintainers |
| A repeated trailing slash on `base_url` is unproven, and the two differ | both | 2026-09-03 | SDK maintainers |
| No test makes a request timeout fire | `sdks/rust` | 2026-09-03 | SDK maintainers |
| No TLS trust-root control, and no TLS test | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `events.retrieve` is served and neither SDK calls it | both | 2026-09-03 | SDK maintainers |
| A refused amount throws a bare `TypeError`, not a `VpayError` | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `request-id` is not surfaced | both | 2026-09-03 | SDK maintainers |
| `stripe-should-retry` is not read | both | 2026-09-03 | SDK maintainers |
| No assertion that a thrown error cannot carry a token | `sdks/rust` | 2026-09-03 | SDK maintainers |
| `client_secret` is not redacted from `PaymentIntent` diagnostics | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `exactOptionalPropertyTypes` has no Rust analogue | `sdks/rust` | 2026-09-03 | n/a |
| A verified-but-undecodable body is not a distinct error | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| No `async-stripe` authenticator (three rows) | `sdks/rust` | 2026-09-03 | SDK maintainers |
| The browser package has never run against a live stack | `sdks/stripe-js` | 2026-09-03 | SDK maintainers |
| Checkout Sessions have never run against a live stack | both | 2026-09-04 | SDK maintainers |
| `account_holders.retrieve` has never run against a live stack | both | 2026-09-05 | SDK maintainers |

## What this matrix does not claim

- **Not that a ✅ row is bug-free.** It claims a named test in that SDK
  fails when the capability breaks. Nothing more.
- **Not that a capability works against a deployed vpay.** Both merchant
  SDKs test against in-process stubs; see [`../status.md`](../status.md) for
  what has and has not spoken to a real server.
- **Not that the server offers every capability.** ~~`/v1/refunds` and
  `/v1/balance` are not mounted at all~~ **— corrected 2026-09-05 in the same
  change that added the `refunds.retrieve` row above: `GET /v1/refunds/{id}`
  is mounted (issue #45).** `refunds.create` and `balance.retrieve` are the
  two SDK methods left with no route, and they reach the nest's 404 — a
  server gap, tracked in `docs/status.md`, not a parity gap.
