//! Which `/v1` operation a request addresses, as the signed binding names it
//! (VP-3).
//!
//! cratestack's `RestBindingResolver` reads a generated router's
//! `ROUTE_TRANSPORTS`. `/v1` is hand-written, so [`V1Binding`] is the same
//! decision over [`V1_ROUTES`]: the route template axum matched, with the
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
use vpay_wire::{V1_MOUNT, operation_key};

use crate::v1::V1_ROUTES;

/// The request payload types every `/v1` operation declares.
pub const REQUEST_TYPES: &[&str] = &["application/x-www-form-urlencoded"];
/// The response payload types every `/v1` operation answers in.
pub const RESPONSE_TYPES: &[&str] = &["application/json"];

/// The [`BindingResolver`] over [`V1_ROUTES`].
#[derive(Debug, Clone, Copy, Default)]
pub struct V1Binding;

/// `matched` without the `/v1` mount, or `None` if it is not under it.
///
/// A path is under the mount only at a segment boundary: `/v1x/...` is not
/// `/v1`'s.
fn template_of(matched: &str) -> Option<&str> {
    matched
        .strip_prefix(V1_MOUNT)
        .filter(|rest| rest.starts_with('/'))
}

impl BindingResolver for V1Binding {
    fn resolve(&self, request: &RouteRequest<'_>) -> Resolution {
        // No route matched at all (a 404 the nest's fallback answers): nothing
        // to bind, and the layer passes it through. Anything *matched* that is
        // not a `V1_ROUTES` row — the bare `/v1`, the nest's own `/v1/{*rest}`
        // catch-all that an unknown path or `/v1//x` lands on — is `NotAnOp`,
        // and never `Unresolved`: under `Required`, `Unresolved` on a matched
        // route is cratestack's "envelope misconfigured" unsigned `500`, which
        // is the wrong answer for a path that simply does not exist. `NotAnOp`
        // lets a plain request through to the fallback's honest 404 and refuses
        // a COSE body for it with the unsigned 415 (there is no operation to
        // have bound it to).
        let Some(matched) = request.matched_path() else {
            return Resolution::Unresolved;
        };
        let Some(template) = template_of(matched) else {
            return Resolution::NotAnOp;
        };
        // axum answers a `HEAD` with the `GET` handler and `V1_ROUTES` lists
        // only the latter; the binding keeps the real method, the contract
        // key does not.
        let method = match request.method().as_str() {
            "HEAD" => "GET",
            other => other,
        };
        let mut on_path = V1_ROUTES.iter().filter(|route| route.path == template);
        if let Some(route) = on_path
            .clone()
            .find(|route| route.methods.contains(&method))
        {
            let params = request
                .path_params()
                .iter()
                .map(|(_, value)| value.clone())
                .collect();
            return Resolution::Op(
                ResolvedRoute::new(route.path, params)
                    .with_contract_key(operation_key(method, route.path))
                    .with_payload_types(REQUEST_TYPES, RESPONSE_TYPES),
            );
        }
        let allow: Vec<Method> = on_path
            .by_ref()
            .flat_map(|route| route.methods.iter())
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
        V1Binding.resolve(&RouteRequest::new(
            &method,
            matched.unwrap_or("/not/matched"),
            matched,
            &params,
        ))
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
}
