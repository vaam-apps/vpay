//! Holding [`V1_ROUTES`] and the contract table to each other (VP-2).
//!
//! A signed request binds the digest of the operation it calls, and the
//! `EnvelopeLayer` (VP-3) finds that operation's accepted digests in
//! `vpay_wire::V1_CONTRACTS`. The two lists are written in two crates because
//! this one carries handlers and cannot be linked into a client, so nothing
//! in the type system ties them together, and the failure when they drift is
//! not subtle but it is late:
//!
//! * a **route with no contract** is a route no signed client can call. The
//!   layer, which cannot tell what the request was signed under, answers its
//!   `500 envelope misconfigured` to every request for it (cratestack's
//!   `ResolvedRoute::with_contract_key`: "a route with no row is a `500`");
//! * a **contract with no route** is a digest a client may bind for an
//!   operation the server will `404`: harmless at runtime, and a sign that the
//!   table was edited by hand or a route was removed and its row was not.
//!
//! [`contract_gaps`] is the comparison, in both directions, over whatever it is
//! handed, so the same function the tests run on the real tables is what a boot
//! self-check would run, and the tests can prove it fails on tables built to
//! disagree.

use std::collections::BTreeSet;

use cratestack_core::find_contract;
use vpay_wire::{AcceptedContracts, operation_key};

use super::V1Route;

/// One way a route table and a contract table disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractGap {
    /// The router mounts `method path` and the contract table has no digest for
    /// it. `path` is relative to `/v1`, as in [`V1Route::path`].
    RouteWithoutContract {
        /// Upper-case HTTP method.
        method: String,
        /// The axum route template.
        path: String,
    },
    /// The contract table has a row (`"<METHOD> <path>"`) no mounted route
    /// answers.
    ContractWithoutRoute {
        /// The row's key.
        key: String,
    },
}

/// Every disagreement between `routes` and `contracts`, in both directions;
/// empty means each `(method, path)` the router answers has at least one
/// digest, and each row of the table belongs to one.
///
/// A row whose digest list is empty counts as no row for its route, because
/// that is how the layer reads it (`accepted()` filters an empty list out).
///
/// ```
/// use vpay_api::v1::{V1_ROUTES, contracts::contract_gaps};
/// use vpay_wire::V1_CONTRACTS;
///
/// assert!(contract_gaps(V1_ROUTES, *V1_CONTRACTS).is_empty());
/// ```
#[must_use]
pub fn contract_gaps(routes: &[V1Route], contracts: AcceptedContracts) -> Vec<ContractGap> {
    let mut gaps = Vec::new();
    let mut mounted = BTreeSet::new();
    for route in routes {
        for method in route.methods {
            mounted.insert(operation_key(method, route.path));
            let covered = find_contract(contracts, method, route.path)
                .is_some_and(|digests| !digests.is_empty());
            if !covered {
                gaps.push(ContractGap::RouteWithoutContract {
                    method: (*method).to_owned(),
                    path: route.path.to_owned(),
                });
            }
        }
    }
    for (key, _) in contracts {
        if !mounted.contains(*key) {
            gaps.push(ContractGap::ContractWithoutRoute {
                key: (*key).to_owned(),
            });
        }
    }
    gaps
}

#[cfg(test)]
mod tests {
    use axum::routing::{get, post};
    use vpay_wire::{V1_CONTRACTS, V1_OPERATIONS, contract_digest};

    use super::*;
    use crate::v1::V1_ROUTES;

    fn route(path: &'static str, methods: &'static [&'static str]) -> V1Route {
        V1Route {
            path,
            methods,
            mount: || get(|| async {}).post(|| async {}),
        }
    }

    /// The real tables agree, in both directions. This is the test that fails
    /// the day someone adds a `V1_ROUTES` row and not an operation to
    /// `vpay_wire::V1_OPERATIONS`, or the reverse.
    #[test]
    fn every_v1_route_has_a_contract_and_every_contract_has_a_route() {
        assert_eq!(contract_gaps(V1_ROUTES, *V1_CONTRACTS), []);
        // The comparison above is over method *and* path, so the counts
        // agree too — checked on its own so a failure names a number.
        let mounted: usize = V1_ROUTES.iter().map(|route| route.methods.len()).sum();
        assert_eq!(mounted, V1_OPERATIONS.len());
        assert_eq!(mounted, V1_CONTRACTS.len());
    }

    /// The contract is the one the wire crate derives, for each mounted
    /// operation, not merely a row of the right name.
    #[test]
    fn each_mounted_operation_binds_the_digest_the_wire_crate_derives() {
        for route in V1_ROUTES {
            for method in route.methods {
                assert_eq!(
                    find_contract(&V1_CONTRACTS, method, route.path).copied(),
                    Some(&[contract_digest(method, route.path)][..]),
                    "{method} {}",
                    route.path
                );
            }
        }
    }

    #[test]
    fn a_route_with_no_contract_is_a_gap() {
        let mut routes: Vec<V1Route> = Vec::new();
        routes.extend(V1_ROUTES.iter().map(|r| V1Route {
            path: r.path,
            methods: r.methods,
            mount: || get(|| async {}),
        }));
        routes.push(route("/widgets", &["GET", "POST"]));

        assert_eq!(
            contract_gaps(&routes, *V1_CONTRACTS),
            vec![
                ContractGap::RouteWithoutContract {
                    method: "GET".to_owned(),
                    path: "/widgets".to_owned(),
                },
                ContractGap::RouteWithoutContract {
                    method: "POST".to_owned(),
                    path: "/widgets".to_owned(),
                },
            ]
        );
    }

    /// A new *verb* on an existing path is a new operation: `PUT /events` is
    /// not covered by `GET /events`.
    #[test]
    fn a_method_added_to_a_mounted_path_is_a_gap() {
        let routes = [route("/events", &["GET", "PUT"])];
        let gaps = contract_gaps(&routes, *V1_CONTRACTS);
        assert!(
            gaps.contains(&ContractGap::RouteWithoutContract {
                method: "PUT".to_owned(),
                path: "/events".to_owned(),
            }),
            "{gaps:?}"
        );
        assert!(
            !gaps.contains(&ContractGap::RouteWithoutContract {
                method: "GET".to_owned(),
                path: "/events".to_owned(),
            }),
            "GET /events has a contract: {gaps:?}"
        );
    }

    #[test]
    fn a_contract_with_no_route_is_a_gap() {
        // A table with one real row and one for a route nothing mounts.
        static TABLE: &[(&str, &[[u8; 32]])] =
            &[("GET /events", &[[1; 32]]), ("GET /ghosts", &[[2; 32]])];
        let routes = [route("/events", &["GET"])];

        assert_eq!(
            contract_gaps(&routes, TABLE),
            vec![ContractGap::ContractWithoutRoute {
                key: "GET /ghosts".to_owned()
            }]
        );
    }

    /// And the real table against a router that lost a route: removing the
    /// `V1_ROUTES` row without the operation is the reverse direction of the
    /// first test, shown on the real contracts.
    #[test]
    fn dropping_a_route_leaves_its_contracts_without_a_route() {
        let remaining: Vec<V1Route> = V1_ROUTES
            .iter()
            .filter(|route| route.path != "/customers/{id}")
            .map(|route| V1Route {
                path: route.path,
                methods: route.methods,
                mount: || post(|| async {}),
            })
            .collect();

        let mut orphans: Vec<String> = contract_gaps(&remaining, *V1_CONTRACTS)
            .into_iter()
            .map(|gap| match gap {
                ContractGap::ContractWithoutRoute { key } => key,
                other => format!("unexpected: {other:?}"),
            })
            .collect();
        orphans.sort();
        assert_eq!(
            orphans,
            [
                "DELETE /customers/{id}",
                "GET /customers/{id}",
                "POST /customers/{id}"
            ]
        );
    }

    /// An empty digest list is how the layer reads "no row": it filters it out
    /// and answers the misconfiguration `500`.
    #[test]
    fn a_row_with_no_digest_does_not_cover_its_route() {
        static TABLE: &[(&str, &[[u8; 32]])] = &[("GET /events", &[])];
        let routes = [route("/events", &["GET"])];

        assert_eq!(
            contract_gaps(&routes, TABLE),
            vec![ContractGap::RouteWithoutContract {
                method: "GET".to_owned(),
                path: "/events".to_owned(),
            }]
        );
    }

    // ---- the SDK's vendored copy (VP-5) --------------------------------------
    //
    // `sdks/rust` cannot depend on `vpay-wire` (it has to stay publishable
    // without this workspace), so it carries its own copy of the table and of
    // the digest derivation (`vpay_sdk::contracts`). These tests are where the
    // copy is held to the original: they live in the one crate that can see both.

    /// Every way the SDK's table and `vpay-wire`'s disagree: a row on one side
    /// only, the same rows in a different order (the order is the file's, and
    /// a reordering is a hand edit), and the two digests of a row.
    fn sdk_table_gaps(sdk: &[(&str, &str)], wire: &[vpay_wire::V1Operation]) -> Vec<String> {
        let mut gaps = Vec::new();
        for (method, template) in sdk {
            if !wire
                .iter()
                .any(|op| op.method == *method && op.path == *template)
            {
                gaps.push(format!("SDK only: {method} {template}"));
            }
        }
        for op in wire {
            if !sdk
                .iter()
                .any(|(method, template)| op.method == *method && op.path == *template)
            {
                gaps.push(format!("wire only: {} {}", op.method, op.path));
            }
        }
        let same_order = sdk.len() == wire.len()
            && sdk
                .iter()
                .zip(wire)
                .all(|((method, template), op)| op.method == *method && op.path == *template);
        if gaps.is_empty() && !same_order {
            gaps.push("same rows, different order".to_owned());
        }
        for op in wire {
            if vpay_sdk::contracts::contract_digest(op.method, op.path)
                != vpay_wire::contract_digest(op.method, op.path)
            {
                gaps.push(format!("digest differs: {} {}", op.method, op.path));
            }
        }
        gaps
    }

    /// The real tables agree: same rows, same order, same digests, same domain.
    #[test]
    fn the_sdks_vendored_table_is_the_wire_crates_table() {
        assert_eq!(
            vpay_sdk::contracts::CONTRACT_DOMAIN,
            vpay_wire::CONTRACT_DOMAIN
        );
        assert_eq!(
            sdk_table_gaps(vpay_sdk::contracts::OPERATIONS, V1_OPERATIONS),
            Vec::<String>::new()
        );
    }

    /// The same check **fails** on a copy that drifted — shown, not assumed. A
    /// parity test that cannot fail is a comment.
    #[test]
    fn a_drifted_sdk_table_is_reported_in_every_direction() {
        let sdk: Vec<(&str, &str)> = vpay_sdk::contracts::OPERATIONS.to_vec();

        // A row the SDK has and the server does not.
        let mut extra = sdk.clone();
        extra.push(("GET", "/balance"));
        assert_eq!(
            sdk_table_gaps(&extra, V1_OPERATIONS),
            ["SDK only: GET /balance"]
        );

        // A row the server has and the SDK lacks.
        let missing: Vec<(&str, &str)> = sdk.iter().copied().skip(1).collect();
        assert_eq!(
            sdk_table_gaps(&missing, V1_OPERATIONS),
            ["wire only: POST /payment_intents"]
        );

        // A verb changed: one row on each side.
        let mut verb = sdk.clone();
        *verb.get_mut(1).expect("the table has a second row") = ("PUT", "/payment_intents");
        let gaps = sdk_table_gaps(&verb, V1_OPERATIONS);
        assert!(
            gaps.contains(&"SDK only: PUT /payment_intents".to_owned()),
            "{gaps:?}"
        );
        assert!(
            gaps.contains(&"wire only: GET /payment_intents".to_owned()),
            "{gaps:?}"
        );

        // Same rows, shuffled.
        let mut shuffled = sdk.clone();
        shuffled.swap(0, 1);
        assert_eq!(
            sdk_table_gaps(&shuffled, V1_OPERATIONS),
            ["same rows, different order"]
        );
    }

    /// The SDK finds the same operation, with the same parameters, for a
    /// concrete path as the server's router does for the route it matches: for
    /// every `V1_ROUTES` row and method, a path built from the template binds
    /// back to that template.
    #[test]
    fn the_sdk_binds_every_router_route_to_its_own_template() {
        for route in V1_ROUTES {
            for method in route.methods {
                let concrete = route.path.replace("{id}", "x_1");
                let bound = vpay_sdk::contracts::operation_for(method, &concrete)
                    .unwrap_or_else(|error| panic!("{method} {concrete}: {error}"));
                assert_eq!(bound.template, route.path, "{method} {concrete}");
                assert_eq!(
                    bound.params.len(),
                    route.path.matches('{').count(),
                    "{method} {concrete}"
                );
            }
        }
    }

    /// And the converse: an operation the router does not serve is one the SDK
    /// does not bind (`GET /balance` is declared in the wire contract and
    /// mounted nowhere).
    #[test]
    fn the_sdk_binds_nothing_the_router_does_not_serve() {
        for (method, path) in [
            ("GET", "/balance"),
            ("PUT", "/events"),
            ("DELETE", "/events/e"),
        ] {
            assert!(vpay_sdk::contracts::operation_for(method, path).is_err());
            assert!(
                !V1_ROUTES.iter().any(|route| route.methods.contains(&method)
                    && route.path.replace("{id}", "e") == path),
                "{method} {path} is served after all"
            );
        }
    }
}
