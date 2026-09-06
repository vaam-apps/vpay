//! `/dash/v1` — the staff dashboard's read surface (ADR-0008, ADR-0009,
//! [docs/flows/dashboard.md](../../../../../docs/flows/dashboard.md)).
//!
//! # STATUS, before anything else
//!
//! **No client of this deployment can obtain a token for this surface.**
//! `/dash/v1` requires an access token whose `aud` is
//! [`vpay_config::DASHBOARD_AUDIENCE`], and the only grant vpay serves is
//! `client_credentials` on `/v1/oauth`, which is registered for merchant
//! clients alone and now *refuses* to be registered for this audience
//! (`vpay_config::ConfigError::MerchantClaimsDashboardAudience`). The
//! authorization-code grant that would mint one is **not served**: it needs a
//! decision about how a human staff member authenticates that this
//! repository has never taken. See `docs/flows/dashboard-auth.md`'s Status
//! section and `docs/status.md`.
//!
//! So what is mounted here is a *resource server with no issuer*. That is a
//! deliberate half, not an oversight, and the half it is matters: the
//! tenancy boundary — which rows a dashboard credential may read — is the
//! part that has to be right before a login exists, not after. Nothing here
//! fabricates a session, a token or a row.
//!
//! # The boundary, stated once
//!
//! > **A `/dash/v1` request reads exactly one tenant's rows: the one
//! > `dashboard_client.merchant_id` names, fixed in YAML and checked at
//! > boot.**
//!
//! Four things have to hold for that, and each is checked in a different
//! place so that no single edit removes the boundary:
//!
//! 1. the token validates against vpay's own JWKS for
//!    [`crate::resource_auth::Surface::Dashboard`] — signature, expiry,
//!    issuer and audience ([`crate::resource_auth::JwtValidator`]);
//! 2. its `client_id` is the registered dashboard client's. The audience
//!    alone is not enough: `aud` says which *surface*, `sub` says which
//!    *credential*, and a deployment that ever registers two dashboard
//!    clients must not let one read the other's tenant;
//! 3. it carries the registration's single scope
//!    (`docs/flows/dashboard-auth.md`'s "Scope");
//! 4. every query filters by the bound `merchant_id`, through the same
//!    [`crate::MerchantScope`] the merchant surface uses — so a handler
//!    here cannot query unfiltered any more than a `/v1` handler can.
//!
//! (1)–(3) are [`crate::require_dashboard_token`]. (4) is the repository
//! methods, which take a `merchant_id` and have no unscoped variant on this
//! path.
//!
//! # Read-only, and why that is structural rather than a promise
//!
//! ADR-0008's dashboard performs per-record write operations with an
//! `audit_log` row each. None of that is built (`docs/flows/dashboard-auth.md`'s
//! "Scope"), so this surface mounts `GET` and nothing else — and
//! [`crate::require_dashboard_token`] refuses any other method outright,
//! before the router matches, rather than relying on no `post(..)` being
//! present. A write mounted here without an audit log would then be a
//! *refused* request rather than an unlogged one.
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
use axum::http::Method;
use axum::routing::{MethodRouter, get};

use crate::v1::DashboardBinding;

pub mod payment_intents;

/// The path [`crate::router`] mounts this surface at.
///
/// `/dash/v1`, not `/dashboard/v1`: it is the spelling ADR-0008 and ADR-0009
/// fix, the one `vpay_config::DASHBOARD_AUDIENCE` (`vpay:dash/v1`) is built
/// from, and the one every `dashboard_client.redirect_uris` in
/// `config/application.yml` already points at. A prefix is cheap to change
/// and expensive to change *twice*, so it is named here once and nowhere
/// else.
pub const DASH_NEST: &str = "/dash/v1";

/// One mounted `/dash/v1` route — the same shape as [`crate::V1Route`], and
/// for the same reason: axum 0.8 cannot enumerate a built `Router`, so the
/// boundary test that proves every route answers `401` without a token has
/// to walk a table rather than the router.
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
/// Returns a `Router` that still needs state and the authentication layer,
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

/// Which scopes satisfy a `/dash/v1` request.
///
/// One scope, from the registration, and only for read methods: everything
/// else is refused before the router matches. That is the opposite shape
/// from [`crate::v1::required_scopes`], which maps *every* method to a scope
/// — and deliberately so. `/v1` has writes; this surface has none, so
/// "which scope may write here?" has no answer to give, and the honest
/// encoding of that is `None`.
///
/// Returning `None` means **refuse**, not "no scope needed". The caller
/// ([`crate::require_dashboard_token`]) answers `405` for it, because the
/// request named a method this surface does not serve.
#[must_use]
pub(crate) fn required_scope<'a>(
    method: &Method,
    binding: &'a DashboardBinding,
) -> Option<&'a str> {
    match *method {
        // `HEAD` alongside `GET` for `v1::required_scopes`' reason: axum
        // answers it from the same `get(..)` handler.
        Method::GET | Method::HEAD => Some(binding.scope.as_str()),
        _ => None,
    }
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
