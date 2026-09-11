# Outbound webhooks — Status: endpoints, boot validation and the egress guard

_Split out of [docs/flows/webhooks.md](../webhooks.md) on 2026-09-11 by exp57, which broke a 811-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

**Endpoints are configuration, never a resource.** `merchant_clients[].webhooks[]`
in YAML (ADR-0003), keyed for fan-out on `merchant_id` and not on `client_id`;
each carries an operator-authored `id`, unique within a merchant and refused at
boot, stored verbatim on every delivery row so a URL correction does not orphan
the history. Secrets are covered by **two** livemode rules, and they are a
pair: the literal-secret rule reads the file's _text_ and says the value came
from the environment, and a 32-byte floor reads the _resolved_ value and says
the environment holds something worth having — an HMAC-SHA256 key shorter than
the hash's own output adds nothing over a 32-byte one and is what makes offline
guessing cheap (`ConfigError::WeakWebhookSecret`,
`a_livemode_webhook_secret_below_the_floor_is_refused`). Only the resolved value
can answer the second, because `${MERCHANT_WEBHOOK_SECRET}` is a placeholder of
fixed length whatever it holds. In sandbox neither applies and the rule is only
"not blank". There is no `/v1/webhook_endpoints` and no `webhook_endpoints`
table.

**What boot-time URL validation actually checks — and what it does not.** Every
endpoint URL is parsed once and then goes through
`vpay_config::validate_webhook_url`, plus shape checks. The
`id` is 1–64 characters and the `url` 1–2048, **in both modes**, counted in
characters exactly as migration `0022`'s `endpoint_id_length` and `url_length`
CHECKs count them — so a document the database would refuse is refused at boot
instead, and the constants are pinned against the migration by
`the_length_bounds_are_migration_0022s`. The URL must also parse, must not
carry embedded credentials (`https://user:pw@…`) and **must name a host — in
both modes**, so a `file:///var/spool/…` or a `mailto:ops@example` is refused
at the boot that introduced it rather than discovered as a delivery walking the
ladder to `exhausted` in a sandbox. `validate_webhook_url` itself is **two
rules and only two**, both livemode-only: the scheme must be `https`
(compared as a scheme, so `HTTPS://Hooks.Example/x` is fine), and the **host**
must not contain any of four substrings — `wiremock`, `stub`, `mock`,
`localhost`. The host and nothing else: `https://hooks.example/mockups` is a
merchant's own path and is accepted, `https://mock.example/x` is not
(`a_livemode_endpoint_may_be_uppercase_and_may_have_a_stub_word_in_its_path`,
and the refusal table beside it). It is a sibling of the rails'
`validate_host` rather than the same function, because that one's substring
tests are right for a bare origin and wrong for a URL. **Neither ever inspects
the destination address.** Under `livemode: false` neither rule applies.

So `https://127.0.0.1/hook`, `https://10.0.0.5/hook` and
`https://169.254.169.254/latest/meta-data/…` all boot cleanly in livemode.
**This is not SSRF protection and must not be described as any.** It is a guard
against shipping a stub host into production, which is a different problem.
What stops those three being _delivered to_ is the next section, and it is a
different mechanism at a different time.

**The egress guard, at delivery time.** `vpay_worker::ssrf` runs on every
attempt, immediately before the socket and after the body has been rendered but
before it is signed. It parses the URL and refuses any scheme but
`http`/`https`; resolves the host **once**, with `tokio::net::lookup_host`;
classifies **every** address that lookup returned — loopback, unspecified,
RFC 1918, IPv6 unique-local `fc00::/7`, link-local `169.254.0.0/16` and
`fe80::/10`, CGNAT `100.64.0.0/10`, multicast, broadcast, `0.0.0.0/8`,
`240.0.0.0/4`, the IANA special-purpose IPv4 blocks (including the 6to4 relay
anycast `192.88.99.0/24`), every IPv6 address outside global unicast
`2000::/3`, the special-purpose prefixes inside it — 6to4 `2002::/16`, Teredo
`2001::/32`, IETF protocol assignments `2001:1::/32`, benchmarking
`2001:2::/48`, ORCHIDv2 `2001:20::/28` and documentation `2001:db8::/32` — and
the **IPv4-mapped (`::ffff:10.0.0.1`) and IPv4-compatible (`::10.0.0.1`)
spellings of all of them** — and refuses the delivery if _any_ of them is not
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
`ssrf_blocked: ` and naming the address _class_ — `loopback`, `link_local`,
`private`, `cgnat`, … — and **never the address**, because the address is
exactly what the request was trying to learn and `response_excerpt` is a column
the merchant's operator reads. Exactly one `ERROR … alert = true` is emitted, at
that transition, naming the endpoint id, the delivery, the event, the merchant
and the class. The ladder is not walked: eight identical refusals over 31 hours
tell nobody anything. A host that merely fails to **resolve** is _not_ a refusal
— it is an ordinary failed attempt recorded `delivery_target_unavailable: …`
that walks `delivery_delay` exactly as the transport error it replaces did,
because a resolver outage heals and a merchant must not lose an event to one.

`webhooks.allow_private_targets` (default `false`) is the only value that
changes the verdict, and it changes nothing else: the guard resolves
identically either way, and under `true` it simply does not classify what it
resolved (`ssrf.rs:420-427`). `config/application-sandbox.yml` and the
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
