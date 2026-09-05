//! The `checkout_sessions` repository (`backends/migrations/
//! 0028_create-checkout-sessions.sql`) — the reads and writes behind
//! `/v1/checkout/sessions` and the three `/v1/browser/checkout` routes.
//!
//! Three rules this module exists to keep, and they are the three
//! [`crate::payment_intents`] keeps plus one:
//!
//! * **every merchant-facing query is merchant-scoped in SQL**, so a `/v1`
//!   handler cannot forget to filter and another tenant's session is
//!   indistinguishable from a missing one;
//! * **status changes are compare-and-swap**, so [`CheckoutSessions::expire`]
//!   *is* the check rather than a validation beside it;
//! * **the one unscoped read is named for its caller**
//!   ([`CheckoutSessions::get_by_id_unscoped`]) and exists because the
//!   browser surface has no tenant to trust yet — exactly the split
//!   `PaymentIntents::get_by_id` draws.
//!
//! `docs/reference/vpay-db.md` §"`checkout_sessions`" carries the reasoning:
//! what each rule prevents, why the settlement flip is `pub(crate)`, and why
//! `find_open_by_intent` returns the whole row.

use std::fmt;

// `AssertSqlSafe`: sqlx 0.9 accepts a statement only as `&'static str` or
// through this wrapper (sqlx#3723). Every `format!` below interpolates crate
// constants and nothing else — never a caller's value — which is the audit the
// wrapper's name demands, written down in `docs/reference/vpay-db.md` § dynamic
// SQL strings and sqlx 0.9 and enforced by `crate::sql_audit`.
use sqlx::AssertSqlSafe;
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};
use crate::payment_intents::LIVE_CHARGE_STATES;

/// Every column of `checkout_sessions`, in one place so the queries below
/// cannot drift on the shape they decode into [`CheckoutSessionRow`].
///
/// No `::TEXT` casts: unlike `payment_intents.status`, none of this table's
/// three vocabularies is a Postgres enum. They are `TEXT` with a `CHECK`
/// (migration `0028`, following `events.fanout_state` and `jobs.kind`), so
/// `sqlx` decodes them into `String` directly.
const COLUMNS: &str = "id, seq, merchant_id, payment_intent_id, livemode, ui_mode, status, \
                       payment_status, success_url, cancel_url, return_url, publishable_key, \
                       client_secret_suffix, return_token, expires_at, created_at, updated_at";

/// The `status` a session is in while it can still be driven — the one label
/// the partial unique index `checkout_sessions_one_open_per_intent` is built
/// over, so every `WHERE status = 'open'` below is an index lookup.
///
/// A constant rather than five string literals: the index's predicate and the
/// guards have to be the same label, and two spellings would either let a
/// second open session exist or stop the settlement flip from finding the
/// first.
const OPEN: &str = "open";

/// The `type` of the event a horizon expiry emits.
///
/// One of the eight `type_is_a_documented_event` allows (migration `0029`),
/// spelled here rather than passed in by the caller for
/// [`crate::settlement`]'s reason: the event type is a property of *which
/// transition this is*, and a caller free to choose it could report an
/// abandoned checkout as a settled payment. The `data` — the wire object,
/// which only `vpay-api` knows how to shape — is the caller's.
///
/// It is Stripe's own spelling. `docs/flows/webhooks.md`'s rule is "only real
/// Stripe event types", because a custom one is silently dropped by any
/// merchant switching exhaustively over `stripe-node`'s typed event union.
const EVENT_SESSION_EXPIRED: &str = "checkout.session.expired";

/// One `checkout_sessions` row, exactly as stored.
///
/// Not the wire object: `vpay-api` owns that shape (`expires_at`/`created`
/// as unix seconds, the derived `url`). This struct is deliberately
/// one-to-one with the table so a change to either is a compile error rather
/// than a silently dropped column.
///
/// `Debug` is **hand-written** below rather than derived, because
/// [`Self::client_secret_suffix`] and [`Self::return_token`] are both live
/// payer credentials — see that impl.
#[derive(Clone, PartialEq, sqlx::FromRow)]
pub struct CheckoutSessionRow {
    /// Public `cs_…` id, supplied by the caller before the insert.
    pub id: String,
    /// Pagination order. Database-generated, never written by this crate,
    /// and never exposed on the wire.
    pub seq: i64,
    /// The owning merchant. Every `/v1` query in this module filters on it.
    pub merchant_id: String,
    /// The intent this session drives — a real foreign key onto
    /// `payment_intents(id)`.
    pub payment_intent_id: String,
    /// Live or test money, copied from the deployment at creation.
    pub livemode: bool,
    /// `hosted` or `embedded`. `String`, not an enum, for the reason this
    /// crate carries every other vocabulary as text: the parse belongs to
    /// the layer that renders it.
    pub ui_mode: String,
    /// `open`, `complete` or `expired` (D10).
    pub status: String,
    /// `unpaid`, `paid` or `failed`, written by the settlement transaction.
    pub payment_status: String,
    /// Hosted mode's forward destination; `None` for an embedded session.
    /// The `urls_match_ui_mode` CHECK pairs it with [`Self::ui_mode`].
    pub success_url: Option<String>,
    /// Hosted mode's abandon destination; `None` for an embedded session.
    pub cancel_url: Option<String>,
    /// Embedded mode's forward destination; `None` for a hosted session.
    pub return_url: Option<String>,
    /// The merchant publishable key every URL vpay mints for this session
    /// carries as `?key=` — the hosted page, the embedded iframe and the
    /// return page.
    ///
    /// **Not a secret**: it names a tenant and authorises nothing, so unlike
    /// the two fields below it it is printed in this struct's `Debug`. Pinned
    /// on the row rather than derived at render time so a key rotation cannot
    /// strand a payer already on a rail's page — see migration `0028`'s
    /// comment on the column.
    pub publishable_key: String,
    /// The stored half of this session's payer-facing `client_secret`. Join
    /// it to [`Self::id`] with `vpay_core::ids::client_secret` — never by
    /// hand.
    ///
    /// **A credential, not an identifier.** Whoever holds the joined value
    /// can read this session *and the intent's own `client_secret`* through
    /// `/v1/browser/checkout/sessions/{id}`, which is enough to confirm the
    /// payment. Redacted in this struct's `Debug`.
    pub client_secret_suffix: String,
    /// The credential the return page presents in a query string (D6).
    ///
    /// **Also a credential**, and redacted for the same reason — but a
    /// deliberately smaller one: it authorises reading the session and its
    /// intent *without* the intent's secret. See migration `0028`'s comment
    /// on the column for why the two are separate values.
    pub return_token: String,
    /// When this session stops being `open` on its own (24 h from create).
    pub expires_at: OffsetDateTime,
    /// When the session was created, as supplied to
    /// [`CheckoutSessions::create`].
    pub created_at: OffsetDateTime,
    /// When the row last changed. Maintained by the writers here, not by a
    /// trigger.
    pub updated_at: OffsetDateTime,
}

impl CheckoutSessionRow {
    /// The **return page** URL for this session:
    /// `{checkout_base}/c/{id}/return?t={return_token}&key={publishable_key}`.
    ///
    /// # Why this lives on the row rather than in `vpay-api`
    ///
    /// It is the one URL vpay builds and then hands to something outside its
    /// own control: a redirect rail is given it at submit, stores it, and
    /// replays it when the payer finishes (D2/D6). Two callers construct it —
    /// `vpay_api::v1::payment_intents`' confirm path, when a session drives
    /// the charge, and `vpay_api::v1::return_trip` — and every character of
    /// it has to be identical between them, because the *rail* holds the copy
    /// that matters. A `format!` at each call site is two chances to
    /// disagree about a separator.
    ///
    /// It is here rather than in `vpay-api`'s model layer because both of
    /// those callers already hold the row and neither holds a rendered
    /// object, and because putting it beside the two columns it reads is what
    /// keeps "which credential goes in the query string?" answerable from one
    /// place.
    ///
    /// # Why the token comes before the key
    ///
    /// `t` is what *authorises* the read and `key` names the tenant, so the
    /// ordering matches the browser routes' own parameter order. Nothing
    /// depends on it — a query string is a set — but a fixed order means the
    /// stored `charges.return_url` a rail replays is byte-identical to the
    /// one any later code would rebuild, which is what makes comparing them
    /// meaningful if that is ever needed.
    ///
    /// # Both values are URL-safe by construction
    ///
    /// [`Self::return_token`] is `vpay_core::ids`' base32 alphabet, which
    /// that module proves `encodeURIComponent` is the identity on, and
    /// [`Self::publishable_key`] is `pk_` plus `[A-Za-z0-9]`
    /// (`vpay_config`'s `MalformedPublishableKey`). So this is a `format!`
    /// and not an escaping routine — and a future key alphabet that needed
    /// escaping would break `vpay_core::ids`' own test first.
    ///
    /// `checkout_base` is expected to carry **no trailing slash**;
    /// `vpay_api::ResourceConfig::from_config` strips one once at boot so no
    /// call site has to remember. A base ending in `/` would produce `//c/…`,
    /// which is a protocol-relative URL naming a different host entirely.
    ///
    /// ```
    /// # use vpay_db::CheckoutSessionRow;
    /// # use time::OffsetDateTime;
    /// # let row = CheckoutSessionRow {
    /// #     id: "cs_0123456789abcdefghjkmnpq".to_owned(),
    /// #     seq: 1,
    /// #     merchant_id: "acme-cameroon-tenant".to_owned(),
    /// #     payment_intent_id: "pi_0123456789abcdefghjkmnpq".to_owned(),
    /// #     livemode: false,
    /// #     ui_mode: "hosted".to_owned(),
    /// #     status: "open".to_owned(),
    /// #     payment_status: "unpaid".to_owned(),
    /// #     success_url: None,
    /// #     cancel_url: None,
    /// #     return_url: None,
    /// #     publishable_key: "pk_test_acmecameroonsandbox01".to_owned(),
    /// #     client_secret_suffix: "0".repeat(32),
    /// #     return_token: "wxyz0123456789abcdefghjkmnpqrstv".to_owned(),
    /// #     expires_at: OffsetDateTime::UNIX_EPOCH,
    /// #     created_at: OffsetDateTime::UNIX_EPOCH,
    /// #     updated_at: OffsetDateTime::UNIX_EPOCH,
    /// # };
    /// assert_eq!(
    ///     row.return_page_url("https://checkout.example"),
    ///     "https://checkout.example/c/cs_0123456789abcdefghjkmnpq/return\
    ///      ?t=wxyz0123456789abcdefghjkmnpqrstv&key=pk_test_acmecameroonsandbox01"
    /// );
    /// ```
    #[must_use]
    pub fn return_page_url(&self, checkout_base: &str) -> String {
        format!(
            "{}/c/{}/return?t={}&key={}",
            checkout_base.trim_end_matches('/'),
            self.id,
            self.return_token,
            self.publishable_key,
        )
    }
}

/// Redacts both credentials, leaving every other column visible.
///
/// A row is `{:?}`-ed in more places than anyone tracks: a `tracing` field, a
/// `Result` unwrapped in a test failure message, a `#[derive(Debug)]` on some
/// future struct that happens to hold one. Every one of those is a place a
/// payer credential would otherwise be written to a log an operator can read
/// — and both of these are directly actionable: the first reaches the
/// intent's own `client_secret` (and therefore `confirm`), the second reaches
/// the session and its outcome.
///
/// The *lengths* stay, because "is this row's suffix the right shape?" is a
/// question an operator debugging migration `0028`'s CHECKs actually asks and
/// answering it needs no secret. Mirrors [`crate::PaymentIntentRow`]'s impl
/// in shape and in what it keeps.
///
/// Nothing else is hidden. The urls in particular stay visible: they are the
/// merchant's own, they are what an operator debugging "the payer was
/// forwarded to the wrong place" starts from, and hiding them to be thorough
/// would make this impl worse at the job it exists for.
impl fmt::Debug for CheckoutSessionRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckoutSessionRow")
            .field("id", &self.id)
            .field("seq", &self.seq)
            .field("merchant_id", &self.merchant_id)
            .field("payment_intent_id", &self.payment_intent_id)
            .field("livemode", &self.livemode)
            .field("ui_mode", &self.ui_mode)
            .field("status", &self.status)
            .field("payment_status", &self.payment_status)
            .field("success_url", &self.success_url)
            .field("cancel_url", &self.cancel_url)
            .field("return_url", &self.return_url)
            // In full, deliberately, unlike the two below it: a publishable
            // key is public by design, and "which key did this session
            // pin?" is the first question asked when a payer's return page
            // answers 404 after a rotation.
            .field("publishable_key", &self.publishable_key)
            .field(
                "client_secret_suffix",
                &format_args!("[{} chars redacted]", self.client_secret_suffix.len()),
            )
            .field(
                "return_token",
                &format_args!("[{} chars redacted]", self.return_token.len()),
            )
            .field("expires_at", &self.expires_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// The columns a caller supplies when creating a session: [`CheckoutSessionRow`]
/// minus `seq`, `status`, `payment_status` and `updated_at`.
///
/// `status` and `payment_status` are **not** fields, unlike
/// [`crate::NewPaymentIntent::status`], and that is a deliberate difference
/// rather than an oversight: a payment intent's initial status comes from
/// `vpay_core::state`'s own machine (`Transition::Create`), which has more
/// than one legal answer as the machine grows. A session has exactly one
/// birth state — `open`/`unpaid` — with no machine to consult, so a
/// parameter would only be a way to create a session that is already
/// `complete` and has never been paid.
///
/// `created_at` and `expires_at` **are** fields, for
/// `NewPaymentIntent::created_at`'s reason and one more: the horizon has to
/// be `created_at` plus a constant the *API* owns (D10's 24 hours), and
/// computing it in SQL would put a product rule in a migration.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCheckoutSession {
    /// Public `cs_…` id, generated by `vpay_core::ids::checkout_session_id`
    /// before the insert — never by the database, so a crash mid-insert
    /// still leaves a name to reconcile by.
    pub id: String,
    /// The owning merchant, from the authenticated client's mapping.
    pub merchant_id: String,
    /// The intent this session drives. Must exist and belong to
    /// [`Self::merchant_id`] — the API reads it through
    /// `PaymentIntents::get_for_merchant` first, and the foreign key is the
    /// backstop.
    pub payment_intent_id: String,
    /// From `config.deployment.livemode`; never inferred per request.
    pub livemode: bool,
    /// `hosted` or `embedded`.
    pub ui_mode: String,
    /// Hosted mode's two destinations, or `None` twice for embedded.
    pub success_url: Option<String>,
    /// See [`Self::success_url`].
    pub cancel_url: Option<String>,
    /// Embedded mode's one destination, or `None` for hosted.
    pub return_url: Option<String>,
    /// The publishable key to pin on this session. Must be one of
    /// [`Self::merchant_id`]'s registered keys — a rule only `vpay-config`
    /// can see, so the API checks it and the column's CHECK is a shape
    /// backstop.
    pub publishable_key: String,
    /// The payer credential's stored half, from
    /// `vpay_core::ids::client_secret_suffix`.
    pub client_secret_suffix: String,
    /// The return page's credential, from `vpay_core::ids::return_token`.
    /// A separate draw from [`Self::client_secret_suffix`] — see D6 and that
    /// generator's own doc.
    pub return_token: String,
    /// When this session stops being `open` on its own.
    pub expires_at: OffsetDateTime,
    /// Creation instant, supplied by the caller.
    pub created_at: OffsetDateTime,
}

/// One page request for [`CheckoutSessions::list_page`].
///
/// The same shape and the same cursor rule as [`crate::ListPage`] — cursors
/// are public `cs_…` ids, never `seq` values — plus one filter this resource
/// has and payment intents do not.
///
/// A separate type rather than a `ListPage` with an extra field, because
/// `ListPage` is the payment-intent list's contract and a filter on it would
/// be a parameter that resource silently ignores. `docs/reference/vpay-db.md`
/// says the same about why the two are not merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListPage {
    /// How many rows the caller wants. `vpay-api` applies the product limits
    /// (default 10, ceiling 100); this layer only refuses a non-positive
    /// limit, which Postgres would reject outright.
    pub limit: i64,
    /// Return sessions strictly *older* than this id.
    pub starting_after: Option<String>,
    /// Return sessions strictly *newer* than this id — scans ascending and
    /// is reversed in Rust, so `data` is newest-first either way.
    pub ending_before: Option<String>,
    /// Only sessions on this payment intent (the wire contract's
    /// `payment_intent` filter). `None` means every session of the tenant's.
    ///
    /// Applied in SQL beside the tenant filter rather than in Rust after the
    /// page is fetched, which is the only version that pages correctly: a
    /// filter applied after `LIMIT` would return short pages and a
    /// `has_more` that describes the wrong set.
    pub payment_intent: Option<String>,
}

/// Flips a session's `payment_status`/`status` to match the intent that has
/// just settled — inside the settlement transaction, never after it.
///
/// # Why this is a `pub(crate)` free function and not a trait method
///
/// The plan calls this "the worker hook", and the worker is where the
/// *decision* is made — but the write has to land in the same transaction as
/// the intent's own status change, or a crash between the commit and a
/// follow-up write leaves a `succeeded` intent under an `open` session, which
/// is exactly the disagreement `payment_status` was denormalised to avoid. So
/// it is called from [`crate::settlement`], which owns that transaction, and
/// its visibility is what enforces that rather than a paragraph: a caller
/// elsewhere — a handler, a repair script — would have to move a `pub` here
/// in the same diff, which is the moment the argument has to be re-made. Same
/// device, same reasoning, as `payment_intents::succeed_after_submission`.
///
/// # What it writes
///
/// `paid: true` → `payment_status = 'paid'`, `status = 'complete'`.
/// `paid: false` → `payment_status = 'failed'`, `status = 'expired'` (D10:
/// there is no `failed` session status; a session whose intent failed
/// terminally is reported as `expired` carrying `payment_status: failed`).
///
/// Guarded on `status = 'open'`, so a session an operator already expired, or
/// one a duplicate settlement already flipped, is left exactly as it is. That
/// makes the write idempotent by compare-and-swap, like every other write in
/// the settlement transaction.
///
/// `Ok(0)` — no session, or one that is no longer `open` — is the **normal**
/// answer and never an error: most intents have no session at all (a direct
/// `/v1` or `/v1/browser` confirm), and a settlement must not fail because a
/// checkout page was not involved. It returns the count rather than `()` so
/// the caller can log which happened.
///
/// # Errors
///
/// [`DbError::Query`] if the write fails.
pub(crate) async fn settle_for_intent(
    tx: &mut sqlx::PgConnection,
    payment_intent_id: &str,
    paid: bool,
) -> Result<u64, DbError> {
    // Two labels chosen here rather than passed in, for the reason
    // `settlement::EVENT_SUCCEEDED` is a constant: which pair belongs to
    // which outcome is a property of *this* transition, and a caller free to
    // choose could mark a failed payment `complete`/`paid`.
    let (payment_status, status) = if paid {
        ("paid", "complete")
    } else {
        ("failed", "expired")
    };

    let affected = sqlx::query(
        "UPDATE checkout_sessions \
         SET payment_status = $2, status = $3, updated_at = now() \
         WHERE payment_intent_id = $1 AND status = 'open'",
    )
    .bind(payment_intent_id)
    .bind(payment_status)
    .bind(status)
    .execute(&mut *tx)
    .await
    .map_err(classify_write)?
    .rows_affected();

    Ok(affected)
}

#[async_trait::async_trait]
pub trait CheckoutSessions: Send + Sync {
    /// Inserts a new session and returns the row the database actually
    /// stored — including the columns it filled in itself (`seq`, `status`,
    /// `payment_status`, `updated_at`), so a caller never has to re-read to
    /// render its response.
    ///
    /// # Errors
    ///
    /// [`DbError::ForeignKeyViolation`] if `payment_intent_id` names no
    /// intent — which the API refuses first, with a `400` naming the
    /// parameter, so reaching it means the intent was deleted between the
    /// read and this write. [`DbError::UniqueViolation`] naming
    /// `checkout_sessions_one_open_per_intent` if the intent already has an
    /// **open** session (the guard the API turns into a `409`), or naming
    /// the primary key if `id` is already taken. [`DbError::Query`] for
    /// anything else, including a `ui_mode`/URL combination the
    /// `urls_match_ui_mode` CHECK refuses — which is a vpay bug, since the
    /// API validates the same pairing before the write.
    async fn create(&self, new: &NewCheckoutSession) -> Result<CheckoutSessionRow, DbError>;

    /// Reads one session *for this merchant*. `None` means "no such session
    /// for you", which covers both a missing id and another merchant's id —
    /// see the module comment for why those two must be indistinguishable.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError>;

    /// Reads one session by its own id, with no merchant scope.
    ///
    /// # Why this exists in a module whose whole rule is "merchant-scoped in SQL"
    ///
    /// [`CheckoutSessions::get_for_merchant`] is the *merchant's* read, and
    /// the scope is what stops a `/v1` handler leaking another tenant's
    /// object. This one is the **browser's** read: the caller is a payer with
    /// a publishable key and a session secret, and the tenant that key names
    /// is not trusted until it has been compared against the row's own. Taking
    /// a `merchant_id` here would have to be the value the caller supplied,
    /// which is an authorisation check against the attacker's own input.
    ///
    /// The comparison happens in `vpay_api::browser::checkout_sessions`,
    /// immediately, and answers the same uniform 404 as every other failure
    /// on that surface. The name carries `_unscoped` so a `/v1` handler that
    /// reached for it has to type the word.
    ///
    /// **Not for use in a `/v1` handler.** A handler with this function and a
    /// merchant id in scope will eventually compare them in Rust, which is
    /// the read-then-compare this module exists to make impossible.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn get_by_id_unscoped(&self, id: &str) -> Result<Option<CheckoutSessionRow>, DbError>;

    /// The **open** session on this intent, if there is one.
    ///
    /// # Why lane 2 calls this, and what its signature promises
    ///
    /// `docs/plans/2026-09-04-step9-hosted-checkout.md`'s D2: the
    /// per-charge `return_url` handed to a redirect rail is vpay's own
    /// return page *when a session drives the charge*, and the merchant's
    /// stored `charges.return_url` otherwise. `vpay_api::v1::payment_intents`
    /// therefore has to ask this question on the confirm path, at a point
    /// where it knows only the intent.
    ///
    /// Unscoped, and deliberately so: the confirm path has already resolved
    /// and authorised the intent through a `MerchantScope`, so the intent id
    /// it passes is one the caller may act on, and re-filtering by a tenant
    /// derived from that same intent would be an authorisation check against
    /// itself. That is `PaymentIntents::get_by_id`'s argument, unchanged.
    ///
    /// Returns the whole row rather than the `return_token` alone, because
    /// building the return URL needs the `id` too and a two-value tuple is a
    /// shape that grows a third value the next time something is needed.
    ///
    /// `None` is the **common** answer and not an error: most intents are
    /// confirmed without a checkout session.
    ///
    /// Only ever one row: `checkout_sessions_one_open_per_intent` is a unique
    /// index over exactly this predicate, so "the open session" is a
    /// well-formed phrase rather than a `LIMIT 1` over an ambiguous set.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn find_open_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError>;

    /// The **newest** session on this intent, whatever its `status`, if the
    /// intent has ever had one.
    ///
    /// # Why the newest row is the one that decides a confirm
    ///
    /// [`CheckoutSessions::find_open_by_intent`] answers "is a session
    /// driving this charge?" and is right for the return URL. It cannot
    /// answer "may this intent be confirmed at all?", because the interesting
    /// case is exactly the one where **no** session is open: a sweep or a
    /// merchant expired it, and a payer still holding the intent's
    /// `client_secret` would otherwise pay for a checkout the merchant has
    /// already been told was abandoned.
    ///
    /// One row is enough, and the newest is the right one, because
    /// `checkout_sessions_one_open_per_intent` makes **"an open session is
    /// always the newest"** a property of the schema rather than a hope: a
    /// second session cannot be inserted while one is open, so any row newer
    /// than an open session would have had to be inserted through that index.
    /// The consequence a caller depends on is the useful direction — an
    /// intent whose first session expired and whose merchant then created a
    /// second, open one reads back the *open* one, so the ordinary "expire
    /// and offer a fresh link" flow is not refused.
    ///
    /// Ordered by `seq`, the table's own insertion order, and not by
    /// `created_at`: `created_at` is supplied by the caller
    /// ([`NewCheckoutSession::created_at`]), so two sessions could carry the
    /// same instant, and the tie would decide whether a payer can pay.
    ///
    /// Unscoped, for [`CheckoutSessions::find_open_by_intent`]'s reason
    /// unchanged: the confirm path has already resolved and authorised the
    /// intent through a `MerchantScope`, so re-filtering by a tenant derived
    /// from that same intent would be an authorisation check against itself.
    ///
    /// `None` is the **common** answer and not an error: most intents are
    /// confirmed without a checkout session ever existing.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn find_latest_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError>;

    /// One page of this merchant's sessions, newest first, plus whether more
    /// exist beyond it.
    ///
    /// Ordering, cursors and `has_more` work exactly as
    /// [`crate::PaymentIntents::list_page`]'s do — read that one for the
    /// direction-of-travel argument and for why an unknown cursor yields an
    /// empty page rather than the newest rows. The one addition is
    /// [`SessionListPage::payment_intent`], applied in the same statement.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn list_page(
        &self,
        merchant_id: &str,
        page: &SessionListPage,
    ) -> Result<(Vec<CheckoutSessionRow>, bool), DbError>;

    /// Moves an `open` session to `expired` — provided its intent has no
    /// charge the rail may still be acting on — atomically, and returns the
    /// row as it now stands.
    ///
    /// # Why the live-charge check is a predicate of the `UPDATE`
    ///
    /// Exactly [`crate::PaymentIntents::cancel`]'s argument, which
    /// `docs/reference/vpay-db.md` §"`cancel` checks for a live charge inside
    /// the statement" spells out in full. `status = 'open'` is not on its own
    /// enough to make an expiry safe: a payer's page may have confirmed
    /// seconds ago, and a `confirm` commits its charge *before* it calls the
    /// rail. Expiring there would tell a merchant the checkout was abandoned
    /// while the payer's handset is prompting — and would then be
    /// contradicted by the settlement transaction flipping the same row to
    /// `complete`/`paid`.
    ///
    /// A check in the caller cannot close that window: between reading "no
    /// live charge" and writing `expired`, a concurrent confirm can commit
    /// one. Only the write statement can decide it.
    ///
    /// The four live labels are `payment_intents::LIVE_CHARGE_STATES`,
    /// shared rather than restated so this guard and `cancel`'s cannot drift.
    ///
    /// `Ok(None)` therefore carries three meanings — no such session for this
    /// merchant, one that is no longer `open`, or a live charge — and the
    /// caller that must tell them apart re-reads. That is deliberate: the
    /// three are one answer at this layer, and how much to reveal is the
    /// boundary's decision.
    ///
    /// `payment_status` is **not** touched. An expired session that was
    /// already `paid` keeps saying so: the money is a fact about the intent,
    /// and an expiry that rewrote it would be vpay telling a merchant a
    /// completed payment had not happened.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the write fails.
    async fn expire(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError>;

    /// Every `open` session whose horizon has passed and which nothing is
    /// driving, oldest first, at most `limit` of them — the sweep's backlog
    /// query.
    ///
    /// The **read** half of what used to be one bulk `UPDATE`. It was split
    /// in two when expiry started emitting `checkout.session.expired`
    /// (migration `0029`): the event's `data` is the *rendered* wire object,
    /// which only `vpay-api` knows how to shape, so the caller has to hold
    /// the row before the write that describes it — the same order
    /// `vpay_worker::handlers::intent_snapshot` and
    /// [`crate::Settlement::apply_succeeded`] have always been in.
    ///
    /// # Not tenant-scoped, because its caller is a sweep
    ///
    /// Every other read in this module is merchant-scoped in SQL. This one
    /// has no merchant to scope by: it is called by
    /// `vpay_worker::handlers::sweep_expired`, the hourly housekeeping job,
    /// which acts for the deployment rather than for a tenant. It selects a
    /// *time*, not a tenant.
    ///
    /// # It carries the same live-charge guard the write does
    ///
    /// `NOT EXISTS` over [`crate::payment_intents::LIVE_CHARGE_STATES`],
    /// identical to [`CheckoutSessions::expire_due`]'s. Duplicated rather
    /// than left to the write alone so a session a rail is still holding is
    /// never *rendered* either: rendering it would mint an `evt_…` and build
    /// an object claiming the checkout was abandoned, which the write would
    /// then correctly refuse — work done for nothing, and one more place a
    /// future change could leak that object out of. The write keeps its own
    /// copy because this read's answer is stale the moment it returns; see
    /// that function.
    ///
    /// # `limit`, where the bulk statement had none
    ///
    /// The old statement returned nothing, so an unbounded `UPDATE` cost one
    /// number. This one materialises rows that each carry two live payer
    /// credentials, and the sweep now does a transaction *per* row, so the
    /// page is what bounds both the memory and the time one housekeeping
    /// pass may take. `vpay_worker::handlers::EXPIRY_PAGE` owns the value and
    /// the sweep reschedules itself immediately when a page comes back full,
    /// exactly as `vpay_worker::webhooks::handle_fan_out` does — so a backlog
    /// drains rather than waiting an hour a page.
    ///
    /// Ordered by `expires_at`, so the session that has been over its horizon
    /// longest is the one a merchant hears about first.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the read fails.
    async fn due_for_expiry(
        &self,
        now: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<CheckoutSessionRow>, DbError>;

    /// Moves one `open` session past its horizon to `expired` **and** appends
    /// the `checkout.session.expired` event that tells its merchant so — in
    /// one transaction.
    ///
    /// The other half of [`CheckoutSessions::expire`]: that one is a merchant
    /// saying "I am done with this", this one is D10's 24 hours arriving.
    /// `expires_at` was written at create and read by **nothing** until Step
    /// 9's lane 1b — a session past its horizon reported `status: open`
    /// forever, so a merchant reading it could not tell "still payable" from
    /// "abandoned yesterday" — and until migration `0029` it moved silently:
    /// nothing was emitted, nothing was delivered, and a merchant learned by
    /// polling.
    ///
    /// # Why the event is written here and not by the caller
    ///
    /// The same argument [`crate::Settlement::apply_succeeded`] makes, and it
    /// is the whole point of this function existing rather than the caller
    /// running two writes. A crash between the flip and the event would leave
    /// a session reporting `expired` that no merchant will ever be told
    /// about, and nothing would notice: there is no sweep over "expired
    /// sessions with no event", and D10 adds none. One transaction makes that
    /// window not exist. The `type` is this module's own constant for
    /// [`crate::settlement`]'s reason too — it is a property of *which
    /// transition this is*, and a caller free to choose it could emit
    /// `payment_intent.succeeded` for an expiry.
    ///
    /// `event_data` is the wire object as it will stand once this commits
    /// (`vpay-api`'s shape — this crate does not know it) and `event_id` is a
    /// caller-generated `evt_…` ([`crate::events::event_id`]). Both are the
    /// caller's for exactly the reasons the settlement's are.
    ///
    /// # The guard is the statement, and it is checked again here
    ///
    /// `status = 'open'`, `expires_at <= now`, and the `NOT EXISTS` over
    /// [`crate::payment_intents::LIVE_CHARGE_STATES`] — the same predicate
    /// [`CheckoutSessions::expire`] carries, re-evaluated inside this
    /// transaction rather than trusted from
    /// [`CheckoutSessions::due_for_expiry`]. A payer can confirm between the
    /// read and the write, and a session whose rail is holding a live payment
    /// must not be told the checkout was abandoned — it would be contradicted
    /// by the settlement transaction minutes later, with no request anywhere
    /// to correlate the two.
    ///
    /// `Ok(None)` therefore means "no longer due", covering all four ways
    /// that can be true, and is the **normal** answer for a concurrent sweep
    /// or a merchant who expired the session by hand in between. No event is
    /// written on that path — which is what makes a second sweep produce no
    /// second event.
    ///
    /// `payment_status` is untouched, exactly as in
    /// [`CheckoutSessions::expire`] — the money is a fact about the intent,
    /// and a sweep that rewrote it would be vpay telling a merchant a
    /// completed payment had not happened.
    ///
    /// # `now` is the caller's, unlike the other two sweeps
    ///
    /// `Idempotency::sweep_expired` and `Jobs::reap_expired_leases` compare
    /// against Postgres's `now()`. This one takes the instant, for the reason
    /// [`NewCheckoutSession::expires_at`] is computed in Rust: the horizon is
    /// a **product** rule (D10's 24 hours) that the API owns, and both sides
    /// of that comparison belonging to the same layer is what keeps it one
    /// rule. It also means a test can sweep a future instant instead of
    /// rewriting a stored horizon.
    ///
    /// # Errors
    ///
    /// [`DbError::UniqueViolation`] on `events_pkey` if `event_id` has
    /// already been emitted. [`DbError::Query`] if any statement or the
    /// commit fails — including an `event_data` that is not a JSON object
    /// (`data_is_object`) or a `type` outside migration `0029`'s eight, both
    /// of which are vpay bugs. **The transaction is rolled back either way,
    /// so the session is left `open` and the next sweep retries it.**
    async fn expire_due(
        &self,
        id: &str,
        now: OffsetDateTime,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<CheckoutSessionRow>, DbError>;
}

#[async_trait::async_trait]
impl CheckoutSessions for crate::repository::PgRepositories {
    async fn create(&self, new: &NewCheckoutSession) -> Result<CheckoutSessionRow, DbError> {
        // `status` and `payment_status` are literals rather than binds: see
        // `NewCheckoutSession` for why a session has exactly one birth state.
        let sql = format!(
            "INSERT INTO checkout_sessions (id, merchant_id, payment_intent_id, livemode, \
             ui_mode, status, payment_status, success_url, cancel_url, return_url, \
             publishable_key, client_secret_suffix, return_token, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, '{OPEN}', 'unpaid', $6, $7, $8, $9, $10, $11, $12, \
             $13) \
             RETURNING {COLUMNS}"
        );

        sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(&new.id)
            .bind(&new.merchant_id)
            .bind(&new.payment_intent_id)
            .bind(new.livemode)
            .bind(&new.ui_mode)
            .bind(new.success_url.as_deref())
            .bind(new.cancel_url.as_deref())
            .bind(new.return_url.as_deref())
            .bind(&new.publishable_key)
            .bind(&new.client_secret_suffix)
            .bind(&new.return_token)
            .bind(new.expires_at)
            .bind(new.created_at)
            .fetch_one(&self.pool)
            .await
            .map_err(classify_write)
    }

    async fn get_for_merchant(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError> {
        let sql =
            format!("SELECT {COLUMNS} FROM checkout_sessions WHERE merchant_id = $1 AND id = $2");

        sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn get_by_id_unscoped(&self, id: &str) -> Result<Option<CheckoutSessionRow>, DbError> {
        let sql = format!("SELECT {COLUMNS} FROM checkout_sessions WHERE id = $1");

        sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn find_open_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM checkout_sessions \
             WHERE payment_intent_id = $1 AND status = '{OPEN}'"
        );

        sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(payment_intent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn find_latest_by_intent(
        &self,
        payment_intent_id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError> {
        // No `status` predicate at all — that is the whole difference from
        // `find_open_by_intent`, and it is what lets the caller tell "no
        // session was ever created" from "the session that was created is
        // over".
        let sql = format!(
            "SELECT {COLUMNS} FROM checkout_sessions \
             WHERE payment_intent_id = $1 \
             ORDER BY seq DESC \
             LIMIT 1"
        );

        sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(payment_intent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn list_page(
        &self,
        merchant_id: &str,
        page: &SessionListPage,
    ) -> Result<(Vec<CheckoutSessionRow>, bool), DbError> {
        let limit = page.limit.max(1);
        let backwards = page.ending_before.is_some();
        let direction = if backwards { "ASC" } else { "DESC" };

        // The cursor subqueries are merchant-scoped exactly as
        // `payment_intents::list_page`'s are, so a cursor from another tenant
        // resolves to NULL rather than to a position in their range.
        //
        // The `payment_intent` filter is *not* scoped a second time: it is
        // combined with `merchant_id = $1` in the same `WHERE`, so an intent
        // id belonging to another tenant simply matches no row of this one's.
        let sql = format!(
            "SELECT {COLUMNS} FROM checkout_sessions \
             WHERE merchant_id = $1 \
               AND ($2::TEXT IS NULL \
                    OR seq < (SELECT seq FROM checkout_sessions \
                              WHERE id = $2 AND merchant_id = $1)) \
               AND ($3::TEXT IS NULL \
                    OR seq > (SELECT seq FROM checkout_sessions \
                              WHERE id = $3 AND merchant_id = $1)) \
               AND ($4::TEXT IS NULL OR payment_intent_id = $4) \
             ORDER BY seq {direction} \
             LIMIT $5"
        );

        let mut rows = sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(page.starting_after.as_deref())
            .bind(page.ending_before.as_deref())
            .bind(page.payment_intent.as_deref())
            .bind(limit.saturating_add(1))
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)?;

        let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
        if backwards {
            rows.reverse();
        }

        Ok((rows, has_more))
    }

    async fn expire(
        &self,
        merchant_id: &str,
        id: &str,
    ) -> Result<Option<CheckoutSessionRow>, DbError> {
        let sql = format!(
            "UPDATE checkout_sessions SET status = 'expired', updated_at = now() \
             WHERE merchant_id = $1 AND id = $2 AND status = '{OPEN}' \
               AND NOT EXISTS (SELECT 1 FROM charges \
                               WHERE charges.payment_intent_id \
                                     = checkout_sessions.payment_intent_id \
                                 AND charges.state IN ({LIVE_CHARGE_STATES})) \
             RETURNING {COLUMNS}"
        );

        sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(merchant_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(classify_write)
    }

    async fn due_for_expiry(
        &self,
        now: OffsetDateTime,
        limit: i64,
    ) -> Result<Vec<CheckoutSessionRow>, DbError> {
        // Postgres refuses a negative LIMIT, and a zero-row page is never
        // what a caller means — `Events::pending_page`'s clamp, for the same
        // reason.
        let limit = limit.max(1);
        let sql = format!(
            "SELECT {COLUMNS} FROM checkout_sessions \
             WHERE status = '{OPEN}' AND expires_at <= $1 \
               AND NOT EXISTS (SELECT 1 FROM charges \
                               WHERE charges.payment_intent_id \
                                     = checkout_sessions.payment_intent_id \
                                 AND charges.state IN ({LIVE_CHARGE_STATES})) \
             ORDER BY expires_at \
             LIMIT $2"
        );

        sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(now)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn expire_due(
        &self,
        id: &str,
        now: OffsetDateTime,
        event_id: &str,
        event_data: &serde_json::Value,
    ) -> Result<Option<CheckoutSessionRow>, DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        // The whole guard, re-evaluated here: `due_for_expiry`'s answer is
        // stale the moment it returns, and a payer who confirmed in between
        // is exactly the case the `NOT EXISTS` exists for.
        let sql = format!(
            "UPDATE checkout_sessions SET status = 'expired', updated_at = now() \
             WHERE id = $1 AND status = '{OPEN}' AND expires_at <= $2 \
               AND NOT EXISTS (SELECT 1 FROM charges \
                               WHERE charges.payment_intent_id \
                                     = checkout_sessions.payment_intent_id \
                                 AND charges.state IN ({LIVE_CHARGE_STATES})) \
             RETURNING {COLUMNS}"
        );

        let expired = sqlx::query_as::<_, CheckoutSessionRow>(AssertSqlSafe(sql))
            .bind(id)
            .bind(now)
            .fetch_optional(&mut *tx)
            .await
            .map_err(classify_write)?;

        let Some(expired) = expired else {
            // Not due any more. Nothing was written, so there is nothing to
            // commit and nothing to roll back either — but the transaction is
            // closed explicitly rather than dropped, exactly as
            // `settlement::apply_succeeded` closes its already-settled arm,
            // so the connection returns to the pool without waiting for a
            // background rollback.
            tx.rollback().await.map_err(DbError::Query)?;
            return Ok(None);
        };

        crate::events::insert_in_tx(
            &mut tx,
            &crate::events::NewEvent {
                id: event_id.to_owned(),
                // From the row this transaction just wrote, never from
                // configuration read at emit time — the event has to describe
                // what was true of the object, and the session is the object.
                merchant_id: expired.merchant_id.clone(),
                livemode: expired.livemode,
                event_type: EVENT_SESSION_EXPIRED.to_owned(),
                // The `cs_…`, which is what a merchant's handler re-reads.
                object_id: expired.id.clone(),
                data: event_data.clone(),
            },
        )
        .await?;

        tx.commit().await.map_err(DbError::Query)?;

        Ok(Some(expired))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row whose two credentials are values no other test could produce, so
    /// the assertions below cannot pass by the strings simply being absent.
    fn row() -> CheckoutSessionRow {
        CheckoutSessionRow {
            id: "cs_00000000000000000000000x".to_owned(),
            seq: 1,
            merchant_id: "acme-cameroon-tenant".to_owned(),
            payment_intent_id: "pi_00000000000000000000000y".to_owned(),
            livemode: false,
            ui_mode: "hosted".to_owned(),
            status: "open".to_owned(),
            payment_status: "unpaid".to_owned(),
            success_url: Some("https://shop.example/ok".to_owned()),
            cancel_url: Some("https://shop.example/cancel".to_owned()),
            return_url: None,
            publishable_key: "pk_test_acmecameroonsandbox01".to_owned(),
            client_secret_suffix: "neverlogthissessioncredential000".to_owned(),
            return_token: "neverlogthisreturntoken000000000".to_owned(),
            expires_at: OffsetDateTime::UNIX_EPOCH,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// Both stored halves are live payer credentials: one reaches the
    /// intent's own `client_secret` (and therefore `confirm`), the other
    /// reaches the session's outcome. A derived `Debug` would put both in
    /// every `tracing` field, every test failure message and every future
    /// struct that happens to hold a row.
    ///
    /// Decisive: replacing the hand-written impl with `#[derive(Debug)]`
    /// fails this test on its first assertion.
    #[test]
    fn a_checkout_session_rows_debug_output_never_contains_either_credential() {
        let row = row();
        let formatted = format!("{row:?}");

        assert!(
            !formatted.contains("neverlogthissessioncredential000"),
            "Debug output must not contain the client_secret_suffix: {formatted}"
        );
        assert!(
            !formatted.contains("neverlogthisreturntoken000000000"),
            "Debug output must not contain the return_token: {formatted}"
        );
        // Not even a prefix of either: a redaction that truncated rather than
        // replaced would still hand a guesser most of the credential.
        assert!(
            !formatted.contains("neverlog"),
            "Debug output must not contain even a prefix of either credential: {formatted}"
        );
        assert_eq!(
            formatted.matches("[32 chars redacted]").count(),
            2,
            "both credentials must be redacted, not just the first: {formatted}"
        );
    }

    /// Everything an operator investigating a checkout starts from stays
    /// visible — a `Debug` that redacted the whole row to be thorough would
    /// be worse at the job it exists for.
    #[test]
    fn a_checkout_session_rows_debug_output_still_names_the_row() {
        let formatted = format!("{:?}", row());

        for expected in [
            "cs_00000000000000000000000x",
            "pi_00000000000000000000000y",
            "acme-cameroon-tenant",
            "hosted",
            "open",
            "unpaid",
            "https://shop.example/ok",
        ] {
            assert!(
                formatted.contains(expected),
                "{expected:?} is missing from Debug output: {formatted}"
            );
        }
    }

    /// The return page URL is the one string a **rail** holds a copy of, so
    /// its shape is pinned here rather than left to whichever caller builds
    /// it.
    ///
    /// The two values in it are URL-safe by construction — the token is
    /// `vpay_core::ids`' base32 alphabet and the key is `pk_` plus
    /// `[A-Za-z0-9]` — which is why this is a `format!` and not an escaping
    /// routine. The last assertion is what would catch that stopping being
    /// true.
    #[test]
    fn a_return_page_url_carries_the_token_and_the_key_and_needs_no_escaping() {
        let row = row();

        assert_eq!(
            row.return_page_url("https://checkout.example"),
            format!(
                "https://checkout.example/c/{}/return?t={}&key={}",
                row.id, row.return_token, row.publishable_key
            )
        );

        // A trailing slash is absorbed rather than producing `//c/…`, which
        // is a protocol-relative URL naming a different host entirely.
        assert_eq!(
            row.return_page_url("https://checkout.example/"),
            row.return_page_url("https://checkout.example")
        );
        // A path prefix survives — one of the two production topologies the
        // Step 9 plan leaves to the maintainer.
        assert!(
            row.return_page_url("https://api.example/checkout")
                .starts_with("https://api.example/checkout/c/")
        );

        // No character in it needs escaping, so a rail storing it verbatim
        // and replaying it gets the same URL back.
        let url = row.return_page_url("https://checkout.example");
        assert!(
            url.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#&=".contains(c)),
            "{url} carries a character a rail might re-encode"
        );
        // …and exactly one `?`, so the token is a parameter rather than part
        // of the path.
        assert_eq!(url.matches('?').count(), 1, "{url}");
        assert!(
            !url.contains('#'),
            "the return page carries no fragment: {url}"
        );
    }

    /// The label the partial unique index is built over, and the four places
    /// this module spells it, are one constant.
    ///
    /// Pinned as a literal because the *index* is written in SQL in migration
    /// `0028` and cannot import this constant: renaming it here alone would
    /// compile, and would then leave `find_open_by_intent` and the settlement
    /// flip matching nothing while the index still refused a second session
    /// under the old label.
    #[test]
    fn the_open_label_is_the_one_the_partial_index_is_built_over() {
        assert_eq!(OPEN, "open");
    }
}
