//! The plugin's error type.
//!
//! Two rules govern every message in this file, and both come from D6:
//!
//! - **The messages are fixed strings.** Not one of them interpolates the
//!   session URL, the session secret, a stop URL, or a value thrown by
//!   something else. The session URL's fragment *is* the checkout session's
//!   secret, and an error message is the easiest way for it to reach a log
//!   aggregator. `no_error_message_mentions_a_url` pins this.
//! - **Guest-JS maps every rejection to the fixed `platform_window_failed`
//!   anyway**, so a richer message would buy a merchant nothing and cost the
//!   payer their secret. The codes below exist for the merchant developer
//!   reading a devtools console, not for the state machine.
//!
//! The `Serialize` impl renders the message string rather than a tagged
//! object, which is the pattern every plugin in `tauri-apps/plugins-workspace`
//! uses (`plugins/geolocation/src/error.rs`) and what makes `invoke()` reject
//! with a plain `string` on the JS side — the shape the brief's wire contract
//! specifies.

use serde::{Serialize, Serializer};

/// The result type every fallible operation in this crate returns.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything this plugin can refuse or fail to do.
///
/// Marked `#[non_exhaustive]` because a host gaining a new refusal (a
/// desktop dismissal signal, say) must not be a breaking change for a
/// merchant who wrote an exhaustive `match`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A `show` arrived while a checkout window was already in flight.
    ///
    /// Two concurrent checkouts would give the payer two sessions and
    /// guest-JS two polls, and nothing downstream could say which window the
    /// event it just received belonged to. Refusing is the only honest
    /// answer.
    #[error("already_open")]
    AlreadyOpen,

    /// The URL did not parse, or its scheme is neither `http` nor `https`.
    ///
    /// Handing an unvalidated string to the platform's "open this" call is
    /// how a `file://` or a custom scheme reaches the payer's machine; the
    /// desktop host parses first and refuses here.
    #[error("invalid_url")]
    InvalidUrl,

    /// The URL parsed, is `http`, and `allowInsecureUrl` was not set.
    ///
    /// D6: plaintext is a demo-stack affordance and must be asked for by
    /// name. A host never infers it from the scheme it was handed — that
    /// would make the opt-in meaningless.
    #[error("insecure_url")]
    InsecureUrl,

    /// The platform refused to open a browser for the URL.
    ///
    /// Deliberately carries no `io::Error`: the underlying message on some
    /// platforms echoes the command line, and the command line contains the
    /// URL.
    #[error("open_failed")]
    OpenFailed,

    /// Sending the one [`crate::CheckoutWindowEvent`] over the IPC channel
    /// failed — in practice, the webview went away between `show` and
    /// `dismiss`.
    ///
    /// The message is fixed rather than `transparent` on purpose. `tauri::Error`'s
    /// `Display` can quote an evaluated JS payload, and although the event
    /// payload carries no secret today, "today" is not a guarantee worth
    /// spending a payer's session secret on. The cause is still reachable
    /// through [`std::error::Error::source`] for a developer who wants it.
    #[error("channel_send_failed")]
    Tauri(#[from] tauri::Error),

    /// The Kotlin or Swift host rejected the call, or the payload could not
    /// cross the JNI/Objective-C boundary.
    ///
    /// This one *is* `transparent`, and it is the single exception to the
    /// fixed-string rule above. The brief's wire contract names the mobile
    /// rejections by string — `already_open`, `no_activity`, `no_presenter`,
    /// `invalid_url` — and those strings arrive as
    /// `PluginInvokeError::InvokeRejected(ErrorResponse { message, .. })`,
    /// whose `Display` is `[code] - message`
    /// (crates/tauri/src/plugin/mobile.rs lines 135-159 at tag
    /// `tauri-v2.11.6`). Flattening them to one opaque code would delete the
    /// contract on exactly the two platforms that implement all of it.
    ///
    /// **This makes Lanes B and C responsible for D6 on their side**: a
    /// Kotlin or Swift host must reject with one of those fixed tokens and
    /// must never pass the URL, the secret, or a caught exception's message
    /// to `invoke.reject`.
    #[cfg(mobile)]
    #[error(transparent)]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

impl Serialize for Error {
    /// Renders the error as its message string, so `invoke()` rejects with a
    /// `string` rather than an object — the shape guest-JS's host adapter
    /// expects, and the shape every upstream Tauri plugin produces.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four codes the brief fixes for the desktop host, plus the channel
    /// failure. A rename here is a silent break of the guest-JS contract.
    #[test]
    fn the_refusal_codes_are_the_fixed_tokens_the_brief_names() {
        assert_eq!(Error::AlreadyOpen.to_string(), "already_open");
        assert_eq!(Error::InvalidUrl.to_string(), "invalid_url");
        assert_eq!(Error::InsecureUrl.to_string(), "insecure_url");
        assert_eq!(Error::OpenFailed.to_string(), "open_failed");
    }

    /// D6. Every message this crate can produce on its own is a bare token —
    /// no host, no path, no query, no fragment, no scheme.
    #[test]
    fn no_error_message_mentions_a_url() {
        let messages = [
            Error::AlreadyOpen.to_string(),
            Error::InvalidUrl.to_string(),
            Error::InsecureUrl.to_string(),
            Error::OpenFailed.to_string(),
        ];

        for message in messages {
            assert!(!message.contains("://"), "{message}");
            assert!(!message.contains('/'), "{message}");
            assert!(!message.contains('.'), "{message}");
            assert!(
                message.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{message}"
            );
        }
    }

    /// `invoke()` must reject with a JSON string, not `{"AlreadyOpen":null}`.
    #[test]
    fn an_error_serialises_to_its_message_string() {
        assert_eq!(
            serde_json::to_string(&Error::AlreadyOpen).ok(),
            Some("\"already_open\"".to_string())
        );
    }
}
