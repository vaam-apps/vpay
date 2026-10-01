//! Protected diagnostic representations — the four shared wrappers every
//! hand-written `Debug` in this workspace composes (RFC-0002 PR 3, ADR-0020
//! § 2, issue [#147](https://github.com/vaam-apps/vpay/issues/147)).
//!
//! A `#[derive(Debug)]` on a struct that holds a payer's reference, a rail's
//! own words for a failure, a rendered API body or a credential prints all
//! of it, and a row is `{:?}`-ed in more places than anyone tracks: a
//! `tracing` field on a settlement, a `Result` unwrapped in a test failure
//! message, a `#[derive(Debug)]` on some future struct that happens to hold
//! one. ADR-0020 § 2 decides the shape: a value crosses a telemetry boundary
//! only through an explicit positive projection, never by serialising the
//! internal row and subtracting a list of known fields afterwards.
//!
//! These four types are that projection's vocabulary for `Debug`. They are
//! **borrowed printing adapters, not storage types**: each wraps a reference
//! for the duration of one `{:?}` and owns nothing. A field stored as a raw
//! `String` stays a raw `String` — the row types keep their shapes, their
//! `sqlx::FromRow` derives and their `PartialEq`, and their hand-written
//! `Debug` impls compose these wrappers per protected field.
//!
//! Nothing in this module decides **policy**. Which identifier may appear
//! masked — and where, and whether a digest may be unkeyed — is open decision
//! D3 of [RFC-0002](../../../../docs/rfc/0002-gdpr-policy-and-operator-decisions.md),
//! and the shape each wrapper prints is an engineering representation, not
//! an approved disclosure. `Masked` and `Pseudonymous` have **no shipping
//! consumer on the day they land**: they are built now so PR 4's telemetry
//! work and #56's audit rows inherit one representation instead of four
//! bespoke ones, and their first real caller must come after D3 names its
//! keying and masking answers. ADOPTING EITHER FOR A LIVE SURFACE BEFORE D3
//! IS ANSWERED IS THE MISTAKE THIS MODULE EXISTS TO PREVENT.
//!
//! Which types must carry a hand-written `Debug` at all is not a convention
//! left to memory: each one is registered in
//! [`schemas/privacy-inventory.yaml`](../../../../schemas/privacy-inventory.yaml)'s
//! `debug_protections` list, and `cargo xtask verify-privacy-inventory`
//! fails the build if a registered type derives `Debug` or no longer exists —
//! in both directions, exactly like the rest of that gate.
//!
//! [docs/reference/personal-data-inventory.md](../../../../docs/reference/personal-data-inventory.md)
//! is the human-facing inventory; [ADR-0020](../../../../docs/adr/0020-privacy-controls-and-evidence.md)
//! is the architecture.

use std::fmt;

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// One value, hidden entirely, for `Debug`.
///
/// The choice this type is: nothing of the value prints — not its length,
/// not its shape, not `Some`/`None` beyond what the caller's own
/// `Option`-wrapping shows. That is the only representation that cannot
/// leak by being *almost* right, so it is the default one. Where an
/// operator legitimately needs a shape fact ("is this row's suffix the
/// right length?"), the call site states it itself with `format_args!` —
/// `vpay-db`'s `PaymentIntentRow` prints `[N chars redacted]` for its
/// `client_secret_suffix`, and that impl is the precedent — rather than
/// every secret printing its length because one of them once needed it
/// checked.
///
/// For a field that is `Option<T>`, wrap the `Some` half so the
/// `Some`/`None` distinction — a fact about the row, not the payer — stays
/// visible: `self.payer_ref.as_ref().map(Secret::new)`.
///
/// # Examples
///
/// ```
/// use vpay_core::privacy::Secret;
///
/// let reference = "a value a payer could be named by";
/// // The value never appears, and the output names what happened to it.
/// assert_eq!(format!("{:?}", Secret::new(&reference)), "[redacted]");
/// // An Option keeps its shape, because the shape is the row's fact.
/// let absent: Option<String> = None;
/// assert_eq!(format!("{:?}", absent.as_ref().map(Secret::new)), "None");
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Secret<'a, T: ?Sized>(&'a T);

impl<'a, T: ?Sized> Secret<'a, T> {
    /// Borrows one value for the duration of a `{:?}`.
    #[must_use]
    pub const fn new(value: &'a T) -> Self {
        Self(value)
    }
}

impl<T: ?Sized> fmt::Debug for Secret<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// One value, reduced to a fixed mechanical mask, for `Debug`.
///
/// The rule is deliberately mechanical so that it can be pinned by test and
/// reasoned about without seeing the value: a value of **eight characters or
/// more keeps its last four**, everything else replaced by four asterisks; a
/// shorter value hides entirely, because four visible characters of a
/// six-character secret is most of it. The rule is the *representation*.
/// **Whether any identifier may appear masked in any live surface — and
/// which — is open decision D3 of RFC-0002, and nothing ships this type
/// today.** A low-entropy identifier (a payer's number has an enumerable
/// input space) may be re-identifiable from four characters even without
/// the rest, which is exactly the question D3 exists to answer before the
/// first real caller exists.
///
/// # Examples
///
/// ```
/// use vpay_core::privacy::Masked;
///
/// // Eight or more: the last four characters, nothing else.
/// assert_eq!(format!("{:?}", Masked::new("237600000789")), "Masked(****0789)");
/// // Fewer than eight: entirely hidden — half a short value is most of it.
/// assert_eq!(format!("{:?}", Masked::new("0789")), "Masked([redacted])");
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Masked<'a>(&'a str);

impl<'a> Masked<'a> {
    /// Borrows one string for the duration of a `{:?}`.
    #[must_use]
    pub const fn new(value: &'a str) -> Self {
        Self(value)
    }
}

impl fmt::Debug for Masked<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chars = self.0.chars().count();
        if chars < 8 {
            return write!(f, "Masked([redacted])");
        }
        let tail = self.0.chars().rev().take(4).collect::<Vec<_>>();
        let tail: String = tail.into_iter().rev().collect();
        write!(f, "Masked(****{tail})")
    }
}

/// One value, replaced by a domain-separated keyed digest, for `Debug`.
///
/// Pseudonymisation here is **keyed by construction**: an unkeyed digest of
/// a low-entropy identifier is reversible by dictionary enumeration no
/// matter how the domain is separated, so the type takes its key from the
/// caller and has no unkeyed path. Where the key lives, how it rotates and
/// who may hold one is open decision D3 of RFC-0002; **nothing ships this
/// type today**, and its first real caller must arrive after D3 names its
/// key custody. The `domain` parameter exists so that one key cannot make
/// two different surfaces agree on a digest by accident — correlation
/// across surfaces must be a decision, not a default.
///
/// # Examples
///
/// ```
/// use vpay_core::privacy::Pseudonymous;
///
/// let digest = Pseudonymous::new("a payer's number", "account-holder-audit", b"audit-key");
/// let rendered = format!("{digest:?}");
/// // The value is gone; a stable, domain-bound digest stands in for it.
/// assert!(rendered.starts_with("Pseudonymous("));
/// assert!(!rendered.contains("payer"));
/// // A different domain disagrees, by construction.
/// let other = Pseudonymous::new("a payer's number", "incident-evidence", b"audit-key");
/// assert_ne!(format!("{other:?}"), rendered);
/// ```
#[derive(Clone, Copy)]
pub struct Pseudonymous<'a> {
    value: &'a str,
    domain: &'a str,
    key: &'a [u8],
}

impl<'a> Pseudonymous<'a> {
    /// Borrows one value, one domain label and one key for the duration of
    /// a `{:?}`.
    #[must_use]
    pub const fn new(value: &'a str, domain: &'a str, key: &'a [u8]) -> Self {
        Self { value, domain, key }
    }
}

impl fmt::Debug for Pseudonymous<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut mac = match <Hmac<Sha256> as Mac>::new_from_slice(self.key) {
            Ok(mac) => mac,
            // SHA-256's HMAC accepts every key length, so this arm is
            // unreachable in practice and exists only because the `hmac`
            // crate's constructor returns a `Result` for generality. If it
            // ever fires, the honest output is a fixed token — never the
            // value the caller was trying to hide.
            Err(_) => return f.write_str("Pseudonymous([key rejected])"),
        };
        mac.update(self.domain.as_bytes());
        mac.update(self.value.as_bytes());
        let digest = mac.finalize().into_bytes();
        write!(f, "Pseudonymous({})", hex::encode(digest))
    }
}

/// One URL, reduced to its `scheme://host[:port]`, for `Debug`.
///
/// A URL's query string and fragment can carry a receiver's credential
/// (open decision D4 of RFC-0002 exists because a configured webhook
/// endpoint may hold one), and a path can hold a token a rail issued. The
/// one part of a URL that is both safe to print and useful in a log —
/// "which host was this sent to?" — is the authority, so that is all that
/// prints. A URL that fails to parse prints a fixed token rather than the
/// raw text: half a URL is not a smaller fact, and unparsed junk is exactly
/// the input most likely to be carrying something.
///
/// # Examples
///
/// ```
/// use vpay_core::privacy::SafeUrl;
///
/// let endpoint = "https://merchant.example/hooks?vpay_token=secref01";
/// // The host is useful in a log; the query is where a credential hides.
/// assert_eq!(
///     format!("{:?}", SafeUrl::new(endpoint)),
///     "SafeUrl(https://merchant.example)",
/// );
/// // Unparseable text prints a fixed token, never itself.
/// assert_eq!(format!("{:?}", SafeUrl::new("not a url")), "SafeUrl([unparsed])");
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SafeUrl<'a>(&'a str);

impl<'a> SafeUrl<'a> {
    /// Borrows one URL for the duration of a `{:?}`.
    #[must_use]
    pub const fn new(value: &'a str) -> Self {
        Self(value)
    }
}

impl fmt::Debug for SafeUrl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Ok(parsed) = url::Url::parse(self.0) else {
            return f.write_str("SafeUrl([unparsed])");
        };
        let Some(host) = parsed.host() else {
            // A URL with no host (`mailto:`, `data:`) has no authority worth
            // printing, and its scheme alone can still name a person — a
            // `mailto:` is an email address wearing a scheme.
            return f.write_str("SafeUrl([no host])");
        };
        // An IPv6 literal prints with its brackets, because the authority is
        // what an operator reads and an unbracketed `::1:8443` is not the
        // host that was configured. `Host`'s `Display` already does the
        // bracketing; `host_str` would not.
        match parsed.port() {
            Some(port) => write!(f, "SafeUrl({}://{host}:{port})", parsed.scheme()),
            None => write!(f, "SafeUrl({}://{host})", parsed.scheme()),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Every wrapper's output is pinned, in both directions: the value's
    //! characters never appear, and the fixed shape always does — a
    //! `Debug` that printed nothing at all would pass every substring
    //! search and tell an operator nothing, exactly as
    //! `no_customer_type_ever_prints_a_payers_identifiers_street_or_gps_point`
    //! argues for the customer rows.

    use super::{Masked, Pseudonymous, SafeUrl, Secret};

    /// The canary every negative assertion in this module keys on: unique,
    /// pronounceable, and nothing any fixture would accidentally contain.
    const CANARY: &str = "canary-xiphoid-ferrous-9";

    #[test]
    fn secret_prints_a_fixed_token_and_never_the_value() {
        let printed = format!("{:?}", Secret::new(&CANARY));
        assert_eq!(printed, "[redacted]");
        assert!(!printed.contains(CANARY));
    }

    #[test]
    fn secret_hides_types_that_have_no_printable_form() {
        // A borrowed adapter over an opaque type must still print the one
        // fixed token, not a derive of the inner value.
        let bytes = CANARY.as_bytes();
        assert_eq!(format!("{:?}", Secret::new(bytes)), "[redacted]");
    }

    #[test]
    fn masked_keeps_only_the_last_four_of_a_long_value() {
        let printed = format!("{:?}", Masked::new("237600000789"));
        assert_eq!(printed, "Masked(****0789)");
        assert!(!printed.contains("2376000"));
    }

    #[test]
    fn masked_hides_a_short_value_entirely() {
        // Four visible characters of a six-character value is most of it.
        for short in ["0789", "07", ""] {
            assert_eq!(format!("{:?}", Masked::new(short)), "Masked([redacted])");
        }
    }

    #[test]
    fn masked_counts_characters_not_bytes() {
        // A multi-byte character is one character to a payer and must not
        // split into half a character because the rule counted bytes.
        let printed = format!("{:?}", Masked::new("déjà-vu-password"));
        assert_eq!(printed, "Masked(****word)");
    }

    #[test]
    fn pseudonymous_prints_a_stable_domain_bound_digest() {
        let key: &[u8] = b"test-key-material";
        let first = format!("{:?}", Pseudonymous::new(CANARY, "audit", key));
        let second = format!("{:?}", Pseudonymous::new(CANARY, "audit", key));
        assert_eq!(first, second);
        assert!(first.starts_with("Pseudonymous("));
        assert!(!first.contains(CANARY));
    }

    #[test]
    fn pseudonymous_changes_with_domain_and_with_key() {
        let one = format!("{:?}", Pseudonymous::new(CANARY, "audit", b"key-one"));
        let other_domain = format!("{:?}", Pseudonymous::new(CANARY, "incident", b"key-one"));
        let other_key = format!("{:?}", Pseudonymous::new(CANARY, "audit", b"key-two"));
        assert_ne!(one, other_domain);
        assert_ne!(one, other_key);
    }

    #[test]
    fn safe_url_prints_scheme_host_and_port_and_nothing_else() {
        let printed = format!(
            "{:?}",
            SafeUrl::new("https://hooks.example:8443/path?token=secref01#frag")
        );
        assert_eq!(printed, "SafeUrl(https://hooks.example:8443)");
        assert!(!printed.contains("token"));
        assert!(!printed.contains("path"));
    }

    #[test]
    fn safe_url_hides_userinfo_and_defaults_its_port() {
        // Userinfo in a URL is a credential wearing a delimiter.
        let userinfo = format!("{:?}", SafeUrl::new("https://user:pass@hooks.example/h"));
        assert_eq!(userinfo, "SafeUrl(https://hooks.example)");
        let default_port = format!("{:?}", SafeUrl::new("https://hooks.example:443/h"));
        assert_eq!(default_port, "SafeUrl(https://hooks.example)");
    }

    #[test]
    fn safe_url_brackets_an_ipv6_host() {
        // An IPv6 literal is an authority and must print as one — a
        // bracketless `::1:8443` is not the host that was configured.
        let printed = format!("{:?}", SafeUrl::new("http://[::1]:8080/h"));
        assert_eq!(printed, "SafeUrl(http://[::1]:8080)");
    }

    #[test]
    fn safe_url_refuses_to_print_an_unparseable_or_hostless_url() {
        assert_eq!(format!("{:?}", SafeUrl::new(CANARY)), "SafeUrl([unparsed])");
        assert_eq!(
            format!("{:?}", SafeUrl::new("mailto:someone@example.com")),
            "SafeUrl([no host])"
        );
    }
}
