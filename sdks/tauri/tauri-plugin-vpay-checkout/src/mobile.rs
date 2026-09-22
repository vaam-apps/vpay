//! The Android and iOS host: a thin forwarder onto the Kotlin and Swift
//! plugin classes.
//!
//! Nothing here decides anything. Both methods serialise their payload and
//! hand it to [`PluginHandle::run_mobile_plugin`]; the state machine — the
//! `already_open` guard, the one-event-per-`show` rule, the partial Custom
//! Tab and the detented `SFSafariViewController` — lives in the native
//! sources that Lanes B and C own. Keeping the Rust side empty is deliberate:
//! a second copy of the guard here could disagree with theirs, and the native
//! one is the one that knows whether a window is actually on screen.
//!
//! # How the callback channel gets there
//!
//! [`crate::ShowCheckoutRequest::on_event`] is a [`tauri::ipc::Channel`], and
//! a `Channel` serialises to the string `"__CHANNEL__:<id>"` — `IPC_PAYLOAD_PREFIX`
//! at crates/tauri/src/ipc/channel.rs line 28, applied by
//! `impl<TSend> Serialize for Channel<TSend>` at lines 132-139 (tag
//! `tauri-v2.11.6`). `app.tauri.plugin.Channel` on Kotlin and `Tauri.Channel`
//! on Swift parse exactly that format out of the invoke payload.
//!
//! Registration is automatic, so this module performs none: every `Channel`
//! is inserted into a process-global map at construction
//! (`Channel::new_with_id` → `#[cfg(mobile)] crate::plugin::mobile::register_channel`,
//! channel.rs lines 265-284; the map is `CHANNELS` at plugin/mobile.rs line
//! 35), including the one Tauri builds for us when it deserialises the
//! command argument. A `channel.send(...)` on the native side comes back
//! through `send_channel_data` (plugin/mobile.rs lines 114-132, Android) or
//! `send_channel_data_handler` (lines 404-424, iOS), which looks the id up in
//! that map and forwards the JSON to the webview.
//!
//! **What Lanes B and C must know:** the payload of `show` is
//! `{ "url": …, "stopUrls": [...], "allowInsecureUrl": bool, "onEvent": "__CHANNEL__:<id>" }`
//! and `dismiss` is invoked with `{}`. The native classes are registered
//! under the names in [`ANDROID_PLUGIN_IDENTIFIER`] / the
//! `init_plugin_vpay_checkout` C entry point below.

use serde::de::DeserializeOwned;
use tauri::plugin::{PluginApi, PluginHandle};
use tauri::{AppHandle, Runtime};

use crate::models::ShowCheckoutRequest;

/// The Kotlin package the `@TauriPlugin`-annotated class lives in.
///
/// Tauri's Android loader concatenates this with the class name, so the
/// Kotlin source Lane B writes must declare exactly
/// `package dev.vpay.tauri.checkout` and exactly `class VpayCheckoutPlugin`.
/// A mismatch is a run-time `ClassNotFoundException`, not a compile error.
#[cfg(target_os = "android")]
const ANDROID_PLUGIN_IDENTIFIER: &str = "dev.vpay.tauri.checkout";

// Declares the `init_plugin_vpay_checkout` symbol that the Swift package's
// `@_cdecl("init_plugin_vpay_checkout")` function defines (Lane C). A `//`
// comment rather than a `///` one: rustdoc generates nothing for a macro
// invocation, so a doc comment here is an `unused_doc_comments` warning.
#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_vpay_checkout);

/// Access to the checkout window on Android and iOS.
///
/// Managed by [`crate::init`] and reachable as `app.vpay_checkout()`.
pub struct VpayCheckout<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> VpayCheckout<R> {
    /// Asks the native host to present the checkout browser.
    ///
    /// Resolves once the window is showing; the outcome arrives later on
    /// `request.on_event`, exactly once.
    ///
    /// # Errors
    ///
    /// [`crate::Error::PluginInvoke`], carrying the native rejection
    /// verbatim: `already_open` when a window is already up, `no_activity`
    /// (Android) / `no_presenter` (iOS) when there is nothing to present
    /// from, `invalid_url` when the URL will not parse. Those tokens are the
    /// brief's, and Lanes B and C are bound to them.
    pub fn show(&self, request: ShowCheckoutRequest) -> crate::Result<()> {
        self.0
            .run_mobile_plugin("show", request)
            .map_err(Into::into)
    }

    /// Asks the native host to close the checkout browser if one is open.
    ///
    /// A no-op otherwise. The native side, not this one, is what knows
    /// whether a window exists.
    ///
    /// # Errors
    ///
    /// [`crate::Error::PluginInvoke`] if the call could not cross the
    /// JNI/Objective-C boundary.
    pub fn dismiss(&self) -> crate::Result<()> {
        self.0.run_mobile_plugin("dismiss", ()).map_err(Into::into)
    }
}

/// Registers the Kotlin or Swift plugin class and returns the handle.
/// Called from [`crate::init`]'s `setup`.
///
/// # Errors
///
/// [`crate::Error::PluginInvoke`] if the native class cannot be found or
/// instantiated — which is what a typo in the Kotlin package name or a
/// missing `@_cdecl` symbol looks like at run time.
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<VpayCheckout<R>> {
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin(ANDROID_PLUGIN_IDENTIFIER, "VpayCheckoutPlugin")?;
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_vpay_checkout)?;

    Ok(VpayCheckout(handle))
}
