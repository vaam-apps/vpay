# `vpay-config` reference

Why the code in `backends/crates/vpay-config` looks the way it does, and the
boot sequence both binaries in `backends/apps` follow. The crates' own doc
comments say *what* each item is and link here.

Tier: an [ADR](../adr/) records a decision, a [flow](../flows/) describes a
process, and a reference page like this one explains why a particular piece of
code is shaped the way it is. The *process* this page's boot section supports is
[configuration.md](../flows/configuration.md); what follows is the code's side
of it.

- [The boot sequence](#the-boot-sequence)
  - [Why this order](#why-this-order)
  - [What is shared and what stays per-binary](#what-is-shared-and-what-stays-per-binary)
  - [Exit codes](#exit-codes)
- [Optional flags that are required in practice](#optional-flags-that-are-required-in-practice)
- [OAuth client shapes](#oauth-client-shapes)
- [The `checkout:` block, and `checkout_origins`](#the-checkout-block-and-checkout_origins)

---

## The boot sequence

Both `vpay-server` and `vpay-worker-bin` run the same steps in the same order.
`vpay-server`'s `run` is written as that list and nothing else; each step is a
named function, so "what happens before what" is answerable by reading a dozen
lines.

| # | Step | Where |
|---|---|---|
| 1 | Install SIGINT/SIGTERM handlers | `vpay_config::ShutdownSignals::install` |
| 2 | Install the rustls `CryptoProvider` | each binary's `install_crypto_provider` |
| 3 | Install the Prometheus recorder | each binary's `install_recorder` |
| 4 | Initialise tracing per `--log-format` | each binary's `init_tracing` |
| 5 | Build the one outbound HTTP client the rails share | `vpay_provider::http::client_with_timeouts` |
| 6 | Key this binary's linked adapters by `providers.code` | `vpay_api::boot::adapters_by_code` |
| 7 | Load, resolve `${ENV}` in, and validate the YAML | `vpay_api::boot::load_config` → `vpay_config::Config::load` |
| 8 | Join the YAML's rails against the linked adapters | `vpay_api::boot::boot_seeds` |
| 9 | Connect to Postgres and run migrations | `vpay_api::boot::open_migrated_database` |
| 10 | Reconcile `currencies` and `providers` (boot step 4 of the flow doc) | `vpay_api::boot::reconcile_reference_tables` |
| 11 | Everything binary-specific (signing key, listeners, the job loop) | each `main.rs` |

### Why this order

**Signal handlers first, before tracing.** A handler installed later is only
live once its future is first polled, which is the startup race
`ShutdownSignals` exists to close. A failure here is a hard startup failure
rather than a logged warning that lets the process continue with no graceful
shutdown path at all.

**Process-wide defaults before anything that could consume them.** The crypto
provider must be installed before the first `reqwest::Client` is built, and the
metrics recorder before the first `metrics::` macro runs — a metric recorded
before the recorder exists goes nowhere and is never recovered.

**The cheapest hard failure first.** Steps 7 and 8 need no network round trip, so
a broken YAML file, or a `providers[]` entry naming a rail this binary links no
adapter for, fails in milliseconds instead of after paying for a Postgres
connection and a migration run the process is about to throw away.
`vpay-server/tests/cli.rs`'s `a_provider_code_with_no_linked_adapter_is_exit_78`
needs no container precisely because of that placement, so moving `boot_seeds`
below the connect would break it.

**Connect and migrate before binding a listener.** A server that binds its port
before proving the database is reachable and up to date would start accepting
connections it cannot serve correctly. `/healthz` runs a real `SELECT 1`, so a
process with no database behind it would be lying about its own readiness.

**Reconcile after the migrations, before anything else assumes the database
agrees with the config.** The tables have to exist first. It is fatal on
failure: a `providers` table that still enables a rail an operator removed is a
deployment that would keep taking charges on it.

Two things make it safe to run the reconcile from **both** binaries rather than
nominating one as the writer, and neither is "idempotence". Idempotence covers
*repeating* a reconcile, which is not what happens during a rollout — there, two
of them *overlap*:

* they cannot interleave, because the reconcile's transaction opens by taking
  `vpay_db::lock_keys::CONFIG_RECONCILE` (proven taken by
  `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`);
* they cannot disagree about *what* to write, because the seeds come from one
  shared derivation — `vpay_api::boot::boot_seeds`, over each binary's own
  linked rails.

What is still true and worth stating plainly: two processes configured with
*different* YAML will each write their own view, last commit winning. Nothing
detects that, and nothing should — the lock makes the outcome one of the two
inputs rather than a mixture of both.

**The observability listener is bound last**, after the config, the database,
the migrations, the reconcile and (on the server) the validator. That ordering is
the entire definition of `/livez`: this process answers `ok` only once every one
of those has succeeded, so a probe against a server that is still starting, or
one that is about to exit 78 or 69, gets a connection refusal rather than a
cheerful 200. Nothing in the `/livez` handler checks anything; the bind is the
check. It is never bound on `--bind`: `/metrics` names every rail, route and
error code this deployment has, and `--bind` is the port an Ingress fronts.

### What is shared and what stays per-binary

Shared, in `vpay_api::boot`, because both binaries write the same two tables in
the same database and a divergence there would be silent: keying the adapters,
the YAML→seed join, the connect/migrate call and the reconcile call.

Per-binary, deliberately:

- **The `adapters()` list of linked rails** (Step 2's D6). A worker that learned
  which rails exist from `vpay-server`'s crate would make its capabilities a
  function of the API server, and the two deploy independently.
  `cargo xtask verify-no-mocks` walks the dependency graph from each binary
  root.
- **`install_crypto_provider`, `init_tracing`, `install_recorder`,
  `exit_code_for`.** These are near-copies on purpose. `exit_code_for` takes an
  `&anyhow::Error`, and ADR-0011 keeps `anyhow` out of every library crate's
  `[dependencies]` — a shared helper could not take one. The exporter's
  configuration is a property of what a process measures, and the two measure
  different things. "Which leaf errors this binary knows how to classify" is a
  property of the binary, not a library boundary, and the two are free to
  diverge as they grow. Each copy is pinned by its own CLI tests.
- **Everything after step 10**: the server loads the RS256 signing key, binds
  the traffic listener and mounts the router; the worker validates
  `--worker-concurrency`, projects each rail's `ProviderConfig` and the merchant
  webhook endpoints, and runs the job loop.

### Exit codes

Per [ADR-0011](../adr/0011-error-modelling.md)'s Tier 3 and
[errors.md](../flows/errors.md)'s table. `exit_code_for` walks the `anyhow`
chain for typed leaves and asks each one for its own `Category`; the code is
derived from that category, never chosen at a call site.

The order it looks in is load-bearing, not alphabetical: `ConfigError` and
`SigningKeyError` are looked for **before** `DbError`, because a chain can
plausibly contain both (a config that names an unreachable database) and in that
case the operator's actual problem is the configuration — `78` ("fix the
deploy") is more useful than `69` ("wait for Postgres").

`DbError` being last does **not** mean it always means `69`: the arm asks the
leaf for its own category, and `DbError::SigningKeyRetired` (a deployed Secret
naming a key this database has already retired) classifies as
`Category::Configuration`, so it exits `78` from that same arm. That is the point
of deriving the code from `Classify` rather than from which leaf matched.

Anything the walk does not recognise — a `clap` failure that got this far, a
bind error, an `anyhow!` from somewhere new — falls through to
`Category::Internal`, i.e. exit `1`. That fallback is deliberately the
pessimistic one: an unclassified startup failure in a payment binary should look
like a bug, not like a known condition.

`main` is synchronous and returns `ExitCode` rather than `anyhow::Result<()>`:
the `Termination` impl for `Result` prints the error with `Debug` and always
exits `1`, which is exactly the "a supervisor cannot tell 'fix the YAML' from
'Postgres is down'" problem ADR-0011 was written to fix. The message goes to
stderr with `eprintln!` rather than `tracing::error!`, because the earliest
failures happen *before* a subscriber is installed and a `tracing` event would be
dropped on the floor; `{error:#}` renders the whole context chain on one line, so
the `.context(..)` calls actually reach an operator.

---

## Optional flags that are required in practice

`--config`/`VPAY_CONFIG`, `--database-url`/`DATABASE_URL` and (on the server)
`--oauth-signing-key-file`/`VPAY_OAUTH_SIGNING_KEY_FILE` are all
`Option<...>` at the clap level and all required by the binary that reads them.

The split is deliberate. clap's own "required" would make the flag mandatory for
`--help` and for every subcommand a binary might grow, and it would report a
missing value as a usage error with clap's exit code rather than as a classified
startup failure. Requiring them *at the point of use* means each one produces a
typed leaf — `ConfigError::MissingPath`, or the binary's own `StartupError` —
that `exit_code_for` classifies as `Category::Configuration` and turns into exit
`78`, "fix the deploy".

Which inputs a process requires is a property of *that process*, which is why
`StartupError` is defined in each binary rather than in this crate:
`vpay-worker-bin` takes no `--oauth-signing-key-file` at all (it issues no
tokens, so mounting the signing key into it would widen the Secret's blast
radius for no capability), and `vpay-server` takes no `--worker-concurrency`.

A payment gateway that boots with no validated deployment configuration, or with
no database, is exactly the half-configured process
[ADR-0003](../adr/0003-yaml-configuration.md) says must never serve traffic —
`/healthz` included.

---

## OAuth client shapes

Statically registered OAuth2/OIDC clients
([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md),
[dashboard-auth.md](../flows/dashboard-auth.md)). Two kinds, both loaded from
YAML (ADR-0003) and both, structurally, carrying no client secret:

- `MerchantClient` — a merchant's `/v1` credential: `client_credentials` plus
  `private_key_jwt` (RFC 7523). vpay never sees a merchant's private key, only
  the **public** JWK set that verifies the assertion it signs.
- `DashboardClient` — the single `/dash/v1` client: authorization-code plus
  PKCE, a public client with no secret at all, requesting exactly one read-only
  scope.

### Why these types, and not `authkestra_op::client::ClientRegistration` directly

`vpay-config` deliberately does not depend on `authkestra-op` — that crate, and
the `ClientStore` that converts these types into a real `ClientRegistration`,
belong to the auth-wiring work that owns `backends/crates/vpay-api/**`, not to
config loading. These types are shaped to make that conversion mechanical:

| This type | `ClientRegistration` field | Fixed by client kind, not YAML |
|---|---|---|
| `MerchantClient::client_id` / `DashboardClient::client_id` | `client_id` | |
| `MerchantClient::jwks` | `jwks` (wrapped in `Some`) | |
| `MerchantClient::grant_types` | `grant_types` | |
| `MerchantClient::scopes` | `scopes` | |
| `MerchantClient::allowed_audiences` | `allowed_audiences` | |
| `DashboardClient::redirect_uris` | `redirect_uris` | |
| `DashboardClient::scope` | `scopes` (wrapped in a single-element `vec![]`) | |
| — | `client_secret_hash` | always `None` — see "No secret, ever" below |
| — | `token_endpoint_auth_method` | `PrivateKeyJwt` for merchants, `NoAuth` for the dashboard (RFC 7523 / public client) |
| — | `require_pkce` | always `false` for merchants (server-to-server, no browser step), always `true` for the dashboard |

`token_endpoint_auth_method` and `require_pkce` are not YAML fields on purpose:
they are invariants of *being* a merchant client or *being* the dashboard
client, never a per-deployment choice, so there is nothing for an operator to
configure — or misconfigure — there. `grant_types` stays a real YAML field on
`MerchantClient` specifically because ADR-0010 needs something to enforce
*against*: "declares any grant other than `client_credentials` is fatal" is a
validation rule over a value an operator could actually type, not a tautology
over a hardcoded constant.

### No secret, ever

Both types carry a `client_secret: Option<String>` field whose only legitimate
value is `None`. It exists so a config that accidentally carries a secret is
refused at boot (`ConfigError::ClientSecretPresent`, checked in
`Config::validate_all`) rather than silently ignored — the field would otherwise
just vanish into "unknown YAML key" territory, which is not the fail-fast story
ADR-0003 promises. Never populate it in a real config; it is a trap, not a
feature.

---

## The `checkout:` block, and `checkout_origins`

Step 9. Two additions, one deployment-wide and one per merchant, and the
reason they are separate is that they answer different questions:
*where is vpay's own checkout page*, and *who may frame it*.

### `checkout.public_base_url`

Its own block rather than a field on `deployment`, because it describes a
**second deployable**: `frontends/apps/checkout` is its own image and its own
Ingress, while `deployment.public_base_url` is the API's origin and is what
every rail callback URL is derived from. Conflating them would mean a
deployment serving checkout on its own host could not say so without also
moving every rail callback.

Every payer link vpay mints is built on it:

```text
hosted    {public_base_url}/c/{cs_id}#{client_secret}
embedded  {public_base_url}/e/{cs_id}?key={pk}#{client_secret}
return    {public_base_url}/c/{cs_id}/return?t={return_token}
```

`Option`, and **absent is a complete answer rather than a missing one**: most
deployments of this repository serve no checkout page, and the honest
behaviour for one of them is to refuse `POST /v1/checkout/sessions` with
`checkout_not_configured` rather than hand a merchant a `url` that resolves to
nothing. That is AGENTS.md's second rule applied to a configuration gap.

The rules (`validate_checkout_base_url`) are: a parseable `http(s)` URL with a
host, no userinfo, no query, no fragment, at most 2048 characters — and
`https` under `deployment.livemode`, as its own variant
(`InsecureCheckoutBaseUrl`) rather than one more shape reason, because the fix
is different. That last rule matters more here than for most hosts: a
plaintext base URL puts a live payer credential in clear on the wire on every
checkout, which is the one thing a URL fragment's own protection cannot help
with.

**A path prefix is accepted on purpose.** Whether production puts the checkout
app on its own host (`https://checkout.example`) or under a path on the API
host (`https://api.example/checkout`) is a decision the Step 9 plan reserves
for the maintainer, and the chart templates both — so refusing a path here
would be this crate taking a decision that was reserved. A query or a fragment
*is* refused, because vpay appends path segments to this value and there is no
correct way to append a path to a URL that already has one.

A trailing slash is accepted and normalised away once, in
`vpay_api::ResourceConfig::from_config`, so no link-building call site has to
remember: `format!("{base}/c/{id}")` over `https://checkout.example/` would
otherwise produce `//c/…`, which is a protocol-relative URL naming a
different host entirely.

Stub markers (`localhost`, `wiremock`) are **not** refused, unlike
`validate_host`. Those markers describe a *rail* host that a livemode
deployment must never talk to; this is a page a developer's own browser opens,
and the livemode `https` rule already refuses `http://localhost:3001`.

### `merchant_clients[].checkout_origins`

The list `Content-Security-Policy: frame-ancestors` is built from (D4) —
which sites may put vpay's embedded checkout page in an iframe.

Each entry is an **origin**: scheme, host, optional port, and nothing else.
That is what a CSP `host-source` is, and the check is unusually literal about
it — including refusing a bare trailing slash — because the failure mode is
silent. A browser handed `https://shop.example/checkout` in `frame-ancestors`
does not treat it as `https://shop.example`; depending on the browser it
ignores that source or discards the whole directive, and both look, from the
merchant's side, exactly like "vpay will not let me embed". The check is
against the **raw text** rather than the parse for the same reason:
`url::Url::parse` normalises `https://shop.example` to a `path()` of `/`, so
asking the parse "is there a path?" answers yes for the one spelling that is
correct.

**Not secret**, on the same footing as `publishable_keys` and for a stronger
reason: an origin is the merchant's own public website. `GET
/v1/browser/checkout/origins?key=…` therefore takes a publishable key and no
`client_secret` at all — there is nothing here to protect, and a secret in a
lookup the checkout app makes server-side would end up in the Next server's
logs.

**An empty list is the fail-closed default and is what most registrations
should have.** It means no site may embed; the page answers `frame-ancestors
'none'`, and hosted checkout is unaffected because it is never framed.

Uniqueness is checked across *every* merchant, before any single origin's
shape, for the reason the publishable-key walk runs in that order — and the
consequence is sharper here than for a key. The checkout app looks the list up
**by publishable key**, so two merchants sharing an origin means whichever of
them a payer's key names decides whether the *other* merchant's site may frame
the page: a security answer with two values depending on iteration order.

Origins with **no** `checkout.public_base_url` are refused
(`CheckoutOriginsWithoutBaseUrl`): there would be no page for them to frame,
and an operator who wrote them believes embedding works. The reverse pairing —
a base URL and no origins anywhere — is legal and normal; it is a
hosted-checkout-only deployment.

### An origin must be spelled the way a browser spells it

Every rule above is about a value that is not an origin. This one is about a
value that *is* one, spelled a way the thing that consumes it does not
recognise: `https://Shop.example`, `https://shop.example:443` and
`https://shöp.example` all parse, name a host, are `https`, and carry no path,
query, fragment or credentials.

The checkout app filters its `frame-ancestors` list by comparing each entry
against the browser's `URL.origin`
(`frontends/apps/checkout/src/lib/origins.ts`) — lower-cased host, IDNA-encoded
to ASCII, default port elided. An entry that differs from that is an entry the
browser never sees, and the symptom is *silence*: the list loads, the route
answers `200`, and the merchant's site simply cannot frame the page.

So the raw text must equal `parsed.origin().ascii_serialization()`, and
`ConfigError::NonCanonicalCheckoutOrigin` names **what to write instead**
rather than which rule was broken — the useful part of that message is a value,
which the `&'static str` reason on `MalformedCheckoutOrigin` cannot carry.

Refused rather than normalised on load, for the reason nothing else here
rewrites what an operator wrote: normalising would make the configuration file
and the running policy two different documents, and the next person to read the
YAML would see a spelling vpay is not using.

### `merchant_clients[].display_name`

What a payer is told they are paying, on vpay's own checkout page. Optional;
non-blank and at most 80 characters when present.

The bound is a **rendering** rule, not a storage one — the value is painted
into "Pay {merchant}" in a heading on a phone-sized page — and it is refused at
boot rather than truncated at render time because a payer's first impression of
a merchant they are about to pay is worth hearing about on the deploy that
introduced it. Characters, not bytes: a name in any script gets the same 80.

There is no character-class rule. A merchant's name is theirs, in whatever
language their payers read, and the page escapes it as text.

Absent is legal and is what most registrations carry. The browser reads then
fall back to `merchant_id`, which is a true name for who is being paid but an
internal one — see
[vpay-api.md](vpay-api.md#merchantname-and-why-there-is-a-fallback) for why the
fallback exists at all rather than the read answering without a name. A
deployment that serves hosted checkout should set this for every merchant.

There is nowhere else this could live: there is no merchants table (ADR-0003),
so a registration is the only place a human-readable name for a tenant exists.

### One unreachable branch, and the test that proves it is

Both validators refuse a URL that names no host. Neither branch is reachable:
`url::Url::parse` rejects `http://` and `https://` outright with `EmptyHost`,
and every other scheme is refused a line earlier. The branch is kept anyway —
a total expression rather than an assumption about a third-party parser's
exhaustiveness, the same posture `vpay_core::ids::push_base32`'s
`.unwrap_or(b'0')` takes — and
`a_hostless_http_url_never_reaches_the_host_branch` is what *proves* it
unreachable rather than merely asserting it. It is a tripwire on a dependency:
if a future `url` release started accepting an empty host, that test says so
before the branch quietly becomes the only thing between a config file and a
payer link with no host in it.

---

## Shutdown and drain

`axum::serve(..).with_graceful_shutdown(..)` waits *indefinitely* for in-flight
connections once its signal future resolves — there is no built-in bound on that
wait. `vpay-server` adds one by observing the shutdown signal twice through a
oneshot: axum's graceful-shutdown future uses it to start draining, and a second
consumer starts a `--shutdown-grace-seconds` clock at that same moment.
Whichever finishes first — the drain, or the clock — decides the outcome.

The signal has a third observer: the observability listener stops accepting at
the moment the drain starts. A detached task with no shutdown of its own would
keep the port open past the drain and answer `/livez` with `ok` while the
process was on its way out.

`grace_clock` is deliberately a pure function of a oneshot receiver and a
`Duration`, so the timing logic can be tested without a real HTTP server,
socket, or in-flight request. That matters here specifically because there is no
honest way to produce a genuinely slow in-flight request against the real
router: `/healthz` answers instantly, and adding a slow test-only route to
`vpay-api` would put a test double in the shipping router, which
`cargo xtask verify-no-mocks` forbids and AGENTS.md rules out outright.

### Why a timed-out drain exits non-zero

A container orchestrator (docker compose, k8s) already treats the container as
"stopped" the moment the process exits at all, whatever the code — it does not
retry or block shutdown on a non-zero exit here, so this changes nothing about
the orchestration outcome. But unlike the clean path, this exit means real
in-flight work was cut off rather than finished, which is not "successful" from
the process's own point of view. A non-zero exit lets anything that *does* watch
the exit code — a supervisor, `docker inspect --format
'{{.State.ExitCode}}'`, a monitoring rule on container restarts — tell a forced
cutoff apart from a clean drain without parsing logs. `1` rather than a
SIGKILL-style `128+n` encoding, since nothing signalled the process; it chose to
stop waiting on its own.

The worker's version of "cut off" is materially worse than the server's, which
is why it is worth the number there too: a job aborted mid-flight has already had
its `attempts` incremented and may have called a rail. Its lease is handed back
so another worker re-runs it at once, and every handler is a compare-and-swap so
the re-run is a no-op if the first pass committed — but repeated timeouts mean
the grace period is below what a poll actually takes.

Failures of the *observability* listener on the way out are logged and never
propagated: the observability port is not a payment path, and letting it change
the exit code would make the forced-cutoff `1` ambiguous. The timed-out path
calls `std::process::exit(1)` and takes that task with it, which is correct
there — a process that has already given up on in-flight payments should not
then wait on a metrics socket.

---

## The rustls `CryptoProvider` process default

The root `Cargo.toml` pins `reqwest` with `rustls-no-provider`: the alternative
selects `aws-lc-rs`, which `deny.toml` bans outright because two providers in
one process are exactly what makes `install_default()` panic. The cost of
picking nothing is that reqwest 0.13's `ClientBuilder::build()` calls
`CryptoProvider::get_default()` and **panics** — "No rustls crypto provider is
configured" — when there is no process default. That is a panic in a shipping
payment binary, i.e. a defect under [ADR-0007](../adr/0007-lint-policy.md), on a
path no unit test reaches. [status.md](../status.md)'s "rustls `CryptoProvider`
process default" row tracks it as a documented landmine.

The one ordering constraint is "before the first `reqwest::Client` is built", so
the install sits at the top of each binary's boot, above tracing init, where no
future edit can slip a client construction in ahead of it. It is deliberately
*not* done in a library: installing a process-wide default from a library takes
the decision out of the application's hands (the reasoning `sdks/rust` records
for why it hands reqwest a pre-built `ClientConfig` instead).

**What it does not cover, and why it stays anyway.** No path either binary
reaches today depends on it: `vpay_provider::http` hands reqwest a finished
`rustls::ClientConfig`, which takes a branch that consults neither the process
default nor the OS trust store, and `sqlx` passes its own provider explicitly. It
stays because the hazard it guards is one `use` away, not gone:
`authkestra-engine` still writes `reqwest::Client::new()` in its captcha and
device/client-credentials flows, and tomorrow's first HTTPS-speaking rail adapter
is another candidate. Note that this call is **not** sufficient protection for
either: inside the `FROM scratch` runtime image a bare `reqwest::Client::new()`
panics on the *trust store* ("No CA certificates were loaded from the system")
whether or not a provider is installed. `vpay_provider::http::client` is the only
client constructor that works there, and any new outbound HTTP in either binary
should use it.

`install_default()` returns `Err(Arc<CryptoProvider>)` for exactly one reason: a
default was already installed. In a binary that means some other code got there
first, which is the state the call wanted anyway — so `.ok()`. `unwrap`/`expect`
are denied here (ADR-0007) and would turn a harmless double install into a
startup crash.

---

## `vpay_build_info`'s `git_sha`, and when it is `unknown`

`vpay_core::metrics::record_build_info` stamps the gauge from
`vpay_core::metrics::git_sha`, which is `option_env!("VPAY_GIT_SHA")` resolved
when *`vpay-core`* was compiled (that crate's `build.rs` puts the variable in
cargo's fingerprint, so changing it rebuilds rather than silently reusing the
previous label). `backends/Dockerfile` declares `ARG VPAY_GIT_SHA` and exports it
into the builder stage, and `.github/workflows/release.yml` passes
`${{ github.sha }}`.

Every build that nobody passed one to — every local `cargo build`, every
`just demo`, every `docker build` without `--build-arg` — reads `unknown`, and
that is the honest value rather than a placeholder: deriving one from
`git rev-parse` at runtime would report the sha of whatever tree the *process* is
standing in, which for a `FROM scratch` image is nothing at all.

The recorder is installed by the application, never by a library:
`vpay_core::metrics` owns the names and nothing else, and installing a global
recorder from a library takes the decision out of the application's hands and
makes two linked libraries a startup failure. The exporter's configuration —
which quantiles, which buckets, which idle timeout — is a property of what a
process measures, and the two binaries measure different things, which is why
neither shares the other's `install_recorder`.

---

## Why the `/v1` validator fetches its JWKS over loopback

The public URL is what a *merchant* uses and what the discovery document
advertises, but a pod is not guaranteed to be able to reach its own public
hostname: split-horizon DNS may not resolve it inside the cluster, an ingress may
terminate somewhere the process cannot route back through, an egress
`NetworkPolicy` may forbid the hairpin, and a deployment behind a not-yet-warm
DNS record would fail its first validation. All of those turn "verify a token"
into a network dependency on infrastructure that exists to serve *inbound*
traffic. Loopback has none of those failure modes and reaches the same handler,
backed by the same database rows, that a merchant's fetch would.

The port comes from `TcpListener::local_addr`, not from `--bind`, because `:0` is
a real configuration. An unspecified bind address (`0.0.0.0`, `[::]`) is mapped
to the corresponding loopback address rather than used as-is: `0.0.0.0` means
"listen on every interface" and is not a *destination* — connecting to it is
platform-dependent, and that is not something to rely on in a payment binary. The
address family is preserved, so an IPv6-only deployment dials `[::1]` and not
`127.0.0.1`. A specific bind address is used verbatim: an operator who bound one
interface on purpose gets a URL on that interface.

The whole round trip is an HTTP call to ourselves and could later be replaced by
an in-process key source, which would remove a socket from the path entirely. It
is not done because the alternative — publishing the one key *this* process holds
— is exactly the mistake `vpay_api::op::jwks` rejects: during a rotation the JWKS
must carry every key still inside its overlap window, which is a property of the
database and not of this process's memory. An in-process source would have to read
the same rows and cache them, which is a real design with its own invalidation
question, not a simplification.
