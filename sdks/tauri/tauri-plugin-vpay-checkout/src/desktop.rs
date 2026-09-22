//! The macOS / Windows / Linux host.
//!
//! # What it does
//!
//! It hands the session URL to the platform's default browser and remembers
//! the channel the caller gave it. That is the whole of it. The page runs in
//! the browser's own process — not in a `WebviewWindow`, not in a WebView
//! (D5, revised 2026-09-16) — so this host can neither observe a navigation
//! nor read a cookie, which is the point: a compromised merchant app must not
//! be able to watch the payer type a PAN.
//!
//! # What it cannot do, stated rather than invented
//!
//! **There is no dismissal signal on desktop.** Once the URL is handed to the
//! default browser, no API on any of the three platforms tells this process
//! that the payer closed the tab, switched away, or finished paying. This is
//! exactly the Flutter plugin's macOS situation and it is reported the same
//! way: [`VpayCheckout::dismiss`] sends
//! [`CheckoutWindowOutcome::Dismissed`](crate::CheckoutWindowOutcome::Dismissed)
//! when the *application* asks for it, and nothing else ever will. This host
//! does **not** invent a focus-based or window-activation-based signal —
//! "the app regained focus" is not "the payer finished", and treating it as
//! one would be a guess dressed as an event.
//!
//! What makes that correctness-complete rather than a gap is D4: guest-JS
//! polls `GET /v1/browser/payment_intents/{id}` on a budget and answers
//! `pending` when the budget elapses. The window never decides an outcome
//! (D1), so a window that reports nothing costs latency, not correctness.
//!
//! Deep links, and therefore
//! [`StopUrlReached`](crate::CheckoutWindowOutcome::StopUrlReached), are not
//! implemented here at all. A desktop deep link needs an OS-registered URL
//! scheme owned by the *application* (`tauri-plugin-deep-link`'s territory),
//! which this plugin cannot register on a merchant's behalf.
//!
//! # The browser seam
//!
//! [`BrowserOpener`] exists so that the state machine below can be tested
//! without launching a browser on the machine running `cargo test`. It is
//! crate-private, it has exactly one shipping implementation
//! ([`SystemBrowserOpener`], which [`init`] always installs), and no
//! production code path can select another. It is a test seam, not a mock
//! adapter: nothing in this crate can be configured into a fake at run time.

use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::de::DeserializeOwned;
use tauri::ipc::Channel;
use tauri::plugin::PluginApi;
use tauri::{AppHandle, Runtime};
use url::Url;

use crate::error::Error;
use crate::models::{CheckoutWindowEvent, CheckoutWindowOutcome, ShowCheckoutRequest};

/// How the desktop host reaches the payer's browser.
///
/// Crate-private and single-implementation on purpose — see the module doc.
pub(crate) trait BrowserOpener: Send + Sync + 'static {
    /// Opens `url` in whatever the platform considers the default browser,
    /// without waiting for it to exit.
    fn open(&self, url: &str) -> std::io::Result<()>;
}

/// The one shipping [`BrowserOpener`]: the `open` crate's detached launch.
///
/// `that_detached` rather than `that`: `that` waits for the spawned process,
/// and on Linux the "process" is frequently the browser itself, so a blocking
/// call would hold the Tauri command's task for the entire checkout.
struct SystemBrowserOpener;

impl BrowserOpener for SystemBrowserOpener {
    fn open(&self, url: &str) -> std::io::Result<()> {
        open::that_detached(url)
    }
}

/// Access to the checkout window on a desktop platform.
///
/// Managed by [`crate::init`] and reachable as `app.vpay_checkout()`.
pub struct VpayCheckout<R: Runtime> {
    /// `fn() -> R` rather than `R`: it keeps the type parameter used without
    /// requiring `R: Send + Sync` for this struct to be, which is what
    /// `Manager::manage` needs. The same trick `tauri-plugin-opener` uses.
    runtime: PhantomData<fn() -> R>,
    opener: Box<dyn BrowserOpener>,
    /// The channel of the `show` currently in flight, or `None`.
    ///
    /// This single `Option` *is* the "already open" rule: its occupancy is
    /// the only record that a checkout is outstanding, so there is no second
    /// flag to fall out of step with it.
    in_flight: Mutex<Option<Channel<CheckoutWindowEvent>>>,
}

impl<R: Runtime> VpayCheckout<R> {
    fn with_opener(opener: Box<dyn BrowserOpener>) -> Self {
        Self {
            runtime: PhantomData,
            opener,
            in_flight: Mutex::new(None),
        }
    }

    /// Takes the in-flight lock, recovering from poisoning rather than
    /// panicking.
    ///
    /// ADR-0007 denies `unwrap`/`expect` in production code, and a payment
    /// path is the last place to re-panic on someone else's panic. Nothing in
    /// the critical sections below can panic — no indexing, no arithmetic, no
    /// allocation-free assumptions — so the guarded `Option` cannot be left
    /// in a half-written state, and `into_inner` is the honest recovery.
    fn lock_in_flight(&self) -> MutexGuard<'_, Option<Channel<CheckoutWindowEvent>>> {
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens the payer's default browser on `request.url`.
    ///
    /// Returns once the browser has been launched — not once the payer has
    /// done anything. The outcome, if one ever arrives, comes over
    /// `request.on_event`.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidUrl`] if the URL does not parse or is not `http(s)`.
    ///   Validation happens **before** the already-open check so that a
    ///   malformed request cannot be mistaken for a concurrency problem.
    /// - [`Error::InsecureUrl`] if it is `http` and `allow_insecure_url` is
    ///   not set (D6).
    /// - [`Error::AlreadyOpen`] if a checkout is already in flight.
    /// - [`Error::OpenFailed`] if the platform refused to launch a browser.
    ///   The in-flight slot is cleared again in that case, so a failed `show`
    ///   does not wedge the plugin.
    pub fn show(&self, request: ShowCheckoutRequest) -> crate::Result<()> {
        validate_url(&request.url, request.allow_insecure_url)?;

        // The lock is held across the launch deliberately: releasing it first
        // would let a second `show` slip past the emptiness check while the
        // first is still spawning. `that_detached` returns as soon as the
        // child process is spawned, so the window is short.
        let mut in_flight = self.lock_in_flight();
        if in_flight.is_some() {
            return Err(Error::AlreadyOpen);
        }
        *in_flight = Some(request.on_event.clone());

        if self.opener.open(&request.url).is_err() {
            *in_flight = None;
            return Err(Error::OpenFailed);
        }

        Ok(())
    }

    /// Ends the in-flight checkout, reporting
    /// [`CheckoutWindowOutcome::Dismissed`] exactly once.
    ///
    /// A no-op when nothing is in flight, which is what the brief's wire
    /// contract specifies and what makes a double `dismiss()` from guest-JS's
    /// cleanup path harmless. It does **not** close anything: the browser is
    /// the payer's, not the app's.
    ///
    /// # Errors
    ///
    /// [`Error::Tauri`] if the event could not be delivered to the webview —
    /// in practice, if the webview is already gone.
    pub fn dismiss(&self) -> crate::Result<()> {
        // `take()` under the lock is what makes "exactly once" true: a
        // concurrent second `dismiss` finds `None`.
        let channel = self.lock_in_flight().take();

        match channel {
            Some(channel) => channel
                .send(CheckoutWindowEvent {
                    outcome: CheckoutWindowOutcome::Dismissed,
                    reached_url: None,
                })
                .map_err(Error::from),
            None => Ok(()),
        }
    }
}

/// Refuses a URL before it can reach the platform's "open this" call.
///
/// Parsing rather than prefix-matching: `open::that_detached` will happily
/// hand `file:///etc/passwd` or an arbitrary registered scheme to the OS, and
/// "starts with https://" is not a scheme check.
fn validate_url(url: &str, allow_insecure_url: bool) -> crate::Result<()> {
    let parsed = Url::parse(url).map_err(|_| Error::InvalidUrl)?;

    match parsed.scheme() {
        "https" => Ok(()),
        // D6: plaintext must be asked for by name, never inferred.
        "http" if allow_insecure_url => Ok(()),
        "http" => Err(Error::InsecureUrl),
        _ => Err(Error::InvalidUrl),
    }
}

/// Builds the desktop host. Called from [`crate::init`]'s `setup`.
///
/// # Errors
///
/// Infallible today; the `Result` matches [`crate::mobile::init`]'s signature
/// so that `lib.rs` can `#[cfg]` between them without branching on the shape.
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<VpayCheckout<R>> {
    Ok(VpayCheckout::with_opener(Box::new(SystemBrowserOpener)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tauri::ipc::InvokeResponseBody;

    use super::*;
    use crate::models::CheckoutStopUrl;

    /// A [`BrowserOpener`] that records instead of launching, and can be told
    /// to fail. Test-only: `init` has no way to install it.
    struct RecordingOpener {
        opened: Mutex<Vec<String>>,
        fail: bool,
    }

    impl RecordingOpener {
        fn new(fail: bool) -> Arc<Self> {
            Arc::new(Self {
                opened: Mutex::new(Vec::new()),
                fail,
            })
        }

        fn opened(&self) -> Vec<String> {
            self.opened.lock().unwrap().clone()
        }
    }

    impl BrowserOpener for Arc<RecordingOpener> {
        fn open(&self, url: &str) -> std::io::Result<()> {
            self.opened.lock().unwrap().push(url.to_string());
            if self.fail {
                Err(std::io::Error::other("no browser in this test"))
            } else {
                Ok(())
            }
        }
    }

    /// The injectable sink: a real `Channel` whose handler records the JSON
    /// each event serialises to. Nothing is stubbed — this is the same
    /// `Channel::send` path production uses, with the webview's `eval`
    /// replaced by a `Vec`.
    fn recording_channel() -> (Channel<CheckoutWindowEvent>, Arc<Mutex<Vec<String>>>) {
        let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&sink);

        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(json) = body {
                recorder.lock().unwrap().push(json);
            }
            Ok(())
        });

        (channel, sink)
    }

    fn host(opener: &Arc<RecordingOpener>) -> VpayCheckout<tauri::Wry> {
        VpayCheckout::with_opener(Box::new(Arc::clone(opener)))
    }

    fn request(
        url: &str,
        allow_insecure_url: bool,
    ) -> (ShowCheckoutRequest, Arc<Mutex<Vec<String>>>) {
        let (on_event, sink) = recording_channel();
        (
            ShowCheckoutRequest {
                url: url.to_string(),
                stop_urls: vec![CheckoutStopUrl {
                    scheme: "https".to_string(),
                    host: "shop.example".to_string(),
                    port: 443,
                    path: "/return".to_string(),
                }],
                allow_insecure_url,
                on_event,
            },
            sink,
        )
    }

    const SESSION_URL: &str = "https://checkout.vpay.example/c/cs_1?key=pk_1#cs_1_secret_abc";

    #[test]
    fn a_valid_https_show_launches_the_browser_on_exactly_the_url_it_was_given() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (req, _sink) = request(SESSION_URL, false);

        assert!(checkout.show(req).is_ok());
        assert_eq!(opener.opened(), vec![SESSION_URL.to_string()]);
    }

    #[test]
    fn a_second_show_while_one_is_in_flight_is_refused_as_already_open() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (first, _first_sink) = request(SESSION_URL, false);
        let (second, second_sink) = request(SESSION_URL, false);

        assert!(checkout.show(first).is_ok());
        let refusal = checkout.show(second);

        assert!(matches!(refusal, Err(Error::AlreadyOpen)));
        // The refused request's browser was never launched, and its channel
        // never heard anything — the caller learns from the rejection only.
        assert_eq!(opener.opened().len(), 1);
        assert!(second_sink.lock().unwrap().is_empty());
    }

    #[test]
    fn a_show_after_a_dismiss_is_accepted_again() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (first, _first_sink) = request(SESSION_URL, false);
        let (second, _second_sink) = request(SESSION_URL, false);

        assert!(checkout.show(first).is_ok());
        assert!(checkout.dismiss().is_ok());
        assert!(checkout.show(second).is_ok());
        assert_eq!(opener.opened().len(), 2);
    }

    #[test]
    fn dismiss_sends_exactly_one_dismissed_event() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (req, sink) = request(SESSION_URL, false);

        assert!(checkout.show(req).is_ok());
        assert!(checkout.dismiss().is_ok());
        // A second and third dismiss must add nothing.
        assert!(checkout.dismiss().is_ok());
        assert!(checkout.dismiss().is_ok());

        let events = sink.lock().unwrap().clone();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0], r#"{"outcome":"dismissed","reachedUrl":null}"#);
    }

    #[test]
    fn dismiss_while_idle_is_a_noop_and_sends_nothing() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (_req, sink) = request(SESSION_URL, false);

        assert!(checkout.dismiss().is_ok());

        assert!(sink.lock().unwrap().is_empty());
        assert!(opener.opened().is_empty());
    }

    #[test]
    fn an_unparseable_url_is_refused_before_the_browser_is_touched() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (req, sink) = request("not a url at all", false);

        assert!(matches!(checkout.show(req), Err(Error::InvalidUrl)));
        assert!(opener.opened().is_empty());
        assert!(sink.lock().unwrap().is_empty());
    }

    #[test]
    fn a_non_http_scheme_is_refused_as_an_invalid_url() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);

        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vpay://checkout/cs_1",
        ] {
            let (req, _sink) = request(url, true);
            assert!(
                matches!(checkout.show(req), Err(Error::InvalidUrl)),
                "expected {url} to be refused"
            );
        }

        assert!(opener.opened().is_empty());
    }

    #[test]
    fn an_http_url_is_refused_unless_the_insecure_opt_in_is_set() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (req, _sink) = request("http://localhost:4242/c/cs_1", false);

        assert!(matches!(checkout.show(req), Err(Error::InsecureUrl)));
        assert!(opener.opened().is_empty());
    }

    #[test]
    fn an_http_url_is_opened_when_the_insecure_opt_in_is_set() {
        let opener = RecordingOpener::new(false);
        let checkout = host(&opener);
        let (req, _sink) = request("http://localhost:4242/c/cs_1", true);

        assert!(checkout.show(req).is_ok());
        assert_eq!(
            opener.opened(),
            vec!["http://localhost:4242/c/cs_1".to_string()]
        );
    }

    #[test]
    fn a_browser_that_refuses_to_launch_clears_the_in_flight_slot() {
        let opener = RecordingOpener::new(true);
        let checkout = host(&opener);
        let (first, first_sink) = request(SESSION_URL, false);
        let (second, _second_sink) = request(SESSION_URL, false);

        assert!(matches!(checkout.show(first), Err(Error::OpenFailed)));
        // Not `AlreadyOpen`: the failed show must not wedge the plugin.
        assert!(matches!(checkout.show(second), Err(Error::OpenFailed)));
        // And the failed show's channel is never told anything.
        assert!(first_sink.lock().unwrap().is_empty());
    }

    #[test]
    fn a_dismiss_after_a_failed_show_sends_nothing() {
        let opener = RecordingOpener::new(true);
        let checkout = host(&opener);
        let (req, sink) = request(SESSION_URL, false);

        assert!(matches!(checkout.show(req), Err(Error::OpenFailed)));
        assert!(checkout.dismiss().is_ok());

        assert!(sink.lock().unwrap().is_empty());
    }
}
