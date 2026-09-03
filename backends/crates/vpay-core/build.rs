//! One line of build script, for one reason: put `VPAY_GIT_SHA` into this
//! crate's cargo fingerprint.
//!
//! `vpay_core::metrics::git_sha` reads the variable with `option_env!`, which
//! resolves against the environment *rustc* is invoked with — so
//! `VPAY_GIT_SHA=<sha> cargo build` bakes the value in with no code
//! generation. What `option_env!` does **not** do is tell cargo that the
//! variable is an input: without the line below, changing the sha and
//! rebuilding reuses the cached `vpay-core` and ships the previous build's
//! label, silently. That is the whole failure this file exists to prevent.
//!
//! Deliberately absent:
//!
//! * any `cargo::rustc-env=` output. Passing the value through a second
//!   channel would give the label two sources that can disagree; `option_env!`
//!   already reads the one the operator set.
//! * any call to `git`. `backends/Dockerfile` builds from a `COPY`ed source
//!   tree with no `.git` in it, and a `git rev-parse` that "worked" on a
//!   build machine would stamp a sha describing some other tree. The honest
//!   answer when nobody passed one is `unknown`, and that is what
//!   `git_sha()` returns.
//!
//! `cargo::` (double colon) rather than the older `cargo:` spelling: the
//! workspace pins Rust 1.95 (`rust-toolchain.toml`), well past the 1.77 that
//! introduced it.

fn main() {
    println!("cargo::rerun-if-env-changed=VPAY_GIT_SHA");
}
