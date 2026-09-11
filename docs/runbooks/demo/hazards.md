# The demo — §8. The two hazards

_Moved out of [docs/runbooks/demo.md](../demo.md) on 2026-09-11 by exp57, which split a 1 426-line runbook into the procedure and its steps. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a `§` cross-reference pointed at a section that is now on another page, the page it moved to._

## 8. The two hazards

Neither is closed by this demo, and issue #11 asks that they be fixed or made
explicit here. They are made explicit.

### 8.1 The rustls `CryptoProvider` panic — **closed**

`docs/status.md`, row _"rustls `CryptoProvider` process default, for
`authkestra_resource::jwt::Jwks::fetch`"_: **✅, closed 2026-09-02.** Both
binaries call `rustls::crypto::ring::default_provider().install_default()` as
the second thing in `run()`, before tracing init, so no client construction can
precede it. The workspace pins reqwest with `rustls-no-provider`, under which
`ClientBuilder::build()` _panics_ if no process default was installed — an
application may install one, a library may not.

A unit test per binary asserts `CryptoProvider::get_default()` is `Some`
afterwards and that a second call does not panic; emptying the function's body
fails both. `examples/merchant-demo` does the same thing in its own `main()`
for the same reason, and the run pasted above is a process that did it.

What the row still says is weaker than "proven in production": no
containerised `/v1` request had been made when it was written. **The run on
this page is one** — six confirms and thirty-odd authenticated calls through a
`FROM scratch` `vpay-server` container.

### 8.2 RUSTSEC-2023-0071 — **open, accepted, and `ignore`d**

`deny.toml`'s `[advisories] ignore` list, with its reasoning in full at
`deny.toml:14-49`. The "Marvin Attack": a timing side-channel in the `rsa`
crate's PKCS#1 v1.5 _decryption_.

- **There is no patched release.** The advisory has carried no fixed version
  since 2023, and `rsa` is an unconditional, non-optional dependency of
  `authkestra-engine`, which vpay uses to run its own OpenID Provider. It
  cannot be feature-gated away, and `authkestra-op` signs RS256 only.
- **The exposure is on-topic, not incidental**: this is the crate that signs
  the tokens the walkthrough above obtained. What limits it is that the attack
  needs a _decryption_ oracle, and vpay's use is JWT signing and verification.
- **Accepted deliberately by the maintainer on 2026-08-09.** The entry genuinely
  fires — `cargo deny -L info check advisories` reports
  `note[advisory-ignored]` against `rsa v0.9.10`. Revisit if a fixed `rsa`
  appears, if authkestra gains non-RSA signing, or if the Keycloak/ZITADEL
  comparison [ADR-0009](../../adr/0009-dashboard-oidc-provider.md) leaves open is
  carried out.

**Every token in the run above was signed by that crate.** That is the honest
statement of the blast radius on this page.

### 8.3 The authkestra pin, checked

Issue #11's last checklist item says `docs/status.md` cites `=0.3.4` while
`Cargo.toml` says `=0.5.4`. **Checked against the tree on 2026-09-04: the
discrepancy is gone, and neither number is current.** `Cargo.toml` pins all
four crates at `=0.7.1` (`Cargo.toml:257`, `:258`, `:262`, `:264`), and
`docs/status.md` says so — its only `=0.3.4` mention is a historical statement
about where migration `0006`'s DDL was transcribed from, not a claim about the
current pin. Nothing to reconcile; the item is answered by "already done, by
the SDK/authkestra pass".
