//! The egress guard on webhook delivery: classify every address a merchant's
//! endpoint resolves to, refuse the ones that are not on the public internet,
//! and connect only to the ones that were checked.
//!
//! # The hole this closes
//!
//! `docs/flows/webhooks.md`, until this module existed: "**No SSRF protection
//! of any kind.** `validate_webhook_url` checks the scheme and four host
//! substrings and never looks at the destination address, so
//! `https://127.0.0.1/…`, `https://10.0.0.5/…` and
//! `https://169.254.169.254/latest/meta-data/…` are all valid livemode
//! endpoints and all delivered to, with the answer's first 512 characters
//! stored in `webhook_deliveries.response_excerpt`."
//!
//! Every part of that sentence is the attack. The endpoint URL is
//! merchant-supplied configuration; the request goes out from *inside* the
//! deployment's network, authenticated by nothing but its position on that
//! network; and the receiver's answer is stored where the merchant's operator
//! can read it. That is a read primitive against the cloud metadata service,
//! the Postgres service, and every peer the worker pod can reach.
//!
//! # Why a resolve-then-pin, and not a resolve-then-connect
//!
//! The Step 5 plan (decision 4) rejected a runtime check on the grounds that
//! "a resolve-then-connect check is TOCTOU without a custom reqwest
//! connector", and that was right about the check and wrong about the only
//! remedy. Checking a name and then handing the *name* to reqwest resolves it
//! a second time, and a DNS record with a one-second TTL answers the two
//! lookups differently on purpose — that is the whole of DNS rebinding. But
//! reqwest does not need a custom connector to be told the answer: a client
//! built with [`reqwest::ClientBuilder::resolve_to_addrs`] never asks the
//! resolver for that host at all. So the sequence here is
//!
//! 1. parse the URL and refuse any scheme but `http`/`https`;
//! 2. resolve the host **once**, with [`tokio::net::lookup_host`];
//! 3. classify every address the lookup returned;
//! 4. build a client pinned to exactly those addresses
//!    ([`vpay_provider::http::client_pinned_to`]), redirects still refused;
//!
//! and the address the socket connects to is one of the addresses step 3
//! looked at, because nothing resolves the name again. Redirects staying off
//! is the other half: a pinned client that followed a `302` would resolve the
//! hop's host freshly and connect to whatever it named
//! (`vpay_provider::http::preconfigured_builder` is where that policy lives,
//! and it is why this module does not have to re-state it).
//!
//! # Why the whole answer is refused when any address is bad
//!
//! A name that resolves to a public address *and* a private one is the shape
//! of a rebind, not of a healthy receiver, and hyper tries a resolved list in
//! order — so filtering the bad ones out would make the outcome depend on
//! which record the resolver happened to put first. [`vet`] refuses the
//! delivery.
//!
//! # What this deliberately does not do
//!
//! * It does not consult an allowlist. There is no `webhooks.allowed_hosts`;
//!   the rule is about address *class*, so a merchant on a public IP needs no
//!   registration and an operator maintains no list.
//! * It does not protect the rail adapters. They call operator-configured
//!   hosts (`providers[].host`), not merchant-supplied ones, and
//!   `validate_host` already refuses a stub host in livemode. If a rail host
//!   ever becomes merchant-supplied, this module moves to `vpay-provider` and
//!   both callers use it — see `docs/plans/2026-09-03-step8-production-gate.md`
//!   lane B, which names that as the alternative it did not take.
//! * It does not stop a merchant naming a host whose *public* address is
//!   someone else's server. That is not SSRF; it is a merchant configuring
//!   their own endpoint badly, and the signature is what makes it harmless.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use vpay_core::error::{Category, Classify};
use vpay_provider::http::HttpClientError;

use crate::webhooks::{WEBHOOK_CONNECT_TIMEOUT, WEBHOOK_REQUEST_TIMEOUT};

/// The two schemes a webhook may be delivered over.
///
/// `http` is here because a sandbox delivers to a WireMock container over
/// plaintext; livemode already refuses anything but `https`
/// (`vpay_config::validate_webhook_url`), at boot, where the operator can see
/// it. Everything else — `file`, `gopher`, `ftp`, and the redirect-to-`file`
/// trick — is refused here as well as by the boot-time host check, because
/// this is the last place before a socket.
const ALLOWED_SCHEMES: [&str; 2] = ["http", "https"];

/// Whether this deployment may deliver a webhook to a non-public address.
///
/// Projected out of `vpay_config::WebhookPolicy` by the worker binary, exactly
/// as the endpoint table is projected out of `merchant_clients[].webhooks[]`:
/// a handler must not read the shape of a YAML document
/// ([`crate::WebhookContext`]).
///
/// [`Self::default`] is the safe answer, and that is load-bearing rather than
/// tidy — a caller that forgets to thread the deployment's value through gets
/// the guard, not the hole.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EgressPolicy {
    /// `true` only where the receiver genuinely is private: the compose
    /// stack's `wiremock-webhook`, and the integration suite's receiver
    /// container. `vpay_config` refuses this together with
    /// `deployment.livemode`.
    pub allow_private_targets: bool,
}

impl EgressPolicy {
    /// The shipping default: a non-public address is refused.
    pub const DENY_PRIVATE: Self = Self {
        allow_private_targets: false,
    };

    /// What a sandbox or the integration suite passes, because its receiver
    /// is on the same private network it is.
    pub const ALLOW_PRIVATE: Self = Self {
        allow_private_targets: true,
    };
}

/// What a resolved address is, when it is not an ordinary public one.
///
/// The vocabulary a refusal is reported in: it names a *class*, never an
/// address. A merchant's operator reads `ssrf_blocked: loopback` off
/// `webhook_deliveries.response_excerpt`, and that tells them what to fix
/// without telling anyone which internal address answered — see [`vet`]'s
/// note on why the address never leaves this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressClass {
    /// `127.0.0.0/8`, `::1`. The worker's own process, and every port on it.
    Loopback,
    /// `0.0.0.0`, `::`. "This host", which a connect() turns into loopback.
    Unspecified,
    /// RFC 1918 (`10/8`, `172.16/12`, `192.168/16`) and IPv6 unique-local
    /// (`fc00::/7`) — the deployment's own network: Postgres, the rail stubs,
    /// every peer pod.
    Private,
    /// `169.254.0.0/16` and `fe80::/10`. The cloud metadata service
    /// (`169.254.169.254`) lives here, which is the single most valuable
    /// target an SSRF has.
    LinkLocal,
    /// `100.64.0.0/10`, carrier-grade NAT (RFC 6598) — routable inside a
    /// provider's network and nowhere else.
    Cgnat,
    /// `224.0.0.0/4`, `ff00::/8`. Nothing a POST should be aimed at.
    Multicast,
    /// `255.255.255.255`.
    Broadcast,
    /// Everything else IANA has reserved or that embeds an address of one of
    /// the classes above: `0.0.0.0/8`, `240.0.0.0/4`, the IPv4 documentation
    /// and benchmarking blocks, and every IPv6 address outside global unicast
    /// (`2000::/3`) plus the deprecated tunnelling prefixes inside it.
    Reserved,
}

impl AddressClass {
    /// The word this class is written as in a delivery's recorded reason and
    /// in the alert log line.
    ///
    /// A fixed vocabulary rather than `Debug`: it is written into
    /// `webhook_deliveries.response_excerpt`, which an operator greps and a
    /// runbook quotes, so it must not change when a variant is renamed.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Unspecified => "unspecified",
            Self::Private => "private",
            Self::LinkLocal => "link_local",
            Self::Cgnat => "cgnat",
            Self::Multicast => "multicast",
            Self::Broadcast => "broadcast",
            Self::Reserved => "reserved",
        }
    }
}

/// Why a webhook target may not be connected to.
///
/// Every variant but [`Self::Unresolvable`] and [`Self::Client`] is
/// **permanent**: retrying delivers the same refusal on the same schedule for
/// 31 hours, so [`Self::is_permanent`] is what
/// `vpay_worker::webhooks::handle_deliver` uses to exhaust the delivery on the
/// spot instead.
///
/// The address itself is never a field, and never in a `Display`. A refusal
/// that printed `169.254.169.254` would put the answer to "what is reachable
/// from inside this deployment?" into `webhook_deliveries.response_excerpt`,
/// which is a column the merchant's operator can read — the guard would be
/// answering the question the request was asking.
#[derive(Debug, thiserror::Error)]
pub enum EgressRefusal {
    /// The endpoint URL does not parse. Boot-time validation
    /// (`vpay_config::config::validate_webhook_endpoints`) refuses this too;
    /// reaching it here means the row predates the endpoint's correction, and
    /// no number of retries parses it.
    #[error("the endpoint url does not parse: {reason}")]
    Unparseable {
        /// `url::ParseError`'s own words, which name the defect and not the
        /// URL.
        reason: String,
    },

    /// The URL names a scheme other than `http`/`https`.
    #[error("the endpoint url names the scheme `{scheme}`, and only http and https are delivered")]
    Scheme {
        /// The scheme as `url` normalised it (always lowercase).
        scheme: String,
    },

    /// The URL has no host at all — `file:///…`, `mailto:…`.
    #[error("the endpoint url names no host")]
    NoHost,

    /// The host resolved, and at least one of its addresses is not public.
    ///
    /// `class` is the *first* offending address's class in resolution order.
    /// One is enough: the delivery is refused, and naming more of them would
    /// describe more of the network.
    #[error("the endpoint host resolves to a {} address", class.as_str())]
    Blocked {
        /// What the offending address is.
        class: AddressClass,
    },

    /// The host did not resolve. **Transient**: a resolver outage, or a
    /// receiver mid-migration, both heal without anyone editing configuration,
    /// so this walks the ordinary retry ladder exactly as the transport
    /// failure it replaces did.
    #[error("the endpoint host could not be resolved")]
    Unresolvable {
        /// The resolver's own error. Names the host and the failure, never a
        /// body or a secret.
        #[source]
        source: std::io::Error,
    },

    /// The pinned client did not build. Not reachable from any input this
    /// module has (see [`HttpClientError`]'s own doc: its inputs are fixed at
    /// compile time), and treated as transient rather than permanent so an
    /// impossible failure does not silently cost a merchant an event.
    #[error("the pinned delivery client could not be built")]
    Client {
        /// The construction failure.
        #[source]
        source: HttpClientError,
    },
}

impl EgressRefusal {
    /// Whether retrying this delivery is pointless.
    ///
    /// The permanent ones are all statements about *configuration* — a scheme,
    /// a URL, an address class — and the two transient ones are statements
    /// about the network at one instant. `handle_deliver` turns the first kind
    /// into an exhausted delivery immediately and lets the second kind walk
    /// `crate::delivery_delay`.
    #[must_use]
    pub const fn is_permanent(&self) -> bool {
        match self {
            Self::Unparseable { .. }
            | Self::Scheme { .. }
            | Self::NoHost
            | Self::Blocked { .. } => true,
            Self::Unresolvable { .. } | Self::Client { .. } => false,
        }
    }

    /// The short token a delivery row and an alert are keyed on.
    ///
    /// `ssrf_blocked` for the address verdict and the three URL defects that
    /// mean the same thing operationally — this delivery is not going out —
    /// so a runbook greps one string. The transient pair is deliberately *not*
    /// that string: an unresolvable host is an ordinary failed attempt and
    /// must not be counted as a blocked one.
    #[must_use]
    pub const fn token(&self) -> &'static str {
        if self.is_permanent() {
            "ssrf_blocked"
        } else {
            "delivery_target_unavailable"
        }
    }
}

impl Classify for EgressRefusal {
    /// [`Category::Configuration`] throughout, with one `retry` override.
    ///
    /// Every variant is settled by something an operator or a merchant writes
    /// down: an endpoint URL, a DNS record, this deployment's
    /// `webhooks.allow_private_targets`. None is a caller's request, none is
    /// the rail (a merchant's receiver is not a payment rail —
    /// [`Category::Rail`]'s own doc reserves that for a rail that "could not
    /// be reached"), and none is a bug in this code.
    ///
    /// **`Classify` is not what drives the retry here**, and the impl exists
    /// because ADR-0011 requires every leaf to classify itself, not because
    /// the delivery ladder consults it: webhook delivery walks
    /// [`crate::delivery_delay`] and deliberately does not consult
    /// `JobError::decision` (`crate::webhooks`' module doc says why — a
    /// merchant's `500` must not inherit the rail's escalation policy). So
    /// [`Self::is_permanent`] is the answer `handle_deliver` reads, and the
    /// `retry` override below is here so the two cannot disagree if some
    /// later boundary does read this one.
    fn category(&self) -> Category {
        Category::Configuration
    }

    fn retry(&self) -> vpay_core::Retry {
        if self.is_permanent() {
            vpay_core::Retry::Never
        } else {
            vpay_core::Retry::AfterBackoff
        }
    }
}

/// A destination that has been checked, and the addresses it was checked at.
///
/// Only [`vet`] constructs one, which is the point: a `VettedTarget` in hand
/// is evidence that every address in it was classified under a policy, and
/// [`pinned_client`] takes nothing else. There is no way to build a delivery
/// client for a host that has not been through the classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VettedTarget {
    /// The URL's host, as `url` normalised it — the key reqwest's resolve
    /// override is looked up by. For an IP literal it is the address without
    /// the brackets a URL writes IPv6 in, and the override is never consulted
    /// at all: hyper connects to a literal directly rather than resolving it,
    /// which is why a literal has no second lookup to be pinned against.
    host: String,
    /// Every address the one lookup returned, in resolution order.
    addrs: Vec<SocketAddr>,
}

impl VettedTarget {
    /// The host the pin is keyed on.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The vetted addresses, in the order the resolver returned them.
    ///
    /// Kept in order because hyper tries them in order: a receiver whose first
    /// A record is unreachable must still be delivered to.
    #[must_use]
    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }
}

/// Resolves `url`'s host once and checks every address it answers with.
///
/// # What "once" means, and why it is the guarantee
///
/// This is the *only* resolution of that name in the delivery path.
/// [`pinned_client`] installs the addresses returned here as reqwest's answer,
/// so the connect uses one of them rather than asking again — which is what
/// makes the classification below a fact about the socket instead of a fact
/// about a lookup that has since been re-answered.
///
/// A host that is already an IP literal is not resolved at all: there is no
/// second lookup to disagree with, and reqwest connects to the literal
/// directly.
///
/// # Errors
///
/// [`EgressRefusal`], and [`EgressRefusal::is_permanent`] says which kind. The
/// blocked case names the address *class* and never the address: the answer to
/// "what is reachable from inside this deployment?" is exactly what an SSRF is
/// trying to learn, and this function is not the oracle for it. The address is
/// not logged either — [`crate::webhooks`]'s alert line carries the endpoint
/// id, the delivery id and this class, and that is all.
pub async fn vet(url: &str, policy: EgressPolicy) -> Result<VettedTarget, EgressRefusal> {
    // `url::Url`, which is the parser reqwest itself uses (it re-exports this
    // crate's `Url`): vetting a host that a *different* parser produced would
    // be vetting a host the request might not use.
    let parsed = url::Url::parse(url).map_err(|error| EgressRefusal::Unparseable {
        reason: error.to_string(),
    })?;

    if !ALLOWED_SCHEMES.contains(&parsed.scheme()) {
        return Err(EgressRefusal::Scheme {
            scheme: parsed.scheme().to_owned(),
        });
    }

    // `host()` and not `host_str()`: the former discriminates a literal from a
    // domain and hands back an IPv6 literal without the brackets a URL writes
    // it in, which `Ipv6Addr` would refuse to parse.
    let host = parsed.host().ok_or(EgressRefusal::NoHost)?;
    // Only ever used as the port of a `SocketAddr` for the lookup: reqwest
    // replaces it with the URL's own port when it connects
    // (`client_pinned_to`), so nothing downstream depends on this value.
    let port = parsed.port_or_known_default().unwrap_or(0);

    let (key, addrs) = match host {
        url::Host::Ipv4(ip) => (ip.to_string(), vec![SocketAddr::from((ip, port))]),
        url::Host::Ipv6(ip) => (ip.to_string(), vec![SocketAddr::from((ip, port))]),
        url::Host::Domain(domain) => {
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((domain, port))
                .await
                .map_err(|source| EgressRefusal::Unresolvable { source })?
                .collect();
            if resolved.is_empty() {
                // A lookup that succeeds with an empty answer is not a thing
                // `getaddrinfo` normally does, but an empty pin would make
                // reqwest connect to nothing at all with a confusing error —
                // so it is reported as the transient failure it resembles.
                return Err(EgressRefusal::Unresolvable {
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "the host resolved to no addresses",
                    ),
                });
            }
            (domain.to_owned(), resolved)
        }
    };

    if !policy.allow_private_targets {
        for addr in &addrs {
            if let Some(class) = classify(addr.ip()) {
                return Err(EgressRefusal::Blocked { class });
            }
        }
    }

    Ok(VettedTarget { host: key, addrs })
}

/// The delivery client for a target [`vet`] has already checked.
///
/// The webhook budgets ([`WEBHOOK_CONNECT_TIMEOUT`], [`WEBHOOK_REQUEST_TIMEOUT`])
/// rather than parameters: this is the only client webhook delivery uses, and
/// a caller able to choose the timeouts is a caller able to build one that is
/// not the shipping one — the drift `crate::webhooks`' constants exist to stop.
///
/// # Errors
///
/// [`EgressRefusal::Client`], which is not reachable from any input here.
pub fn pinned_client(target: &VettedTarget) -> Result<reqwest::Client, EgressRefusal> {
    vpay_provider::http::client_pinned_to(
        WEBHOOK_CONNECT_TIMEOUT,
        WEBHOOK_REQUEST_TIMEOUT,
        target.host(),
        target.addrs(),
    )
    .map_err(|source| EgressRefusal::Client { source })
}

/// What `ip` is, or `None` if it is an ordinary address on the public
/// internet.
///
/// # Why the ranges are written out rather than taken from `std`
///
/// `Ipv4Addr::is_private`/`is_loopback`/`is_link_local`/`is_multicast` are
/// stable and are used. The rest of what this has to know —
/// `Ipv4Addr::is_shared` (CGNAT), `is_reserved`, `is_benchmarking`,
/// `is_documentation`, `Ipv6Addr::is_unique_local`,
/// `is_unicast_link_local` — is still nightly-only (`#![feature(ip)]`), and
/// this workspace pins a stable toolchain (`rust-toolchain.toml`). Writing the
/// prefixes out keeps the guard on stable and keeps every range visible in one
/// place, which is also what makes the table in this module's tests able to
/// name them one by one.
#[must_use]
pub fn classify(ip: IpAddr) -> Option<AddressClass> {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

/// [`classify`] for IPv4. Ordered most-specific first, because the ranges
/// overlap: `255.255.255.255` is inside `240.0.0.0/4`, and `0.0.0.0` is inside
/// `0.0.0.0/8`.
fn classify_v4(ip: Ipv4Addr) -> Option<AddressClass> {
    let [a, b, c, _] = ip.octets();
    if ip.is_loopback() {
        return Some(AddressClass::Loopback);
    }
    if ip.is_unspecified() {
        return Some(AddressClass::Unspecified);
    }
    // `0.0.0.0/8`, "this network" (RFC 1122). Not routable, and a source
    // address rather than a destination.
    if a == 0 {
        return Some(AddressClass::Reserved);
    }
    if ip.is_private() {
        return Some(AddressClass::Private);
    }
    if ip.is_link_local() {
        return Some(AddressClass::LinkLocal);
    }
    // `100.64.0.0/10` (RFC 6598).
    if a == 100 && (64..128).contains(&b) {
        return Some(AddressClass::Cgnat);
    }
    if ip.is_broadcast() {
        return Some(AddressClass::Broadcast);
    }
    if ip.is_multicast() {
        return Some(AddressClass::Multicast);
    }
    // `240.0.0.0/4`, reserved for future use (RFC 1112) — and never a
    // merchant's receiver.
    if a >= 240 {
        return Some(AddressClass::Reserved);
    }
    // IANA special-purpose blocks. None of them is a place a receiver lives,
    // and `198.18.0.0/15` in particular is routed to real hardware in some
    // lab networks. Refusing them costs nothing and removes the question.
    let special = matches!(
        (a, b, c),
        (192, 0, 0 | 2)      // 192.0.0.0/24 (IETF protocol assignments), 192.0.2.0/24 (TEST-NET-1)
            | (192, 88, 99)     // 192.88.99.0/24 (6to4 relay anycast, deprecated by RFC 7526)
            | (198, 18 | 19, _) // 198.18.0.0/15 (benchmarking)
            | (198, 51, 100)    // 198.51.100.0/24 (TEST-NET-2)
            | (203, 0, 113) // 203.0.113.0/24 (TEST-NET-3)
    );
    if special {
        return Some(AddressClass::Reserved);
    }
    None
}

/// [`classify`] for IPv6.
///
/// The shape is deliberately the other way round from [`classify_v4`]: after
/// the specific classes, anything **outside** global unicast (`2000::/3`) is
/// refused rather than allowed. IPv6's special-purpose space is large,
/// sparsely documented and still growing, and a deny-list over it would be a
/// guess about what IANA does next. A merchant's receiver has a global unicast
/// address; everything else is either a local scope, an IANA special-purpose
/// allocation or a transition mechanism, and the transition mechanisms are the
/// dangerous ones — `::ffff:10.0.0.1`,
/// `::10.0.0.1`, `2002:0a00:0001::` (6to4) and `64:ff9b::10.0.0.1` (NAT64) all
/// mean "reach this IPv4 address" to some stack somewhere.
fn classify_v6(ip: Ipv6Addr) -> Option<AddressClass> {
    if ip.is_loopback() {
        return Some(AddressClass::Loopback);
    }
    if ip.is_unspecified() {
        return Some(AddressClass::Unspecified);
    }
    // `::ffff:a.b.c.d` — the form a dual-stack socket reaches IPv4 with. The
    // embedded address is the one that matters, so it is classified as IPv4:
    // `::ffff:127.0.0.1` is loopback, and `::ffff:8.8.8.8` is public.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    // `::a.b.c.d`, the deprecated IPv4-compatible form (RFC 4291 §2.5.5.1).
    // Classified by its embedded address, and refused outright when that is
    // public: the form is deprecated, nothing legitimate uses it, and a stack
    // that does route it routes it as IPv4.
    if let Some(v4) = ip.to_ipv4() {
        return classify_v4(v4).or(Some(AddressClass::Reserved));
    }

    let segments = ip.segments();
    let first = segments[0];
    // `fe80::/10`.
    if first & 0xffc0 == 0xfe80 {
        return Some(AddressClass::LinkLocal);
    }
    // `fc00::/7`, unique local — IPv6's RFC 1918.
    if first & 0xfe00 == 0xfc00 {
        return Some(AddressClass::Private);
    }
    if ip.is_multicast() {
        return Some(AddressClass::Multicast);
    }
    // Everything outside `2000::/3` — including the deprecated site-local
    // `fec0::/10`, the discard prefix `100::/64` and the NAT64 well-known
    // prefix `64:ff9b::/96`.
    if first & 0xe000 != 0x2000 {
        return Some(AddressClass::Reserved);
    }
    // Inside global unicast, the prefixes that are not ordinary unicast:
    // `2002::/16` (6to4, which embeds an IPv4 address and is deprecated by
    // RFC 7526) and the IANA special-purpose allocations under `2001::/16`.
    if first == 0x2002 {
        return Some(AddressClass::Reserved);
    }
    if first == 0x2001 {
        let second = segments[1];
        let special = second == 0x0000        // 2001::/32, Teredo (embeds two IPv4 addresses)
            || second == 0x0001               // 2001:1::/32, IETF protocol assignments
            || (second == 0x0002 && segments[2] == 0) // 2001:2::/48, benchmarking
            || second & 0xfff0 == 0x0020      // 2001:20::/28, ORCHIDv2
            || second == 0x0db8; // 2001:db8::/32, documentation
        if special {
            return Some(AddressClass::Reserved);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{AddressClass, EgressPolicy, EgressRefusal, classify, vet};

    /// Parses an address the tests wrote out, so a typo is a test failure and
    /// not a silently different address.
    fn ip(literal: &str) -> IpAddr {
        literal.parse().expect("the test wrote a valid IP literal")
    }

    /// Every range the guard knows, both families, one row each — including
    /// the boundary addresses, because a range written `(64..128)` and a range
    /// written `(64..=128)` differ only there.
    ///
    /// A table rather than a test per range so that a *missing* range is
    /// visible as a missing row, and so the same list can be read against
    /// `docs/flows/webhooks.md`'s statement of what is refused.
    #[test]
    fn every_refused_range_is_classified_in_both_families() {
        let cases: Vec<(&str, AddressClass)> = vec![
            // --- IPv4 -------------------------------------------------
            ("127.0.0.1", AddressClass::Loopback),
            ("127.255.255.254", AddressClass::Loopback),
            ("0.0.0.0", AddressClass::Unspecified),
            ("0.1.2.3", AddressClass::Reserved),
            ("10.0.0.1", AddressClass::Private),
            ("10.255.255.255", AddressClass::Private),
            ("172.16.0.1", AddressClass::Private),
            ("172.31.255.255", AddressClass::Private),
            ("192.168.0.1", AddressClass::Private),
            ("192.168.255.255", AddressClass::Private),
            ("169.254.169.254", AddressClass::LinkLocal),
            ("169.254.0.0", AddressClass::LinkLocal),
            ("100.64.0.0", AddressClass::Cgnat),
            ("100.127.255.255", AddressClass::Cgnat),
            ("224.0.0.1", AddressClass::Multicast),
            ("239.255.255.255", AddressClass::Multicast),
            ("255.255.255.255", AddressClass::Broadcast),
            ("240.0.0.1", AddressClass::Reserved),
            ("192.0.0.1", AddressClass::Reserved),
            ("192.0.2.1", AddressClass::Reserved),
            ("198.18.0.1", AddressClass::Reserved),
            ("198.19.255.255", AddressClass::Reserved),
            ("198.51.100.1", AddressClass::Reserved),
            ("203.0.113.1", AddressClass::Reserved),
            ("192.88.99.1", AddressClass::Reserved),
            ("192.88.99.255", AddressClass::Reserved),
            // --- IPv6 -------------------------------------------------
            ("::1", AddressClass::Loopback),
            ("::", AddressClass::Unspecified),
            ("fe80::1", AddressClass::LinkLocal),
            ("febf:ffff::1", AddressClass::LinkLocal),
            ("fc00::1", AddressClass::Private),
            ("fd00::1", AddressClass::Private),
            ("fdff:ffff::1", AddressClass::Private),
            ("ff02::1", AddressClass::Multicast),
            ("ff00::", AddressClass::Multicast),
            ("fec0::1", AddressClass::Reserved),
            ("100::1", AddressClass::Reserved),
            ("64:ff9b::1.2.3.4", AddressClass::Reserved),
            ("2001::1", AddressClass::Reserved),
            ("2001:1::1", AddressClass::Reserved),
            ("2001:2::1", AddressClass::Reserved),
            ("2001:20::1", AddressClass::Reserved),
            ("2001:2f:ffff::1", AddressClass::Reserved),
            ("2001:db8::1", AddressClass::Reserved),
            ("2002:a00:1::1", AddressClass::Reserved),
            // --- IPv4-mapped and -compatible IPv6 ---------------------
            // The whole point of the mapped form: the same guard has to fire
            // on the same address written the other way, or a merchant writes
            // `https://[::ffff:169.254.169.254]/` and walks straight through.
            ("::ffff:127.0.0.1", AddressClass::Loopback),
            ("::ffff:10.0.0.1", AddressClass::Private),
            ("::ffff:169.254.169.254", AddressClass::LinkLocal),
            ("::ffff:100.64.0.1", AddressClass::Cgnat),
            ("::ffff:224.0.0.1", AddressClass::Multicast),
            ("::ffff:255.255.255.255", AddressClass::Broadcast),
            ("::ffff:0.0.0.0", AddressClass::Unspecified),
            // The deprecated compatible form, which some stacks still route.
            ("::10.0.0.1", AddressClass::Private),
            ("::8.8.8.8", AddressClass::Reserved),
        ];

        for (literal, expected) in cases {
            assert_eq!(
                classify(ip(literal)),
                Some(expected),
                "{literal} must classify as {expected:?}"
            );
        }
    }

    /// The other direction, and the one that stops the guard being "refuse
    /// everything": ordinary public addresses are not classified, so a real
    /// merchant's receiver is delivered to.
    ///
    /// Without this row the whole module could return `Some(Reserved)`
    /// unconditionally and every test above would still pass.
    #[test]
    fn ordinary_public_addresses_are_not_classified() {
        for literal in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "172.32.0.1",  // just outside 172.16/12
            "172.15.0.1",  // just below it
            "192.169.0.1", // just outside 192.168/16
            "100.63.255.255",
            "100.128.0.0", // both sides of 100.64/10
            "169.253.0.1",
            "169.255.0.1",     // both sides of 169.254/16
            "223.255.255.255", // just below multicast
            "11.0.0.1",
            "2606:4700:4700::1111",
            "2a00:1450:4001::1",
            // `2001::/16` is ordinary global unicast outside the IANA
            // special-purpose allocations at its bottom — this one is
            // Google's public resolver.
            "2001:4860:4860::8888",
            "192.88.98.255", // both sides of the 6to4 relay's 192.88.99.0/24
            "192.88.100.1",
            "::ffff:8.8.8.8",
            "2000::1",
            "3ffe::1",
        ] {
            assert_eq!(
                classify(ip(literal)),
                None,
                "{literal} is a public address and must be deliverable"
            );
        }
    }

    /// A URL whose host is a literal is refused without any resolution, in
    /// both families and in the bracketed form a URL writes IPv6 in.
    #[tokio::test]
    async fn a_private_literal_host_is_refused_by_class() {
        for (url, expected) in [
            ("http://127.0.0.1:8080/hook", AddressClass::Loopback),
            ("https://10.0.0.5/hook", AddressClass::Private),
            (
                "https://169.254.169.254/latest/meta-data/",
                AddressClass::LinkLocal,
            ),
            ("http://[::1]:9000/hook", AddressClass::Loopback),
            (
                "http://[::ffff:169.254.169.254]/hook",
                AddressClass::LinkLocal,
            ),
        ] {
            let refusal = vet(url, EgressPolicy::DENY_PRIVATE)
                .await
                .expect_err("a private literal must be refused");
            assert!(
                matches!(refusal, EgressRefusal::Blocked { class } if class == expected),
                "{url}: expected {expected:?}, got {refusal:?}"
            );
            assert!(refusal.is_permanent(), "{url}: a blocked target is final");
            assert_eq!(refusal.token(), "ssrf_blocked");
        }
    }

    /// **A name, not a literal.** The classification has to happen after
    /// resolution or a merchant simply writes the hostname of the thing they
    /// want reached — which is what `localhost` is here.
    ///
    /// `localhost` rather than a name this test would have to invent: it is
    /// the one name every machine that can run this suite resolves, and it
    /// resolves to loopback by definition (RFC 6761 §6.3). A failure here is
    /// therefore the guard, not the network.
    #[tokio::test]
    async fn a_name_that_resolves_to_loopback_is_refused() {
        let refusal = vet("http://localhost:8080/hook", EgressPolicy::DENY_PRIVATE)
            .await
            .expect_err("a name resolving to loopback must be refused");
        assert!(
            matches!(
                refusal,
                EgressRefusal::Blocked {
                    class: AddressClass::Loopback
                }
            ),
            "expected a loopback refusal, got {refusal:?}"
        );
    }

    /// The same name, the same resolution, the same classification — and a
    /// different verdict, which is the only thing the flag changes. The
    /// returned target still carries the resolved addresses, because the pin
    /// applies in a sandbox too.
    #[tokio::test]
    async fn the_sandbox_flag_permits_exactly_that_target() {
        let target = vet("http://localhost:8080/hook", EgressPolicy::ALLOW_PRIVATE)
            .await
            .expect("allow_private_targets must permit a private receiver");
        assert_eq!(target.host(), "localhost");
        assert!(
            !target.addrs().is_empty(),
            "the pin must carry the addresses the lookup returned"
        );
        assert!(
            target.addrs().iter().all(|addr| addr.ip().is_loopback()),
            "localhost resolved to something that is not loopback: {:?}",
            target.addrs()
        );
    }

    /// The three URL defects, and the fact that each is final. A scheme is
    /// the one that matters: `file:///etc/passwd` reaches no socket, but a
    /// delivery that retried it for 31 hours would say nothing useful either.
    #[tokio::test]
    async fn a_url_that_is_not_an_http_target_is_refused_permanently() {
        for url in [
            "file:///etc/passwd",
            "ftp://hooks.example/x",
            "gopher://hooks.example:70/",
        ] {
            let refusal = vet(url, EgressPolicy::ALLOW_PRIVATE)
                .await
                .expect_err("only http and https are delivered");
            assert!(matches!(refusal, EgressRefusal::Scheme { .. }), "{url}");
            assert!(refusal.is_permanent(), "{url}");
        }

        let unparseable = vet("not a url", EgressPolicy::ALLOW_PRIVATE)
            .await
            .expect_err("an unparseable url is not a target");
        assert!(matches!(unparseable, EgressRefusal::Unparseable { .. }));
        assert!(unparseable.is_permanent());
    }

    /// A host that does not resolve is **not** permanent, and this is the
    /// case that would be easiest to get wrong: refusing it forever would turn
    /// a five-minute resolver outage into a merchant's event being dropped
    /// after one attempt.
    ///
    /// `.invalid` is reserved by RFC 2606 and resolves nowhere.
    #[tokio::test]
    async fn a_host_that_does_not_resolve_is_a_transient_failure() {
        let refusal = vet("https://receiver.invalid/hook", EgressPolicy::DENY_PRIVATE)
            .await
            .expect_err("an unresolvable host cannot be delivered to");
        assert!(
            matches!(refusal, EgressRefusal::Unresolvable { .. }),
            "got {refusal:?}"
        );
        assert!(
            !refusal.is_permanent(),
            "a resolver failure must walk the ladder, not exhaust the delivery"
        );
        assert_eq!(refusal.token(), "delivery_target_unavailable");
    }

    /// The refusal an operator reads must not answer the question the request
    /// was asking. `Display` names the class; the address appears nowhere.
    #[tokio::test]
    async fn a_refusal_never_names_the_address_it_refused() {
        let refusal = vet(
            "https://169.254.169.254/latest/",
            EgressPolicy::DENY_PRIVATE,
        )
        .await
        .expect_err("the metadata service is refused");
        let rendered = format!("{refusal}");
        assert!(
            !rendered.contains("169.254"),
            "the refusal leaked the address: {rendered}"
        );
        assert!(
            rendered.contains("link_local"),
            "the refusal must name the class: {rendered}"
        );
        // The same for `Debug`, which is what a `tracing` field would print.
        let debugged = format!("{refusal:?}");
        assert!(
            !debugged.contains("169.254"),
            "the refusal's Debug leaked the address: {debugged}"
        );
    }

    /// A public target is vetted, and a pinned client is built for it — the
    /// path a real merchant's delivery takes.
    ///
    /// A literal rather than a name on purpose: this case must not depend on
    /// what the machine's resolver says about anybody's domain, and the
    /// resolving path is already proven by the two `localhost` cases above,
    /// which fail rather than skip when DNS is unavailable.
    #[tokio::test]
    async fn a_public_literal_is_vetted_and_pinned() {
        let target = vet("https://8.8.8.8/hook", EgressPolicy::DENY_PRIVATE)
            .await
            .expect("a public address is deliverable");
        assert_eq!(target.host(), "8.8.8.8");
        assert_eq!(target.addrs().len(), 1);
        assert!(super::pinned_client(&target).is_ok());
    }
}
