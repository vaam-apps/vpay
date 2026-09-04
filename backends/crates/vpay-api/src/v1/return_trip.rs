//! What a **checkout session** has to say about a confirm: whether it may
//! happen at all, and where a redirect rail must send the payer back to.
//!
//! One question, asked once per confirm, answered in one place. It used to be
//! only the second half — is this charge driven by a checkout session vpay is
//! hosting a page for, or is it a direct `/v1` (or `/v1/browser`) confirm
//! whose payer belongs to the merchant's own site? The answer is written to
//! `charges.return_url` before the charge is committed, and from there becomes
//! `vpay_provider::ChargeRef::return_url`, which
//! `vpay-adapter-orange-money` sends as both `return_url` and `cancel_url` —
//! see D2 of `docs/plans/2026-09-04-step9-hosted-checkout.md` and
//! `docs/reference/rails.md`.
//!
//! One row, one URL: what the rail is told, what a later read of the intent
//! renders as `next_action.redirect_to_url.return_url`, and what the worker
//! would resubmit under are the same stored value, because they are the same
//! column. See [`admit_confirm`] for what that replaced.
//!
//! # Why one read answers both halves
//!
//! A session that is not `open` **refuses** the confirm
//! ([`ApiError::CheckoutSessionNotOpen`]), and a session that is open decides
//! where the payer comes back to. Those are one question about one row, and
//! asking them separately would be a race with the hourly sweep: a gate that
//! read `open`, then a return-URL lookup that ran a millisecond after
//! `expire_due` committed, would admit a confirm and then submit it to the
//! rail with **no** return URL at all. One read, one verdict.
//!
//! It is a module of its own, and a trait rather than a few lines inside
//! [`crate::v1::payment_intents`], because the two branches are owned by
//! different things: the merchant's URL is a column this crate already writes,
//! and the session's URL is a row plus a configured origin that
//! `vpay-db`/`vpay-config` own. Keeping the question behind
//! [`CheckoutSessionGate`] is what lets the answer change without the confirm
//! path learning what a checkout session is.
//!
//! The module keeps the name `return_trip` although it now does more than the
//! return trip: `docs/flows/browser-checkout.md` and `docs/reference/rails.md`
//! both name the path, and a rename would leave two flow documents pointing at
//! a file that does not exist for no gain a reader can see.

use async_trait::async_trait;
use time::OffsetDateTime;
use vpay_db::{CheckoutSessionRow, CheckoutSessions, Repositories};

use crate::error::{ApiError, CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP, ClosedSession};

/// The single lookup the confirm path needs about checkout sessions.
///
/// Deliberately narrow: not "give me the session", but "may this confirm
/// proceed, and if so where does the payer come back to". A confirm has no
/// other business with a session — it does not read one, render one, or move
/// one — and a wider trait here would invite it to.
#[async_trait]
pub(crate) trait CheckoutSessionGate: Send + Sync {
    /// `Ok(None)` when no checkout session has ever driven
    /// `payment_intent_id`; `Ok(Some(url))` when an open one does, carrying
    /// vpay's own return page for it; `Err` when a session exists and
    /// forbids the confirm.
    ///
    /// # Errors
    ///
    /// [`ApiError::CheckoutSessionNotOpen`] when the intent's session is
    /// `expired` or `complete` — the refusal this trait exists for, and the
    /// reason a `bool` would not do. [`ApiError::Db`] if the lookup fails,
    /// and [`ApiError::CheckoutNotConfigured`] if an open session drives the
    /// intent on a deployment that serves no checkout page — see
    /// [`SessionGate`]'s impl for why that is refused rather than answered
    /// with the merchant's own URL. It is **not** an error for there to be no
    /// session: that is the ordinary case for every direct confirm.
    async fn admit_confirm(&self, payment_intent_id: &str) -> Result<Option<String>, ApiError>;
}

/// The shipping answer: the newest checkout session on an intent, judged, and
/// rendered as vpay's own return page when it admits the confirm.
///
/// # Why it is a struct and not `impl CheckoutSessionGate for dyn Repositories`
///
/// Which was what lane 2 shipped, answering `None` for every intent because
/// there was no `checkout_sessions` table yet. Building the URL needs two
/// things a request holds separately: the row (from [`Repositories`]) and
/// `checkout.public_base_url` (from [`crate::v1::ResourceConfig`], where the confirm
/// path already reads deployment values from). A blanket impl over the
/// repositories alone cannot see the second, and threading the base through
/// the trait method would put it on every future implementation of a lookup
/// that has no use for it.
///
/// Borrowed rather than owned so the confirm path builds one per request
/// out of what it already has, with nothing cloned.
pub(crate) struct SessionGate<'a> {
    repositories: &'a dyn Repositories,
    /// `ResourceConfig::checkout_public_base_url()`, already stripped of a
    /// trailing slash at boot.
    checkout_base: Option<&'a str>,
}

impl<'a> SessionGate<'a> {
    /// Pairs the repositories with this deployment's checkout origin.
    pub(crate) fn new(repositories: &'a dyn Repositories, checkout_base: Option<&'a str>) -> Self {
        Self {
            repositories,
            checkout_base,
        }
    }
}

/// # `None` is the ordinary answer, and it means "no session drives this"
///
/// Most confirms are direct `/v1` or `/v1/browser` calls from a merchant's
/// own page. The row read is the **newest** session on the intent
/// (`vpay_db::CheckoutSessions::find_latest_by_intent`) rather than the open
/// one, because the case worth refusing is exactly the one where none is
/// open; that method's own doc carries the argument for why the newest row is
/// the one that decides.
///
/// # A session with no checkout app is refused, not fallen back from
///
/// [`CheckoutSessionRow::return_page_url`](vpay_db::CheckoutSessionRow::return_page_url)
/// needs `checkout.public_base_url`, and a deployment without one cannot have
/// an open session: `POST /v1/checkout/sessions` answers
/// `checkout_not_configured` before a row is written
/// (`a_deployment_without_a_checkout_app_refuses_to_create_a_session`, in
/// `backends/tests/integration/tests/checkout_sessions.rs`). The one way to
/// stand in this branch is an operator **removing** the key while sessions
/// are open, and the answer is
/// [`ApiError::CheckoutNotConfigured`] carrying
/// [`CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP`] rather than the merchant's own URL — see that constant for why a fallback
/// here is the silent failure lane 2 warned about, and
/// `a_session_driven_confirm_is_refused_when_the_checkout_app_is_gone`
/// (`backends/tests/integration/tests/confirm_rails.rs`) for the proof that
/// it refuses.
///
/// It fires for a **push** rail too, which has no browser and would have
/// ignored the URL. That is deliberate: the deployment's own checkout page is
/// gone, so the payer on it cannot pay whatever the rail is, and a confirm
/// that succeeded for MTN and failed for Orange would make the outage depend
/// on which rail a payer picked.
///
/// It is reached only **after** [`verdict`] has admitted the session, so an
/// expired session on a deployment that has lost its checkout page still
/// answers the session refusal — the more specific and more actionable of the
/// two.
#[async_trait]
impl CheckoutSessionGate for SessionGate<'_> {
    async fn admit_confirm(&self, payment_intent_id: &str) -> Result<Option<String>, ApiError> {
        let Some(session) =
            CheckoutSessions::find_latest_by_intent(self.repositories, payment_intent_id).await?
        else {
            return Ok(None);
        };

        // `now_utc()` here rather than Postgres's `now()`, matching
        // `crate::browser::checkout_sessions::authenticate`: the horizon is a
        // product rule (D10's 24 hours) that this layer owns, and the two
        // places that enforce it have to read the same clock or a payer could
        // be refused a read and admitted a confirm in the same second.
        verdict(&session, OffsetDateTime::now_utc())?;

        let Some(base) = self.checkout_base else {
            tracing::error!(
                payment_intent_id = %payment_intent_id,
                checkout_session_id = %session.id,
                "an open checkout session drives this confirm but checkout.public_base_url is \
                 not configured; refusing rather than sending the payer to the merchant's own \
                 return_url, which would forward them one step too early"
            );
            return Err(ApiError::CheckoutNotConfigured(
                CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP,
            ));
        };

        Ok(Some(session.return_page_url(base)))
    }
}

/// Whether one session row admits a confirm, at one instant.
///
/// Split out of the impl above so the rule is testable without a database:
/// the three inputs that decide it (`status`, `expires_at`, and the clock)
/// are all on the row, and a unit test can build states — a `complete`
/// session under an unsettled intent, in particular — that no sequence of
/// shipping operations produces.
///
/// # The horizon is read, not written
///
/// An `open` session past `expires_at` that the hourly sweep has not reached
/// yet is `Expired` here, and **nothing is written**: this is a confirm, not
/// a repair, and a read that flipped the row would emit a
/// `checkout.session.expired` outside the sweep's transaction (and outside
/// its `NOT EXISTS` live-charge guard). It is the same rule and the same
/// reasoning as `crate::browser::checkout_sessions::authenticate`'s sixth
/// refusal, which is why a payer whose session read has started answering 404
/// cannot get a different verdict out of the confirm. The sweep is what makes
/// `status` honest to a *merchant*; the read is what makes it honest to a
/// payer, and neither waits for the other.
///
/// # Errors
///
/// [`ApiError::CheckoutSessionNotOpen`] for a session that is over, and
/// [`ApiError::Internal`] for a `status` outside migration `0028`'s
/// `status_is_known` CHECK — which is this deployment's schema disagreeing
/// with this binary, not anything a caller did.
fn verdict(session: &CheckoutSessionRow, now: OffsetDateTime) -> Result<(), ApiError> {
    let refused = |state| {
        Err(ApiError::CheckoutSessionNotOpen {
            session_id: session.id.clone(),
            state,
        })
    };

    match session.status.as_str() {
        crate::v1::checkout_sessions::OPEN if now < session.expires_at => Ok(()),
        crate::v1::checkout_sessions::OPEN => {
            tracing::debug!(
                checkout_session_id = %session.id,
                "a confirm reached a checkout session that is past its horizon and not yet \
                 swept; refusing it as expired without writing anything"
            );
            refused(ClosedSession::Expired)
        }
        "expired" => refused(ClosedSession::Expired),
        "complete" => refused(ClosedSession::Complete),
        other => Err(ApiError::Internal(format!(
            "checkout_sessions.status holds `{other}`, which is not one of open/complete/expired"
        ))),
    }
}

/// What the checkout session says about a confirm on `payment_intent_id`.
///
/// A free function over the trait rather than a method call at the confirm
/// site, so the confirm path names *what it is asking* — "may this be
/// confirmed, and where does the payer come back to?" — without importing
/// `vpay_db::CheckoutSessions` or learning what a session row looks like.
///
/// # Where its answer goes, and why that moved
///
/// Into `charges.return_url`, before the charge is committed — not into the
/// rail call afterwards. Until Step 9's lane 1b this was resolved *after*
/// `open_attempt` and handed straight to the adapter, so a session-driven
/// charge stored the merchant's URL and told the rail vpay's. Three things
/// were wrong with that, and all three are the same bug:
///
/// * `vpay_worker::handlers::charge_ref` fills `ChargeRef::return_url` from
///   `charges.return_url`, so a redirect charge ever resubmitted by the
///   worker would have sent the *merchant's* URL where the confirm sent
///   vpay's. Safe only because no redirect charge is resubmitted today
///   (`docs/plans/step9-notes/lane-2.md` §3), which is a property of another
///   module;
/// * `next_action.redirect_to_url.return_url` is rendered from the charge row
///   on every later read, so a merchant polling their own intent was shown a
///   URL the payer was never sent to;
/// * a value that reaches a rail without being in the committed row is a
///   value a crash can lose (`docs/flows/crash-safety.md`).
///
/// Resolved before `open_attempt` rather than after it, which is also what
/// makes both refusals below cost no charge row, no `provider_requests` row
/// and no job.
///
/// # Errors
///
/// Whatever [`CheckoutSessionGate::admit_confirm`] answers.
pub(crate) async fn admit_confirm<S>(
    gate: &S,
    payment_intent_id: &str,
) -> Result<Option<String>, ApiError>
where
    S: CheckoutSessionGate + ?Sized,
{
    gate.admit_confirm(payment_intent_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use vpay_core::Classify as _;

    /// A source that answers with whatever it was built with, so
    /// [`admit_confirm`] can be exercised without a database.
    ///
    /// The *precedence* between a session's page and the merchant's own URL
    /// used to be tested here, against this fake. It is not a question this
    /// module answers any more: since Step 9's lane 1b the winner is chosen
    /// by `crate::v1::payment_intents`' `payer_instrument`, which is
    /// synchronous and is tested directly rather than through a stand-in —
    /// see `a_session_driven_redirect_confirm_needs_no_return_url_and_ignores_one`.
    struct Fixed(Option<String>);

    #[async_trait]
    impl CheckoutSessionGate for Fixed {
        async fn admit_confirm(&self, _: &str) -> Result<Option<String>, ApiError> {
            Ok(self.0.clone())
        }
    }

    const SESSION: &str = "https://checkout.example/c/cs_123/return?t=tok&key=pk_test_x";

    /// The lookup is a pass-through, and both *admitting* answers are
    /// ordinary.
    #[tokio::test]
    async fn the_lookup_carries_both_answers_and_neither_is_an_error() {
        assert_eq!(
            admit_confirm(&Fixed(Some(SESSION.to_owned())), "pi_1")
                .await
                .expect("the lookup succeeds")
                .as_deref(),
            Some(SESSION)
        );
        assert_eq!(
            admit_confirm(&Fixed(None), "pi_1")
                .await
                .expect("the lookup succeeds"),
            None,
            "no session is the ordinary case for every direct confirm, not an error"
        );
    }

    /// A row in whatever state the caller names, with every other column set
    /// to something a real session would carry.
    ///
    /// Built by hand rather than by inserting through the repository, because
    /// two of the states below (`complete` under an unsettled intent, and an
    /// `open` row past its horizon) are precisely the states no sequence of
    /// shipping operations produces at the moment a confirm arrives — which
    /// is what makes them worth pinning here.
    fn row(status: &str, expires_at: OffsetDateTime) -> CheckoutSessionRow {
        let created_at = expires_at - time::Duration::hours(24);
        CheckoutSessionRow {
            id: "cs_00000000000000000000000001".to_owned(),
            seq: 1,
            merchant_id: "acme".to_owned(),
            payment_intent_id: "pi_00000000000000000000000001".to_owned(),
            livemode: false,
            ui_mode: "hosted".to_owned(),
            status: status.to_owned(),
            payment_status: "unpaid".to_owned(),
            success_url: Some("https://shop.example/ok".to_owned()),
            cancel_url: Some("https://shop.example/cancel".to_owned()),
            return_url: None,
            publishable_key: "pk_test_acmecameroonsandbox01".to_owned(),
            client_secret_suffix: "a".repeat(32),
            return_token: "b".repeat(32),
            expires_at,
            created_at,
            updated_at: created_at,
        }
    }

    /// The whole rule, at one instant, in one table.
    ///
    /// **Revert-proof.** Make [`verdict`] consult `status` alone — drop the
    /// `now < session.expires_at` guard on the `open` arm — and the third row
    /// fails: an abandoned checkout the sweep has not reached yet would admit
    /// a payment. Delete the function's call site in
    /// [`CheckoutSessionGate::admit_confirm`] and the integration cases in
    /// `backends/tests/integration/tests/checkout_sessions.rs` fail instead;
    /// this test would still pass, which is why both exist.
    #[test]
    fn only_an_open_session_inside_its_horizon_admits_a_confirm() {
        let now = OffsetDateTime::now_utc();
        let future = now + time::Duration::hours(1);
        let past = now - time::Duration::seconds(1);

        verdict(&row("open", future), now).expect("an open session inside its horizon admits");

        for (label, session, expected_code) in [
            (
                "an open session past its horizon, which no sweep has reached yet",
                row("open", past),
                "checkout_session_expired",
            ),
            (
                "a session the sweep or the merchant expired",
                row("expired", future),
                "checkout_session_expired",
            ),
            (
                "a session the settlement transaction finished",
                row("complete", future),
                "checkout_session_complete",
            ),
        ] {
            let error = verdict(&session, now).expect_err(label);
            assert_eq!(error.code(), expected_code, "{label}");
            assert_eq!(
                error.category().http_status(),
                409,
                "{label}: the refusal is a conflict about the object's state"
            );
            assert!(
                error.public_message().contains(&session.id),
                "{label}: a merchant must be told which session refused: {}",
                error.public_message()
            );
        }
    }

    /// The horizon is exactly `expires_at`, and the boundary is closed at the
    /// same end `crate::browser::checkout_sessions::authenticate` closes it:
    /// a session is over the instant its `expires_at` arrives, not one tick
    /// later. Two refusals that disagreed by a tick would let a payer be
    /// refused the session read and admitted the confirm.
    #[test]
    fn the_horizon_is_closed_at_the_instant_it_names() {
        let expires_at = OffsetDateTime::now_utc();
        let session = row("open", expires_at);

        verdict(&session, expires_at - time::Duration::nanoseconds(1))
            .expect("a nanosecond before the horizon still admits");
        assert_eq!(
            verdict(&session, expires_at)
                .expect_err("the horizon itself refuses")
                .code(),
            "checkout_session_expired"
        );
    }

    /// A `status` this binary does not know is **ours**, not the caller's:
    /// the column's CHECK is the only thing that writes it, so reading one is
    /// this deployment's schema disagreeing with this build. It answers 500
    /// and pages rather than being folded into either refusal — a merchant
    /// told "your checkout expired" about a row nobody can interpret would be
    /// vpay guessing on a payment path.
    #[test]
    fn an_unknown_status_is_ours_and_pages() {
        let error = verdict(
            &row(
                "cancelled",
                OffsetDateTime::now_utc() + time::Duration::hours(1),
            ),
            OffsetDateTime::now_utc(),
        )
        .expect_err("an unknown status cannot admit a confirm");

        assert_eq!(error.category().http_status(), 500);
        assert_eq!(error.code(), "internal_error");
        assert!(
            !error.public_message().contains("cancelled"),
            "an internal error says nothing about its payload: {}",
            error.public_message()
        );
    }
}
