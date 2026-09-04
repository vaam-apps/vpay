# Rails: the port crate and its two adapters

Why the code under `backends/crates/vpay-provider` and
`backends/crates/vpay-adapter-*` looks the way it does. The *contract* is
[`docs/flows/provider-port.md`](../flows/provider-port.md); the *decision* to
have a port at all is [ADR-0002](../adr/0002-provider-port.md); each rail's
wire is [`adapter-mtn-momo.md`](../flows/adapter-mtn-momo.md) and
[`adapter-orange-money.md`](../flows/adapter-orange-money.md). This page is the
remaining tier: the arguments that used to sit in 200 lines of module headers,
where they were read once and then scrolled past forever.

Every claim here is either pinned by a named test or marked as unproven.

---

## What the port carries, and what each adapter keeps

Three things belong to *every* rail rather than to any one of them, so the port
crate holds them once. The corollary of ADR-0002's "rail-specific code lives
inside an adapter" is that **cross-rail** code must not live inside one: two
adapters each holding their own copy of a decision this consequential is two
copies that can disagree, and a third rail is a third copy someone has to
remember to write.

| In `vpay-provider` | Why it cannot live in an adapter |
|---|---|
| `http` — the outbound client, and `read_rail_body` | An adapter must not build its own client (below), and "an oversize body is not a decline" is a rule about payments, not about a rail |
| `token` — `CachedToken`, `fingerprint`, `usable_until` | A cache that could hand merchant B the bearer minted for merchant A is a cross-tenant leak with no rail-specific content |
| `measured` — the counter and histogram | A metric mounted per adapter is a metric the third rail forgets |

And what stays per-rail, deliberately:

| In each adapter | Why it must not be shared |
|---|---|
| The refresh margin (`REFRESH_MARGIN` / `EXPIRY_MARGIN`) | Two separately reasoned numbers that happen to agree at 60 s today. `CachedToken` has **no default** — each adapter passes its own, pinned by `the_margin_is_the_callers_and_this_type_supplies_none` |
| MTN's `ASSUMED_LIFETIME` | MTN may omit `expires_in`; Orange does not, and treats its absence as "use once, do not cache" |
| `mapping.rs` — the `FailureCode` tables | Each rail's own vocabulary. A shared table would be a guess about a rail nobody has read |
| `wire.rs` — the rail's own casing | See "serde" below |
| Orange's `token_url` derivation | Orange serves OAuth from the host root and the payment API under a path; MTN's token endpoint is under its base URL |
| `Capabilities` | The whole point of the port: the core branches on these values, never on a provider code |

### serde: `rename_all` is for *our* wire, never a rail's

The workspace convention is that every type modelling vpay's own wire or config
carries `#[serde(rename_all = "snake_case")]`, so a field added as `payTo`
fails review rather than shipping. `vpay_provider::Capabilities` carries it.

Neither adapter's `wire.rs` does, nor either `TokenResponse`, and that is a
rule and not an oversight:

- MTN's bodies **are** camelCase (`externalId`, `partyIdType`, `partyId`,
  `financialTransactionId`). A blanket `rename_all` there would be a no-op
  masked by the per-field `#[serde(rename)]` attributes on the fields that have
  one, and a silent wire break on the fields that do not.
- Orange's bodies happen to be snake_case today, which makes the attribute
  *more* dangerous there, not less: it would read as a promise that these names
  are ours to normalise, and the day Orange sends one that is not snake_case
  the attribute would quietly rename it away from the rail's own spelling.

A rail's casing is the rail's. `rename_all` is a statement about ours.

---

## The outbound HTTP client

### Why it lives in the port crate

`vpay_api::http_client` until Step 3, moved here verbatim when the adapters
needed it: an adapter depends on `vpay-provider` and must not depend on
`vpay-api` (the HTTP surface depends on the port, never the reverse), so the
only home both can reach is this crate. `vpay_api::http_client` is a re-export,
which is why no call site changed.

The cost, stated rather than hidden: `vpay-provider` is no longer a pure
interface crate — it links reqwest, rustls and webpki-roots, so a future
non-HTTP rail (a USSD gateway, a file drop) compiles a TLS stack it never uses.
No *binary* grew; both already resolved all three. A separate `vpay-http` crate
was rejected for the workspace member, `deny.toml` entry and second `sdks/rust`
twin note it would have added.

### Why not `reqwest::Client::new()`

The runtime image is `FROM scratch` ([ADR-0004](../adr/0004-musl-mimalloc.md)):
no glibc, no shell, no OS certificate store. reqwest is pinned at 0.13 with
`rustls-no-provider`, and on that version it no longer offers a vendored-roots
feature — it builds a `rustls_platform_verifier::Verifier`, i.e. it reads the
*platform* trust store. It does so **eagerly, inside `ClientBuilder::build()`**,
not lazily at connect time, and when the store turns up empty the verifier
returns `General("No CA certificates were loaded from the system")`, which
`Client::new()` converts into a panic.

That is not hypothetical. `JwtValidator::new` used to reach
`reqwest::Client::new()` through `authkestra_resource`'s JWKS cache, and
`vpay-server` panicked at boot inside its own image while passing every test on
machines that happen to have `/etc/ssl`. The JWKS URL it was about to fetch was
plain `http://` over loopback — TLS was never going to be negotiated — so the
failure had nothing to do with the request and everything to do with *when* the
trust store is read.

`http::client` therefore hands reqwest a finished `rustls::ClientConfig` built
from Mozilla's vendored bundle. That takes reqwest's `TlsBackend::BuiltRustls`
branch, which consults neither the platform verifier nor the process-wide
`CryptoProvider` — so it also cannot hit the *other* panic the
`rustls-no-provider` pin exposes ("No rustls crypto provider is configured"),
whether or not the binary installed a default provider first.

Proven by `a_server_with_no_os_trust_store_boots_and_still_validates_tokens`
(`backends/apps/vpay-server/tests/cli.rs`), which points `SSL_CERT_FILE` and
`SSL_CERT_DIR` at paths that do not exist, and by
`a_client_builds_without_a_process_wide_crypto_provider` and
`the_vendored_bundle_is_not_empty` in `http`'s own tests.

**The trade-off:** vendored roots mean a deployment behind a TLS-intercepting
proxy with a private CA is not served by this client, and `SSL_CERT_FILE` will
not change that. That is the deliberate cost of running in a `scratch` image at
all; the alternative is an image that carries a trust store, which is a
different ADR.

### Why redirects are not followed

reqwest's default is `redirect::Policy::limited(10)`, and on a cross-host hop it
strips exactly three headers: `Authorization`, `Cookie` and
`Proxy-Authorization`. Every *other* header is replayed at the new host — and a
rail adapter's headers are precisely the ones not on that list: MTN's
`Ocp-Apim-Subscription-Key`, `X-Target-Environment`, `X-Reference-Id` and
`X-Callback-Url` — while a 307/308 replays the request **body**, which on
Orange's `webpayment` carries `merchant_key`. A rail, or anyone who can answer
as one, responding `302 Location: https://attacker.example/` would be handed a
merchant's rail credentials and the identity of a live charge, by a client that
was only asked to take a payment.

Neither rail documents a redirect on any call this workspace makes, so there is
nothing to lose by refusing. A 3xx arrives at the adapters' "unexpected status"
arms as `ProviderError::Malformed`, which leaves the charge where a recovery
pass reads it rather than advancing it on the strength of an answer from
somewhere else.

Pinned twice: `a_redirect_is_returned_rather_than_followed` (the transport
itself, against a raw loopback listener) and the conformance suite's
`redirects_are_refused_and_never_followed`, whose decisive half is that the
redirect target is a mapping on the same WireMock which must stay unrequested.

### Why the process environment cannot reroute a rail call

reqwest reads `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` by default. A payment
gateway's egress must be explicit configuration, not ambient: a variable set on
a pod — by a sidecar, a base image, or a helpful default in a chart — must not
silently put a third party in the middle of a call carrying rail credentials. If
a deployment ever genuinely needs an egress proxy, that is a change here and in
an ADR, visible in review, rather than a value in an environment nobody diffed.

Both removals live in `preconfigured_builder` rather than at a call site: a
client built without them is the dangerous one, and there must be no way to
construct it.

### The twin in `sdks/rust`

`sdks/rust/src/client.rs` has a near-identical `rustls_client_config`, and it
stays a separate copy on purpose: `vpay-sdk` is what a *merchant* compiles into
their own process, so making it depend on a server crate would drag axum, sqlx
and the whole OP into a merchant's build. The SDK's copy also carries a
constraint this one does not — a library inside someone else's process may
neither panic nor install a process-wide `CryptoProvider` on that process's
behalf.

The two differ in exactly one place, deliberately: **this** one refuses
redirects and ignores the proxy variables, and the SDK's does neither. A
merchant's process runs on a merchant's network, where a corporate egress proxy
is ordinary and a redirect to their own gateway may be exactly what they
configured. Everything else — the provider, the root source, the ALPN list — is
expected to stay in step; change one and change the other.

### Deadlines are on the config, not on the client

One `reqwest::Client` is built once per process and shared by every adapter,
so a per-rail deadline cannot live on the client without giving each rail its
own connection pool. `ProviderConfig` carries both instead, and
`client_with_timeouts` takes them per call — which is also what lets the
conformance suite ask for 100 ms and assert `ProviderError::Transport` against
a deliberately-slow stub instead of waiting out a 20 s production default.

Both are set, because either alone leaves a hole: a request timeout alone lets
a black-holed rail hold a worker task for the full request budget on a
connection that was never going to establish, and a connect timeout alone
bounds nothing once the socket is open.

Neither has a YAML knob. A config built from YAML always gets
`DEFAULT_CONNECT_TIMEOUT` (5 s) and `DEFAULT_REQUEST_TIMEOUT` (20 s) —
`vpay_config::ProviderHost::to_provider_config` fills them from those
constants — because no deployment has asked for a different budget and a knob
nobody sets is a knob nobody has tested. The conformance suite builds a
`ProviderConfig` directly and is the one caller that overrides them.

The request budget is generous rather than tight on purpose. A push rail's
`submit` returns as soon as the rail has *accepted* the request, but
"accepted" can involve the rail's own upstream; a deadline that fires early on
a rail that did in fact accept the charge leaves a payer prompted for a charge
we recorded as a transport failure — exactly the ambiguity
[`crash-safety.md`](../flows/crash-safety.md) says the status query, never a
retry, must resolve.

### The bounded body read

`Response::text()`/`bytes()` read to end of stream: the peer decides how much
memory this process allocates. One worker task per charge, each willing to
buffer whatever a rail sends, is a memory exhaustion whose size the peer
chooses — and on a bad day the peer is a load balancer's error page, on a worse
one it is not the rail at all. `MAX_RAIL_BODY_BYTES` is 256 KiB: every
documented body either rail answers with is under a kilobyte, so a body that
trips the cap is evidence in itself rather than a tuning problem.

`bounded_body` reads chunk by chunk and gives up the moment the accumulated
length would exceed the cap, so an oversize body costs one chunk of over-read
rather than all of it. `Content-Length` is checked first when the peer supplies
one — an optimisation, never the guard, since a chunked response has none and a
lying one is caught by the running total anyway.

`read_rail_body` is the adapters' entry point and puts each outcome in the
`ProviderError` an adapter has to return. The cap is named in the message
rather than left to the source's `Display`: an operator needs to see that the
limit was ours and what it is, and the conformance case
`an_oversized_rail_body_is_refused_at_the_cap` asserts exactly that. The
variant is `Malformed`, never a decline, because an oversize answer says
nothing about whether the payment happened — and
[`crash-safety.md`](../flows/crash-safety.md) resolves an unknown fate by asking
again.

---

## The token cache

### Why it is keyed at all

`ProviderAdapter` takes `&ProviderConfig` *per call*, so one `Adapter` value can
legitimately be handed two different merchants' credentials for the same rail. A
cache keyed by nothing would hand merchant B the token minted for merchant A —
money moving on the wrong account, and no test of a single-merchant deployment
would ever show it. The fingerprint is what makes that structurally impossible:
a token is reused only when the digest of the credentials that minted it matches
the ones being used now.

A single slot rather than a map keyed by fingerprint: a map would be an
unbounded, never-evicted cache of bearer tokens keyed by credentials, which is a
worse thing to hold in memory than one token re-minted when the configuration it
belongs to changes. The fingerprint is what makes the single slot *safe*; it is
not what makes it fast.

### The two invariants that were each a real defect

Both were found in the Step 3 security review, in different adapters, and a
divergence between them is silent — which is why the shared type now enforces
the shape of each.

1. **`minted_at` is read before the token request is sent**, never from the
   clock after the response arrives. The rail's `expires_in` counts from the
   rail's own mint, so measuring from arrival grants the token the round trip as
   extra life — on a slow rail, the whole of the margin. `CachedToken::new`
   takes it as a parameter for exactly that reason.
   (`the_lifetime_is_measured_from_the_send_not_from_the_answer`.)
2. **The secret half of the credentials is in the fingerprint** — MTN hashes
   `api_key`, Orange hashes `client_secret`. A secret is rotated precisely when
   the old one must stop working *now*; hashing only the identifier meant the
   fingerprint still matched after a rotation, the cache still hit, and the
   bearer minted from a revoked secret kept being sent until it aged out (up to
   an hour) or the rail answered 401.
   (`rotating_only_the_secret_evicts_the_cached_bearer`,
   `different_credentials_fingerprint_differently`.)

Hashing a secret is safe here *because* it is a digest: the cache holds a
SHA-256, never credential material. Each field is length-prefixed, so `("ab",
"c")` and `("a", "bc")` cannot collide by concatenating differently — the whole
point of the key being that distinct credentials are distinct.
(`a_field_boundary_cannot_be_shifted_into_a_collision`, in both adapters.)

### Why the margin is per-rail

`usable_until` subtracts a margin the *caller* supplies; the shared type has no
default. The two are 60 s today and are not the same number: MTN's is reasoned
from "the clock that matters is MTN's and we cannot see it — the round trip, a
retry, and any skew have to fit inside it", Orange's from "a token that expires
in flight is a 401 on a payment call, which on `submit` risks a duplicate on a
rail whose idempotency we are reconstructing from community SDKs". A shared
default would let one rail silently inherit the other's reasoning.

Each adapter pins its own:
`the_refresh_margin_mtn_applies_is_sixty_seconds_of_the_rails_own_lifetime` and
`the_expiry_margin_orange_applies_is_sixty_seconds_of_the_rails_own_lifetime`.
Both fail if the constant changes. Neither can detect a *swap* between two
constants that are numerically equal — what rules that out is structural: the
shared type supplies no margin, so each call site names one.

The arithmetic saturates at both ends: a lifetime shorter than the margin
clamps to "already expired" rather than wrapping into a token that never
expires, and an absurd `expires_in` yields `minted_at` rather than overflowing
an `Instant` (`Instant + Duration` panics) and taking a worker down. MTN's copy
used a plain `+` before the caches were unified and Orange's did not — the exact
kind of drift two copies produce.

### Neither token is persisted

A token is short-lived and re-mintable from credentials we already hold, so
writing it to the database would put a bearer for a merchant's payment account
into backups and replicas for no benefit. Re-minting after a restart costs one
round trip.

Nothing renders a token or a credential: `CachedToken`'s `Debug` is hand-written
to redact the value, header values are marked *sensitive* (via reqwest's
`basic_auth`/`bearer_auth`, which is also why the base64 is not hand-rolled),
and every configuration error names the *key* that was wrong, never its value.
(`debugging_a_cached_token_does_not_print_it`,
`debugging_the_adapter_does_not_print_the_token`,
`debugging_credentials_does_not_print_them`,
`a_missing_credential_names_the_key_and_never_the_value`.)

---

## MTN MoMo: the three things this adapter is careful about

A **push** rail in the sense [`provider-port.md`](../flows/provider-port.md)
gives the word: the payer is prompted on their own handset, we supply the
reference (`X-Reference-Id`) so a submit is idempotent on an id that exists in
our database *before* the call, and status is queryable by that same reference
indefinitely — which is what the poll ladder is built on.

**A 409 is a success.** MTN answers `RESOURCE_ALREADY_EXIST` when it has already
seen our reference. That is the rail confirming the charge exists, which is
exactly what a retry after a crash needs to hear; reporting it as an error would
turn a safe retry into a lost payment.
(`a_duplicate_reference_is_a_success_not_an_error`,
`duplicate_submit_reports_submitted_not_an_error`.)

**A 500 is not automatically retryable.** Several *logical* errors arrive as
HTTP 500 with a `code` in the body, three of which are our own misconfiguration
and will never succeed. The body's `code` is read before anything is decided.
(`a_500_that_names_our_misconfiguration_is_never_retried`.)

**A 404 is not a failure.** "I have no record of that reference" is
`ChargeStatus::NotFound`, never `Failed`: a push rail can answer 404 for a
charge it is about to accept, and failing it here would lose a payment still in
flight. (`no_record_of_a_reference_is_not_a_failure`.)

**Refunds are not built.** MTN refunds are the *Disbursements* product — a
different subscription key, a separately-scoped token, and a `transfer` call
this adapter does not make. No deployment of this system holds those
credentials. `supports_refunds` stays `true` because the rail *does* support
refunds; it is we who have not built them, so the answer is
`ProviderError::NotImplemented("mtn_momo::refund")` and it is listed in
[`status.md`](../status.md). Answering `Unsupported` would be a lie about the
rail.

The 401 path is the adapter's only retry: nothing else is resent, least of all a
500. Resending after a 401 is safe on both calls that use it — `submit` carries
our own `X-Reference-Id`, so a duplicate is a 409 the caller reads as success,
and `query_status` is a read. A *freshly minted* token being refused means the
credentials are wrong rather than stale, which pages.

---

## Orange Money: a redirect rail, reconstructed

Implements `submit`, `query_status` and `parse_callback` against the three calls
transcribed in [`adapter-orange-money.md`](../flows/adapter-orange-money.md).
`refund` is deliberately *not* overridden: Orange documents no refund API for
Web Payment, so the port's default `ProviderError::Unsupported` is the
permanent, correct answer and `Capabilities::supports_refunds` is what the core
branches on. It is not `NotImplemented`, because there is nothing to build.

**Sourcing caveat, and it is the important line on this page.** The flow doc
this adapter is written from is reconstructed from Orange Developer's public
overview and community SDKs, **not** from a vendor specification. The
error-body shapes in particular are inferred. See that doc's "To confirm with
Orange Cameroun" list.

**What is proven, and by what.** The pure halves — URL derivation, status
mapping, body shape, callback parsing — are unit-tested in the crate. The wire
behaviour is proven only by `backends/tests/conformance`, against a real
`wiremock/wiremock` host reached over HTTP exactly as the rail is
([ADR-0006](../adr/0006-no-mocks-in-main-processes.md)). This crate has no in-process HTTP
double and must not grow one.

**No type in `wire.rs` derives `Debug`,** and that is deliberate rather than an
omission: every one of them carries either a credential (`merchant_key`) or rail
key material (`pay_token`, `notif_token`, `access_token`). A derived `Debug` is
how those reach a log line — one `tracing::debug!(?body)` added later and a
token gating a payer's redirect is in the log stream. `pub(crate)` types are
exempt from `missing_debug_implementations`, so the lint does not push back.

**A missing `pay_token` is a configuration error, not `NotFound`.** The rail
will not answer for an `order_id` alone, so there is nothing to ask. `NotFound`
is the answer the recovery path treats as "the rail never saw this"; a charge
whose `pay_token` we lost is the opposite case — the rail may well have it, and
a payer may already have paid. `ProviderError::Config` stops the poll ladder and
puts it in front of a human, which is the only correct outcome.

**A missing `expires_in` means "use it for this call, do not cache it".**
Inventing a lifetime is a guess that expresses itself as intermittent 401s under
load; the honest cost of not guessing is one extra token call per payment call
on a rail that has never been observed to omit the field.

**`parse_callback` fails closed.** A `notif_token` is *required*: it is the only
thing distinguishing Orange's notification from an unauthenticated POST by
anyone who can guess an `order_id`, and comparing it against the stored one is
the caller's job (this adapter holds no state). Returning a `CallbackRef` with
nothing to compare would hand the caller a hint it cannot check, which is worse
than refusing to parse.

That check is now load-bearing in a place it was not before. Since Step 8 lane
C there **is** a caller — `vpay_api::provider_callback` — and it discards
`CallbackRef::ref_extra` entirely rather than comparing the received
`notif_token` against the stored one, because that comparison is not built
(`docs/status.md`). So the adapter refusing the body is the only thing standing
between an unauthenticated POST and a queued poll, and
`an_unparseable_callback_body_is_refused_and_moves_no_job` in
`backends/tests/integration/tests/provider_callback.rs` is decisive about it:
making `notif_token` optional here makes that test's Orange case fail twice
over — first on the `400`, and, with that assertion removed, on the poll job's
`run_at` having been dragged back to now. Measured 2026-09-04, restored
afterwards.

---

## The callback URL is a contract the mappings hold

Both rails carry vpay's callback URL on every `submit` — MTN in the
`X-Callback-Url` header, Orange in the request body's `notif_url` — and both
have done so since Step 3, derived by
`vpay_config::ProviderHost::effective_callback_url` from
`{deployment.public_base_url}/provider/{code}/callback` (ADR-0012 is the
adjacent decision about rail-keyed configuration). Until Step 8 lane C that
address answered a 404, because nothing mounted the route.

What was missing was not the header; it was any way to *notice* it going away.
Polling settles a payment perfectly well on its own, so an adapter that quietly
stopped sending its callback URL would have passed every conformance case,
settled every payment in `backends/tests/integration`, and been discovered by
an MTN sandbox registration failing or by a production deployment whose
settlements were all ten seconds late.

It is now asserted twice, from both directions:

- `requesttopay.json`'s accepted-submit mapping and `webpayment.json`'s
  **require** the URL to match `.+/provider/{code}/callback`, so an adapter
  that stops sending it matches no mapping and gets a 404 rather than an
  accepted submit. Measured: removing MTN's `.header(CALLBACK_URL_HEADER, …)`
  fails `submit_returns_a_reference_and_a_flow_shaped_result` with
  `Config("mtn_momo: requesttopay answered HTTP 404 Not Found; check
  base_url")`, and pointing Orange's `notif_url` at `config.base_url` fails the
  same case with Orange's own 404. Both restored (2026-09-04).
- `the_submit_tells_the_rail_where_to_call_back` reads WireMock's request
  journal and asserts the URL the rail received is the configured one
  **verbatim** — the half that catches an adapter sending *some* URL rather
  than *this* one.

The host is left free in the mapping matchers (`.+`) because it genuinely
varies: the stub's own origin under test, an ingress in production, and MTN
additionally allows an IP-allowlisted callback host of its own
(`ProviderHost::callback_url`). The **path** is pinned, because that is the
half that can drift silently — the route lives in `vpay-api` and the
derivation in `vpay-config`, and neither crate compiles against the other.

---

## The payer's return URL is the *core's* answer, not a rail's

A redirect rail hands the payer to a page vpay does not control, and it has to
be told where to send them back. Until 2026-09-04
`vpay-adapter-orange-money` answered that out of **deployment** settings
(`settings.return_url` / `settings.cancel_url`, falling back to the
notification endpoint): one value per deployment for a question that is per
charge. The merchant's own `return_url` was validated by
`vpay_api`'s `checked_return_url`, written to `charges.return_url` before the
rail was called, and echoed back to them as
`next_action.redirect_to_url.return_url` — and never sent to the rail that
would act on it. Every conformance case and every integration case passed
throughout. That is
[browser-checkout.md](../flows/browser-checkout.md)'s D4, and Step 9's D2 is
the decision that closes it.

`ChargeRef` now carries `return_url: Option<String>`. Three things follow, and
the order matters:

- **The core fills it.** `vpay_api::v1::return_trip::return_url_for_charge`
  answers with vpay's own return page when a checkout session drives the
  charge and with the merchant's committed `charges.return_url` otherwise. The
  value handed to the rail is read off the *committed row*, not recomputed, so
  what the rail is told is what would survive a crash
  ([crash-safety.md](../flows/crash-safety.md)).
- **The adapter decides whether its rail has a use for it.** Orange sends it
  as `return_url` **and** `cancel_url`; a charge with none is
  `ProviderError::Config`, the twin of MTN's "payer_ref required on a push
  rail", because a redirect rail that invents this answer is exactly the
  defect being fixed. MTN ignores it: `requesttopay` has no browser, and a
  field MTN does not document would at best be dropped.
- **Both fields get the same URL, deliberately.** Orange's page distinguishes
  "paid" from "cancelled" and vpay cannot: the outcome comes from the
  authenticated status query, and a charge the payer abandoned is `Pending`
  until it expires. Two different URLs would encode a distinction nothing
  checks.

Asserted from both directions, exactly as the callback URL is.
`webpayment.json` **requires** an `http(s)` `return_url` and `cancel_url` on
every accepted submit — measured 2026-09-04: `#[serde(skip)]` on
`WebPaymentRequest`'s two link fields makes the stub answer 404 and
`the_submit_tells_the_rail_where_to_send_the_payer_back::case_2_orange_money`
fail on `ProviderError::Config`; restored. The matcher is `^https?://.+` and
**not** a prefix under the deployment's own origin, because the correct value
is by design sometimes the merchant's own site. The exact value is pinned by
the conformance case over the request journal, and end to end by
`a_direct_confirm_sends_the_merchants_return_url_to_the_rail` in
`backends/tests/integration/tests/confirm_rails.rs`.

The push rail's half of that case is the sharper one. `Rail::charge` fills
`return_url` for **both** rails — a push charge included, which is not what
production does — because "MTN sent no return URL" proves nothing when there
was none to send. The assertion is that the value appears nowhere in the MTN
stub's whole request journal: not in a body, not in a header, not in a query
string.

### The stub's hosted page, and where it is not the rail

`webpayment.json` has answered every accepted submit with a `payment_url`
under `/stub-hosted-page/{pay_token}` since Step 3, and until Step 9 nothing
served it — a payer following the URL vpay handed them got WireMock's 404.
`stub-hosted-page.json` now serves it as HTML with a Pay link and a Cancel
link, so the redirect leg can be finished by a browser (ADR-0006: it is a stub
of the *rail's* page, a WireMock host in configuration, and never a stub of a
vpay page).

It differs from Orange in one way, and the mapping says so at length rather
than leaving it to be discovered. The real rail *stores* `return_url` and
`cancel_url` against the `pay_token` at submit and renders them from its own
state; WireMock's response templating can only read the current request, and
there is no helper that reaches into the journal for the POST that minted the
token. So the submit's `payment_url` carries the two URLs as query parameters
and the page templates them back out. The pairing is preserved — these are the
bytes *that* submit sent — but nothing here demonstrates that Orange would
accept a `return_url` it had not been told about, and nothing in this
repository claims it would.

`payment_url`'s host is the literal `localhost:8082` that `compose.yml`
publishes, for the same reason: a stub reached over the compose network cannot
know what the host mapped it to. `compose.demo.yml` publishes
`wiremock-orange` on `${VPAY_DEMO_ORANGE_PORT:-8082}` (it used to `!reset` the
publication, so `just demo`'s redirect URL was unopenable), and `just
gen-demo-keys` reads the port back out of the mapping and refuses a
`demo_orange_port` that disagrees with it.

---

## Status

**Built and proven.** The port, both adapters, the shared token cache and the
shared bounded read. `cargo nextest run -p vpay-provider
-p vpay-adapter-mtn-momo -p vpay-adapter-orange-money -p vpay-tests-conformance`
runs **149 tests, 149 passed, 0 skipped, 0 ignored** (2026-09-03), of which the
conformance suite is 26 — 11 cases parameterised over both rails, plus 4 that are not rail-specific. `cargo test --doc` over the
three crates runs **10 doctests**.

Re-measured 2026-09-04 (Step 8 lane C): `cargo nextest run
-p vpay-tests-conformance` runs **28 tests, 28 passed, 0 skipped, 0 ignored** —
12 cases over both rails plus the same 4, the new one being
`the_submit_tells_the_rail_where_to_call_back`.

Re-measured again 2026-09-04 (Step 9 lane 2): **30 tests, 30 passed, 0
skipped, 0 ignored** — 13 cases over both rails plus the same 4, the new one
being `the_submit_tells_the_rail_where_to_send_the_payer_back`.

**Not proven by the return-trip work either.** No payer has been redirected
anywhere by this repository: the *page* that receives one is
`frontends/apps/checkout`, which is Step 9 lane 3's, and vpay serves no route
at a return URL of its own. What is built and proven here is the rail half —
the value reaches the rail, and the rail's stub page links to it.
`vpay_api::v1::return_trip`'s session branch answers `None` for every intent
because there is no `checkout_sessions` table in this tree; that is the truth
today, and the doc comment on that impl names the method Step 9 lane 1 has to
replace it with.

**Not proven by any of it.** Every conformance case talks to WireMock:
**neither adapter has ever called MTN's or Orange's real sandbox**, so a mapping
faithful to the flow doc but not to the rail would pass. The 401-after-a-good-
token re-mint path has no WireMock mapping and is unproven on both rails.
Orange's error-body shapes are inferred, as above. See
[`status.md`](../status.md).
