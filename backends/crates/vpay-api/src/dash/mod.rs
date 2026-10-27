//! `/dash/v1` — the management console's read surface (ADR-0008, ADR-0009,
//! [ADR-0036](../../../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md),
//! [docs/flows/dashboard.md](../../../../../docs/flows/dashboard.md)).
//!
//! # A signed message under a person's key
//!
//! Every request here is a COSE_Sign1 message signed by a **console key**: a
//! P-256 key whose private half lives non-extractable in a person's browser,
//! paired through vaam's admin phone (`POST /svc/v1/console_keys`) or by an
//! operator (`vpay-server staff pair-key`), and living exactly 30 days. There is
//! no session, no access token, no authorization code and no password
//! (owner rulings, 2026-09-29, 2026-10-09 and 2026-10-26); the layer that opens
//! the request is [`crate::envelope`], and the step that turns the verified key
//! into a tenant is `crate::envelope::console::dash_scope`.
//!
//! **Until VP-6b ships the console's own sign-in, the dashboard application
//! cannot reach this surface**: it still signs in with a password it no longer
//! has a server for. `docs/status.md` says so; the way in meanwhile is `staff
//! pair-key`.
//!
//! # The boundary, stated once
//!
//! > **A `/dash/v1` request reads exactly one tenant's rows: the one
//! > `dashboard.merchant_id` names, fixed in YAML and checked at boot — unless
//! > the key belongs to an administrator, who may name another registered
//! > merchant (ADR-0018).**
//!
//! Four things have to hold for that, each checked in a different place so that
//! no single edit removes the boundary:
//!
//! 1. the signature verifies under a **live** console key (the resolver reads
//!    `console_keys` on every request: a revoked or expired key is the layer's
//!    coarse `401`);
//! 2. the member that key belongs to is active and belongs to the dashboard's
//!    merchant (re-read on every request);
//! 3. the tenant is [`DashboardTenancy`], which only that step constructs; and
//! 4. every query filters by the bound `merchant_id`, through the same
//!    [`crate::MerchantScope`] the merchant surface uses — so a handler here
//!    cannot query unfiltered any more than a `/v1` handler can.
//!
//! # Read-only, and why that is structural rather than a promise
//!
//! ADR-0008's dashboard performs per-record write operations with an
//! `audit_log` row each. None of that is built, so this surface mounts `GET`
//! and nothing else — and the layer's binding lists only the methods of
//! [`DASH_ROUTES`], so a write is the layer's own `405` before the router
//! matches, rather than relying on no `post(..)` being present. A write mounted
//! here without an audit log would then be a *refused* request rather than an
//! unlogged one.
//!
//! # Not an SDK surface
//!
//! `docs/sdks/parity.md` does not cover these routes and must not: the
//! merchant SDKs speak `/v1`, and a dashboard endpoint in a merchant SDK
//! would be a merchant credential reaching for a staff surface. The wire
//! shapes below reuse [`crate::model`]'s objects because the dashboard must
//! see what the merchant sees — not because anything generates a client from
//! them.

use std::sync::Arc;

use axum::Router;
use axum::routing::{MethodRouter, get};

pub mod payment_intents;

/// The path [`crate::router`] mounts this surface at.
///
/// `/dash/v1`, not `/dashboard/v1`: it is the spelling ADR-0008 and ADR-0009
/// fix, and the one every `dashboard_client.redirect_uris` in
/// `config/application.yml` already points at. (It was also the spelling the
/// retired `vpay:dash/v1` audience was built from; ADR-0017 retired that
/// constant, and the prefix outlived it.) A prefix is cheap to change
/// and expensive to change *twice*, so it is named here once and nowhere
/// else.
pub const DASH_NEST: &str = "/dash/v1";

/// One mounted `/dash/v1` route — the same shape as [`crate::V1Route`], and
/// for the same reason: axum 0.8 cannot enumerate a built `Router`, so the
/// binding the signed transport resolves operations from, and the boundary test
/// that proves every route answers `401` unsigned, have to walk a table rather
/// than the router.
#[derive(Debug)]
pub struct DashRoute {
    /// The axum path pattern, relative to [`DASH_NEST`].
    pub path: &'static str,
    /// Every HTTP method this path answers, upper-case.
    pub methods: &'static [&'static str],
    /// Builds the handlers.
    pub(crate) mount: fn() -> MethodRouter<crate::AppState>,
}

/// Every route mounted under `/dash/v1`, and the only place they are listed.
///
/// Two, both `GET`. The other slices `docs/flows/dashboard.md` names —
/// webhooks, sessions, balances, settings, rail health — are deliberately
/// absent rather than present-and-empty: a route that answered `{"data":
/// []}` for a surface nobody wrote would be indistinguishable from a
/// deployment that has none of those things.
pub const DASH_ROUTES: &[DashRoute] = &[
    DashRoute {
        path: "/payment_intents",
        methods: &["GET"],
        mount: || get(payment_intents::list),
    },
    DashRoute {
        path: "/payment_intents/{id}",
        methods: &["GET"],
        mount: || get(payment_intents::retrieve),
    },
];

/// The `/dash/v1` router, built by folding [`DASH_ROUTES`].
///
/// Returns a `Router` that still needs state and the signed-transport layers,
/// exactly as [`crate::v1::routes`] does — so this function cannot
/// accidentally be mounted unauthenticated.
pub(crate) fn routes() -> Router<crate::AppState> {
    DASH_ROUTES
        .iter()
        .fold(Router::new(), |router, route| {
            router.route(route.path, (route.mount)())
        })
        // Inside the nest, for `crate::v1::routes`' reason: an unmatched
        // `/dash/v1/...` path is this crate's envelope, and — because axum
        // flattens a nest without its own fallback into the outer path table
        // — without this line it would be answered by whichever other nest's
        // wildcard matched first.
        .fallback(crate::not_found)
}

/// The list URL rendered into a `ListObject`'s `url`, and the prefix every
/// path in this module is relative to.
///
/// Built from [`DASH_NEST`] rather than written out, so the two cannot
/// disagree — a `url` naming a path the router does not serve is a link a
/// reader would follow to a 404.
pub(crate) fn nested(path: &str) -> String {
    format!("{DASH_NEST}{path}")
}

/// The repositories handle a `/dash/v1` handler reads through — the same
/// trait object `/v1` uses, so a dashboard read and a merchant read of the
/// same row cannot diverge.
pub(crate) type Repos = Arc<dyn vpay_db::Repositories>;

/// Which merchant one validated `/dash/v1` request may read, and whether
/// that is the registration's own tenant or one an admin named instead
/// ([ADR-0018](../../../../../docs/adr/0018-cross-tenant-admin-reads.md)).
///
/// **This is the seam.** `crate::envelope::console::dash_scope` is the only
/// place that constructs one — from [`vpay_db::StaffRow::is_admin`] and, for
/// an admin only, a `?merchant_id=` on the request — and the CrateStack
/// procedure transport mints its `auth()` context from exactly this rather than
/// from a second, parallel decision about tenancy. Two
/// things follow from a request having exactly one of these rather than an
/// `Option<String>` plus a stray `bool`:
///
/// * a handler that wants "which merchant" and a handler that wants "was
///   this chosen by an admin" read the same value, so the two can never
///   disagree about a request neither of them decided;
/// * an admin session that named its own tenant is [`Self::Bound`], not
///   [`Self::ChosenByAdmin`] — naming the merchant you already read is not a
///   cross-tenant read, and [`Self::is_cross_tenant`] is exactly the
///   predicate an audit line or a future metric wants, so it must answer
///   `false` for that case rather than `true` for every admin request.
///
/// # What an admin naming no tenant answers, and why
///
/// `dash_scope` resolves an admin request with no
/// `?merchant_id=` to [`Self::Bound`] — the registration's own tenant,
/// exactly like a non-admin. ADR-0018 § "What a cross-tenant read answers
/// when no tenant is named" has the argument in full; the short version is
/// that every repository method this surface calls
/// (`PaymentIntents::list_page_filtered`, `get_for_merchant`, …) takes
/// exactly one `merchant_id` and orders its cursor within that one tenant,
/// so a merged, all-merchants page has no repository behind it to answer
/// from — building one is a pagination design this ADR does not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardTenancy {
    /// The registration's own tenant. Every non-admin session resolves
    /// here unconditionally — nothing on the wire can move it — and so
    /// does an admin session that named no tenant, or named its own.
    Bound(String),
    /// An admin session that named a tenant **other than** the
    /// registration's own, via `?merchant_id=`.
    ChosenByAdmin(String),
}

impl DashboardTenancy {
    /// The tenant every repository read behind this request must filter by.
    #[must_use]
    pub fn merchant_id(&self) -> &str {
        match self {
            Self::Bound(merchant_id) | Self::ChosenByAdmin(merchant_id) => merchant_id,
        }
    }

    /// Whether this request is reading a merchant other than the one its
    /// dashboard registration is bound to — the fact ADR-0018's "blast
    /// radius" section asks to be logged on every occurrence, since it is
    /// the one thing a mis-set [`vpay_db::StaffRow::is_admin`] can do that a
    /// non-admin session never could.
    #[must_use]
    pub fn is_cross_tenant(&self) -> bool {
        matches!(self, Self::ChosenByAdmin(_))
    }
}

impl<S> axum::extract::FromRequestParts<S> for DashboardTenancy
where
    S: Send + Sync,
{
    type Rejection = crate::ApiError;

    /// Fails closed with `ApiError::Internal` (500, paged), for
    /// [`crate::v1::MerchantScope`]'s `FromRequestParts` impl's reason:
    /// reaching a `/dash/v1` handler with no tenancy on the request means
    /// the console scope step (`envelope::console::dash_scope`) is not mounted in front of it.
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Self>().cloned().ok_or_else(|| {
            crate::ApiError::Internal(
                "a /dash/v1 handler ran with no DashboardTenancy on the request: the dashboard \
                 authentication middleware is not mounted in front of this route"
                    .to_owned(),
            )
        })
    }
}

/// The one query parameter the console scope step reads before a
/// handler ever sees the request, and it reads it **only** when the staff
/// row it just re-read carries `is_admin: true` — see
/// [`DashboardTenancy`]'s doc for why a non-admin's copy of this parameter,
/// however spelled, must never even be parsed.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AdminMerchantOverride {
    pub(crate) merchant_id: Option<String>,
}

#[cfg(test)]
mod tenancy_tests {
    use super::DashboardTenancy;

    #[test]
    fn bound_and_chosen_by_admin_both_answer_their_own_merchant_id() {
        assert_eq!(
            DashboardTenancy::Bound("acme".to_owned()).merchant_id(),
            "acme"
        );
        assert_eq!(
            DashboardTenancy::ChosenByAdmin("beta".to_owned()).merchant_id(),
            "beta"
        );
    }

    #[test]
    fn only_chosen_by_admin_is_cross_tenant() {
        assert!(!DashboardTenancy::Bound("acme".to_owned()).is_cross_tenant());
        assert!(DashboardTenancy::ChosenByAdmin("beta".to_owned()).is_cross_tenant());
    }

    /// The seam is **extractable**, and it fails closed when it is not there.
    ///
    /// Added by the ADR-0018 review, because until it existed the
    /// `FromRequestParts` impl above had no caller anywhere: nothing in
    /// `/dash/v1` reads a `DashboardTenancy` yet — the plan's Lane C is what
    /// will — so the whole extractor, including its fail-closed arm, was
    /// code the test suite never ran. An impl the first consumer discovers
    /// is broken is not a seam; this is the cheapest thing that makes it
    /// one.
    ///
    /// The absent arm is the half that matters. It must be a **500**, not a
    /// default tenant: reaching a handler with no tenancy on the request
    /// means the console scope step is not mounted in front of it, and
    /// the one thing an extractor must never do there is invent a merchant
    /// id for an unauthenticated request to read.
    #[tokio::test]
    async fn the_seam_is_extractable_and_fails_closed_when_it_is_absent() {
        use axum::extract::FromRequestParts;

        let (mut parts, _) = axum::http::Request::builder()
            .uri("/dash/v1/payment_intents")
            .body(())
            .expect("building a request")
            .into_parts();

        let rejection = DashboardTenancy::from_request_parts(&mut parts, &())
            .await
            .expect_err("no middleware ran, so there is no tenancy to extract");
        assert!(
            matches!(rejection, crate::ApiError::Internal(_)),
            "an absent tenancy must fail closed, never resolve to a tenant: {rejection:?}"
        );

        parts
            .extensions
            .insert(DashboardTenancy::ChosenByAdmin("beta".to_owned()));
        let extracted = DashboardTenancy::from_request_parts(&mut parts, &())
            .await
            .expect("the tenancy the middleware inserted is readable by a handler");
        assert_eq!(
            extracted,
            DashboardTenancy::ChosenByAdmin("beta".to_owned())
        );
        assert_eq!(extracted.merchant_id(), "beta");
        assert!(extracted.is_cross_tenant());
    }
}
