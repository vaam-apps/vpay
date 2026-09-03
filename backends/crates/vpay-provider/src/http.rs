//! The outbound HTTP client every server-side caller in this workspace uses,
//! and the bounded read every rail answer comes back through.
//!
//! [`client`] hands reqwest a finished vendored-roots `rustls::ClientConfig`
//! instead of letting it read the platform trust store, which is the only
//! kind of client that survives the `FROM scratch` runtime image
//! ([ADR-0004]) — `reqwest::Client::new()` *panics* there. It also removes
//! two reqwest defaults: redirects are not followed, and the environment's
//! proxy variables are ignored. [`bounded_body`] and [`read_rail_body`] stop
//! a rail deciding how much memory this process allocates.
//!
//! Each of those is a decision with a specific attack or outage behind it,
//! and each is argued in `docs/reference/rails.md` — including why this
//! module lives in the port crate, what the vendored roots cost, and the one
//! place the `sdks/rust` twin deliberately differs.
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
/// adapter does — uses [`client_with_timeouts`].
///
/// Two of reqwest's defaults are *removed* rather than left alone, and those
/// are not configurable either: redirects are not followed and the
/// environment's proxy variables are ignored — `docs/reference/rails.md`
/// gives both arguments. This is therefore not "the client
/// `reqwest::Client::new()` would have, minus the trust store": it is that
/// client minus two behaviours a server-side caller in this workspace must
/// never have.
///
/// ```
/// // No process-wide rustls `CryptoProvider` is installed here, and this
/// // still returns `Ok`: that is the whole difference from
/// // `reqwest::Client::new()`, which panics on both of the conditions the
/// // `FROM scratch` runtime image presents.
/// assert!(vpay_provider::http::client().is_ok());
/// ```
pub fn client() -> Result<reqwest::Client, HttpClientError> {
    preconfigured_builder()?
        .build()
        .map_err(HttpClientError::Client)
}

/// The same vendored-roots client, bounded in time.
///
/// The durations are the caller's because one client is shared by every rail
/// in a process: they come from [`crate::ProviderConfig`], which the
/// deployment's YAML feeds. `request` is reqwest's *whole-request* deadline
/// and `connect` bounds only the TCP+TLS handshake; both are set, because
/// either alone leaves a hole. See `docs/reference/rails.md`.
///
/// # Errors
///
/// As [`client`]: [`HttpClientError::Tls`] or [`HttpClientError::Client`].
/// Neither is reachable from a duration — reqwest validates nothing about
/// them — so a failure here means the same fixed TLS stack failed to
/// assemble.
///
/// ```
/// use vpay_provider::{DEFAULT_CONNECT_TIMEOUT, DEFAULT_REQUEST_TIMEOUT};
/// use vpay_provider::http::client_with_timeouts;
///
/// // The pair a `ProviderConfig` built from YAML carries. Durations are not
/// // validated, so construction fails only if the fixed TLS stack does.
/// assert!(client_with_timeouts(DEFAULT_CONNECT_TIMEOUT, DEFAULT_REQUEST_TIMEOUT).is_ok());
/// ```
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
/// can drift onto a different trust store, a different ALPN list, a redirect
/// policy or a proxy.
///
/// The redirect policy and `no_proxy` are *removals* of a reqwest default,
/// and they are here rather than at a call site because a client built
/// without them is the dangerous one: there must be no way to construct it.
/// `docs/reference/rails.md` gives both arguments — in short, a followed
/// redirect replays every header reqwest does not consider sensitive (which
/// is exactly the set a rail adapter uses) and, on a 307/308, the request
/// body, at whatever host answered; and a proxy variable set on a pod by a
/// sidecar or a base image must not be able to put a third party in the
/// middle of a call carrying rail credentials.
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
/// 256 KiB: large enough that no real answer is ever near it, so a body that
/// trips the cap is evidence in itself rather than a tuning problem.
/// `docs/reference/rails.md` has the exhaustion this bounds.
pub const MAX_RAIL_BODY_BYTES: usize = 256 * 1024;

/// Why a response body could not be read within its bound.
///
/// Separate from [`HttpClientError`], which is construction-time only: this
/// one is reachable from a live request path and classifies differently.
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
    /// Both variants are the rail's behaviour, not ours: neither says the
    /// charge failed and neither is fixed by a deploy, so the fate of the
    /// request is unknown — the state `docs/flows/crash-safety.md` resolves
    /// by asking the rail again. An oversize body in particular must never
    /// read as a decline; the rail may have accepted the payment and then
    /// answered with a proxy's error page.
    fn category(&self) -> Category {
        Category::Rail
    }
}

/// Reads a response body, refusing to hold more than `max` bytes of it.
///
/// `text()`/`bytes()` read to end of stream, which lets the peer decide how
/// much memory this process allocates; this gives up the moment the
/// accumulated length would exceed the cap, so an oversize body costs one
/// chunk of over-read rather than all of it. `Content-Length` is an
/// optimisation and never the guard — a chunked response has none and a
/// lying one is caught by the running total. See `docs/reference/rails.md`.
///
/// The status comes back alongside the bytes because the caller gave up
/// ownership of the response to get here, and every caller needs both.
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

/// [`bounded_body`] at [`MAX_RAIL_BODY_BYTES`], with each outcome already in
/// the [`ProviderError`](crate::ProviderError) an adapter has to return.
///
/// `context` is the adapter's own sentence — which rail, doing what — and is
/// what `Display` renders; the library error travels underneath as a
/// `#[source]`. Both adapters had a copy of this four-line match, differing
/// only in that prefix, which is one copy too many for a decision this
/// consequential: an oversize body must never read as a decline. See
/// `docs/reference/rails.md`.
///
/// Bytes, not text: Orange parses JSON from them and MTN decodes lossily for
/// its own diagnostics, and a reader that decoded for both would make one of
/// them re-encode.
///
/// # Errors
///
/// [`ProviderError::Malformed`](crate::ProviderError::Malformed) when the
/// body exceeds the cap, naming the cap in the message — an operator needs to
/// see that the limit was ours and what it is, and the conformance suite
/// asserts it. `Malformed` and not a decline, because an oversize answer says
/// nothing about whether the payment happened, and
/// `docs/flows/crash-safety.md` resolves an unknown fate by asking again.
/// [`ProviderError::Transport`](crate::ProviderError::Transport) if the
/// stream fails part-way, which leaves the same unknown fate.
pub async fn read_rail_body(
    response: reqwest::Response,
    context: &str,
) -> Result<(reqwest::StatusCode, Vec<u8>), crate::ProviderError> {
    match bounded_body(response, MAX_RAIL_BODY_BYTES).await {
        Ok(answered) => Ok(answered),
        // The cap is written into the message rather than left to the
        // source's `Display`: it is the whole diagnostic, and a `Display`
        // that stops at `context` would not carry it.
        Err(error @ HttpBodyError::TooLarge { .. }) => Err(crate::ProviderError::malformed_from(
            format!("{context}: the response exceeded {MAX_RAIL_BODY_BYTES} bytes"),
            error,
        )),
        Err(error @ HttpBodyError::Read(_)) => {
            Err(crate::ProviderError::transport_from(context, error))
        }
    }
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
