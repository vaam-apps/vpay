//! The provider port: the single interface every payment rail implements.
//!
//! The core decides what a payment *means*; an adapter decides how to say it on
//! the wire. If `if provider == "mtn_momo"` appears anywhere outside an adapter
//! crate, the port is wrong — fix the port, not the caller.
//!
//! See `docs/adr/0002-provider-port.md` and `docs/flows/provider-port.md`.
//!
//! Three modules carry what belongs to *every* rail rather than to any one
//! of them, so that no adapter holds a copy of a cross-rail concern:
//! [`http`] (the outbound client and the bounded body read), [`token`] (the
//! bearer cache and its credential fingerprint) and [`measured`] (the counter
//! and histogram a rail call is recorded on). `docs/reference/rails.md` says
//! why each lives here and what each adapter deliberately keeps for itself.
//!
//! `#[warn(clippy::missing_errors_doc)]` is on the crate rather than in
//! `Cargo.toml`: cargo refuses `[lints.clippy]` beside `[lints] workspace =
//! true` ("cannot override `workspace.lints` in `lints`"), and copying the
//! workspace's thirteen lints in here to add a fourteenth is how the two
//! sets drift. Every public fallible item in this crate therefore carries an
//! `# Errors` section, and `cargo clippy -- -D warnings` fails if one stops.
#![warn(clippy::missing_errors_doc)]

pub mod http;
pub mod measured;
pub mod token;

pub use measured::Measured;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vpay_core::{FailureCode, Money, ProviderFlow};

/// Where a refund on this rail sends the money.
///
/// The capability the core branches on so that "does this refund need a
/// payee?" never becomes `if provider == "mtn_momo"` (ADR-0002). It is a
/// fact about the *rail's product*, not about vpay's progress on it: a rail
/// whose refund call this repository has not written still declares the
/// destination its refund would need, because that is what the merchant-facing
/// validation is derived from and it does not change when the adapter is
/// finished.
///
/// `rename_all` for [`Capabilities`]' reason — this is vpay's own shape, not
/// a rail's — and `cargo xtask verify-serde` refuses a serialisable type in
/// this crate that spells it any other way.
///
/// There is deliberately **no** third variant for "this rail cannot refund at
/// all". That is [`Capabilities::supports_refunds`], and duplicating it here
/// would create two places to answer one question and a way for them to
/// disagree. See [`Capabilities::refund_destination`] for what the value means
/// on a rail that refunds nothing.
///
/// RFC-0003 § 1 is the design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundDestination {
    /// Money returns to the instrument that paid and the rail needs no payee
    /// — a card, a wallet.
    ///
    /// The core refuses a refund request that *carries* a [`RefundTarget`]
    /// on such a rail. Accepting one and dropping it would tell a merchant
    /// their nominated payee had been honoured when the rail was never told
    /// about it.
    Origin,
    /// Returning money is an outbound transfer and the rail needs an explicit
    /// payee — every mobile-money rail vpay carries.
    ///
    /// The core refuses a refund request without a [`RefundTarget`] before it
    /// calls [`ProviderAdapter::refund`], so an adapter on such a rail is
    /// entitled to assume the `destination` it is handed is `Some`.
    Required,
}

/// Static declaration of what a rail can do.
///
/// The core reads these instead of special-casing a provider code.
///
/// `rename_all` because this is *vpay's* own shape, not a rail's: it is
/// projected into `/v1` responses and into configuration, and the attribute
/// is what keeps a field added as `supportsRefunds` from becoming public
/// API. The adapters' `wire.rs` types carry the opposite rule, for the
/// opposite reason — see either one's module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Capabilities {
    pub flow: ProviderFlow,
    pub supports_refunds: bool,
    pub supports_partial_refunds: bool,
    pub delivers_callbacks: bool,
    pub requires_ip_allowlist: bool,
    /// Whether this rail exposes a registered-account-holder name for a
    /// payer reference at all.
    ///
    /// The flag the core branches on so that refusing a lookup never needs
    /// `if provider == "…"` (ADR-0002), exactly as `supports_refunds` does.
    /// It is a statement about the *rail*, not about vpay: a rail that has
    /// the API but whose adapter has not written it declares `true` and
    /// overrides [`ProviderAdapter::account_holder_name`] with its own
    /// [`ProviderError::NotImplemented`] token, so `verify-status` sees the
    /// gap rather than `Unsupported` hiding it as a fact about MTN.
    ///
    /// Deliberately **not** persisted: unlike the four flags above it has no
    /// column in `providers` (migration `0002`) and no field on
    /// `vpay_db::ProviderSeed`. Nothing reads a capability out of that table
    /// — `vpay_api` resolves an adapter in-process and asks it — so a column
    /// would be a second copy of an answer the linked code already owns, and
    /// a migration on the strength of it would claim a durability this
    /// capability does not need. `docs/flows/account-holder-lookup.md`
    /// records the decision.
    pub supports_account_holder_lookup: bool,
    /// Whether a refund on this rail needs a payee, and therefore whether the
    /// core demands, refuses or ignores a `destination` on `POST /v1/refunds`
    /// (RFC-0003 § 1).
    ///
    /// **Read only when [`supports_refunds`](Capabilities::supports_refunds)
    /// is true.** On a rail that cannot refund at all the value is inert —
    /// no code path reaches it — and it is still required to be a truthful
    /// statement about the rail's refund product, because that is the field
    /// the moment `supports_refunds` flips. `orange_money` is exactly that
    /// case today: it declares [`RefundDestination::Required`] while
    /// `supports_refunds` is still `false`, because an Orange refund *is* a
    /// transfer back (RFC-0003 § 5) and the flag is what is behind, not the
    /// destination.
    ///
    /// That is also why **no coherence rule pairs with this field** — see
    /// [`Capabilities::is_coherent`], which says why at length.
    ///
    /// Deliberately **not persisted**, on
    /// [`supports_account_holder_lookup`](Capabilities::supports_account_holder_lookup)'s
    /// terms: no column in `providers`, no field on `vpay_db::ProviderSeed`.
    /// Nothing reads a capability out of that table — `vpay_api` resolves an
    /// adapter in-process and asks it — so a column would be a second copy of
    /// an answer the linked code already owns.
    pub refund_destination: RefundDestination,
}

impl Capabilities {
    /// Invariant mirrored by a CHECK constraint in the schema: see
    /// `partial_refunds_imply_refunds` in
    /// `backends/migrations/0002_create-providers.sql`, proven to fire by
    /// `partial_refunds_without_refunds_is_rejected_by_the_database` in
    /// `backends/tests/integration/tests/postgres_smoke.rs`. This Rust check
    /// runs independently — belt and braces, not a substitute for the DB
    /// constraint or vice versa.
    ///
    /// # Why [`refund_destination`](Capabilities::refund_destination) is not
    /// in this rule
    ///
    /// The obvious candidate — "a rail that cannot refund must declare
    /// [`RefundDestination::Origin`]" — was considered and refused, for three
    /// reasons (RFC-0003 § 1):
    ///
    /// 1. **It is false on a live rail.** `orange_money` declares `Required`
    ///    with `supports_refunds: false`: the rail's refunds *are* transfers
    ///    and it is vpay that has not built them. A rule demanding `Origin`
    ///    there would force the declaration to lie, and would flip back the
    ///    day `supports_refunds` does.
    /// 2. **Neither value means anything when refunds are off.** `Origin` is
    ///    no more "the unread value" than `Required` is, so the rule would be
    ///    picking an arbitrary sentinel and calling it coherence.
    /// 3. **There is nothing to mirror it with.** The rule above is one half
    ///    of a pair whose other half is a CHECK constraint on `providers`,
    ///    and `refund_destination` has no column there and is not meant to
    ///    (see the field). A Rust-only "coherence" rule with no database half
    ///    would be a different kind of thing wearing this method's name.
    ///
    /// Reason 1 has a shelf life and is the weakest of the three: it stops
    /// being true the day RFC-0003 § 5 flips `orange_money`'s
    /// `supports_refunds`, and nothing fires on *this* text when it does —
    /// the tripwire in that adapter's tests fires on the flag. The decision
    /// survives on reasons 2 and 3, which do not depend on what any rail
    /// declares today. Reason 2 in particular is an argument about the
    /// field's **type** rather than about this method: a two-variant enum has
    /// no honest value for a rail with no refund product at all, which an
    /// `Option<RefundDestination>` would, and that is a modelling question
    /// RFC-0003 does not answer and this change deliberately did not decide.
    ///
    /// `a_refund_destination_is_inert_to_coherence` pins that decision, and
    /// what would have to change before it may be reversed.
    #[must_use]
    pub const fn is_coherent(&self) -> bool {
        !self.supports_partial_refunds || self.supports_refunds
    }
}

/// Rail-supplied key material the core must persist to query status later —
/// Orange's `pay_token`, for instance. Opaque to the core.
pub type RefExtra = BTreeMap<String, String>;

/// What a charge looks like to an adapter. Deliberately minimal.
#[derive(Debug, Clone)]
pub struct ChargeRef {
    /// The reference *we* generated, durable before any network call.
    pub reference_id: Uuid,
    pub amount: Money,
    /// Payer instrument. `None` on redirect rails, where the payer
    /// authenticates with the rail and we may never learn who they are.
    pub payer_ref: Option<String>,
    /// Rail key material captured from a previous `submit`, if any.
    pub ref_extra: RefExtra,
    /// Where a redirect rail must send the payer's browser when its own page
    /// is done with them. `None` on a push rail, which has no browser.
    ///
    /// **The core fills this, never an adapter** (D2 of
    /// `docs/plans/2026-09-04-step9-hosted-checkout.md`). It is one of two
    /// values, decided per charge by `vpay_api`'s confirm path:
    ///
    /// * the vpay page that receives the payer for the **checkout session**
    ///   driving this charge, when one does — `{checkout.public_base_url}/c/{cs_id}/return?t=…`;
    /// * otherwise the **merchant's own** `charges.return_url`, the URL they
    ///   sent on `confirm` and which is echoed back to them as
    ///   `next_action.redirect_to_url.return_url`.
    ///
    /// Before this field existed, `vpay-adapter-orange-money` answered a
    /// per-charge question out of *deployment* configuration
    /// (`settings.return_url`, falling back to the callback URL), so every
    /// payer on a deployment was returned to the same place whatever the
    /// merchant had asked for. Interpolating the charge into `ProviderConfig`
    /// instead was rejected: it would make deployment configuration
    /// charge-dependent.
    ///
    /// An adapter on a push rail must ignore it — see
    /// `the_submit_tells_the_rail_where_to_send_the_payer_back` in
    /// `backends/tests/conformance`, which asserts MTN's body carries no
    /// return URL at all.
    pub return_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Submitted {
    /// Key material the core must commit. On a redirect rail this MUST be
    /// committed before `redirect_url` is handed to anyone.
    pub ref_extra: RefExtra,
    /// Present iff the rail's flow is [`ProviderFlow::Redirect`].
    pub redirect_url: Option<String>,
}

/// Where a [`RefundDestination::Required`] rail must send the money.
///
/// # Opaque to the core once it exists — but the core is what builds one
///
/// After construction the core's entire business with one of these is
/// **presence**: it checks that a refund on a `Required` rail carries one and
/// that a refund on an `Origin` rail does not, and hands it through
/// untouched. The adapter reads the interior and renders it onto its own
/// wire. A core code path that branched on what is inside would have put the
/// rail's shape back in the core, which is what ADR-0002 exists to prevent.
///
/// The analogy with [`RefExtra`] stops one step short of that, and the gap is
/// worth naming because it is where the *next* shape gets decided.
/// `RefExtra` is untyped and travels adapter → core → adapter, so a rail with
/// new key material costs no change to this crate and none to the core. A
/// destination travels merchant → core → adapter, so something has to turn
/// `destination[<rail_code>][…]` into one of these, and today that something
/// is the core: [`mobile_money`](RefundTarget::mobile_money) is called from
/// `vpay_api`. A non-mobile-money upstream therefore costs a **core** change
/// — not a rail-code branch, so ADR-0002 still holds, but a core that has
/// learned a second set of wire keys.
///
/// The alternative, if that is judged wrong, is the symmetry
/// [`ProviderAdapter::parse_callback`] already has: the adapter parses its
/// own wire shape into the port's type and the core hands over an
/// uninterpreted sub-map, which would make a second destination shape
/// genuinely zero-core-change. That is **not decided here.** It is RFC-0003
/// open question 3, it belongs with whoever has the upstream that needs it,
/// and RFC-0003 § 2's `POST /v1/refunds` is the first code that has to build
/// one of these and so the first place the cost is real.
///
/// # Why one shape, and no bank-account variant
///
/// RFC-0003 open question 3 is unanswered: designing a non-mobile-money
/// destination now means modelling an upstream nobody has chosen, and the
/// shape that gets guessed is the shape every adapter then has to match. Both
/// rails vpay carries are mobile money, so this carries a canonical MSISDN
/// and nothing else. A second shape arrives as a second constructor and a
/// second accessor, added by whoever has a rail that needs one — the change
/// is breaking either way, and it is more honest made against a real rail
/// than guessed at here.
///
/// # Why the field is private, and why [`Debug`] redacts
///
/// A refund destination is a third party's phone number. RFC-0003 rejected
/// carrying it in `metadata` on exactly that ground: metadata is deliberately
/// out of reach of customer erasure (`docs/api/README.md`) and is already
/// inside every signed webhook vpay delivers, so a payer's number parked
/// there outlives an erasure request. A type that left the number one `{:?}`
/// away from a log line would reintroduce the leak the wire shape was chosen
/// to avoid — and its retention is RFC-0003 open question 2, **undecided**,
/// which is the strongest possible argument for keeping the number out of
/// places nobody decided to put it. [`AccountHolder`] carries the same
/// treatment for the same reason.
///
/// There is deliberately no `Serialize`/`Deserialize` derive either. A
/// destination reaches this type from parsed form parameters, and a serde
/// impl would make "persist it" and "put it in an event payload" one derive
/// away while the retention question is still open.
#[derive(Clone, PartialEq, Eq)]
pub struct RefundTarget {
    msisdn: String,
}

/// Redacts the number, on [`AccountHolder`]'s terms and for its reason.
///
/// A masked number for an operator — `+2376••••200` — is written once, by the
/// `/v1` layer that knows a merchant asked, exactly as
/// `docs/flows/account-holder-lookup.md` has it. This impl is what stops a
/// `tracing::debug!(?destination)` anywhere else from printing the whole
/// thing, including through an enclosing `Some(..)` or `Ok(..)`.
impl std::fmt::Debug for RefundTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefundTarget")
            .field("msisdn", &"[redacted]")
            .finish()
    }
}

impl RefundTarget {
    /// Builds the mobile-money destination: a payee's MSISDN.
    ///
    /// Infallible, on [`AccountHolder::new`]'s terms — the caller supplies a
    /// number it has **already canonicalised**. The port cannot do that
    /// itself: the server-side MSISDN rule lives in `vpay_api`, which depends
    /// on this crate, and a second copy of a validation rule is how the two
    /// spellings drift apart.
    ///
    /// Named for the shape rather than `new` so that a future non-mobile-money
    /// destination is a sibling constructor, not a silent change of meaning to
    /// this one.
    #[must_use]
    pub fn mobile_money(msisdn: impl Into<String>) -> Self {
        Self {
            msisdn: msisdn.into(),
        }
    }

    /// The payee's MSISDN, as the adapter must spell it on its rail.
    ///
    /// # Errors
    ///
    /// None — but the caller inherits an obligation the type cannot enforce,
    /// [`AccountHolder::name`]'s: this is a third party's phone number and
    /// must not be logged unmasked, nor used as a metric label.
    ///
    /// ```
    /// use vpay_provider::RefundTarget;
    ///
    /// let destination = RefundTarget::mobile_money("+237600000000");
    /// assert_eq!(destination.msisdn(), "+237600000000");
    /// // Debug redacts: a `{:?}` of a destination — or of anything holding
    /// // one, such as the `Option` the port passes it in — never prints it.
    /// assert_eq!(
    ///     format!("{destination:?}"),
    ///     r#"RefundTarget { msisdn: "[redacted]" }"#
    /// );
    /// assert!(!format!("{:?}", Some(&destination)).contains("237600000000"));
    /// ```
    #[must_use]
    pub fn msisdn(&self) -> &str {
        &self.msisdn
    }
}

/// What a rail answered when asked to return money.
///
/// # Why not [`Submitted`]
///
/// `refund` returned `Submitted` until 2026-09-05 (issue #46). That type
/// carries `redirect_url`, which a refund can never have — no payer's
/// browser is involved in giving money back — so every adapter had to
/// answer `None` to a question nobody could ask. Splitting the two is what
/// lets [`fee`](Refunded::fee) exist here without also appearing on the
/// charge path, where the rail's charge fee is a *different* number with a
/// different owner.
#[derive(Debug, Clone)]
pub struct Refunded {
    /// Key material the core must commit, exactly as on [`Submitted`]: a
    /// refund is addressed on the rail by a reference this side generated
    /// before the call (`docs/flows/crash-safety.md`).
    pub ref_extra: RefExtra,
    /// What the rail charged **us** to move the money, in the refund's own
    /// currency — never a second currency, and never a float
    /// (`docs/flows/money.md`).
    ///
    /// `None` means the rail did not report a fee, and `Some(zero)` means it
    /// reported that the movement was free. Those are different answers and
    /// must stay different all the way to the merchant's `refund.fee`: an
    /// adapter that collapsed "unknown" into `0` would put an invented number
    /// in a settlement statement, which is the exact failure issue #46 was
    /// filed about (an integrator hardcoding `provider_fee_minor: 0`).
    ///
    /// **No adapter populates this today**, and none may until it has a real
    /// rail response carrying a fee — see `docs/status.md`. `Money` rather
    /// than `i64` so the currency travels with the amount and a fee in the
    /// wrong currency cannot be silently rendered as minor units of the
    /// refund's.
    pub fee: Option<Money>,
}

/// The authoritative status read. Never derived from a callback body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChargeStatus {
    Pending,
    Succeeded {
        provider_txn_id: Option<String>,
    },
    Failed {
        code: FailureCode,
        raw: String,
    },
    /// The rail has no record. Never on its own grounds to fail a charge —
    /// see `docs/flows/crash-safety.md`.
    NotFound,
}

/// The registered holder of a payer reference on a rail — **a name, and
/// nothing else**.
///
/// # Why a struct with one field, and why that field is all there is
///
/// MTN's `basicuserinfo` answers an OIDC-shaped body: `given_name`,
/// `family_name`, `birthdate`, `locale`, `gender`, `status`
/// (`docs/flows/adapter-mtn-momo.md`). Every field but the two names is
/// personal data vpay has no use for, so the *port* is where the projection
/// happens: an adapter that deserialised the whole body into a type this
/// crate exposed would put a third party's date of birth one `{:?}` away
/// from a log line, and no amount of care at the call sites would take it
/// back out. Returning `String` would have said the same thing, but a named
/// type is what makes "we return a name" a fact a reader of the trait can
/// see, and gives the redacting [`Debug`] below somewhere to live.
///
/// The one field is private, so the only way to read it is
/// [`AccountHolder::name`] — a call site that wants the name has to ask for
/// it by name, which is the point.
#[derive(Clone, PartialEq, Eq)]
pub struct AccountHolder {
    name: String,
}

/// Redacts the name, for [`ProviderConfig`]'s reason: a value that lives
/// inside a `Result` a handler may log is one `{:?}` from a log line, and a
/// third party's name in a log is exactly what
/// `docs/flows/account-holder-lookup.md` forbids.
///
/// This is belt, not braces: the route logs no name at all, and
/// `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing`
/// in the conformance suite is
/// what proves it. This impl is what stops a *future* `tracing::debug!(?holder)`
/// from silently undoing that.
impl std::fmt::Debug for AccountHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountHolder")
            .field("name", &"[redacted]")
            .finish()
    }
}

impl AccountHolder {
    /// The registered name, as the rail spells it.
    ///
    /// # Errors
    ///
    /// None — but the caller inherits an obligation the type cannot enforce:
    /// this value is a third party's name and must not be logged, stored or
    /// counted as a metric label.
    ///
    /// ```
    /// use vpay_provider::AccountHolder;
    ///
    /// let holder = AccountHolder::new("David Mbarga");
    /// assert_eq!(holder.name(), "David Mbarga");
    /// // Debug redacts: a `{:?}` of a holder — or of anything holding one,
    /// // such as an `Ok(Some(..))` — can never print the name.
    /// assert_eq!(format!("{holder:?}"), r#"AccountHolder { name: "[redacted]" }"#);
    /// assert!(!format!("{:?}", Some(holder)).contains("David"));
    /// ```
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Builds one from a name an adapter has already projected out of its
    /// rail's body.
    ///
    /// Public because an adapter is a separate crate; there is deliberately
    /// no constructor taking a whole rail response, so the projection
    /// happens in the adapter that knows the rail's shape and the discarded
    /// fields never cross this boundary at all.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Identifiers extracted from a callback. Deliberately *not* a status: the core
/// will not trust anything read off an unauthenticated request.
#[derive(Debug, Clone)]
pub struct CallbackRef {
    pub reference_id: Uuid,
    /// May repair a charge whose `ref_extra` write was lost.
    pub ref_extra: RefExtra,
}

/// The foreign error an adapter is holding when it gives up on a rail.
///
/// A closed enum, not `Box<dyn Error>`: ADR-0011 says a foreign error is
/// wrapped with `#[source]` and never boxed, and there are exactly two an
/// adapter can be holding at that point.
///
/// Each variant's `Display` names *which stage* failed and nothing else —
/// deliberately not `#[error(transparent)]`, which would forward `source()`
/// past the library error and put it out of reach of a downcast. Rendered
/// through [`vpay_core::error::source_chain`] a timeout reads "sending the
/// request: error sending request for url (…): operation timed out", where
/// `reqwest`'s own `Display` stops at the first of those.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RailFailure {
    /// DNS, connect, TLS, or a deadline from [`ProviderConfig`].
    #[error("sending the request")]
    Http(#[from] reqwest::Error),
    /// The response body could not be read within its bound.
    #[error("reading the response")]
    Body(#[from] http::HttpBodyError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The rail could not be reached, or could not be finished with.
    ///
    /// `context` is the adapter's own sentence — which rail, which
    /// operation, which status — and is what `Display` renders. `source` is
    /// the library error underneath, when there is one; a rail that
    /// *answered* `HTTP 500` produces no foreign error and leaves it `None`.
    #[error("transport error talking to the rail: {context}")]
    Transport {
        /// Which rail, doing what. Never a secret.
        context: String,
        /// The `reqwest`/body error this was raised from, if any.
        #[source]
        source: Option<RailFailure>,
    },
    #[error("rail rejected the request: {code} — {message}")]
    Rejected { code: FailureCode, message: String },
    /// The rail answered something this adapter cannot act on.
    ///
    /// Same two fields, same rule, as [`ProviderError::Transport`].
    #[error("could not parse the rail's response: {context}")]
    Malformed {
        /// What could not be parsed, and by which rail. Never a secret.
        context: String,
        /// The body error this was raised from, if any. A `serde_json`
        /// failure is *not* one: the parse error's own text is already the
        /// whole diagnostic and belongs in `context`, where an operator
        /// reading a log line sees it.
        #[source]
        source: Option<RailFailure>,
    },
    #[error("adapter configuration is invalid: {0}")]
    Config(String),
    /// Returned by an operation this rail does not support.
    ///
    /// Two facts, and they decide different columns. The *request* cannot
    /// proceed and never will on this rail, so the category is
    /// [`vpay_core::Category::Conflict`] — 409, and a merchant reading the
    /// envelope learns the truth. But the core is supposed to branch on
    /// [`Capabilities`] before it ever calls (ADR-0002), so reaching this
    /// arm at all means *our* check was skipped: the severity is therefore
    /// overridden to [`vpay_core::Severity::Error`] rather than the
    /// `Conflict` default of `Info`, so it shows up in a log an operator
    /// reads instead of being counted alongside merchants' typos.
    #[error("operation not supported by this rail")]
    Unsupported,
    /// Not yet built. This is NOT a mock: it never pretends to succeed.
    /// Every occurrence must appear in `docs/status.md`.
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
}

impl vpay_core::Classify for RailFailure {
    /// Both stages are the rail's behaviour, never ours.
    ///
    /// It is only ever reached through a [`ProviderError::Transport`] or
    /// [`ProviderError::Malformed`], both of which are already
    /// [`vpay_core::Category::Rail`], so this exists to say the same thing
    /// where `cargo xtask verify-errors` can check it rather than to give a
    /// boundary a second opinion — matching
    /// [`http::HttpBodyError`]'s own impl, which classifies the inner half
    /// of `RailFailure::Body` identically.
    fn category(&self) -> vpay_core::Category {
        vpay_core::Category::Rail
    }
}

impl ProviderError {
    /// A transport failure with no foreign error behind it — a rail that
    /// answered, badly.
    ///
    /// ```
    /// use vpay_provider::ProviderError;
    ///
    /// let error = ProviderError::transport("mtn_momo: requesttopay answered HTTP 503");
    /// assert_eq!(
    ///     error.to_string(),
    ///     "transport error talking to the rail: mtn_momo: requesttopay answered HTTP 503"
    /// );
    /// assert!(
    ///     std::error::Error::source(&error).is_none(),
    ///     "a rail that answered has no library error behind it to attach"
    /// );
    /// ```
    #[must_use]
    pub fn transport(context: impl Into<String>) -> Self {
        Self::Transport {
            context: context.into(),
            source: None,
        }
    }

    /// A transport failure raised *from* a `reqwest` or body error, which
    /// stays reachable through [`std::error::Error::source`].
    ///
    /// `Display` still renders `context` alone. That is the point: the
    /// sentence stays one line an operator can read, and the leaf reaches a
    /// log through [`vpay_core::error::source_chain`] rather than by being
    /// `format!`ed into the sentence, where nothing could match on it again.
    ///
    /// ```
    /// use vpay_provider::ProviderError;
    /// use vpay_provider::http::{HttpBodyError, MAX_RAIL_BODY_BYTES};
    ///
    /// let error = ProviderError::transport_from(
    ///     "orange_money: reading the token response",
    ///     HttpBodyError::TooLarge { max: MAX_RAIL_BODY_BYTES },
    /// );
    /// assert_eq!(
    ///     error.to_string(),
    ///     "transport error talking to the rail: orange_money: reading the token response"
    /// );
    ///
    /// let stage = std::error::Error::source(&error).expect("the cause is attached");
    /// assert_eq!(stage.to_string(), "reading the response");
    /// let leaf = std::error::Error::source(stage).expect("and the leaf under it");
    /// assert_eq!(leaf.to_string(), format!("the response exceeded {MAX_RAIL_BODY_BYTES} bytes"));
    /// ```
    #[must_use]
    pub fn transport_from(context: impl Into<String>, source: impl Into<RailFailure>) -> Self {
        Self::Transport {
            context: context.into(),
            source: Some(source.into()),
        }
    }

    /// An unparseable answer, described by `context` alone.
    ///
    /// The constructor a `serde_json` failure uses: that error's own text is
    /// the whole diagnostic and belongs in `context`, not on a `source`
    /// nobody would print.
    ///
    /// ```
    /// use vpay_core::Classify as _;
    /// use vpay_provider::ProviderError;
    ///
    /// let error = ProviderError::malformed("mtn_momo: unknown status WHAT");
    /// assert_eq!(
    ///     error.to_string(),
    ///     "could not parse the rail's response: mtn_momo: unknown status WHAT"
    /// );
    /// // Not a decline: an answer we cannot read says nothing about whether
    /// // the payment happened, so it is the rail's category and the worker
    /// // resolves it by asking again.
    /// assert_eq!(error.category(), vpay_core::Category::Rail);
    /// assert_eq!(error.code(), "provider_error");
    /// ```
    #[must_use]
    pub fn malformed(context: impl Into<String>) -> Self {
        Self::Malformed {
            context: context.into(),
            source: None,
        }
    }

    /// An unparseable answer raised from a body error.
    ///
    /// The oversize-body arm of [`http::read_rail_body`]: the cap is named in
    /// `context` because it is the whole diagnostic and `Display` must carry
    /// it, and the [`http::HttpBodyError`] is attached anyway so a caller can
    /// still tell an oversize body from a truncated one.
    ///
    /// ```
    /// use vpay_provider::ProviderError;
    /// use vpay_provider::http::{HttpBodyError, MAX_RAIL_BODY_BYTES};
    ///
    /// let error = ProviderError::malformed_from(
    ///     format!("orange_money: the response exceeded {MAX_RAIL_BODY_BYTES} bytes"),
    ///     HttpBodyError::TooLarge { max: MAX_RAIL_BODY_BYTES },
    /// );
    /// assert!(error.to_string().contains(&MAX_RAIL_BODY_BYTES.to_string()));
    /// assert!(std::error::Error::source(&error).is_some());
    /// ```
    #[must_use]
    pub fn malformed_from(context: impl Into<String>, source: impl Into<RailFailure>) -> Self {
        Self::Malformed {
            context: context.into(),
            source: Some(source.into()),
        }
    }
}

impl vpay_core::Classify for ProviderError {
    fn category(&self) -> vpay_core::Category {
        use vpay_core::Category;
        match self {
            // The rail could not be reached or spoke gibberish. The worker's
            // poll ladder retries these (docs/flows/reconciler.md); nobody
            // else should, and a merchant re-submitting would risk a double
            // charge on a push rail (docs/flows/crash-safety.md).
            Self::Transport { .. } | Self::Malformed { .. } => Category::Rail,
            // A rail *decision*, not a rail *failure*: the charge is
            // declined, the intent goes back to requires_payment_method with
            // `last_payment_error`, and the merchant starts a new intent.
            // Classified as Conflict (the charge's state now forbids
            // retrying it) with `Retry::NewAttempt` below.
            Self::Rejected { .. } => Category::Conflict,
            Self::Config(_) => Category::Configuration,
            // The request cannot proceed on this rail, so 409 — whose *bug*
            // it is (ours) is carried by the severity override below, not by
            // the category. See the variant's own doc comment.
            Self::Unsupported => Category::Conflict,
            Self::NotImplemented(_) => Category::NotImplemented,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Transport { .. } => "provider_unavailable",
            Self::Malformed { .. } => "provider_error",
            // A constant, *not* the `FailureCode`'s own string. The two
            // vocabularies overlap: `FailureCode::ProviderUnavailable`
            // renders `provider_unavailable`, which is also
            // `Category::Rail`'s default code — the one `Transport` emits
            // with a 502 and "we are retrying this". A merchant branching on
            // the envelope's `code` would then see the same token for "the
            // rail is down, we will retry" (502) and "your charge was
            // declined, start a new intent" (409). One code per outcome:
            // `charge_declined` says the outcome, and the specific
            // `FailureCode` reaches the merchant through the charge's own
            // `failure_code` field (docs/flows/failures.md) and through the
            // public message below.
            Self::Rejected { .. } => "charge_declined",
            Self::Config(_) => "misconfigured",
            Self::Unsupported => "operation_unsupported_by_rail",
            Self::NotImplemented(_) => "not_implemented",
        }
    }

    fn retry(&self) -> vpay_core::Retry {
        match self {
            // "Retry means a new PaymentIntent" (docs/flows/payment-lifecycle.md).
            Self::Rejected { .. } => vpay_core::Retry::NewAttempt,
            // Exhaustive rather than a wildcard: a new variant must state
            // its retry policy here, not inherit one silently from whatever
            // category it happened to pick.
            Self::Transport { .. }
            | Self::Malformed { .. }
            | Self::Config(_)
            | Self::Unsupported
            | Self::NotImplemented(_) => self.category().default_retry(),
        }
    }

    fn severity(&self) -> vpay_core::Severity {
        use vpay_core::Severity;
        match self {
            // A decline is a business outcome, and `Category::Conflict`
            // would log every one of them at `Info`. Most of them should be:
            // an insufficient balance is the payer's problem and a gateway
            // sees thousands a day. But the taxonomy already carries its own
            // policy (docs/flows/failures.md), and two of its codes are not
            // about the payer at all, so the severity follows the
            // `FailureCode` rather than the category.
            Self::Rejected { code, .. } => match code {
                // "**Your** partner account is blocked … Page yourself" —
                // docs/flows/failures.md. Every charge on this rail is
                // failing and no payer can fix it.
                vpay_core::FailureCode::ProviderAccountBlocked => Severity::Page,
                // The rail is down, or the adapter's mapping table has
                // drifted behind it ("`provider_error` is an alert, not a
                // resting place" — same doc). Degraded and self-healing, or
                // degraded and needing a mapping fix: `Warn` either way.
                vpay_core::FailureCode::ProviderUnavailable
                | vpay_core::FailureCode::ProviderError => Severity::Warn,
                // Everything else is about this payer or this merchant's
                // configuration, and is the caller's to act on.
                vpay_core::FailureCode::InsufficientFunds
                | vpay_core::FailureCode::PayerTimeout
                | vpay_core::FailureCode::PayerDeclined
                | vpay_core::FailureCode::InvalidPayer
                | vpay_core::FailureCode::PayerLimitReached
                | vpay_core::FailureCode::PayerAccountBlocked
                | vpay_core::FailureCode::InvalidPayee
                | vpay_core::FailureCode::PayeeAccountBlocked => Severity::Info,
            },
            // Overrides `Conflict`'s `Info`: the 409 is honest about the
            // request, but reaching this arm means the core did not check
            // `Capabilities` first, and that is ours to fix. See the
            // variant's doc comment.
            Self::Unsupported => Severity::Error,
            Self::Transport { .. }
            | Self::Malformed { .. }
            | Self::Config(_)
            | Self::NotImplemented(_) => self.category().default_severity(),
        }
    }

    fn public_message(&self) -> String {
        match self {
            // The taxonomy's meaning is public by design; the rail's raw
            // reason string is not — it is logged via Display, never sent.
            // The `FailureCode` moves here now that the envelope's `code` is
            // a constant, so a merchant still learns *why* without the two
            // vocabularies colliding on one field.
            Self::Rejected { code, .. } => format!("The payment was declined ({code})."),
            Self::Unsupported => "This rail does not support the requested operation.".to_owned(),
            Self::Transport { .. }
            | Self::Malformed { .. }
            | Self::Config(_)
            | Self::NotImplemented(_) => self.category().generic_message().to_owned(),
        }
    }
}

/// Opaque per-merchant, per-rail configuration handed to the adapter.
///
/// `PartialEq`/`Eq` so a test can assert what a deployment's YAML projects
/// onto the port as one value rather than field by field — a per-field
/// comparison silently stops covering a field the moment one is added, which
/// is exactly how `connect_timeout` would have gone unasserted.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub base_url: String,
    pub callback_url: String,
    pub currency: vpay_core::Currency,
    /// Non-secret, adapter-defined.
    pub settings: BTreeMap<String, String>,
    /// Decrypted immediately before use, adapter-defined.
    pub credentials: BTreeMap<String, String>,
    /// How long the TCP+TLS handshake to this rail may take.
    ///
    /// On the *config*, not on the client, because one `reqwest::Client` is
    /// shared by every adapter in the process — a per-rail deadline on the
    /// client would mean a connection pool per rail. Always
    /// [`DEFAULT_CONNECT_TIMEOUT`] for a config built from YAML; there is no
    /// knob, and `docs/reference/rails.md` says why.
    pub connect_timeout: Duration,
    /// The whole-request deadline: handshake, send, and reading the response
    /// body. [`DEFAULT_REQUEST_TIMEOUT`], on the same terms as
    /// `connect_timeout` above.
    pub request_timeout: Duration,
}

/// Hand-written so a `{:?}` of a config — or of anything that embeds one,
/// such as `vpay_api`'s `RailConfig`/`ResourceConfig` — can never print a
/// rail credential. Keys stay visible (an operator needs to see which key is
/// missing), values are replaced, exactly as `vpay_config::ProviderHost` does
/// on the way in. Pinned by `debug_output_never_contains_a_credential_value`.
impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted: BTreeMap<&str, &str> = self
            .credentials
            .keys()
            .map(|key| (key.as_str(), "[redacted]"))
            .collect();
        f.debug_struct("ProviderConfig")
            .field("base_url", &self.base_url)
            .field("callback_url", &self.callback_url)
            .field("currency", &self.currency)
            .field("settings", &self.settings)
            .field("credentials", &redacted)
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// The handshake budget every deployment gets.
///
/// Long enough for a TLS handshake to a rail in the same region, short
/// enough that a black-holed host fails fast instead of parking a worker
/// task for a minute.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The whole-request budget every deployment gets.
///
/// Generous rather than tight on purpose: both rails' submits are
/// synchronous calls into someone else's payment stack, and a deadline that
/// fires on a rail that *did* accept the charge leaves a payer prompted for
/// a charge we recorded as a transport failure. `docs/reference/rails.md`.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Every rail implements exactly this.
///
/// `query_status` takes the whole [`ChargeRef`], not just an id: some rails
/// need the amount and their own token to answer.
///
/// # The error surface
///
/// Which [`ProviderError`] each operation may raise. The table is the
/// contract — a caller branches on it, and
/// [`Classify`](vpay_core::Classify) turns each row into one status, one
/// retry policy and one severity, at the boundary and nowhere else
/// (ADR-0011). "May" is the operative word: an adapter that never has a
/// credential to read may never raise [`ProviderError::Config`], but no
/// adapter may raise a variant this table does not give it.
///
/// | Variant | `submit` | `query_status` | `parse_callback` | `refund` | `account_holder_name` | what it means |
/// |---|:-:|:-:|:-:|:-:|:-:|---|
/// | [`Config`](ProviderError::Config) | ✓ | ✓ | | ✓ | ✓ | a credential, setting or URL this deployment did not supply, or supplied unusably. Stops the poll ladder: no retry against the rail can fix it |
/// | [`Rejected`](ProviderError::Rejected) | ✓ | ✓ | | ✓ | ✓ | the rail *decided*. On `submit`/`refund` that includes a payer decline; on `query_status` and `account_holder_name` only the rail refusing **our** partner credentials, because neither call carries a payment to decline |
/// | [`Transport`](ProviderError::Transport) | ✓ | ✓ | | ✓ | ✓ | the rail could not be reached or could not be finished with — DNS, TLS, a deadline, a 5xx, a body that failed mid-stream. The charge's fate is **unknown**, which is why the worker resolves it by asking again and a merchant must not re-submit |
/// | [`Malformed`](ProviderError::Malformed) | ✓ | ✓ | ✓ | ✓ | ✓ | the rail answered something this adapter cannot act on: an undocumented status string, a 3xx (never followed), a body past [`http::MAX_RAIL_BODY_BYTES`], or on `parse_callback` a body that names no charge of ours. Also an unknown fate, never a decline |
/// | [`Unsupported`](ProviderError::Unsupported) | | | | ✓ | ✓ | this rail has no such API, permanently. The core is supposed to have branched on [`Capabilities`] first |
/// | [`NotImplemented`](ProviderError::NotImplemented) | ✓ | ✓ | ✓ | ✓ | ✓ | unbuilt work, and it says so rather than fabricating a success. Every token appears in `docs/status.md`, which `cargo xtask verify-status` enforces |
///
/// There is deliberately **no** `ProviderError::retryable()`: retry policy is
/// [`Classify::retry`](vpay_core::Classify::retry) and a second oracle beside
/// it is what ADR-0011 exists to prevent.
///
/// # Why `#[async_trait]`, and why `parse_callback` is not async
///
/// A trait with a native `async fn` is not dyn-safe, and this port is *only*
/// ever used as `Box<dyn ProviderAdapter>` — which is what keeps
/// `if provider == "mtn_momo"` structurally impossible outside an adapter
/// crate (ADR-0002). Implementors must write `#[async_trait]` too; the cost
/// is one boxed future per rail call, against a network round trip.
///
/// `parse_callback` stays synchronous so that it *cannot* make a network
/// call: a callback is a hint, and an adapter that could fetch something
/// while "parsing" one could smuggle a status out of an unauthenticated
/// request (`docs/flows/reconciler.md`).
#[async_trait]
pub trait ProviderAdapter: Debug + Send + Sync {
    /// Stable code, equal to the `payment_method_types` value.
    fn code(&self) -> &'static str;

    fn capabilities(&self) -> Capabilities;

    /// Asks the rail to take a payment.
    ///
    /// Idempotent on `charge.reference_id`. A duplicate submission MUST be
    /// reported as [`Submitted`], never as an error — that is what makes
    /// same-reference retry safe after a crash.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] for a credential or URL this deployment did
    /// not supply; [`ProviderError::Rejected`] when the rail declined the
    /// charge or refused our partner credentials;
    /// [`ProviderError::Transport`] when the rail could not be reached or
    /// finished with; [`ProviderError::Malformed`] for an answer the adapter
    /// cannot act on; [`ProviderError::NotImplemented`] if the rail is
    /// unbuilt. Never [`ProviderError::Unsupported`] — a rail that cannot
    /// take a payment is not a rail. See the trait's error-surface table.
    ///
    /// The last two of those leave the charge's fate **unknown**, and that is
    /// the distinction the whole port is arranged around: only
    /// `Rejected` says the money did not move.
    async fn submit(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError>;

    /// The authoritative read, and the only thing that moves money.
    ///
    /// Must remain callable indefinitely, long after any prompt expired.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] for a missing credential — or, on a rail
    /// that needs key material from `submit`, a `ref_extra` that no longer
    /// carries it, which is a case for a human and not for the poll ladder;
    /// [`ProviderError::Rejected`] **only** when the rail refuses our own
    /// credentials; [`ProviderError::Transport`] and
    /// [`ProviderError::Malformed`] as for [`submit`](ProviderAdapter::submit);
    /// [`ProviderError::NotImplemented`] if the rail is unbuilt.
    ///
    /// A rail that has no record of the charge is **not** an error:
    /// [`ChargeStatus::NotFound`] is the answer, because a push rail can say
    /// that about a charge it is about to accept. Neither is a decline —
    /// that is [`ChargeStatus::Failed`].
    async fn query_status(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError>;

    /// Extracts identifiers from a callback body. Returning a status here is
    /// a design error.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`], and in practice nothing else: this
    /// touches no network, holds no credential and reads no configuration,
    /// so there is no transport to fail and no decision to relay. A body that
    /// is not parseable, or that names no charge this deployment could have
    /// generated, must be refused rather than attributed to something.
    fn parse_callback(&self, body: &[u8]) -> Result<CallbackRef, ProviderError>;

    /// Returns part or all of a settled charge.
    ///
    /// Only called when [`Capabilities::supports_refunds`] is true.
    ///
    /// # `destination`, and the invariant the core owes
    ///
    /// `destination` is `Some` **exactly when** this rail declares
    /// [`RefundDestination::Required`]. The core has already refused the two
    /// combinations that do not reach here — a `Required` rail with no payee,
    /// and an [`Origin`](RefundDestination::Origin) rail sent one — on the
    /// capability value alone and never on a rail code (ADR-0002, RFC-0003
    /// § 1). An adapter is entitled to that and does not re-check it.
    ///
    /// What an adapter should do if the invariant is ever *broken* — a
    /// `Required` rail handed `None` — is deliberately **not settled here**.
    /// No adapter in this workspace implements `refund` yet, so any rule
    /// written now would be a guess no code exercises; the first adapter to
    /// make a real transfer call (RFC-0003 § 5) is what decides which variant
    /// of the error table below says "the core handed me a request this rail
    /// cannot address", and this paragraph is what must be replaced when it
    /// does.
    ///
    /// The type does not enforce it because it cannot: one trait serves both
    /// kinds of rail, so the parameter is an `Option` and the guarantee is the
    /// caller's.
    ///
    /// # Errors
    ///
    /// The default is [`ProviderError::Unsupported`], not
    /// [`ProviderError::NotImplemented`]: a rail with no refund API is a
    /// *permanent* answer the core can branch on via
    /// [`Capabilities::supports_refunds`], not unbuilt work. An adapter whose
    /// rail *does* refund but which has not written it must override this
    /// with its own [`ProviderError::NotImplemented`] token, so
    /// `verify-status` can see it and `docs/status.md` must list it.
    ///
    /// An implemented refund raises the same set as
    /// [`submit`](ProviderAdapter::submit).
    async fn refund(
        &self,
        _charge: &ChargeRef,
        _amount: Money,
        _destination: Option<&RefundTarget>,
        _config: &ProviderConfig,
    ) -> Result<Refunded, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    /// The registered account holder's name for `msisdn`, when the rail
    /// exposes one.
    ///
    /// Only called when [`Capabilities::supports_account_holder_lookup`] is
    /// true. Touches no charge and writes nothing: it is a *stateless* read
    /// of the rail, and `docs/flows/account-holder-lookup.md` is the policy
    /// around it — name only, never persisted, never logged.
    ///
    /// # The three answers, and why the middle one is the sharp one
    ///
    /// * `Ok(Some(holder))` — the rail knows this number and named its
    ///   holder.
    /// * `Ok(None)` — **the rail has no record**, and nothing else. It is
    ///   not "we could not ask", not "the rail was down" and not "we have no
    ///   credential": every one of those is an `Err`, classified through
    ///   ADR-0011 so the boundary answers 502/500 rather than a 200 a caller
    ///   would read as "no such holder".
    /// * `Err(..)` — see the table below.
    ///
    /// The distinction is the whole point of the method. The caller this
    /// exists for (issue #47) refuses a nominated refund destination whose
    /// name it cannot match, and it must be able to tell a number that is
    /// not registered from a lookup that never happened — the first is the
    /// payer's problem and the second is ours.
    ///
    /// # Errors
    ///
    /// The default is [`ProviderError::Unsupported`], on exactly
    /// [`refund`](ProviderAdapter::refund)'s terms: a rail with no
    /// account-holder API is a *permanent* answer the core branches on via
    /// the capability, not unbuilt work. An adapter whose rail *does* expose
    /// one but which has not written it must override this with its own
    /// [`ProviderError::NotImplemented`] token, so `verify-status` can see
    /// it and `docs/status.md` must list it.
    ///
    /// An implemented lookup raises [`ProviderError::Config`] for a
    /// credential this deployment did not supply,
    /// [`ProviderError::Rejected`] when the rail refuses **our** partner
    /// credentials, [`ProviderError::Transport`] when the rail could not be
    /// reached or finished with, and [`ProviderError::Malformed`] for an
    /// answer it cannot act on — including a body past
    /// [`http::MAX_RAIL_BODY_BYTES`]. Never a decline: there is no payment
    /// here to decline.
    async fn account_holder_name(
        &self,
        _msisdn: &str,
        _config: &ProviderConfig,
    ) -> Result<Option<AccountHolder>, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use vpay_core::{Category, Classify as _, Retry, Severity};

    use super::*;

    /// Every code in the taxonomy, so the severity table below cannot pass
    /// by only exercising the rows someone remembered. `docs/flows/failures.md`
    /// is the list; adding a code there means adding it here.
    const EVERY_FAILURE_CODE: [FailureCode; 11] = [
        FailureCode::InsufficientFunds,
        FailureCode::PayerTimeout,
        FailureCode::PayerDeclined,
        FailureCode::InvalidPayer,
        FailureCode::PayerLimitReached,
        FailureCode::PayerAccountBlocked,
        FailureCode::InvalidPayee,
        FailureCode::PayeeAccountBlocked,
        FailureCode::ProviderAccountBlocked,
        FailureCode::ProviderUnavailable,
        FailureCode::ProviderError,
    ];

    fn rejected(code: FailureCode) -> ProviderError {
        ProviderError::Rejected {
            code,
            message: "the rail's own words".to_owned(),
        }
    }

    /// The collision this constant exists to prevent: `FailureCode`'s
    /// strings and `Category`'s default codes are two vocabularies that
    /// overlap, and the envelope's `code` field can only carry one meaning.
    #[test]
    fn a_decline_has_one_code_of_its_own_and_never_borrows_the_rails() {
        for code in EVERY_FAILURE_CODE {
            assert_eq!(rejected(code).code(), "charge_declined", "{code}");
        }
        // Same `code` string, wildly different meaning: 502 "we are
        // retrying" vs. 409 "start a new intent". They must not collide.
        assert_eq!(
            ProviderError::transport("timeout".to_owned()).code(),
            "provider_unavailable"
        );
        assert_eq!(
            ProviderError::malformed("not json".to_owned()).code(),
            "provider_error"
        );
        assert_eq!(
            FailureCode::ProviderUnavailable.as_str(),
            "provider_unavailable",
            "if this ever stops overlapping, the constant above is still right but this test's premise changed"
        );
    }

    #[test]
    fn a_decline_names_the_failure_code_in_the_message_a_merchant_sees() {
        assert_eq!(
            rejected(FailureCode::InsufficientFunds).public_message(),
            "The payment was declined (insufficient_funds)."
        );
        // The rail's own reason string stays in `Display` (an operator's
        // half) and never crosses into the merchant's.
        let e = rejected(FailureCode::PayerDeclined);
        assert!(e.to_string().contains("the rail's own words"));
        assert!(!e.public_message().contains("the rail's own words"));
    }

    /// `Conflict` defaults to `Info`, which is right for a payer's declined
    /// charge and wrong for a blocked partner account. The taxonomy already
    /// carries that policy (`docs/flows/failures.md`); this asserts the
    /// severity follows it rather than the category.
    #[test]
    fn a_declines_severity_follows_the_failure_codes_own_policy() {
        assert_eq!(Category::Conflict.default_severity(), Severity::Info);

        // "Page yourself" — every charge on the rail is failing.
        assert_eq!(
            rejected(FailureCode::ProviderAccountBlocked).severity(),
            Severity::Page
        );
        // Degraded, or an adapter mapping that has drifted: warn.
        assert_eq!(
            rejected(FailureCode::ProviderUnavailable).severity(),
            Severity::Warn
        );
        assert_eq!(
            rejected(FailureCode::ProviderError).severity(),
            Severity::Warn
        );
        // Everything else is the payer's or the merchant's, and a gateway
        // sees thousands a day.
        for code in EVERY_FAILURE_CODE {
            let expected = match code {
                FailureCode::ProviderAccountBlocked => Severity::Page,
                FailureCode::ProviderUnavailable | FailureCode::ProviderError => Severity::Warn,
                _ => Severity::Info,
            };
            assert_eq!(rejected(code).severity(), expected, "{code}");
        }
    }

    #[test]
    fn a_decline_is_a_conflict_the_caller_must_start_over_from() {
        for code in EVERY_FAILURE_CODE {
            let e = rejected(code);
            assert_eq!(e.category(), Category::Conflict, "{code}");
            assert_eq!(e.retry(), Retry::NewAttempt, "{code}");
        }
    }

    /// 409 for the merchant, `Error` for us: the request genuinely cannot
    /// proceed, *and* the core skipped the capability check it is supposed
    /// to branch on (ADR-0002). Logging it at `Conflict`'s default `Info`
    /// would bury our own bug among merchants' typos.
    #[test]
    fn an_unsupported_operation_answers_409_but_is_logged_as_our_bug() {
        let e = ProviderError::Unsupported;
        assert_eq!(e.category(), Category::Conflict);
        assert_eq!(e.category().http_status(), 409);
        assert_eq!(Category::Conflict.default_severity(), Severity::Info);
        assert_eq!(e.severity(), Severity::Error);
        assert_eq!(e.retry(), Retry::Never);
    }

    #[test]
    fn partial_refunds_imply_refunds() {
        let bad = Capabilities {
            flow: ProviderFlow::Push,
            supports_refunds: false,
            supports_partial_refunds: true,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
            supports_account_holder_lookup: false,
            refund_destination: RefundDestination::Required,
        };
        assert!(!bad.is_coherent());
    }

    /// Pins the decision recorded on [`Capabilities::is_coherent`]: no
    /// coherence rule pairs with `refund_destination`, in either direction.
    ///
    /// This is not a tautology dressed as a test. It is the assertion that
    /// fails the moment somebody adds such a rule, and the place they will
    /// read *why* they must not add a Rust-only one: this field has no column
    /// in `providers` and no CHECK to mirror it, unlike
    /// `partial_refunds_imply_refunds` above. Reversing the decision means
    /// adding the column and the migration first — and then this test is
    /// deleted rather than weakened.
    ///
    /// `orange_money`'s live declaration is the second row below:
    /// `Required` with `supports_refunds: false`, which the obvious rule
    /// would have refused.
    #[test]
    fn a_refund_destination_is_inert_to_coherence() {
        let base = Capabilities {
            flow: ProviderFlow::Push,
            supports_refunds: false,
            supports_partial_refunds: false,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
            supports_account_holder_lookup: false,
            refund_destination: RefundDestination::Origin,
        };

        for supports_refunds in [false, true] {
            for destination in [RefundDestination::Origin, RefundDestination::Required] {
                let capabilities = Capabilities {
                    supports_refunds,
                    refund_destination: destination,
                    ..base
                };
                assert!(
                    capabilities.is_coherent(),
                    "no refund destination makes a rail incoherent on its own: {capabilities:?}"
                );
            }
        }
    }

    /// The wire spelling is `snake_case`, like every other vpay-owned type —
    /// `verify-serde` checks the *attribute* is present, and this checks it
    /// produces the tokens the RFC's wire shape is written in.
    #[test]
    fn a_refund_destination_spells_itself_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&RefundDestination::Origin).expect("serialises"),
            r#""origin""#
        );
        assert_eq!(
            serde_json::to_string(&RefundDestination::Required).expect("serialises"),
            r#""required""#
        );
        assert_eq!(
            serde_json::from_str::<RefundDestination>(r#""required""#).expect("deserialises"),
            RefundDestination::Required
        );
    }

    /// A payee's phone number must not reach a log through a `{:?}`, whatever
    /// it is nested in. RFC-0003 rejected `metadata.payee_number` on exactly
    /// this ground and the replacement must not reintroduce it.
    #[test]
    fn a_refund_destination_never_prints_the_payees_number() {
        let destination = RefundTarget::mobile_money("+237600000200");
        assert_eq!(destination.msisdn(), "+237600000200");

        for rendered in [
            format!("{destination:?}"),
            format!("{:?}", Some(&destination)),
            format!("{:?}", Ok::<_, ()>(Some(destination.clone()))),
        ] {
            assert!(!rendered.contains("237600000200"), "{rendered}");
            assert!(rendered.contains("[redacted]"), "{rendered}");
        }
    }
}

#[cfg(test)]
mod provider_config_debug_tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_a_credential_value() {
        let mut credentials = BTreeMap::new();
        credentials.insert("api_key".to_owned(), "hunter2-secret-value".to_owned());
        let mut settings = BTreeMap::new();
        settings.insert("target_environment".to_owned(), "sandbox".to_owned());
        let config = ProviderConfig {
            base_url: "http://rail.example".to_owned(),
            callback_url: "http://vpay.example/cb".to_owned(),
            currency: vpay_core::Currency::Eur,
            settings,
            credentials,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("hunter2-secret-value"), "{rendered}");
        assert!(
            rendered.contains("api_key"),
            "keys stay visible: {rendered}"
        );
        assert!(rendered.contains("[redacted]"), "{rendered}");
        assert!(
            rendered.contains("sandbox"),
            "settings are not secrets: {rendered}"
        );
    }
}
