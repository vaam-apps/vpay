//! The outbound HTTP client every server-side caller in this workspace
//! should use, and the reason one has to exist at all.
//!
//! # Why this module lives in the *port* crate
//!
//! It was `vpay_api::http_client` until Step 3 (`docs/plans/2026-09-03-step3-rails.md`,
//! decision 2) and moved here verbatim when the rail adapters needed it: an
//! adapter crate depends on `vpay-provider` and must not depend on
//! `vpay-api` (the HTTP surface depends on the port, never the reverse), so
//! the only home both an adapter and `vpay-api` can reach is this crate.
//! `vpay_api::http_client` is now a re-export of this module, which is why
//! no existing call site changed.
//!
//! The cost, stated plainly rather than hidden: `vpay-provider` is no longer
//! a pure interface crate — it links reqwest, rustls and webpki-roots, so a
//! future non-HTTP rail (a USSD gateway, a file drop) compiles a TLS stack it
//! never uses. No *binary* grew: both already resolved all three. The
//! alternative, a new `vpay-http` crate, was rejected for the workspace
//! member, `deny.toml` entry and second `sdks/rust` twin note it would add.
//!
//! # Why not `reqwest::Client::new()`
//!
//! The runtime image is `FROM scratch` ([ADR-0004]): no glibc, no shell, and
//! no OS certificate store. reqwest is pinned at 0.13 with
//! `rustls-no-provider` (see the long note on the pin in the root
//! `Cargo.toml`), and on that version reqwest no longer offers a
//! vendored-roots feature — it builds a `rustls_platform_verifier::Verifier`,
//! i.e. it reads the *platform* trust store. It does so **eagerly, inside
//! `ClientBuilder::build()`**, not lazily at connect time, and when the store
//! turns up empty the verifier returns
//! `General("No CA certificates were loaded from the system")`, which
//! `Client::new()` converts into a panic.
//!
//! That is not a hypothetical. `JwtValidator::new` used to call
//! `authkestra_resource::jwt::JwksCache::new`, which calls
//! `reqwest::Client::new()`, and `vpay-server` panicked at boot inside its own
//! image while passing every test on machines that happen to have `/etc/ssl`.
//! The JWKS URL it was about to fetch is plain `http://` over loopback — TLS
//! was never going to be negotiated — so the failure had nothing to do with
//! the request being made and everything to do with when the trust store is
//! read. `tests/cli.rs`'s
//! `a_server_with_no_os_trust_store_boots_and_still_validates_tokens`
//! reproduces the condition with `SSL_CERT_FILE`/`SSL_CERT_DIR` pointed at
//! paths that do not exist.
//!
//! [`client`] therefore hands reqwest a finished [`rustls::ClientConfig`]
//! built from Mozilla's vendored bundle. That takes reqwest's
//! `TlsBackend::BuiltRustls` branch, which consults neither the platform
//! verifier nor the process-wide `CryptoProvider` — so it also cannot hit the
//! *other* panic the `rustls-no-provider` pin exposes ("No rustls crypto
//! provider is configured"), independently of whether the binary installed a
//! default provider first.
//!
//! # The trade-off, stated plainly
//!
//! Vendored roots mean a deployment behind a TLS-intercepting proxy with a
//! private CA is not served by this client, and `SSL_CERT_FILE` will not
//! change that. That is the deliberate cost of being able to run in a
//! `scratch` image at all; the alternative is an image that carries a trust
//! store, which is a different ADR.
//!
//! # The twin in `sdks/rust`
//!
//! `sdks/rust/src/client.rs` has a near-identical `rustls_client_config`, and
//! it stays a separate copy on purpose: `vpay-sdk` is what a *merchant*
//! compiles into their own process, so it must not depend on a server crate —
//! making it depend on a server crate would drag axum, sqlx and the whole OP
//! into a merchant's build. The two are expected to stay in step; if you change
//! the provider, the root source or the ALPN list here, change it there too.
//! The SDK's copy carries the extra constraint that a library inside someone
//! else's process may neither panic nor install a process-wide
//! `CryptoProvider` on that process's behalf.
//!
//! The two copies now differ in one place, deliberately: this one refuses
//! redirects and ignores `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`, and the
//! SDK's does neither. A merchant's process runs on a merchant's network,
//! where a corporate egress proxy is ordinary and a redirect to a caller's
//! own gateway may be exactly what they configured; a payment gateway's own
//! egress is not that. See `preconfigured_builder`'s doc comment. Everything
//! else — the provider, the root source, the ALPN list — is still expected
//! to stay in step.
//!
//! [ADR-0004]: ../../../docs/adr/0004-musl-mimalloc.md

use std::sync::Arc;
use std::time::Duration;

use vpay_core::error::{Category, Classify};

/// Why an outbound HTTP client could not be constructed.
///
/// Construction-time only: this is not reachable from a request path, which
/// is why `vpay_api::ApiError` does not `#[from]` it (ADR-0011 asks a
/// composite to cover the leaves its layer can *meet*, and serving a request
/// never builds a client). It surfaces at boot, through `anyhow`, in
/// `backends/apps/*` — both binaries build their one client there and hand
/// clones to the adapters.
#[derive(Debug, thiserror::Error)]
pub enum HttpClientError {
    /// The vendored-roots rustls configuration did not assemble.
    ///
    /// Every input to it is fixed at compile time — the `ring` provider, the
    /// protocol versions the `rustls` pin enables, and a root store baked
    /// into the binary — so nothing an operator supplies can cause this. It
    /// is a bug in this module or a broken `rustls` pin.
    #[error("assembling the vendored-roots rustls configuration")]
    Tls(#[source] rustls::Error),

    /// reqwest refused to build a client around that configuration.
    #[error("building the outbound HTTP client")]
    Client(#[source] reqwest::Error),
}

impl Classify for HttpClientError {
    /// Both variants page, and deliberately so.
    ///
    /// [`Category::Configuration`] would be the wrong answer even for the
    /// reqwest half: there is no configuration file, flag or environment
    /// variable an operator could change to fix either one, because this
    /// function takes no inputs. A failure here means an invariant this crate
    /// guarantees — "the fixed, vendored TLS stack always assembles" — did
    /// not hold, which is [`Category::Internal`] by ADR-0011's definition,
    /// and exit `1` rather than `78` is the honest signal to a supervisor.
    fn category(&self) -> Category {
        Category::Internal
    }
}

/// Mozilla's CA bundle, compiled into the binary, as a rustls root store.
///
/// `webpki_roots` rather than `rustls_native_certs`: see the module doc. It
/// is already in the resolved graph (`sqlx`'s `tls-rustls-ring` feature
/// vendors the same bundle for Postgres TLS), so trusting it here does not
/// widen the dependency surface — it makes the two agree.
fn vendored_root_store() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// An outbound `reqwest::Client` whose trust anchors are compiled in and
/// which never reads the OS certificate store — the only kind that survives
/// the `FROM scratch` runtime image. See the module doc for the panic this
/// exists to prevent.
///
/// # Errors
///
/// [`HttpClientError::Tls`] if the vendored rustls configuration does not
/// assemble, [`HttpClientError::Client`] if reqwest refuses it. Neither is
/// expected to be reachable; they are `Result` rather than a panic because
/// this crate denies `unwrap`/`expect`/`panic` in shipping code (ADR-0007),
/// and because "impossible" is a claim a payment system should not make with
/// a panic.
///
/// # Deliberately not configurable
///
/// No timeout and no header: a caller that needs timeouts — every rail
/// adapter does — uses [`client_with_timeouts`], the "grow a `builder()`
/// sibling" this comment used to only anticipate.
///
/// Two of reqwest's defaults are *removed* rather than left alone, and
/// those are not configurable either: redirects are not followed and the
/// environment's proxy variables are ignored. `preconfigured_builder` says
/// why, at length. This is therefore no longer "the same client
/// `reqwest::Client::new()` would have, minus the trust store" — it is that
/// client minus two behaviours a server-side caller in this workspace must
/// never have.
pub fn client() -> Result<reqwest::Client, HttpClientError> {
    preconfigured_builder()?
        .build()
        .map_err(HttpClientError::Client)
}

/// The same vendored-roots client, bounded in time.
///
/// # Why the caller supplies the durations
///
/// One client is shared by every rail in a process (both binaries build it
/// once at boot and hand clones to the adapters), so the timeouts cannot be
/// a property of *this* function's opinion about rails — they come from
/// [`crate::ProviderConfig::connect_timeout`] /
/// [`crate::ProviderConfig::request_timeout`], which the deployment's YAML
/// feeds. That is also what lets the conformance suite assert
/// [`crate::ProviderError::Transport`] against a deliberately-slow WireMock
/// mapping in 100 ms instead of waiting out a 20 s production default.
///
/// `timeout` is reqwest's *whole-request* deadline (connect, send, and the
/// response body) and `connect_timeout` bounds only the TCP+TLS handshake.
/// Both are set: a request timeout alone would let a black-holed rail hold a
/// worker task for the full request budget on a connection that was never
/// going to establish, and a connect timeout alone bounds nothing once the
/// socket is open.
///
/// # Errors
///
/// As [`client`]: [`HttpClientError::Tls`] or [`HttpClientError::Client`].
/// Neither is reachable from a duration — reqwest validates nothing about
/// them — so a failure here means the same fixed TLS stack failed to
/// assemble.
pub fn client_with_timeouts(
    connect: Duration,
    request: Duration,
) -> Result<reqwest::Client, HttpClientError> {
    preconfigured_builder()?
        .connect_timeout(connect)
        .timeout(request)
        .build()
        .map_err(HttpClientError::Client)
}

/// The vendored-roots TLS configuration, already installed on a
/// `reqwest::ClientBuilder`, shared by the two constructors above so neither
/// can drift onto a different trust store, a different ALPN list, a
/// redirect policy or a proxy.
///
/// # Why redirects are not followed
///
/// reqwest's default is `redirect::Policy::limited(10)`, and on a
/// cross-host hop it strips exactly three headers: `Authorization`,
/// `Cookie` and `Proxy-Authorization`. Every *other* header is replayed at
/// the new host, and a rail adapter's headers are precisely the ones that
/// are not on that list — MTN's `Ocp-Apim-Subscription-Key`,
/// `X-Target-Environment`, `X-Reference-Id` and `X-Callback-Url` — while a
/// 307/308 replays the request **body**, which on Orange's `webpayment`
/// carries `merchant_key`. A rail (or anyone who can answer as one) that
/// responds `302 Location: https://attacker.example/` would therefore be
/// handed a merchant's rail credentials and the identity of a live charge,
/// by a client that was only asked to take a payment.
///
/// Neither rail documents a redirect on any call this workspace makes, so
/// there is nothing to lose by refusing: a 3xx arrives at the adapters'
/// "unexpected status" arms as [`crate::ProviderError::Malformed`], which
/// leaves the charge in the state a recovery pass reads rather than
/// advancing it on the strength of an answer from somewhere else. The
/// conformance suite's `REF_REDIRECT` case pins that, and pins the decisive
/// half: the redirect target is a mapping on the same WireMock that must
/// stay unrequested.
///
/// # Why the process environment cannot reroute a rail call
///
/// reqwest reads `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` from the
/// environment by default. A payment gateway's egress must be explicit
/// configuration, not ambient: a variable set on a pod — by a sidecar, a
/// base image, or a helpful default in a chart — must not silently put a
/// third party in the middle of a call that carries rail credentials. If a
/// deployment ever genuinely needs an egress proxy, it is a change here and
/// in an ADR, visible in review, rather than a value in an environment
/// nobody diffed.
///
/// This is deliberately **not** mirrored in the `sdks/rust` twin: that
/// client runs inside a *merchant's* process, on their network, where a
/// corporate egress proxy is a legitimate and common requirement. The two
/// copies differ here on purpose (see the module doc's note on the twin).
fn preconfigured_builder() -> Result<reqwest::ClientBuilder, HttpClientError> {
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(HttpClientError::Tls)?
    .with_root_certificates(vendored_root_store())
    .with_no_client_auth();
    // Set explicitly because reqwest only fills ALPN in on the branch *not*
    // taken here: hand it a built `ClientConfig` and whatever is in
    // `alpn_protocols` is what goes on the wire, so omitting these two names
    // would silently mean a TLS connection never negotiates HTTP/2.
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(reqwest::Client::builder()
        // The bare config, not `Some(config)`: `tls_backend_preconfigured`
        // wraps its argument in an `Option` itself before downcasting, so an
        // already-`Option` argument silently becomes `UnknownPreconfigured`
        // and `build()` fails with a bare "builder error".
        .tls_backend_preconfigured(tls)
        // See the two sections in this function's doc comment. Both are
        // *removals* of a reqwest default, which is why they are here rather
        // than at a call site: a client built without them is the dangerous
        // one, and there must be no way to construct it.
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy())
}

/// The most of a rail's response body this workspace will hold in memory.
///
/// 256 KiB. Every documented body either rail answers with is under a
/// kilobyte; the sizes that matter are the undocumented ones — a load
/// balancer's HTML error page, a captive portal, or a host that is not the
/// rail at all — and `reqwest`'s `text()`/`bytes()` buffer whatever arrives
/// in full. One worker task per charge, each willing to allocate an
/// unbounded body, is a memory exhaustion an attacker gets to choose the
/// size of.
///
/// Large enough that no real answer is ever near it, so a body that trips
/// the cap is evidence in itself rather than a tuning problem.
pub const MAX_RAIL_BODY_BYTES: usize = 256 * 1024;

/// Why a response body could not be read within its bound.
///
/// Separate from [`HttpClientError`], which is construction-time only: this
/// one is reachable from a live request path, on every call an adapter
/// makes, and the two classify differently — a rail sending an oversize or
/// truncated body is [`Category::Rail`], not a bug in this module.
#[derive(Debug, thiserror::Error)]
pub enum HttpBodyError {
    /// The rail sent more than the caller was willing to hold.
    ///
    /// The message names the cap deliberately: an operator reading it needs
    /// to know the limit was ours and what it is, and a test can assert on
    /// it without depending on how big the offending body actually was.
    #[error("the response exceeded {max} bytes")]
    TooLarge {
        /// The cap that was exceeded, in bytes.
        max: usize,
    },

    /// The connection failed part-way through the body — a reset, a TLS
    /// error, or the request deadline expiring mid-stream.
    #[error("reading the response body")]
    Read(#[source] reqwest::Error),
}

impl Classify for HttpBodyError {
    /// Both variants are the rail's behaviour, not ours.
    ///
    /// [`Category::Rail`] rather than `Internal` because neither says the
    /// charge failed and neither is fixed by a deploy: the fate of the
    /// request is unknown, which is exactly the state
    /// `docs/flows/crash-safety.md` resolves by asking the rail again. An
    /// oversize body in particular must never read as a decline — the rail
    /// may have accepted the payment and then answered with a proxy's error
    /// page.
    fn category(&self) -> Category {
        Category::Rail
    }
}

/// Reads a response body, refusing to hold more than `max` bytes of it.
///
/// # Why this exists rather than `Response::text()`
///
/// `text()` and `bytes()` read to end of stream: the peer decides how much
/// memory this process allocates. That is acceptable for a body whose size
/// a caller controls and unacceptable for a rail's — see
/// [`MAX_RAIL_BODY_BYTES`]. This reads chunk by chunk and gives up the
/// moment the accumulated length would exceed the cap, so an oversize body
/// costs one chunk of over-read rather than all of it, and the connection
/// is dropped with the response.
///
/// `Content-Length` is checked first when the peer supplies one, which
/// turns the common case into a refusal before a single body byte is read.
/// It is only a hint — a chunked response has none, and a lying one is
/// caught by the running total anyway — so it is an optimisation, never the
/// guard.
///
/// The status is returned alongside the bytes because the caller has given
/// up ownership of the response to get here, and every caller needs both.
///
/// # Errors
///
/// [`HttpBodyError::TooLarge`] if the body exceeds `max`;
/// [`HttpBodyError::Read`] if the stream fails part-way.
pub async fn bounded_body(
    mut response: reqwest::Response,
    max: usize,
) -> Result<(reqwest::StatusCode, Vec<u8>), HttpBodyError> {
    let status = response.status();

    if let Ok(cap) = u64::try_from(max)
        && response
            .content_length()
            .is_some_and(|declared| declared > cap)
    {
        return Err(HttpBodyError::TooLarge { max });
    }

    // No `with_capacity`: sizing the allocation from `Content-Length` would
    // let a peer that declares 200 KiB and sends nothing still cost 200 KiB.
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(HttpBodyError::Read)? {
        if body.len().saturating_add(chunk.len()) > max {
            return Err(HttpBodyError::TooLarge { max });
        }
        body.extend_from_slice(&chunk);
    }

    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: building a client must not depend on a process-wide
    /// `CryptoProvider` having been installed, and must not read the platform
    /// trust store. This unit test can only prove the first half (the second
    /// needs a process whose environment says the store is empty, which is
    /// `tests/cli.rs`'s job) — but the first half is the one that would
    /// otherwise panic rather than return an `Err`.
    #[test]
    fn a_client_builds_without_a_process_wide_crypto_provider() {
        assert!(client().is_ok(), "the vendored-roots client must build");
    }

    /// A `RootCertStore` with nothing in it would still let `client()` return
    /// `Ok` — and would then reject every TLS peer on earth at connect time,
    /// far from here. Asserting the bundle is non-empty pins the vendoring
    /// itself rather than the plumbing around it.
    #[test]
    fn the_vendored_bundle_is_not_empty() {
        assert!(
            !vendored_root_store().is_empty(),
            "webpki-roots must supply trust anchors"
        );
    }

    /// The timed client must be the *same* client: same vendored roots, same
    /// absence of a process-wide provider requirement. Durations are not
    /// validated by reqwest, so this is the whole of what construction can
    /// fail on.
    #[test]
    fn a_timed_client_builds_on_the_same_terms() {
        assert!(
            client_with_timeouts(Duration::from_secs(5), Duration::from_secs(20)).is_ok(),
            "the vendored-roots client must build with timeouts too"
        );
    }

    /// The whole of F1, asserted on the client itself: a 3xx is *returned*,
    /// and the `Location` is never requested.
    ///
    /// The `Location` is a path on the **same** listener on purpose. A
    /// cross-host target would make this pass for the wrong reason — an
    /// unroutable host errors out whatever the policy is — whereas a
    /// same-host, same-connection redirect is the one reqwest's default
    /// `Policy::limited(10)` follows most eagerly, carrying every header it
    /// does not consider sensitive. So a recorded second request is proof
    /// the policy is gone, and no recorded second request is proof it is
    /// there.
    ///
    /// This is a raw TCP listener, not an HTTP double: it speaks no rail's
    /// protocol and stands in for no rail, so ADR-0006 is not in play — the
    /// thing under test is this module's own transport configuration, and
    /// the same pattern is already how the timeout below is proven. The
    /// rail-level half (that an adapter turns the returned 3xx into
    /// `Malformed` rather than into a charge outcome) is the conformance
    /// suite's `REF_REDIRECT` case, against a real WireMock.
    #[tokio::test]
    async fn a_redirect_is_returned_rather_than_followed() {
        use std::io::{Read as _, Write as _};
        use std::sync::{Arc, Mutex};

        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("binding a loopback listener");
        let address = listener.local_addr().expect("the bound address");
        let paths: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let recorded = Arc::clone(&paths);
        // Detached: nextest gives every test its own process, so the thread
        // dies with it. Blocking std sockets rather than tokio's keep this
        // free of an `io-util` feature no shipping crate here needs.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0_u8; 1024];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(buffer.get(..read).unwrap_or_default());
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("<no request line>")
                    .to_owned();
                recorded
                    .lock()
                    .expect("the path recorder is not poisoned")
                    .push(path);
                // `Connection: close` so a followed redirect is a fresh
                // accept rather than a second request this one-shot read
                // would miss.
                let _ = stream.write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /followed\r\nContent-Length: 0\r\n\
                      Connection: close\r\n\r\n",
                );
            }
        });

        let client = client().expect("the vendored-roots client builds");
        let response = client
            .get(format!("http://{address}/submitted"))
            .send()
            .await
            .expect("a 302 is an answer to be interpreted, not a transport failure");

        assert_eq!(
            response.status().as_u16(),
            302,
            "the 3xx must reach the caller, so an adapter can refuse it"
        );
        let seen = paths
            .lock()
            .expect("the path recorder is not poisoned")
            .clone();
        assert_eq!(
            seen,
            vec!["/submitted".to_owned()],
            "the Location was requested; a redirect would have replayed the rail headers \
             and body at whatever host answered: {seen:?}"
        );
    }

    /// The assertion worth having: a client is not "configured with a
    /// timeout", it *gives up*. A listener that accepts the connection and
    /// then says nothing is the shape of a rail that has stopped answering —
    /// the case where a missing deadline hangs a worker task indefinitely
    /// rather than surfacing `ProviderError::Transport`.
    ///
    /// The request timeout is the half that can be proven hermetically. A
    /// connect timeout needs an address that black-holes packets (a routable
    /// but unreachable host), which is not something a test may assume about
    /// the network it runs on — so `connect_timeout`'s effect is asserted by
    /// the conformance suite against a real WireMock instead, and is
    /// deliberately not faked here.
    #[tokio::test]
    async fn a_request_timeout_actually_fires_against_a_silent_peer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding a loopback listener");
        let address = listener.local_addr().expect("the bound address");
        // Accept and hold: never write a byte, never close. Dropped with the
        // test.
        let _accepting = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });

        let client = client_with_timeouts(Duration::from_secs(5), Duration::from_millis(150))
            .expect("the timed client builds");
        let started = std::time::Instant::now();
        let outcome = client.get(format!("http://{address}/")).send().await;

        let error = outcome.expect_err("a peer that never answers must not resolve");
        assert!(error.is_timeout(), "expected a timeout, got: {error}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the 150 ms request timeout did not bound the call: waited {:?}",
            started.elapsed()
        );
    }
}
