<!-- Lane notes for Step 8 (docs/plans/2026-09-03-step8-production-gate.md), lane B.
     Written for lane E, which is the only thing that edits docs/status.md,
     docs/roadmap.md and docs/flows/*.md. Everything below marked "verbatim"
     is meant to be pasted; everything else is the evidence for it. -->

# Lane B — the runtime SSRF guard on webhook delivery

Branch `claude/step8-lane-b-ssrf`, based on `679e65a` (master `ca94eac` + the
Step 8 plan). Six commits, all `feat`/`test`/`docs`/`chore` prefixed.

## What is now true

A webhook delivery no longer connects to an address nobody looked at.
`vpay_worker::ssrf` parses the endpoint URL, refuses any scheme but
`http`/`https`, resolves the host **once** (`tokio::net::lookup_host`),
classifies **every** address the lookup answered with, and hands back a
`VettedTarget`. The client that then sends is built from that target with
`reqwest::ClientBuilder::resolve_to_addrs`, so reqwest never resolves the name
a second time and the socket goes to an address the classifier saw. Redirects
stay `Policy::none()` and the proxy environment stays ignored, which is what
keeps the pin from being escaped by a `302`.

A refused target is a **permanent** delivery failure: `state = 'exhausted'` on
the first attempt, `next_attempt_at = NULL`, `response_excerpt` beginning
`ssrf_blocked: `, and exactly one `ERROR … alert = true` naming the endpoint
id, the delivery, the event and the classified reason. A host that merely fails
to **resolve** is not that: it is an ordinary failed attempt that walks
`delivery_delay`, recorded `delivery_target_unavailable: …`, because a resolver
outage must not cost a merchant their event on the first try.

## Files

| Path | What is there |
|---|---|
| `backends/crates/vpay-worker/src/ssrf.rs` (new, 874 lines incl. 9 unit tests) | `EgressPolicy`, `AddressClass`, `EgressRefusal`, `VettedTarget`, `vet` (:373), `pinned_client` (:441), `classify` (:466) / `classify_v4` (:476) / `classify_v6` (:537) |
| `backends/crates/vpay-worker/src/webhooks.rs:849` | the doc section "Where the URL is checked, since Step 8", replacing "What is *not* checked here" |
| `backends/crates/vpay-worker/src/webhooks.rs:975-995` | **the whole hook**: `ssrf::vet` then `ssrf::pinned_client`, immediately before `signature_header` |
| `backends/crates/vpay-worker/src/webhooks.rs:1209` / `:1262` | `record_refused_target` / `refusal_excerpt` |
| `backends/crates/vpay-worker/src/handlers.rs:88-125` | `WebhookContext.http: &reqwest::Client` → `WebhookContext.egress: EgressPolicy`; `:219` the dispatch arm |
| `backends/crates/vpay-worker/src/run_loop.rs:612,675,793,803` | the same swap through `run_loop`/`claim_loop` |
| `backends/crates/vpay-provider/src/http.rs:266` | `client_pinned_to`; its unit test at `:594` |
| `backends/crates/vpay-config/src/config.rs:355,430,524` | `WebhookPolicy`, the `webhooks:` field on `Config`, the livemode rule in `validate_all` |
| `backends/crates/vpay-config/src/lib.rs:590` | `ConfigError::PrivateWebhookTargetsInLivemode` |
| `backends/apps/vpay-worker-bin/src/main.rs:388-406` | builds no webhook client any more; projects the policy instead |
| `config/application-sandbox.yml:6-24` | `webhooks.allow_private_targets: true` |
| `justfile` (`gen-demo-keys`, `:624`, `:638`, `:698-709`) | the same block in the generated `demo` overlay, plus the staleness check that regenerates an overlay predating it |
| `backends/tests/integration/tests/webhooks.rs:2336-2649` | the two container-backed cases and their two helpers |

## Decisions taken in this lane

**D1 — the module lives in `vpay-worker`, not `vpay-provider`.** The plan
offered both. `handle_deliver` is the only caller: the rail adapters connect to
`providers[].host`, which is operator-configured and already guarded at boot by
`validate_host`. Moving it later is a file move — the module names that in its
own header.

**D2 — a per-delivery client, not a per-host cache.** The pin is a property of
the `ClientBuilder`, so pinning at all means building a client after the
lookup. Measured cost of the construction: **4.0 µs** per `client_pinned_to`
against 2.9 µs for `client_with_timeouts`, 200 builds each, debug build, warm
(measured with a temporary test in `vpay-provider`, removed afterwards; the
numbers are in `client_pinned_to`'s doc comment). The construction is therefore
not the cost — **the connection pool is**: two deliveries to one receiver no
longer share a connection and each pays a fresh TCP+TLS handshake. A per-host
cache would keep the pool and would be a cache of *pins*: a client held past
its DNS answer keeps delivering to the address that name used to have. Given a
delivery rate bounded by settlements, one handshake per delivery is the cheaper
mistake. **This is a real regression for a high-volume merchant and is stated
in the flow-doc replacement below rather than hidden.**

**D3 — `WebhookContext` lost its shared client rather than keeping a dead
field.** With D2 there is no client for the binary to build, so keeping
`http: &reqwest::Client` would have left a field that is never the client
anything sends on. `run_loop` takes `egress: EgressPolicy` where it took
`http: reqwest::Client`. This is the one signature change outside new files,
and it is the piece most likely to conflict with Step 7's refactor: the swap is
mechanical (one field, one parameter, six call sites).

**D4 — the whole DNS answer is refused if any address is bad.** A name
answering with one public and one private address is the rebind shape, and
hyper tries a resolved list in order, so filtering would make the verdict
depend on record order.

**D5 — IPv6 is an allow-list below the specific classes.** After
loopback/unspecified/mapped/compatible/link-local/ULA/multicast, anything
outside global unicast (`2000::/3`) is refused, plus `2002::/16` (6to4),
`2001::/32` (Teredo) and `2001:db8::/32` inside it. IPv6 special-purpose space
is large and still growing; a deny-list over it would be a guess about what
IANA does next. Cost: a receiver on `64:ff9b::/96` (NAT64) is refused even when
the embedded IPv4 is public. No such receiver is believed to exist; it is
listed as a residual below rather than left undiscovered.

**D6 — beyond the plan's list, deliberately.** IPv6 unique-local `fc00::/7`
(the v6 RFC 1918 — the plan named RFC 1918 for v4 only), the deprecated
IPv4-compatible form `::a.b.c.d`, and the IANA special-purpose IPv4 blocks
`192.0.0.0/24`, `192.0.2.0/24`, `198.18.0.0/15`, `198.51.100.0/24`,
`203.0.113.0/24`. Each has a row in the unit table.

**D7 — `EgressRefusal` implements `Classify` even though the delivery ladder
does not consult it.** `cargo xtask verify-errors` counts it (**13** error types
now, 12 before it) and ADR-0011 asks every leaf to classify itself.
`Category::Configuration` throughout, with `retry()` overridden to follow
`is_permanent()` so a later boundary that *does* read it cannot disagree with
`handle_deliver`. The doc comment says this plainly rather than implying the
ladder is derived from it.

**D8 — no new dependency.** `url` and `tokio`'s `net` feature were added to
`backends/crates/vpay-worker/Cargo.toml`; both are already in every binary's
resolved graph (url via reqwest, tokio `net` via reqwest/hyper/sqlx), so no
package is new anywhere and `cargo deny check` is unchanged (`advisories ok,
bans ok, licenses ok, sources ok`). `Cargo.lock` gained one line: `url` under
`vpay-worker`'s dependency list.

## Guard-failure proofs (both run, both restored)

**The classifier.** `classify` patched to `return None` unconditionally
(`backends/crates/vpay-worker/src/ssrf.rs`), then the two container-backed
cases:

```
FAIL a_delivery_to_a_private_address_is_refused_permanently_and_delivered_when_allowed
  assertion `left == right` failed: a refused target is permanent …
    left: "succeeded"   right: "exhausted"
FAIL a_host_that_resolves_to_a_private_address_is_refused_and_an_unresolvable_one_retries
  assertion `left == right` failed: a name resolving to loopback is refused exactly as the literal is
    left: "succeeded"   right: "exhausted"
Summary [165.607s] 2 tests run: 0 passed, 2 failed
```

`"succeeded"` is the decisive word: with the classifier bypassed the POST
reached the loopback receiver and the receiver answered `200`. Patch removed;
`git status` clean against the commit; both tests re-run: `2 tests run:
2 passed`.

**The config rule.** The `livemode && allow_private_targets` check removed from
`Config::validate_all`, then
`a_livemode_deployment_may_not_allow_private_webhook_targets`: `1 test run:
0 passed, 1 failed` (the livemode fixture loaded cleanly). Restored; `1 passed`.

## Gate, as run on the authoring machine (2026-09-03/04)

Every row below was re-run in this worktree's **own** `target/` after the
orchestrator's mid-lane correction: the shared `step8-target` directory was
serving artifacts fingerprinted from sibling lanes' trees, which produced two
readings that were not about this tree (a phantom "`client_pinned_to` not found
in `vpay_provider::http`" against a function that was on disk, and
`verify-errors: 12 error type(s)` where this tree has 13). Nothing below comes
from the shared directory.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy -p vpay-worker -p vpay-config --all-targets -- -D warnings` | clean |
| `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |
| `cargo nextest run -p vpay-worker` | **67 passed, 0 skipped** — 9 of them new (`ssrf::tests`), so 58 before |
| `cargo nextest run -p vpay-config` | **97 passed, 0 skipped** — 3 new, so 94 before |
| `cargo nextest run -p vpay-provider` | **17 passed, 0 skipped** — 1 new (the pin test), so 16 before |
| `VPAY_REQUIRE_NODE=1 cargo nextest run -p vpay-tests-integration -E 'binary(webhooks)' --no-fail-fast --retries 2` | **17 passed, 0 skipped**, 539 s (3 slow, all container starts) — 2 new, so 15 before |
| `just verify` | `verify-no-mocks: ok` / `verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md` / `verify-errors: ok — 13 error type(s), all classified; anyhow confined to binaries` |
| `just verify-ignored` | `0 ignored (expected 0), 39 test binaries (expected 39), 984 total (minimum 900)` — no binary added, no counter needs bumping |

One caveat on the integration run, stated because it cost a run: the first
attempt failed `the_delivered_signature_verifies_with_the_shipping_node_sdk`
with ``sh: 1: tsc: not found`` — this worktree had no `node_modules`. That is
the test working as designed (it fails rather than skips under
`VPAY_REQUIRE_NODE=1`) and is unrelated to this lane;
`CYPRESS_INSTALL_BINARY=0 pnpm install --frozen-lockfile` fixed it and the
suite is green as reported above.

## What lane E must change in the record

### 1. `docs/status.md` — the ⛔ row (currently line 960)

The row to replace is the one whose first cell reads exactly
`| Webhook URL validation — boot-time only, **no runtime SSRF filtering** | ⛔ |`
— title and body both change, and the ⛔ becomes 🟡 rather than ✅ for the three
residuals named at the end of it. **Verbatim replacement for the whole table
row:**

```
| Webhook URL validation (boot) and the runtime egress guard | 🟡 | **Changed 2026-09-03 (Step 8): the ⛔ this row used to carry is closed, and what remains is narrower.** Boot-time validation is unchanged and is still not SSRF protection: the `id` is 1–64 characters and the `url` 1–2048 in **both** modes, counted exactly as migration `0022`'s CHECKs count them (`the_length_bounds_are_migration_0022s`), the URL must parse, must carry no embedded credentials and must **name a host in both modes**, and `vpay_config::validate_webhook_url` adds exactly two livemode rules — the scheme must be `https` (compared as a scheme, so `HTTPS://Hooks.Example/x` is accepted) and the **host** must contain none of `wiremock`, `stub`, `mock`, `localhost` (so `https://hooks.example/mockups` is accepted and `https://mock.example/x` is not). None of that inspects an address, and it cannot: an address is not a property of the configuration. **The address is now checked at delivery, on every attempt, by `vpay_worker::ssrf`.** `handle_deliver` parses the URL, refuses any scheme but `http`/`https`, resolves the host **once** (`tokio::net::lookup_host`), classifies **every** address the lookup returned — loopback, unspecified, RFC 1918, IPv6 unique-local `fc00::/7`, link-local `169.254/16` and `fe80::/10`, CGNAT `100.64/10`, multicast, broadcast, `0.0.0.0/8`, `240/4`, the IANA special-purpose IPv4 blocks, every IPv6 address outside global unicast, the 6to4/Teredo/documentation prefixes inside it, and **the IPv4-mapped and IPv4-compatible IPv6 spellings of all of them** — and then builds the delivery client with `reqwest::ClientBuilder::resolve_to_addrs` pinned to exactly those addresses, so the name is never resolved a second time and a DNS rebind between check and connect has nothing to rebind. Redirects stay refused and the proxy environment stays ignored (`vpay_provider::http`), which is what stops a `302` leaving the pin. A refused target is a **permanent** failure — `state = 'exhausted'` on the first attempt, no next attempt, `response_excerpt` beginning `ssrf_blocked: ` and naming the address *class* but never the address, and exactly one `ERROR … alert = true` — while a host that merely fails to **resolve** stays an ordinary failed attempt on `delivery_delay`, because a resolver blip must not cost a merchant an event. `webhooks.allow_private_targets` (default `false`; `vpay_config::WebhookPolicy`) is the one value that changes that verdict, `config/application-sandbox.yml` and the generated `demo` overlay set it `true` because their receiver is a compose service, and **livemode plus `true` is a refusal to boot** (`ConfigError::PrivateWebhookTargetsInLivemode`, `a_livemode_deployment_may_not_allow_private_webhook_targets`). Proven by 9 unit cases over every range in both families including the mapped forms and by two container-backed cases against a real receiver: `a_delivery_to_a_private_address_is_refused_permanently_and_delivered_when_allowed` (the **same** address, both verdicts, and the receiver's own request journal holding exactly one POST — the allowed one) and `a_host_that_resolves_to_a_private_address_is_refused_and_an_unresolvable_one_retries` (a *name*, refused after resolution, and `.invalid` walking the ladder instead). Bypassing the classifier makes both fail with the delivery `succeeded` — that revert was run, and restored. **🟡 and not ✅ for three named residuals:** the guard is on webhook delivery only and not on the rail adapters (their hosts are operator-configured, not merchant-supplied); a receiver behind NAT64 (`64:ff9b::/96`) is refused even when the embedded IPv4 is public, which is fail-closed and unproven against any real receiver; and pinning costs the shared connection pool — each delivery now builds its own client (4.0 µs, measured) and re-handshakes, which nothing has measured under load |
```

### 2. `docs/status.md` — the Webhooks row (currently line 953)

One sentence inside it is now false. **Replace, verbatim, the fragment**

```
(2) **There is no SSRF protection of any kind** — validation is boot-time only and never inspects the destination address, so a livemode operator can point an endpoint at a loopback, RFC1918 or link-local address and vpay will deliver to it; the honest fix is a custom reqwest connector and it is out of scope (decision 4 of `docs/plans/2026-09-03-step5-webhooks.md`). See this file's own row for it.
```

**with**

```
(2) **The runtime egress guard landed in Step 8 and this reason is retired** — `vpay_worker::ssrf` resolves each endpoint's host once, refuses every loopback, private, link-local, CGNAT, multicast or otherwise non-public address (both families, mapped forms included) and pins the connection to the addresses it classified, so the TOCTOU that made a resolve-then-connect check worthless is closed without a custom connector. What is left is a *scope* limit, not an absence: the guard is on webhook delivery only, a NAT64 receiver is refused fail-closed, and the pin costs the shared connection pool. See this file's own row for it.
```

### 3. `docs/status.md` — the Step 5 pass note (currently lines 386-390)

**Replace, verbatim, the fragment**

```
is a host in configuration, exactly as the rails are; there is **no runtime SSRF
filtering** (boot-time `validate_webhook_url` only, stated plainly rather than
implied);
```

**with**

```
is a host in configuration, exactly as the rails are; there was **no runtime SSRF
filtering** at the time of that pass (boot-time `validate_webhook_url` only) —
closed in Step 8 by `vpay_worker::ssrf`, see this file's own row;
```

### 4. `docs/flows/webhooks.md` — the residual paragraph

In "**What boot-time URL validation actually checks — and what it does not**",
**replace, verbatim**

```
So `https://127.0.0.1/hook`, `https://10.0.0.5/hook` and
`https://169.254.169.254/latest/meta-data/…` all boot cleanly in livemode and
are all delivered to. **This is not SSRF protection and must not be described as
any.** It is a guard against shipping a stub host into production, which is a
different problem. The residual is stated in "What is not built" below and in
[../status.md](../status.md)'s own row for it.
```

**with**

```
So `https://127.0.0.1/hook`, `https://10.0.0.5/hook` and
`https://169.254.169.254/latest/meta-data/…` all boot cleanly in livemode.
**This is not SSRF protection and must not be described as any.** It is a guard
against shipping a stub host into production, which is a different problem.
What stops those three being *delivered to* is the next section, and it is a
different mechanism at a different time.

**The egress guard, at delivery time.** `vpay_worker::ssrf` runs on every
attempt, immediately before the socket and after the body has been rendered but
before it is signed. It parses the URL and refuses any scheme but
`http`/`https`; resolves the host **once**, with `tokio::net::lookup_host`;
classifies **every** address that lookup returned — loopback, unspecified,
RFC 1918, IPv6 unique-local `fc00::/7`, link-local `169.254.0.0/16` and
`fe80::/10`, CGNAT `100.64.0.0/10`, multicast, broadcast, `0.0.0.0/8`,
`240.0.0.0/4`, the IANA special-purpose IPv4 blocks, every IPv6 address outside
global unicast `2000::/3`, the 6to4/Teredo/documentation prefixes inside it,
and the **IPv4-mapped (`::ffff:10.0.0.1`) and IPv4-compatible (`::10.0.0.1`)
spellings of all of them** — and refuses the delivery if *any* of them is not
public, because a name answering with one public and one private address is the
shape of a rebind and hyper would try them in order. It then builds the
delivery client with `reqwest::ClientBuilder::resolve_to_addrs` pinned to those
addresses. **That pin is what makes the check mean anything**: reqwest never
resolves the name again, so the address that was classified is the address the
socket connects to. The Step 5 plan's decision 4 concluded that a
resolve-then-connect check is TOCTOU "unless reqwest is given a custom
connector"; `resolve_to_addrs` is the third answer it did not consider, and it
needs no connector. Redirects remain `Policy::none()` — a followed `302` would
resolve the hop's host freshly and be the one way back out of the pin.

A refused target is a **permanent** delivery failure: `state = 'exhausted'` on
the first attempt, `next_attempt_at` null, `payload_sha256` null (the guard runs
before signing, so no bytes were signed), `response_excerpt` beginning
`ssrf_blocked: ` and naming the address *class* — `loopback`, `link_local`,
`private`, `cgnat`, … — and **never the address**, because the address is
exactly what the request was trying to learn and `response_excerpt` is a column
the merchant's operator reads. Exactly one `ERROR … alert = true` is emitted, at
that transition, naming the endpoint id, the delivery, the event, the merchant
and the class. The ladder is not walked: eight identical refusals over 31 hours
tell nobody anything. A host that merely fails to **resolve** is *not* a refusal
— it is an ordinary failed attempt recorded `delivery_target_unavailable: …`
that walks `delivery_delay` exactly as the transport error it replaces did,
because a resolver outage heals and a merchant must not lose an event to one.

`webhooks.allow_private_targets` (default `false`) is the only value that
changes the verdict, and it changes nothing else: the guard resolves and
classifies identically either way. `config/application-sandbox.yml` and the
generated `demo` overlay set it `true` because `wiremock-webhook` is a service
on a compose network, and `deployment.livemode: true` together with it is a
refusal to boot (`ConfigError::PrivateWebhookTargetsInLivemode`) — a profile
selects a file, never a code path (ADR-0003).

**The cost, stated where it is paid.** A pin belongs to a `reqwest::Client`
builder, so each delivery builds its own client and no two deliveries share a
pooled connection: a receiver taking many events pays a TCP and TLS handshake
per event. Building the client itself is 4.0 µs (measured); the handshakes are
the real price. A per-host client cache would keep the pool and would be a cache
of pins — a client held past its DNS answer keeps delivering to the address that
name used to have — so the pool was the thing given up. Nothing has measured
this under load.
```

### 5. `docs/flows/webhooks.md` — the "What is not built" bullet

**Replace, verbatim, the bullet**

```
- **No SSRF protection of any kind.** `validate_webhook_url` checks the scheme
  and four host substrings and never looks at the destination address, so
  `https://127.0.0.1/…`, `https://10.0.0.5/…` and
  `https://169.254.169.254/latest/meta-data/…` are all valid livemode endpoints
  and all delivered to, with the answer's first 512 characters stored in
  `webhook_deliveries.response_excerpt`. A resolve-then-connect check is TOCTOU
  without a custom reqwest connector, so the honest options were "nothing" or
  "a connector", and the connector is out of scope (decision 4 of the Step 5
  plan). Stated here rather than implied.
```

**with**

```
- **The egress guard covers webhook delivery and nothing else.** The rail
  adapters are not behind it: `providers[].host` is operator-configured, not
  merchant-supplied, and `validate_host` already refuses a stub host in
  livemode. If a rail host ever becomes merchant-supplied, `vpay_worker::ssrf`
  moves to `vpay-provider` and both callers use it.
- **A receiver behind NAT64 (`64:ff9b::/96`) is refused**, even when the IPv4
  address it embeds is public, because the guard treats everything outside
  IPv6 global unicast as non-public rather than guessing at IANA's
  special-purpose space. That is fail-closed and has never been met in
  practice; it is written down so it is a decision rather than a surprise.
- **Pinning cost the shared connection pool.** One client per delivery, one
  handshake per delivery to the same receiver. Unmeasured under load.
```

### 6. `docs/flows/webhooks.md` — the "Acknowledge first, then work" paragraph

Its first sentence describes a client that no longer exists. **Replace,
verbatim**

```
**Acknowledge first, then work.** The delivery client is
`vpay_provider::http::client_with_timeouts(5s, 10s)` — 5 seconds to connect,
**10 seconds for the whole request**, redirects refused and the proxy
environment ignored — and it reads at most 8 KiB of the acknowledgement body,
```

**with**

```
**Acknowledge first, then work.** The delivery client is
`vpay_provider::http::client_pinned_to`, built per delivery from
`vpay_worker::ssrf`'s vetted addresses over the same two budgets
(`WEBHOOK_CONNECT_TIMEOUT` = 5 s to connect, `WEBHOOK_REQUEST_TIMEOUT` = **10 s
for the whole request**), with redirects refused and the proxy environment
ignored — and it reads at most 8 KiB of the acknowledgement body,
```

### 7. `docs/flows/webhooks.md` — the Status section's first sentence

It ends "No merchant endpoint has ever been POSTed to." That is still true and
must stay. Lane E should add, in the same paragraph or the next:
`Since Step 8 every delivery goes through the egress guard first
(`vpay_worker::ssrf`), including the ones in the compose stack — the sandbox
profile permits its private receiver explicitly rather than the guard being
absent.`

## Residuals this lane leaves open (all named above, repeated here for the PR)

1. The guard is on webhook delivery only, not on rail adapters. Deliberate; the
   move is a file move if that changes.
2. NAT64 receivers are refused fail-closed.
3. The pin costs the connection pool; the load characteristics are unmeasured.
4. `webhook_deliveries.sent_at` is set on a refused attempt even though nothing
   was sent — the same wart the pre-existing "endpoint has no secret" branch
   has. Left alone rather than changed under this lane; the column pair that
   says nothing came back is `status_code IS NULL AND responded_at IS NULL`,
   and both are null.
5. No deployment has ever *refused* a real merchant's endpoint; the evidence is
   the container-backed suite and the revert proof, not production.

## Rebase note for the orchestrator

* The only edits outside new files that Step 7 is likely to touch are
  `WebhookContext` (one field swapped), `run_loop`'s two signatures (one
  parameter swapped) and the ~20-line hook in `handle_deliver`. If Step 7 split
  `handle_deliver` into steps, the hook belongs in the step that owns "send",
  before signing, and `record_refused_target` moves with `record_failure`.
* The `justfile` edit (`gen-demo-keys`) will conflict with **lane A**, which
  rewrites the demo recipes. Keep both: lane A's recipe structure and this
  lane's `webhooks: allow_private_targets: true` block in the generated overlay
  plus the `allow_private_targets` staleness grep. Without it `just demo`'s
  webhook step fails against a receiver that is working perfectly.
* `Config` gained a field, so every `vpay_config::Config { … }` struct literal
  in the tree gained one line (11 sites, all `webhooks:
  vpay_config::WebhookPolicy::default(),`). A new literal added by another lane
  needs the same line.
