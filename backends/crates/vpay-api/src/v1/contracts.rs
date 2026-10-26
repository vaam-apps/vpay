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
}
