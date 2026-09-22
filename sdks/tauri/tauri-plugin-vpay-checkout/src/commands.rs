//! The two `#[tauri::command]`s guest-JS invokes:
//! `plugin:vpay-checkout|show` and `plugin:vpay-checkout|dismiss`.
//!
//! # Why the arguments are flat rather than one `request` object
//!
//! The brief fixes the `show` payload as four **top-level** camelCase keys —
//! `url`, `stopUrls`, `allowInsecureUrl`, `onEvent` — and Tauri derives a
//! command's JS argument names from its Rust parameter names (camelCased by
//! default: `ArgumentCase::Camel` in tauri-macros' `command/wrapper.rs`), so
//! a single `request: ShowCheckoutRequest` parameter would put every key one
//! level down under `{ "request": … }` and break that contract.
//!
//! It could not have been written that way in any case: `tauri::ipc::Channel`
//! implements `CommandArg` — which reaches into the `Webview` to build the
//! callback — but **not** `Deserialize`, because a channel id alone is not
//! enough to make a channel. Upstream's own guidance is to use
//! `JavaScriptChannelId` when a channel must sit inside a JSON object
//! (crates/tauri/src/ipc/channel.rs lines 141-165, tag `tauri-v2.11.6`), and
//! that would have added a second request type on the wire for no gain. The
//! flat signature gives the brief's keys exactly and reassembles
//! [`ShowCheckoutRequest`] on this side of the boundary, where the host
//! implementations want it.

use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime};

use crate::models::{CheckoutStopUrl, CheckoutWindowEvent, ShowCheckoutRequest};
use crate::{Result, VpayCheckoutExt};

/// `plugin:vpay-checkout|show` — opens the payer's browser on the session's
/// hosted URL.
///
/// Resolves with no value once the window is showing. The single outcome
/// arrives later on `on_event`; this command never reports one, because no
/// window may decide what a payment did (D1).
///
/// # Errors
///
/// Rejects with the host's fixed token string — `already_open`,
/// `invalid_url`, `insecure_url`, `open_failed` on desktop; `already_open`,
/// `no_activity`/`no_presenter`, `invalid_url` from the native hosts. Never
/// with anything that quotes the URL (D6); guest-JS maps them all to
/// `platform_window_failed` regardless.
#[tauri::command]
pub(crate) async fn show<R: Runtime>(
    app: AppHandle<R>,
    url: String,
    stop_urls: Vec<CheckoutStopUrl>,
    allow_insecure_url: bool,
    on_event: Channel<CheckoutWindowEvent>,
) -> Result<()> {
    app.vpay_checkout().show(ShowCheckoutRequest {
        url,
        stop_urls,
        allow_insecure_url,
        on_event,
    })
}

/// `plugin:vpay-checkout|dismiss` — ends the in-flight checkout.
///
/// Takes no arguments and resolves with no value. Closes the window where a
/// host can (Android, iOS) and reports `dismissed` on the in-flight channel;
/// a no-op when nothing is in flight, so guest-JS's cleanup path may call it
/// unconditionally.
///
/// # Errors
///
/// Rejects if the event could not be delivered to the webview, or if the
/// native call could not be made.
#[tauri::command]
pub(crate) async fn dismiss<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.vpay_checkout().dismiss()
}
