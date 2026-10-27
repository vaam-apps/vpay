//! Which operation of a hand-written surface a request addresses, as the signed
//! binding names it (VP-3; `/dash/v1` and `/svc/v1` since VP-6a).
//!
//! cratestack's `RestBindingResolver` reads a generated router's
//! `ROUTE_TRANSPORTS`. `/v1`, the console's reads and `/svc/v1` are
//! hand-written, so [`TableBinding`] is the same decision over each surface's
//! route table ([`V1_ROUTES`], [`DASH_ROUTES`], [`SVC_ROUTES`]): the route template axum matched, with the
//! `/v1` mount stripped, is the `route` a client binds; the matched path
//! parameters, in axum's order, are the `path_params`; and the operation's key
//! into `vpay_wire::V1_CONTRACTS` is `"<METHOD> <template>"`.
//!
//! # Payload types, and why they are the same on every route
//!
//! `/v1` takes `application/x-www-form-urlencoded` bodies and answers
//! `application/json` — the Stripe shape, kept (owner's ruling D6) — so every
//! operation declares exactly that pair and the layer is configured with it.
//! No `/v1` handler reads JSON (`VpayForm` is the only body extractor), so no
//! route declares it; the day one does, it is a row here and not a second layer
//! of configuration. A `GET` or `DELETE` seals an empty payload; the layer does
//! not check the type that empty payload names.
//!
//! # What it deliberately does not resolve
//!
//! * A path the router did not match (`matched_path()` is `None`), or matched
//!   but not a `V1_ROUTES` row (bare `/v1`, `/v1//x`, an unknown path — all of
//!   which land on the nest's `/v1/{*rest}` catch-all): [`Resolution::NotAnOp`]
//!   or, for no match at all, `Unresolved`. A plain request reaches the nest's
//!   fallback and gets its honest `404`; a COSE body for it is refused with the
//!   unsigned `415`. There is no operation to bind and so nothing to seal.
//! * A method `V1_ROUTES` does not list for a known path: the layer answers its
//!   own `405` (with `Allow`) before the router can, under
//!   [`EnvelopeMode::Required`](cratestack_axum::envelope_layer::EnvelopeMode).
//! * `/v1/browser/*` and `/v1/oauth/*`: separate nests, mounted beside this one
//!   and outside the layer; their matched paths are never `/v1/<template>` of a
//!   `V1_ROUTES` row (and the test below proves it for every row of both
//!   tables that could collide).

use axum::http::Method;
use cratestack_axum::envelope_layer::{BindingResolver, Resolution, ResolvedRoute, RouteRequest};
use vpay_wire::{DASH_MOUNT, SVC_MOUNT, V1_MOUNT, operation_key};

use crate::dash::DASH_ROUTES;
use crate::svc::SVC_ROUTES;
use crate::v1::V1_ROUTES;

/// The request payload types every `/v1` operation declares.
pub const REQUEST_TYPES: &[&str] = &["application/x-www-form-urlencoded"];
/// The response payload types every `/v1` operation answers in.
pub const RESPONSE_TYPES: &[&str] = &["application/json"];
/// The payload types `/dash/v1`'s reads and `/svc/v1` take and answer in: JSON
/// both ways. The console's reads carry an empty payload (a `GET` seals one);
/// `/svc/v1`'s writes are JSON bodies.
pub const JSON_TYPES: &[&str] = &["application/json"];

/// The [`BindingResolver`] over one surface's route table.
///
/// Built once at boot from a table of `(template, methods)` rows and the
/// payload types the surface declares; the three constructors are the three
/// hand-written surfaces, and a fourth is a new constructor and not a new
/// resolver.
#[derive(Debug, Clone)]
pub struct TableBinding {
    mount: &'static str,
    routes: Vec<(&'static str, &'static [&'static str])>,
    request_types: &'static [&'static str],
    response_types: &'static [&'static str],
}

impl TableBinding {
    /// `/v1`: forms in, JSON out.
    #[must_use]
    pub fn v1() -> Self {
        Self {
            mount: V1_MOUNT,
            routes: V1_ROUTES
                .iter()
                .map(|route| (route.path, route.methods))
                .collect(),
            request_types: REQUEST_TYPES,
            response_types: RESPONSE_TYPES,
        }
    }

    /// `/dash/v1`'s hand-written reads: JSON.
    #[must_use]
    pub fn dash() -> Self {
        Self {
            mount: DASH_MOUNT,
            routes: DASH_ROUTES
                .iter()
                .map(|route| (route.path, route.methods))
                .collect(),
            request_types: JSON_TYPES,
            response_types: JSON_TYPES,
        }
    }

    /// `/svc/v1`: JSON.
    #[must_use]
    pub fn svc() -> Self {
        Self {
            mount: SVC_MOUNT,
            routes: SVC_ROUTES
                .iter()
                .map(|route| (route.path, route.methods))
                .collect(),
            request_types: JSON_TYPES,
            response_types: JSON_TYPES,
        }
    }

    /// The request and response payload types this surface declares and its
    /// layer is configured to seal.
    #[must_use]
    pub fn payload_types(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (self.request_types, self.response_types)
    }

    /// `matched` without this surface's mount, or `None` if it is not under it.
    ///
    /// A path is under the mount only at a segment boundary: `/v1x/...` is not
    /// `/v1`'s.
    fn template_of<'a>(&self, matched: &'a str) -> Option<&'a str> {
        matched
            .strip_prefix(self.mount)
            .filter(|rest| rest.starts_with('/'))
    }
}

impl BindingResolver for TableBinding {
    fn resolve(&self, request: &RouteRequest<'_>) -> Resolution {
        // No route matched at all (a 404 the nest's fallback answers): nothing
        // to bind, and the layer passes it through. Anything *matched* that is
        // not a table row — the bare mount, the nest's own `/{*rest}` catch-all
        // that an unknown path lands on — is `NotAnOp`, and never
        // `Unresolved`: under `Required`, `Unresolved` on a matched route is
        // cratestack's "envelope misconfigured" unsigned `500`, which is the
        // wrong answer for a path that simply does not exist. `NotAnOp` lets a
        // plain request through to the fallback's honest 404 and refuses a COSE
        // body for it with the unsigned 415 (there is no operation to have
        // bound it to).
        let Some(matched) = request.matched_path() else {
            return Resolution::Unresolved;
        };
        let Some(template) = self.template_of(matched) else {
            return Resolution::NotAnOp;
        };
        // axum answers a `HEAD` with the `GET` handler and the tables list
        // only the latter; the binding keeps the real method, the contract
        // key does not.
        let method = match request.method().as_str() {
            "HEAD" => "GET",
            other => other,
        };
        let mut on_path = self.routes.iter().filter(|(path, _)| *path == template);
        if let Some((path, _)) = on_path
            .clone()
            .find(|(_, methods)| methods.contains(&method))
        {
            let params = request
                .path_params()
                .iter()
                .map(|(_, value)| value.clone())
                .collect();
            return Resolution::Op(
                ResolvedRoute::new(*path, params)
                    .with_contract_key(operation_key(method, path))
                    .with_payload_types(self.request_types, self.response_types),
            );
        }
        let allow: Vec<Method> = on_path
            .by_ref()
            .flat_map(|(_, methods)| methods.iter())
            .filter_map(|method| method.parse().ok())
            .collect();
        if allow.is_empty() {
            Resolution::NotAnOp
        } else {
            Resolution::MethodNotAllowed(allow)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(method: &str, matched: Option<&str>, params: &[(&str, &str)]) -> Resolution {
        let method: Method = method.parse().expect("a method");
        let params: Vec<(String, String)> = params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        TableBinding::v1().resolve(&RouteRequest::new(
            &method,
            matched.unwrap_or("/not/matched"),
            matched,
            &params,
        ))
    }

    fn resolve_in(
        binding: &TableBinding,
        method: &str,
        matched: &str,
        params: &[(&str, &str)],
    ) -> Resolution {
        let method: Method = method.parse().expect("a method");
        let params: Vec<(String, String)> = params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        binding.resolve(&RouteRequest::new(&method, matched, Some(matched), &params))
    }

    fn op(resolution: Resolution) -> ResolvedRoute {
        match resolution {
            Resolution::Op(route) => route,
            other => panic!("expected an operation, got {other:?}"),
        }
    }

    #[test]
    fn a_route_binds_its_template_without_the_mount_and_its_path_parameters() {
        let route = op(resolve(
            "POST",
            Some("/v1/payment_intents/{id}/confirm"),
            &[("id", "pi_123")],
        ));
        assert_eq!(route.route(), "/payment_intents/{id}/confirm");
        assert_eq!(route.path_params(), ["pi_123"]);
        assert_eq!(
            route.contract_key(),
            Some("POST /payment_intents/{id}/confirm")
        );
        assert_eq!(route.payload_request_types(), Some(REQUEST_TYPES));
        assert_eq!(route.payload_response_types(), Some(RESPONSE_TYPES));
    }

    #[test]
    fn a_head_binds_as_head_and_looks_up_the_get() {
        let route = op(resolve("HEAD", Some("/v1/events/{id}"), &[("id", "evt_1")]));
        assert_eq!(route.route(), "/events/{id}");
        assert_eq!(route.contract_key(), Some("GET /events/{id}"));
    }

    #[test]
    fn every_v1_route_method_resolves_to_a_contract_key_the_wire_table_holds() {
        for route in V1_ROUTES {
            for method in route.methods {
                let matched = format!("{V1_MOUNT}{}", route.path);
                let resolved = op(resolve(method, Some(&matched), &[]));
                let key = resolved.contract_key().expect("a contract key");
                assert_eq!(key, operation_key(method, route.path));
                assert!(
                    vpay_wire::V1_CONTRACTS.iter().any(|(row, _)| *row == key),
                    "{key} has no row in V1_CONTRACTS"
                );
            }
        }
    }

    #[test]
    fn another_method_on_a_known_path_is_a_405_naming_what_the_path_answers() {
        match resolve("PUT", Some("/v1/customers/{id}"), &[("id", "cus_1")]) {
            Resolution::MethodNotAllowed(allow) => {
                assert!(allow.contains(&Method::GET), "{allow:?}");
                assert!(allow.contains(&Method::DELETE), "{allow:?}");
                assert!(!allow.contains(&Method::PUT), "{allow:?}");
            }
            other => panic!("expected a 405, got {other:?}"),
        }
    }

    #[test]
    fn what_is_not_a_v1_route_is_not_an_operation() {
        for matched in [
            Some("/v1"),
            Some("/v1/{*rest}"),
            Some("/v1x/payment_intents"),
            Some("/payment_intents"),
            Some("/v1/browser/payment_intents/{id}"),
            Some("/v1/oauth/token"),
            Some("/v1/not_a_route"),
        ] {
            assert!(
                matches!(resolve("GET", matched, &[]), Resolution::NotAnOp),
                "{matched:?} must not bind as a /v1 operation, and must not be `Unresolved` \
                 (which a Required layer answers with an unsigned 500)"
            );
        }
        // No matched route at all is the only `Unresolved`.
        assert!(matches!(resolve("GET", None, &[]), Resolution::Unresolved));
    }

    /// Every row of `/dash/v1`'s and `/svc/v1`'s tables binds to a contract key
    /// the matching `vpay-wire` table holds (VP-6a). A route the layer could not
    /// find a digest for is a `500` for every signed call to it.
    #[test]
    fn every_dash_and_svc_route_method_has_a_row_in_its_contract_table() {
        for (binding, mount, routes, table) in [
            (
                TableBinding::dash(),
                DASH_MOUNT,
                DASH_ROUTES
                    .iter()
                    .map(|route| (route.path, route.methods))
                    .collect::<Vec<_>>(),
                *vpay_wire::DASH_CONTRACTS,
            ),
            (
                TableBinding::svc(),
                SVC_MOUNT,
                SVC_ROUTES
                    .iter()
                    .map(|route| (route.path, route.methods))
                    .collect::<Vec<_>>(),
                *vpay_wire::SVC_CONTRACTS,
            ),
        ] {
            for (path, methods) in routes {
                for method in methods {
                    let resolved = op(resolve_in(&binding, method, &format!("{mount}{path}"), &[]));
                    let key = resolved.contract_key().expect("a contract key");
                    assert_eq!(key, operation_key(method, path));
                    assert!(table.iter().any(|(row, _)| *row == key), "{key}");
                    assert_eq!(resolved.payload_request_types(), Some(JSON_TYPES));
                }
            }
        }
    }

    /// One surface's binding knows nothing of another's: `/svc/v1/console_keys`
    /// is not a `/dash/v1` operation and a `/dash/v1` read is not a `/svc/v1` one.
    #[test]
    fn a_binding_resolves_only_its_own_mount() {
        let dash = TableBinding::dash();
        let svc = TableBinding::svc();
        assert!(matches!(
            resolve_in(&dash, "POST", "/svc/v1/console_keys", &[]),
            Resolution::NotAnOp
        ));
        assert!(matches!(
            resolve_in(&svc, "GET", "/dash/v1/payment_intents", &[]),
            Resolution::NotAnOp
        ));
        // And a write to a read-only route is the layer's own 405.
        assert!(matches!(
            resolve_in(&dash, "POST", "/dash/v1/payment_intents", &[]),
            Resolution::MethodNotAllowed(_)
        ));
        assert!(matches!(
            resolve_in(&svc, "GET", "/svc/v1/console_keys", &[]),
            Resolution::MethodNotAllowed(_)
        ));
    }
}
