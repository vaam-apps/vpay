//! Test-only capture of `tracing` output, so an assertion can be made
//! against what an operator would actually see rather than against the
//! arguments this crate passed to a macro.
//!
//! `#[cfg(test)]` in its entirety: nothing here is compiled into a shipping
//! binary. It exists as its own module, rather than as a helper inside one
//! `mod tests`, because two modules now need the same sink — [`error`]'s
//! tests, which pin "the leaf's text reaches the log and never the body",
//! and [`crate`]'s own router tests, which pin "the request id reaches the
//! span enclosing the handler". Two copies of a `MakeWriter` would be two
//! things to keep in step, and the second copy is exactly where a subtly
//! different subscriber configuration (no span fields, a level filter) could
//! make an assertion pass for the wrong reason.
//!
//! [`error`]: crate::error

use std::io;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

/// An in-memory `tracing` sink.
///
/// The `Arc<Mutex<..>>` is not incidental: [`MakeWriter`] hands out a fresh
/// writer per event, so the buffer has to be shared by handle for the
/// caller to read back what every event wrote.
#[derive(Clone, Default)]
pub(crate) struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl CapturedLog {
    /// Everything written so far, lossily decoded.
    ///
    /// A poisoned mutex is read through rather than treated as a failure: a
    /// panic on another thread is a *test* failure that the test's own
    /// assertions will report far more usefully than "poisoned mutex"
    /// would, and losing the captured output at that moment would hide the
    /// log line that explains it.
    pub(crate) fn contents(&self) -> String {
        self.0.lock().map_or_else(
            |poisoned| String::from_utf8_lossy(&poisoned.into_inner()).into_owned(),
            |bytes| String::from_utf8_lossy(&bytes).into_owned(),
        )
    }
}

impl io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.lock() {
            Ok(mut sink) => sink.write(buf),
            Err(poisoned) => poisoned.into_inner().write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLog {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A subscriber writing into `sink`, configured the way these tests need to
/// be able to trust a negative assertion.
///
/// `with_max_level(TRACE)`: a test that asserts a string is *absent* from
/// the log must not be able to pass because the event was filtered out.
/// `with_ansi(false)`: colour escapes would sit between a field name and its
/// value and break a plain `contains`. Span fields are rendered by the
/// default `Full` formatter, which is what lets a caller assert on a field
/// recorded on the enclosing span rather than on the event itself.
pub(crate) fn captured_log_subscriber(
    sink: CapturedLog,
) -> impl tracing::Subscriber + Send + Sync + 'static {
    tracing_subscriber::fmt()
        .with_writer(sink)
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish()
}

/// Captures `tracing` output for the duration of one closure.
///
/// Scoped with `with_default` (thread-local) rather than a global
/// subscriber, so it holds whether the suite runs process-per-test under
/// nextest or threaded under `cargo test`.
///
/// Synchronous by design. An `async` caller cannot use this — a closure
/// returning a future would install the subscriber only while the future is
/// *built*, not while it runs — and must instead hold the guard from
/// [`tracing::subscriber::set_default`] across its `.await` points on a
/// current-thread runtime; see `lib.rs`'s `serve_capturing_log`.
pub(crate) fn with_captured_log<T>(f: impl FnOnce() -> T) -> (T, String) {
    let sink = CapturedLog::default();
    let out = tracing::subscriber::with_default(captured_log_subscriber(sink.clone()), f);
    let captured = sink.contents();
    (out, captured)
}
