//! Which address the sign-in rate limiter counts, and the allow-list that
//! decides it (issue #79 item 1).
//!
//! # The address a caller can choose is not an address you can limit by
//!
//! [ADR-0017](../../../../../docs/adr/0017-staff-authentication.md) shipped
//! with the transport peer as the client address and read no forwarding
//! header at all. Its Consequences said why, and the sentence is still
//! correct: `X-Forwarded-For` is caller-supplied on an unauthenticated route,
//! so honouring it from an arbitrary peer hands an attacker **a fresh per-IP
//! bucket per request** — the limiter would then bound nothing, while
//! reporting that it did.
//!
//! It also said what that costs: "Under an Ingress the peer is the Ingress,
//! and the per-IP half then bounds the deployment rather than the caller."
//! Every staff member behind one proxy shares one budget, so ten wrong
//! passwords from anybody refuse everybody. That is the *other* denial of
//! service, and it is the one a real deployment actually meets.
//!
//! [`TrustedProxies`] is what makes both statements false at once, and it is
//! the only thing that can: the header is believed **exactly when the
//! transport peer is one of the addresses the operator named**, and never
//! otherwise.
//!
//! # The walk is from the right, and stops at the first untrusted hop
//!
//! `X-Forwarded-For` is `client, proxy₁, proxy₂, …` in the order the hops
//! appended themselves, so the **rightmost** entries are the ones this
//! deployment's own infrastructure added and the leftmost is whatever the
//! original caller claimed. [`client_address`] therefore starts at the right
//! and walks left while the entries are trusted; the first untrusted one is
//! the client, because it is the last value a trusted machine vouched for.
//!
//! Taking the **leftmost** entry instead — the shape almost every "get the
//! real IP" snippet has — is the defect this module exists to not have: it is
//! whatever the caller wrote, and a caller writes a new one per request.
//!
//! Four things fall out of the walk, each of which is a hole if it is
//! dropped:
//!
//! * **An untrusted peer ends it immediately.** The header is not read at
//!   all, not even to log it.
//! * **An unparseable hop ends it too**, at the peer. A hop that is not an
//!   address cannot be checked against the allow-list, so nothing to its left
//!   has been vouched for by anything.
//! * **All-trusted hops end it at the peer.** A header consisting only of
//!   this deployment's own proxies names no client, and inventing one from
//!   the leftmost entry would be inventing one from a value a proxy chose.
//! * **An empty or absent header ends it at the peer**, which is exactly the
//!   behaviour ADR-0017 shipped.
//!
//! # `X-Forwarded-For` and not `Forwarded`
//!
//! RFC 7239's `Forwarded: for=…` is the standardised spelling and this module
//! deliberately reads only the de-facto one. Two spellings is two parsers
//! over the same caller-supplied string, and the failure mode when they
//! disagree is not symmetric: the *more permissive* answer wins, because
//! whichever one yields an address a limiter has not seen before is the one
//! that buys a fresh bucket. A deployment whose proxy emits only `Forwarded`
//! is a deployment where this list is empty in effect, and the per-IP budget
//! is the proxy's — the behaviour ADR-0017 already documents.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use axum::http::HeaderMap;

/// The header this module reads, lower-cased as `axum` stores it.
pub const FORWARDED_FOR: &str = "x-forwarded-for";

/// A configured `staff_auth.trusted_proxies` entry that is not an address or
/// a CIDR block.
///
/// A leaf in ADR-0011's sense, and it exists rather than a `bool` because the
/// entry that failed has to reach an operator's log: an allow-list with a
/// typo in it is an allow-list that trusts one fewer machine than the
/// operator believes, and the symptom is a per-IP budget that has silently
/// become the whole deployment's again.
#[derive(Debug, thiserror::Error)]
pub enum TrustedProxyError {
    /// The entry is neither an IP address nor `address/prefix`.
    #[error(
        "staff_auth.trusted_proxies[{index}] is not an address or a CIDR block: {entry:?}. \
         Write 198.51.100.7, 2001:db8::1, 10.0.0.0/8 or fd00::/8"
    )]
    NotAnAddress {
        /// Which entry, so an operator can find it in a long list.
        index: usize,
        /// The entry as written. Not a secret: it is an address in a
        /// configuration file.
        entry: String,
    },

    /// The prefix length is out of range for the address family — `/33` on an
    /// IPv4 block, `/129` on an IPv6 one.
    ///
    /// Its own variant rather than folded into
    /// [`TrustedProxyError::NotAnAddress`] because the two have different
    /// fixes and the wrong one is the easier mistake: `10.0.0.0/16` written
    /// as `10.0.0.0/160` is a typo an operator finds in a second once told
    /// the maximum.
    #[error(
        "staff_auth.trusted_proxies[{index}] has a prefix of /{prefix}, and an {family} block \
         allows at most /{maximum}: {entry:?}"
    )]
    PrefixTooLong {
        /// Which entry.
        index: usize,
        /// The entry as written.
        entry: String,
        /// The prefix the operator wrote.
        prefix: u32,
        /// `IPv4` or `IPv6`.
        family: &'static str,
        /// 32 or 128.
        maximum: u32,
    },
}

impl vpay_core::Classify for TrustedProxyError {
    /// `Internal`, for [`crate::staff_auth::StaffAuthError`]'s reason: this
    /// is a deployment's own configuration and nothing a caller did. It is
    /// raised at boot and never inside a request.
    fn category(&self) -> vpay_core::Category {
        vpay_core::Category::Internal
    }
}

/// One entry of the allow-list: an address and how many of its leading bits
/// have to match.
///
/// A bare address is stored as a full-length prefix rather than as a second
/// variant, so [`Block::contains`] has one path and cannot be right for one
/// spelling and wrong for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    /// An IPv4 network. The address is already masked at construction.
    V4(Ipv4Addr, u32),
    /// An IPv6 network. The address is already masked at construction.
    V6(Ipv6Addr, u32),
}

impl Block {
    /// Whether `address` is inside this block.
    ///
    /// **An IPv4 address is never inside an IPv6 block and vice versa**, with
    /// no `to_ipv6_mapped` bridge in either direction. A deployment that
    /// wants `::ffff:10.0.0.1` trusted writes an IPv6 entry for it; silently
    /// mapping would mean `10.0.0.0/8` also trusted a v6 address, which is a
    /// wider allow-list than the operator wrote.
    fn contains(self, address: IpAddr) -> bool {
        match (self, address) {
            (Block::V4(network, prefix), IpAddr::V4(candidate)) => {
                mask_v4(candidate, prefix) == network
            }
            (Block::V6(network, prefix), IpAddr::V6(candidate)) => {
                mask_v6(candidate, prefix) == network
            }
            _ => false,
        }
    }
}

/// `address` with everything below `prefix` cleared.
///
/// `prefix` is `0..=32`, checked at construction. The shift is guarded
/// because `u32 << 32` is undefined-behaviour-adjacent in every language that
/// has the operator and a panic in debug Rust — and `/0` is a legal, if
/// alarming, thing for an operator to write.
fn mask_v4(address: Ipv4Addr, prefix: u32) -> Ipv4Addr {
    let bits = u32::from(address);
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX.wrapping_shl(32_u32.saturating_sub(prefix))
    };
    Ipv4Addr::from(bits & mask)
}

/// `address` with everything below `prefix` cleared. `prefix` is `0..=128`.
fn mask_v6(address: Ipv6Addr, prefix: u32) -> Ipv6Addr {
    let bits = u128::from(address);
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX.wrapping_shl(128_u32.saturating_sub(prefix))
    };
    Ipv6Addr::from(bits & mask)
}

/// The addresses whose `X-Forwarded-For` this deployment believes.
///
/// **Empty is the default and empty means "believe nobody"**, which is
/// exactly what ADR-0017 shipped. It is not a degraded mode: a deployment
/// with no reverse proxy in front of it is correctly configured with an
/// empty list, and one *with* a proxy and an empty list gets the
/// deployment-wide per-IP budget ADR-0017's Consequences already describes.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxies {
    blocks: Vec<Block>,
}

impl TrustedProxies {
    /// Parses `staff_auth.trusted_proxies`.
    ///
    /// Each entry is an address (`198.51.100.7`, `2001:db8::1`) or a CIDR
    /// block (`10.0.0.0/8`, `fd00::/8`). A block's host bits are cleared here
    /// rather than at match time, so `10.1.2.3/8` and `10.0.0.0/8` are one
    /// value: an operator who writes a host address with a prefix meant the
    /// network, and refusing them would be pedantry with a boot failure
    /// attached.
    ///
    /// # Errors
    ///
    /// [`TrustedProxyError`] for the first entry that is not one of those
    /// shapes. The first and not all of them, because the caller's response
    /// is the same either way — refuse to mount the login — and an operator
    /// fixing a list fixes it one line at a time.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::IpAddr;
    /// use vpay_api::staff::client_address::TrustedProxies;
    ///
    /// let proxies = TrustedProxies::parse(&["10.0.0.0/8".to_owned()])?;
    /// assert!(proxies.contains("10.4.5.6".parse::<IpAddr>()?));
    /// assert!(!proxies.contains("11.0.0.1".parse::<IpAddr>()?));
    ///
    /// // An IPv4 address is not inside an IPv6 block, ever.
    /// let v6 = TrustedProxies::parse(&["::/0".to_owned()])?;
    /// assert!(!v6.contains("10.4.5.6".parse::<IpAddr>()?));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(entries: &[String]) -> Result<Self, TrustedProxyError> {
        let mut blocks = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            blocks.push(parse_block(index, entry.trim())?);
        }
        Ok(Self { blocks })
    }

    /// Whether the list names nobody. `true` is the shipping default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Whether `address` is one of the machines this deployment believes.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        self.blocks.iter().any(|block| block.contains(address))
    }
}

/// One entry, as a block.
fn parse_block(index: usize, entry: &str) -> Result<Block, TrustedProxyError> {
    let not_an_address = || TrustedProxyError::NotAnAddress {
        index,
        entry: entry.to_owned(),
    };

    let (address_part, prefix_part) = match entry.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (entry, None),
    };

    let address: IpAddr = address_part.parse().map_err(|_| not_an_address())?;
    let maximum = match address {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };

    let prefix = match prefix_part {
        None => maximum,
        Some(written) => {
            let parsed: u32 = written.parse().map_err(|_| not_an_address())?;
            if parsed > maximum {
                return Err(TrustedProxyError::PrefixTooLong {
                    index,
                    entry: entry.to_owned(),
                    prefix: parsed,
                    family: if maximum == 32 { "IPv4" } else { "IPv6" },
                    maximum,
                });
            }
            parsed
        }
    };

    Ok(match address {
        IpAddr::V4(v4) => Block::V4(mask_v4(v4, prefix), prefix),
        IpAddr::V6(v6) => Block::V6(mask_v6(v6, prefix), prefix),
    })
}

/// The address the rate limiter counts this request under.
///
/// `peer` is the transport peer (`axum`'s `ConnectInfo`) and `forwarded_for`
/// is the raw `X-Forwarded-For` value, if the caller sent one. The result is
/// `None` only when the peer is `None` — which
/// [`crate::staff::rate_limit`] counts under one shared key rather than
/// exempting, and which cannot happen at all once the service is built with
/// `into_make_service_with_connect_info`.
///
/// The rules are the module header's, in order. The decisive one, and the
/// mutation that proves this function is wired up, is the first: **delete the
/// `proxies.contains(peer)` test and a spoofed header buys a fresh bucket per
/// request**, which
/// `a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget` reads
/// as a `401` where it demands a `429`.
///
/// # Examples
///
/// ```
/// use std::net::IpAddr;
/// use vpay_api::staff::client_address::{client_address, TrustedProxies};
///
/// let peer: IpAddr = "10.0.0.9".parse()?;
/// let claimed = Some("203.0.113.7");
///
/// // Nobody is trusted by default, so the header is not read.
/// let none = TrustedProxies::default();
/// assert_eq!(client_address(&none, Some(peer), claimed), Some(peer));
///
/// // The peer is a named proxy, so the first untrusted hop is the client.
/// let trusted = TrustedProxies::parse(&["10.0.0.0/8".to_owned()])?;
/// assert_eq!(
///     client_address(&trusted, Some(peer), claimed),
///     Some("203.0.113.7".parse::<IpAddr>()?),
/// );
///
/// // A header of nothing but this deployment's own proxies names no client.
/// assert_eq!(
///     client_address(&trusted, Some(peer), Some("10.1.1.1, 10.2.2.2")),
///     Some(peer),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn client_address(
    proxies: &TrustedProxies,
    peer: Option<IpAddr>,
    forwarded_for: Option<&str>,
) -> Option<IpAddr> {
    // The two arms say what an empty slice says: an absent header is no hops,
    // and no hops ends the walk at the peer.
    match forwarded_for {
        Some(header) => client_address_over(proxies, peer, &[header]),
        None => client_address_over(proxies, peer, &[]),
    }
}

/// [`client_address`], over a request's `X-Forwarded-For` **field lines**
/// rather than over one of them.
///
/// # Why a slice, and not the single value `HeaderMap::get` answers
///
/// A field may be repeated, and RFC 9110 s5.2 says a recipient reads the
/// repetitions as one comma-separated list **in the order received**.
/// `HeaderMap::get` answers the *first* value only. So a deployment whose
/// proxy appends its hop as a **new** `X-Forwarded-For` line — rather than
/// rewriting the existing one, which is a per-proxy configuration difference
/// and not a rare one — would have had this module read the caller's own line
/// and nothing else. The walk would then start at the right-hand end of a
/// string the caller wrote in full, the "first untrusted hop" would be
/// whatever they put there, and **a trusted-proxy deployment would hand every
/// caller a fresh rate-limit bucket per request** — the exact hole the
/// allow-list exists to close, reopened one function lower down.
///
/// `forwarded_for` is therefore every line, in the order received, and the
/// walk runs right to left **across** them: the last hop of the last line is
/// the nearest one, because it is what the most downstream proxy appended.
///
/// `a_repeated_header_is_one_chain_and_the_callers_line_is_not_the_end` is the
/// case; the mutation is reading only the first line again.
#[must_use]
pub fn client_address_over(
    proxies: &TrustedProxies,
    peer: Option<IpAddr>,
    forwarded_for: &[&str],
) -> Option<IpAddr> {
    let peer = peer?;

    // THE CHECK. Without it every branch below reads a value the caller
    // wrote, and the limiter counts a key the caller chose.
    if !proxies.contains(peer) {
        return Some(peer);
    }

    // Right to left, across the lines and then within each one. An empty
    // slice is an absent header and falls out of the loop at the peer — which
    // is deliberate, and is why there is no `?` on a header value anywhere
    // here. The `?` spelling compiles, reads identically, and is wrong: it
    // answers `None` *from this function*, which is "no address at all"
    // rather than "the peer", and every trusted-proxy deployment would then
    // count every request with no `X-Forwarded-For` under the limiter's one
    // shared unknown-address key.
    // `an_all_trusted_header_falls_back_to_the_peer` is what found it.
    for value in forwarded_for.iter().rev() {
        for hop in value.rsplit(',') {
            let Some(address) = parse_hop(hop.trim()) else {
                // A hop that is not an address vouches for nothing to its
                // left — including nothing on any earlier line.
                return Some(peer);
            };
            if !proxies.contains(address) {
                return Some(address);
            }
        }
    }

    // Every hop was one of ours, or there were none.
    Some(peer)
}

/// One `X-Forwarded-For` element.
///
/// A bare address, or `address:port` / `[address]:port` — proxies that log a
/// port are common enough (and `SocketAddr` parses both bracketed forms) that
/// refusing them would silently fall back to the peer for a whole class of
/// correctly-configured deployment. Anything else is `None`, and `None` ends
/// the walk.
fn parse_hop(hop: &str) -> Option<IpAddr> {
    if let Ok(address) = hop.parse::<IpAddr>() {
        return Some(address);
    }
    hop.parse::<SocketAddr>().ok().map(|socket| socket.ip())
}

/// [`client_address_over`], reading the header out of a request's own map.
///
/// A separate function so the decision above is a pure one over strings and
/// therefore a unit test rather than something only a socket can exercise —
/// the split `crate::staff_auth`'s modules already use.
///
/// **`get_all` and not `get`.** `get` answers the first field line only, and
/// a proxy that appends its hop as a new `X-Forwarded-For` line rather than
/// rewriting the existing one would then have this module walk the *caller's*
/// line and stop — see [`client_address_over`] for what that costs.
///
/// A non-ASCII value ends the whole thing at the peer, for
/// [`crate::staff::session_token`]'s reason and one more: the value is a list
/// of addresses by construction, so anything else was not written by a proxy
/// this deployment would believe — and skipping it to keep walking would mean
/// stepping *past* an unreadable hop into a line further from the peer, which
/// is the one direction this module never goes.
#[must_use]
pub fn client_address_from(
    proxies: &TrustedProxies,
    peer: Option<IpAddr>,
    headers: &HeaderMap,
) -> Option<IpAddr> {
    let mut lines = Vec::new();
    for value in headers.get_all(FORWARDED_FOR) {
        match value.to_str() {
            Ok(text) => lines.push(text),
            Err(_) => return peer,
        }
    }
    client_address_over(proxies, peer, &lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("a literal address in a test")
    }

    fn trusted(entries: &[&str]) -> TrustedProxies {
        TrustedProxies::parse(
            &entries
                .iter()
                .map(|entry| (*entry).to_owned())
                .collect::<Vec<_>>(),
        )
        .expect("a literal allow-list in a test")
    }

    /// **The whole point, in one assertion pair**: the same request, the same
    /// header, two allow-lists, two answers.
    ///
    /// ADR-0017's Consequences says the header must be honoured only behind
    /// an "authenticated trusted-proxy list". This is what "only" means.
    #[test]
    fn the_header_is_read_from_a_trusted_peer_and_from_nobody_else() {
        let peer = ip("10.0.0.9");
        let claimed = Some("203.0.113.7");

        assert_eq!(
            client_address(&TrustedProxies::default(), Some(peer), claimed),
            Some(peer),
            "an empty allow-list must read no header at all: this is what shipped, and a \
             regression here is a fresh rate-limit bucket per request"
        );
        assert_eq!(
            client_address(&trusted(&["10.0.0.9"]), Some(peer), claimed),
            Some(ip("203.0.113.7")),
        );
    }

    /// The walk is from the RIGHT. The leftmost entry is what the caller
    /// wrote and is never the answer when a proxy appended after it.
    #[test]
    fn the_client_is_the_first_untrusted_hop_from_the_right() {
        let proxies = trusted(&["10.0.0.0/8"]);
        // client, an untrusted intermediary, then two of ours.
        let header = Some("198.51.100.1, 203.0.113.5, 10.1.1.1, 10.2.2.2");

        assert_eq!(
            client_address(&proxies, Some(ip("10.0.0.9")), header),
            Some(ip("203.0.113.5")),
            "the last address a trusted machine vouched for, not the first one the caller \
             wrote: taking the leftmost entry is the defect this module exists to not have"
        );
    }

    /// A hop that is not an address ends the walk at the peer. Nothing to its
    /// left has been vouched for by anything that can be checked.
    #[test]
    fn an_unparseable_hop_falls_back_to_the_peer() {
        let proxies = trusted(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.9");

        assert_eq!(
            client_address(&proxies, Some(peer), Some("203.0.113.5, unknown, 10.1.1.1")),
            Some(peer),
            "`unknown` is a legal X-Forwarded-For element and names no address; treating what \
             is left of it as the client would be trusting an unchecked value"
        );
    }

    /// A header naming only this deployment's own proxies names no client.
    #[test]
    fn an_all_trusted_header_falls_back_to_the_peer() {
        let proxies = trusted(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.9");

        assert_eq!(
            client_address(&proxies, Some(peer), Some("10.1.1.1, 10.2.2.2")),
            Some(peer),
        );
        assert_eq!(client_address(&proxies, Some(peer), Some("")), Some(peer));
        assert_eq!(client_address(&proxies, Some(peer), None), Some(peer));
    }

    /// A proxy that logs a port is still a proxy.
    #[test]
    fn a_hop_may_carry_a_port() {
        let proxies = trusted(&["10.0.0.0/8"]);

        assert_eq!(
            client_address(&proxies, Some(ip("10.0.0.9")), Some("203.0.113.5:41234")),
            Some(ip("203.0.113.5")),
        );
        assert_eq!(
            client_address(&proxies, Some(ip("10.0.0.9")), Some("[2001:db8::5]:443")),
            Some(ip("2001:db8::5")),
        );
    }

    /// Blocks match on the leading bits and on nothing else, and the two
    /// address families never cross.
    #[test]
    fn a_block_matches_its_own_family_and_its_own_bits() {
        let proxies = trusted(&["10.0.0.0/8", "192.168.1.0/24", "fd00::/8", "2001:db8::1"]);

        assert!(proxies.contains(ip("10.255.255.255")));
        assert!(!proxies.contains(ip("11.0.0.1")));
        assert!(proxies.contains(ip("192.168.1.7")));
        assert!(!proxies.contains(ip("192.168.2.7")), "the /24 is a /24");
        assert!(proxies.contains(ip("fd12::1")));
        assert!(
            proxies.contains(ip("2001:db8::1")),
            "a bare address is a /128"
        );
        assert!(!proxies.contains(ip("2001:db8::2")));

        // `::ffff:10.0.0.1` is an IPv4-mapped IPv6 address and is NOT inside
        // the IPv4 block: mapping would widen every entry an operator wrote.
        assert!(!proxies.contains(ip("::ffff:10.0.0.1")));
    }

    /// `/0` is legal, alarming, and does what it says. Pinned because the
    /// mask arithmetic special-cases it, and a wrong special case would be a
    /// silent `contains(_) == false` for the widest entry there is.
    #[test]
    fn a_zero_prefix_matches_its_whole_family() {
        let v4 = trusted(&["0.0.0.0/0"]);
        assert!(v4.contains(ip("198.51.100.7")));
        assert!(!v4.contains(ip("2001:db8::1")));

        let v6 = trusted(&["::/0"]);
        assert!(v6.contains(ip("2001:db8::1")));
        assert!(!v6.contains(ip("198.51.100.7")));
    }

    /// Host bits below the prefix are cleared, so a host address written with
    /// a prefix is the network the operator meant.
    #[test]
    fn a_host_address_with_a_prefix_is_the_network() {
        assert_eq!(
            parse_block(0, "10.1.2.3/8").expect("a legal block"),
            parse_block(0, "10.0.0.0/8").expect("a legal block"),
        );
    }

    /// A typo is a boot failure that names the entry, not a silently narrower
    /// list.
    #[test]
    fn an_unparseable_entry_names_itself() {
        let error = TrustedProxies::parse(&["10.0.0.0/8".to_owned(), "not-an-ip".to_owned()])
            .expect_err("`not-an-ip` is not an address");
        let rendered = error.to_string();
        assert!(rendered.contains("trusted_proxies[1]"), "{rendered}");
        assert!(rendered.contains("not-an-ip"), "{rendered}");

        let error = TrustedProxies::parse(&["10.0.0.0/33".to_owned()])
            .expect_err("/33 is not an IPv4 prefix");
        let rendered = error.to_string();
        assert!(rendered.contains("IPv4"), "{rendered}");
        assert!(rendered.contains("/32"), "the maximum is named: {rendered}");
    }

    /// **A repeated `X-Forwarded-For` is ONE chain, and the caller's line is
    /// not the end of it.**
    ///
    /// The shape this pins is a real deployment and not a curiosity: a proxy
    /// that appends its hop as a *new* field line, in front of a caller who
    /// sent one of their own. RFC 9110 s5.2 says those are one list in the
    /// order received, so the nearest hop is the last hop of the LAST line.
    ///
    /// **The mutation is `headers.get(..)` in place of `get_all`**, which is
    /// what this module did until the exp36 review: the walk then starts at
    /// the right-hand end of a string the caller wrote in full, the "first
    /// untrusted hop" is whatever they put there, and every request buys a
    /// fresh rate-limit bucket from a trusted-proxy deployment — the hole the
    /// allow-list exists to close.
    #[test]
    fn a_repeated_header_is_one_chain_and_the_callers_line_is_not_the_end() {
        let proxies = trusted(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.9");

        // The caller wrote line one. The proxy appended line two.
        let mut headers = HeaderMap::new();
        headers.append(FORWARDED_FOR, "203.0.113.7".parse().expect("a header"));
        headers.append(FORWARDED_FOR, "10.1.1.1".parse().expect("a header"));
        assert_eq!(
            client_address_from(&proxies, Some(peer), &headers),
            Some(ip("203.0.113.7")),
            "the two lines are one chain: the proxy's hop is trusted, so the client is the \
             caller's entry, which is where a single-line header would also have landed"
        );

        // And the case that separates the two readings. The caller writes a
        // whole fake chain ending in an address they chose; the proxy appends
        // the address it actually saw, which is untrusted.
        let mut spoofed = HeaderMap::new();
        spoofed.append(
            FORWARDED_FOR,
            "1.1.1.1, 198.51.100.99".parse().expect("a header"),
        );
        spoofed.append(FORWARDED_FOR, "203.0.113.7".parse().expect("a header"));
        assert_eq!(
            client_address_from(&proxies, Some(peer), &spoofed),
            Some(ip("203.0.113.7")),
            "the nearest hop is the last hop of the LAST line — the address the proxy vouched \
             for. Reading only the first line answers 198.51.100.99, which the caller chose, \
             and a caller chooses a new one per request"
        );
    }

    /// A value that is not ASCII ends the walk at the peer rather than being
    /// stepped over.
    ///
    /// Skipping it would mean walking *past* an unreadable hop into a line
    /// further from the peer — trusting what is on the far side of something
    /// this deployment could not check, which is the one direction the walk
    /// never goes.
    #[test]
    fn an_unreadable_line_ends_the_walk_at_the_peer() {
        let proxies = trusted(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.9");

        let mut headers = HeaderMap::new();
        headers.append(FORWARDED_FOR, "203.0.113.7".parse().expect("a header"));
        headers.append(
            FORWARDED_FOR,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).expect("a non-ASCII header value"),
        );
        assert_eq!(
            client_address_from(&proxies, Some(peer), &headers),
            Some(peer)
        );
    }

    /// `client_address_from` on a map with no header at all is the peer, and
    /// on an untrusted peer it reads nothing — the two ends of the map-bound
    /// half, which no unit test covered before the exp36 review.
    #[test]
    fn the_map_bound_half_agrees_with_the_pure_one() {
        let proxies = trusted(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.9");

        assert_eq!(
            client_address_from(&proxies, Some(peer), &HeaderMap::new()),
            Some(peer),
            "no header is the peer"
        );

        let mut headers = HeaderMap::new();
        headers.append(FORWARDED_FOR, "203.0.113.7".parse().expect("a header"));
        let stranger = ip("198.51.100.4");
        assert_eq!(
            client_address_from(&proxies, Some(stranger), &headers),
            Some(stranger),
            "an untrusted peer reads no header, through this entry point too"
        );
    }

    /// No peer is no address, whatever the header says. The limiter counts
    /// that under one shared key — see its own header — rather than exempting
    /// it, and a header must not be able to turn "no peer" into a key of the
    /// caller's choosing.
    #[test]
    fn no_peer_reads_no_header() {
        let proxies = trusted(&["0.0.0.0/0"]);
        assert_eq!(client_address(&proxies, None, Some("203.0.113.7")), None);
    }
}
