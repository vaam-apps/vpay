//! Where a redirect rail sends the payer when its own page is done with them.
//!
//! One question, asked once per confirm, answered in one place: is this charge
//! driven by a **checkout session** vpay is hosting a page for, or is it a
//! direct `/v1` (or `/v1/browser`) confirm whose payer belongs to the
//! merchant's own site? The answer is written to `charges.return_url` before
//! the charge is committed, and from there becomes
//! `vpay_provider::ChargeRef::return_url`, which
//! `vpay-adapter-orange-money` sends as both `return_url` and `cancel_url` —
//! see D2 of `docs/plans/2026-09-04-step9-hosted-checkout.md` and
//! `docs/reference/rails.md`.
//!
//! One row, one URL: what the rail is told, what a later read of the intent
//! renders as `next_action.redirect_to_url.return_url`, and what the worker
//! would resubmit under are the same stored value, because they are the same
//! column. See [`session_return_page`] for what that replaced.
//!
//! It is a module of its own, and a trait rather than two lines inside
//! [`crate::v1::payment_intents`], because the two branches are owned by
//! different things: the merchant's URL is a column this crate already writes,
//! and the session's URL is a row plus a configured origin that
//! `vpay-db`/`vpay-config` own. Keeping the question behind
//! [`ReturnUrlSource`] is what lets the answer change without the confirm path
//! learning what a checkout session is.

use async_trait::async_trait;
use vpay_db::{CheckoutSessions, Repositories};

use crate::error::{ApiError, CHECKOUT_SESSION_WITHOUT_CHECKOUT_APP};

/// The single lookup the confirm path needs about checkout sessions.
///
/// Deliberately narrow: not "give me the session", but "give me the URL, if
/// any". A confirm has no other business with a session — it does not read
/// one, render one, or move one — and a wider trait here would invite it to.
#[async_trait]
pub(crate) trait ReturnUrlSource: Send + Sync {
    /// vpay's own return page for the open checkout session driving
    /// `payment_intent_id`, or `None` when no session drives it.
    ///
    /// # Errors
    ///
    /// [`ApiError::Db`] if the lookup fails, and
    /// [`ApiError::CheckoutNotConfigured`] if a session drives the intent on
    /// a deployment that serves no checkout page — see
    /// [`SessionReturnPage`]'s impl for why that is refused rather than
    /// answered with the merchant's own URL. It is **not** an error for there
    /// to be no session: that is the ordinary case for every direct confirm.
    async fn session_return_url(&self, payment_intent_id: &str)
    -> Result<Option<String>, ApiError>;
}

/// The shipping answer: the open checkout session driving an intent,
/// rendered as vpay's own return page.
///
/// # Why it is a struct and not `impl ReturnUrlSource for dyn Repositories`
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
pub(crate) struct SessionReturnPage<'a> {
    repositories: &'a dyn Repositories,
    /// `ResourceConfig::checkout_public_base_url()`, already stripped of a
    /// trailing slash at boot.
    checkout_base: Option<&'a str>,
}

impl<'a> SessionReturnPage<'a> {
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
/// own page, and `checkout_sessions_one_open_per_intent` makes "the open
/// session" a well-formed phrase rather than a `LIMIT 1` over an ambiguous
/// set (`vpay_db::CheckoutSessions::find_open_by_intent`).
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
#[async_trait]
impl ReturnUrlSource for SessionReturnPage<'_> {
    async fn session_return_url(
        &self,
        payment_intent_id: &str,
    ) -> Result<Option<String>, ApiError> {
        let Some(session) =
            CheckoutSessions::find_open_by_intent(self.repositories, payment_intent_id).await?
        else {
            return Ok(None);
        };

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

/// The session's return page for `payment_intent_id`, or `None`.
///
/// A free function over the trait rather than a method call at the confirm
/// site, so the confirm path names *what it is asking* — "is a checkout
/// session driving this?" — without importing `vpay_db::CheckoutSessions` or
/// learning what a session row looks like.
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
/// makes the refusal below cost no charge row.
///
/// # Errors
///
/// Whatever [`ReturnUrlSource::session_return_url`] answers.
pub(crate) async fn session_return_page<S>(
    source: &S,
    payment_intent_id: &str,
) -> Result<Option<String>, ApiError>
where
    S: ReturnUrlSource + ?Sized,
{
    source.session_return_url(payment_intent_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source that answers with whatever it was built with, so
    /// [`session_return_page`] can be exercised without a database.
    ///
    /// The *precedence* between a session's page and the merchant's own URL
    /// used to be tested here, against this fake. It is not a question this
    /// module answers any more: since Step 9's lane 1b the winner is chosen
    /// by `crate::v1::payment_intents`' `payer_instrument`, which is
    /// synchronous and is tested directly rather than through a stand-in —
    /// see `a_session_driven_redirect_confirm_needs_no_return_url_and_ignores_one`.
    struct Fixed(Option<String>);

    #[async_trait]
    impl ReturnUrlSource for Fixed {
        async fn session_return_url(&self, _: &str) -> Result<Option<String>, ApiError> {
            Ok(self.0.clone())
        }
    }

    const SESSION: &str = "https://checkout.example/c/cs_123/return?t=tok&key=pk_test_x";

    /// The lookup is a pass-through, and both answers are ordinary.
    #[tokio::test]
    async fn the_lookup_carries_both_answers_and_neither_is_an_error() {
        assert_eq!(
            session_return_page(&Fixed(Some(SESSION.to_owned())), "pi_1")
                .await
                .expect("the lookup succeeds")
                .as_deref(),
            Some(SESSION)
        );
        assert_eq!(
            session_return_page(&Fixed(None), "pi_1")
                .await
                .expect("the lookup succeeds"),
            None,
            "no session is the ordinary case for every direct confirm, not an error"
        );
    }
}
