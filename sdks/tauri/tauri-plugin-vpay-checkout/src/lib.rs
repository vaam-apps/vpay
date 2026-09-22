//! `tauri-plugin-vpay-checkout` — the host half of vpay's Tauri v2 payer
//! checkout surface.
//!
//! # What this is
//!
//! It opens vpay's **hosted checkout page in the payer's own browser** — a
//! partial Custom Tab on Android, a detented `SFSafariViewController` on iOS,
//! the platform's default browser on desktop — and reports, over one IPC
//! channel and exactly once per `show`, that the window closed or that a deep
//! link matching a configured stop URL arrived. That is the entire vocabulary.
//!
//! Three consequences, each of which is a design decision and not an
//! omission:
//!
//! - **Browser, not WebView.** No `WebView`, `WKWebView` or Tauri
//!   `WebviewWindow` appears in the payment path (D5, revised 2026-09-16).
//!   Rendering vpay's payment form inside the merchant app's own process puts
//!   `evaluateJavascript`, the cookie store and a navigation delegate within
//!   reach of that app, i.e. lets a compromised merchant app read the payer's
//!   PAN and OTP without the payer being able to tell. The browser's process
//!   cannot be inspected that way, and the payer gets a real URL bar. The
//!   cost is stated rather than hidden: no host can see a navigation any
//!   more, so stop URLs arrive only as deep links.
//! - **Poll, not URL.** No outcome is ever read off a URL, a message payload
//!   or an event (D1). `dismissed` is the *ordinary* end of a successful
//!   payment since the browser cutover, which is why the JavaScript half
//!   answers by polling `GET /v1/browser/payment_intents/{id}` (D4). Nothing
//!   in this crate knows what `succeeded` means, and the
//!   [`CheckoutWindowOutcome`] enum has no member for it.
//! - **No merchant credential, ever.** This plugin holds one secret — the
//!   session URL it was handed, whose fragment is the checkout session's
//!   secret — for exactly as long as the window is open, and it never logs,
//!   `Display`s or `Debug`s it (D6; see [`ShowCheckoutRequest`]'s hand-written
//!   `Debug`). It never sees a merchant API key, and no code path here can
//!   authenticate to vpay's merchant `/v1` surface.
//!
//! # Where the rest of it is
//!
//! The state machine — pre-flight, stop-URL derivation, the poll, the
//! terminal rule — is the guest-JS package `@vaam-apps/vpay-tauri-checkout`,
//! not this crate. This crate is the window, and the window is deliberately
//! stupid.
//!
//! The contract both halves implement is
//! `docs/plans/2026-09-22-tauri-plugin-brief.md`; the decision it inherits
//! unchanged is `docs/adr/0021-flutter-checkout-plugin.md` and the Flutter
//! plugin's D1–D9.
//!
//! # Registering it
//!
//! ```
//! # fn main() {
//! let plugin = tauri_plugin_vpay_checkout::init::<tauri::Wry>();
//! assert_eq!(tauri::plugin::Plugin::name(&plugin), "vpay-checkout");
//! # }
//! ```
//!
//! In a real application that goes into the builder, and the app's capability
//! file has to grant the two commands:
//!
//! ```text
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_vpay_checkout::init())
//!     .run(tauri::generate_context!())
//!
//! # src-tauri/capabilities/default.json
//! { "permissions": ["vpay-checkout:default"] }
//! ```

mod commands;
mod error;
mod models;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

pub use error::{Error, Result};
pub use models::{
    CheckoutStopUrl, CheckoutWindowEvent, CheckoutWindowOutcome, ShowCheckoutRequest,
};

#[cfg(desktop)]
pub use desktop::VpayCheckout;
#[cfg(mobile)]
pub use mobile::VpayCheckout;

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Reaches the managed [`VpayCheckout`] from anything that is a
/// [`tauri::Manager`] — `App`, `AppHandle`, `Window`, `Webview`,
/// `WebviewWindow`.
///
/// It exists so that application code can drive the window from Rust (a
/// native "pay" button, an integration test) without going through the JS
/// IPC, and so that the two `#[tauri::command]`s have one way of reaching
/// the host on both desktop and mobile.
pub trait VpayCheckoutExt<R: Runtime> {
    /// The checkout host for this application.
    fn vpay_checkout(&self) -> &VpayCheckout<R>;
}

impl<R: Runtime, T: Manager<R>> VpayCheckoutExt<R> for T {
    fn vpay_checkout(&self) -> &VpayCheckout<R> {
        self.state::<VpayCheckout<R>>().inner()
    }
}

/// Builds the plugin. Pass the result to `tauri::Builder::plugin`.
///
/// The identifier is `vpay-checkout`, which fixes the two invoke names
/// guest-JS uses — `plugin:vpay-checkout|show` and
/// `plugin:vpay-checkout|dismiss` — and the permission namespace an app's
/// capability file names (`vpay-checkout:default`). Renaming it would break
/// the JavaScript half and every merchant's capability file at once, which is
/// why the string is written here exactly once.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("vpay-checkout")
        .invoke_handler(tauri::generate_handler![commands::show, commands::dismiss])
        .setup(|app, api| {
            #[cfg(mobile)]
            let checkout = mobile::init(app, api)?;
            #[cfg(desktop)]
            let checkout = desktop::init(app, api)?;
            app.manage(checkout);
            Ok(())
        })
        .build()
}
