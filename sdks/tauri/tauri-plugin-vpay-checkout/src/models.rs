//! The wire types shared by the guest-JS package, this crate, and the Kotlin
//! and Swift hosts.
//!
//! These are a direct port of `sdks/flutter/vpay_checkout_flutter/pigeons/checkout.dart`
//! — the Flutter plugin's frozen platform seam — onto Tauri's IPC. The
//! vocabulary is deliberately the same, because the thing being described is
//! the same payer surface on a different host framework, and a second
//! vocabulary for it would be a second place to get it wrong.
//!
//! Every struct here is `#[serde(rename_all = "camelCase")]`. That is this
//! plugin's **JavaScript** wire, not vpay's `/v1` HTTP wire: `cargo xtask
//! verify-serde` scans `backends/crates` only, and the keys on this seam are
//! read by TypeScript, Kotlin and Swift.
//!
//! The one rule that is not obvious from the shapes: **nothing here decides an
//! outcome.** There is no `succeeded`/`canceled`/`failed` member anywhere, by
//! design (D1). A window can report only that it closed, or that a deep link
//! matching a stop URL arrived; the payment's outcome comes from guest-JS's
//! poll of `GET /v1/browser/payment_intents/{id}` and from nowhere else.

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;

/// One stop URL, already normalised by guest-JS the way the Flutter
/// controller's `StopUrlSpec` is: scheme, host, port and path only.
///
/// Query and fragment are ignored when matching (D2), so they are not carried
/// across this seam at all — there is nothing here for a host to compare
/// against them by mistake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutStopUrl {
    /// URL scheme, lowercased by guest-JS (`"https"` in practice; a merchant
    /// may configure an app scheme, and the host must not assume otherwise).
    pub scheme: String,
    /// Host, lowercased by guest-JS. Compared verbatim — no suffix matching,
    /// no wildcard.
    pub host: String,
    /// Port, with the scheme's default already substituted by guest-JS (443
    /// for `https`, 80 for `http`), so a host never has to know what a
    /// default port is.
    pub port: u16,
    /// Path, compared verbatim. Query and fragment are not part of the match.
    pub path: String,
}

/// What `show` hands the host.
///
/// `stop_urls` is empty exactly when the session carries neither a
/// `success_url` nor a `cancel_url`. An embedded session never reaches here —
/// guest-JS refuses it at the pre-flight (D2) — but a hosted session that only
/// ever forwards one way is real, and no host may require both.
///
/// `Debug` is implemented by hand, immediately below, and that is a security
/// property rather than a formatting preference: see [`ShowCheckoutRequest::url`].
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowCheckoutRequest {
    /// The session's own hosted URL. The host loads exactly this and
    /// constructs no URL of its own.
    ///
    /// **It is a credential.** The fragment carries the checkout session's
    /// secret (D6), so this string may never be logged, `Display`ed,
    /// interpolated into an error message, or rendered by `Debug`. The manual
    /// `Debug` impl below renders it as `[N chars redacted]`, matching
    /// `redaction.dart`'s `redacted()` in the Flutter plugin, and
    /// `the_debug_rendering_of_a_show_request_never_contains_the_url` pins
    /// that.
    pub url: String,
    /// The deep links the host watches for (D2). Since D5's 2026-09-16
    /// revision the page runs in the browser's own process, where no host on
    /// any platform can observe a navigation, so these match an **incoming**
    /// App Link / Universal Link and nothing else.
    pub stop_urls: Vec<CheckoutStopUrl>,
    /// D6's named insecure opt-in, forwarded so that a host never re-derives
    /// "is this the demo stack" from the URL's scheme itself.
    pub allow_insecure_url: bool,
    /// Where the host reports the single [`CheckoutWindowEvent`] for this
    /// `show`.
    ///
    /// On mobile this field is what makes the round trip work: a
    /// [`Channel`] serialises to the string `"__CHANNEL__:<id>"`
    /// (`tauri::ipc::channel`'s `IPC_PAYLOAD_PREFIX` and its
    /// `impl<TSend> Serialize for Channel<TSend>`, crates/tauri/src/ipc/channel.rs
    /// lines 28 and 132-139 at tag `tauri-v2.11.6`), which is exactly the
    /// format `app.tauri.plugin.Channel` and `Tauri.Channel` parse on the
    /// Kotlin and Swift sides. Every `Channel` is registered in a
    /// process-global `CHANNELS` map when it is constructed
    /// (`Channel::new_with_id` → `crate::plugin::mobile::register_channel`,
    /// channel.rs lines 275-281 and plugin/mobile.rs lines 35 and 58-64), and
    /// the Kotlin/Swift `send` is routed back to it by id
    /// (`send_channel_data`, plugin/mobile.rs lines 114-132 for Android and
    /// `send_channel_data_handler`, lines 404-424 for iOS). So passing this
    /// struct straight to `PluginHandle::run_mobile_plugin` is sufficient —
    /// there is no separate registration step for a plugin to perform.
    pub on_event: Channel<CheckoutWindowEvent>,
}

impl std::fmt::Debug for ShowCheckoutRequest {
    /// Renders the session URL as `[N chars redacted]`.
    ///
    /// Written by hand rather than derived because a derived `Debug` would put
    /// the session secret into every `tracing` field, every `{:?}` in a test
    /// failure and every panic message that happens to carry this struct —
    /// which is D6's entire concern, and the reason the Flutter plugin has a
    /// `redaction.dart` at all. The channel is rendered by its id, which is a
    /// process-local `u32` and carries nothing.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShowCheckoutRequest")
            .field("url", &Redacted(self.url.len()))
            .field("stop_urls", &self.stop_urls)
            .field("allow_insecure_url", &self.allow_insecure_url)
            .field("on_event", &self.on_event.id())
            .finish()
    }
}

/// `[N chars redacted]`, rendered without quotes so it cannot be mistaken for
/// the value itself. The Flutter plugin's `redacted()` prints the same text
/// for the same reason.
struct Redacted(usize);

impl std::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{} chars redacted]", self.0)
    }
}

/// Which of the two signals guest-JS's poll can be triggered by actually
/// happened.
///
/// Never a `succeeded`/`canceled`/`failed` member: the whole point of D1 is
/// that this seam **cannot** say that, and only the poll of
/// `/v1/browser/payment_intents/{id}` can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckoutWindowOutcome {
    /// The payer closed the browser sheet — Done, back press, swipe-away —
    /// with no matching deep link having arrived.
    ///
    /// Since the browser cutover (D5, revised 2026-09-16) this is the
    /// **ordinary** end of a *successful* payment too, not only of a
    /// cancellation, which is exactly why D4 polls before answering.
    Dismissed,
    /// An **incoming deep link** (Android App Link, iOS Universal Link)
    /// matched one of [`ShowCheckoutRequest::stop_urls`] on
    /// scheme+host+port+path, query and fragment ignored (D2).
    ///
    /// **Unverified end to end on every platform**, for the same reason the
    /// Flutter plugin's is: a verified App Link / Universal Link needs an
    /// HTTPS origin serving `assetlinks.json` / `apple-app-site-association`
    /// for the merchant's own `success_url` host, and this repository serves
    /// neither. Every host therefore reports [`Self::Dismissed`] in practice
    /// today; D1 and D4 make that correctness-complete.
    StopUrlReached,
}

/// The one event a host reports per `show` — exactly once, never twice, never
/// none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutWindowEvent {
    /// Which signal occurred.
    pub outcome: CheckoutWindowOutcome,
    /// The full deep-link URL, present only for
    /// [`CheckoutWindowOutcome::StopUrlReached`].
    ///
    /// **Diagnostics only.** Guest-JS reads nothing off it (D1); it exists so
    /// that a merchant debugging a stop-URL configuration can see what
    /// actually arrived.
    pub reached_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session URL is a credential (D6). This is the test the manual
    /// `Debug` impl exists for: if someone replaces it with `#[derive(Debug)]`
    /// the secret starts appearing in log lines and this fails.
    #[test]
    fn the_debug_rendering_of_a_show_request_never_contains_the_url() {
        let url = "https://checkout.vpay.example/c/cs_live_abc?key=pk_live_x#cs_live_abc_secret_TOPSECRET";
        let request = ShowCheckoutRequest {
            url: url.to_string(),
            stop_urls: vec![CheckoutStopUrl {
                scheme: "https".to_string(),
                host: "shop.example".to_string(),
                port: 443,
                path: "/return".to_string(),
            }],
            allow_insecure_url: false,
            on_event: Channel::new(|_| Ok(())),
        };

        let rendered = format!("{request:?}");

        assert!(!rendered.contains("TOPSECRET"), "{rendered}");
        assert!(!rendered.contains("cs_live_abc"), "{rendered}");
        assert!(!rendered.contains("pk_live_x"), "{rendered}");
        assert!(!rendered.contains("checkout.vpay.example"), "{rendered}");
        assert!(
            rendered.contains(&format!("[{} chars redacted]", url.len())),
            "{rendered}"
        );
        // The non-secret fields are still useful, and still there.
        assert!(rendered.contains("shop.example"), "{rendered}");
        assert!(rendered.contains("allow_insecure_url: false"), "{rendered}");
    }

    /// The alternate-form `{:#?}` rendering goes through the same
    /// `debug_struct` builder, but a reader should not have to know that.
    #[test]
    fn the_pretty_debug_rendering_of_a_show_request_never_contains_the_url() {
        let request = ShowCheckoutRequest {
            url: "https://checkout.vpay.example/c/cs_1#cs_1_secret_TOPSECRET".to_string(),
            stop_urls: Vec::new(),
            allow_insecure_url: true,
            on_event: Channel::new(|_| Ok(())),
        };

        let rendered = format!("{request:#?}");

        assert!(!rendered.contains("TOPSECRET"), "{rendered}");
        assert!(rendered.contains("chars redacted"), "{rendered}");
    }

    /// The brief fixes these two strings; guest-JS, Kotlin and Swift all
    /// compare against them literally.
    #[test]
    fn the_outcome_serialises_to_the_two_fixed_wire_strings() {
        assert_eq!(
            serde_json::to_string(&CheckoutWindowOutcome::Dismissed).ok(),
            Some("\"dismissed\"".to_string())
        );
        assert_eq!(
            serde_json::to_string(&CheckoutWindowOutcome::StopUrlReached).ok(),
            Some("\"stopUrlReached\"".to_string())
        );
    }

    /// `{ "outcome": …, "reachedUrl": … }` — camelCase, and `reachedUrl`
    /// present-and-null rather than absent, because guest-JS's
    /// `CheckoutWindowEvent` types it `string | null`.
    #[test]
    fn the_event_serialises_with_the_camel_case_wire_keys() {
        let json = serde_json::to_value(CheckoutWindowEvent {
            outcome: CheckoutWindowOutcome::Dismissed,
            reached_url: None,
        });

        assert_eq!(
            json.ok(),
            Some(serde_json::json!({ "outcome": "dismissed", "reachedUrl": null }))
        );
    }

    /// The `show` payload the mobile hosts receive. `onEvent` must be the
    /// `__CHANNEL__:<id>` string their `Channel` types parse, and the other
    /// three keys must be exactly the brief's.
    #[test]
    fn the_show_payload_serialises_with_the_camel_case_wire_keys_and_a_channel_string() {
        let channel = Channel::<CheckoutWindowEvent>::new(|_| Ok(()));
        let id = channel.id();
        let request = ShowCheckoutRequest {
            url: "https://checkout.vpay.example/c/cs_1".to_string(),
            stop_urls: vec![CheckoutStopUrl {
                scheme: "https".to_string(),
                host: "shop.example".to_string(),
                port: 443,
                path: "/return".to_string(),
            }],
            allow_insecure_url: false,
            on_event: channel,
        };

        let json = serde_json::to_value(&request).unwrap_or(serde_json::Value::Null);

        assert_eq!(
            json,
            serde_json::json!({
                "url": "https://checkout.vpay.example/c/cs_1",
                "stopUrls": [{ "scheme": "https", "host": "shop.example", "port": 443, "path": "/return" }],
                "allowInsecureUrl": false,
                "onEvent": format!("__CHANNEL__:{id}"),
            })
        );
    }
}
