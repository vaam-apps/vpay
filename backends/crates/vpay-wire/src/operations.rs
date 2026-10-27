//! The operations under `/v1`, as the contract table sees them.

/// Where the `/v1` router is mounted. An [`Operation::path`] is relative to
/// it, and the `EnvelopeLayer` is told it as `.mount_prefix(V1_MOUNT)`, so the
/// route a client binds is the template without it, as for a generated
/// router.
pub const V1_MOUNT: &str = "/v1";

/// Where the `/dash/v1` router is mounted (VP-6a). A [`DASH_OPERATIONS`] path is
/// relative to it, and the `EnvelopeLayer` is told it as
/// `.mount_prefix(DASH_MOUNT)`.
pub const DASH_MOUNT: &str = "/dash/v1";

/// Where the `/svc/v1` router is mounted (VP-6a): the routes a registered
/// service (vaam) calls, never a merchant and never a person.
pub const SVC_MOUNT: &str = "/svc/v1";

/// One operation a caller can make: a method on a route template, under one of
/// the three mounts ([`V1_MOUNT`], [`DASH_MOUNT`], [`SVC_MOUNT`]).
///
/// A struct and not a bare pair so a call site cannot swap the two strings:
/// `("/events", "GET")` compiles as a tuple of `&str` and signs a different
/// operation from the one meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Operation {
    /// Upper-case HTTP method.
    pub method: &'static str,
    /// The axum route template, relative to its mount, with `{name}`
    /// placeholders, exactly as the surface's own route table spells it
    /// (`vpay_api::v1::V1Route::path`, `vpay_api::dash::DashRoute::path`,
    /// `vpay_api::svc::SvcRoute::path`).
    pub path: &'static str,
}

impl Operation {
    const fn new(method: &'static str, path: &'static str) -> Self {
        Self { method, path }
    }
}

/// Every operation mounted under `/v1`, and the only
/// list a contract is derived from.
///
/// **This is a second list, beside `vpay_api::v1::V1_ROUTES`, and that is
/// deliberate**: the server's table carries handlers and cannot be linked
/// into a client, and this one has to be. They are held together by two
/// tests in `vpay-api` that read both, in both directions
/// (`v1::contracts::tests`): a route with no operation here fails, and so
/// does an operation here with no route. Adding a route means adding it in
/// both places, and the build says so if you forget one.
///
/// `HEAD` is not listed: axum answers it from the `GET` handler, and
/// `cratestack_core::find_contract` resolves a `HEAD` to the `GET` row.
pub const V1_OPERATIONS: &[Operation] = &[
    Operation::new("POST", "/payment_intents"),
    Operation::new("GET", "/payment_intents"),
    Operation::new("GET", "/payment_intents/{id}"),
    Operation::new("POST", "/payment_intents/{id}/confirm"),
    Operation::new("POST", "/payment_intents/{id}/cancel"),
    Operation::new("GET", "/events"),
    Operation::new("GET", "/account_holders"),
    Operation::new("GET", "/events/{id}"),
    Operation::new("POST", "/checkout/sessions"),
    Operation::new("GET", "/checkout/sessions"),
    Operation::new("GET", "/checkout/sessions/{id}"),
    Operation::new("POST", "/checkout/sessions/{id}/expire"),
    Operation::new("POST", "/refunds"),
    Operation::new("GET", "/refunds"),
    Operation::new("GET", "/refunds/{id}"),
    Operation::new("POST", "/refunds/{id}"),
    Operation::new("POST", "/refunds/{id}/cancel"),
    Operation::new("POST", "/customers"),
    Operation::new("GET", "/customers"),
    Operation::new("GET", "/customers/{id}"),
    Operation::new("POST", "/customers/{id}"),
    Operation::new("DELETE", "/customers/{id}"),
    Operation::new("POST", "/invoices"),
    Operation::new("GET", "/invoices"),
    Operation::new("GET", "/invoices/{id}"),
    Operation::new("POST", "/invoices/{id}"),
    Operation::new("PATCH", "/invoices/{id}"),
    Operation::new("DELETE", "/invoices/{id}"),
    Operation::new("POST", "/invoices/{id}/finalize"),
    Operation::new("POST", "/invoices/{id}/void"),
    Operation::new("POST", "/invoices/{id}/mark_uncollectible"),
    Operation::new("POST", "/invoices/{id}/pay"),
    Operation::new("POST", "/invoice_items"),
    Operation::new("GET", "/invoice_items/{id}"),
    Operation::new("POST", "/invoice_items/{id}"),
    Operation::new("PATCH", "/invoice_items/{id}"),
    Operation::new("DELETE", "/invoice_items/{id}"),
];

/// The hand-written `/dash/v1` operations: the console's two reads.
///
/// The procedure transport (`POST /dash/v1/$procs/<name>`) is **not** listed:
/// its contract table is the one cratestack generates from `schemas/vpay.cstack`
/// (`ACCEPTED_CONTRACTS`), which a generated client already binds. Two lists
/// again, held together by `vpay_api::dash::contracts` in both directions.
pub const DASH_OPERATIONS: &[Operation] = &[
    Operation::new("GET", "/payment_intents"),
    Operation::new("GET", "/payment_intents/{id}"),
];

/// The `/svc/v1` operations: pair a console key, revoke one.
///
/// Held together with `vpay_api::svc::SVC_ROUTES` by `vpay_api::svc::contracts`.
pub const SVC_OPERATIONS: &[Operation] = &[
    Operation::new("POST", "/console_keys"),
    Operation::new("DELETE", "/console_keys/{thumbprint}"),
];
