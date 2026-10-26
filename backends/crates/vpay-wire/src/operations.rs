//! The operations under `/v1`, as the contract table sees them.

/// Where the `/v1` router is mounted. A [`V1Operation::path`] is relative to
/// it, and the `EnvelopeLayer` is told it as `.mount_prefix(V1_MOUNT)`, so the
/// route a client binds is the template without it, as for a generated
/// router.
pub const V1_MOUNT: &str = "/v1";

/// One operation a merchant can call: a method on a route template.
///
/// A struct and not a bare pair so a call site cannot swap the two strings:
/// `("/events", "GET")` compiles as a tuple of `&str` and signs a different
/// operation from the one meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct V1Operation {
    /// Upper-case HTTP method.
    pub method: &'static str,
    /// The axum route template, relative to [`V1_MOUNT`], with `{name}`
    /// placeholders, exactly as `vpay_api::v1::V1Route::path` spells it.
    pub path: &'static str,
}

impl V1Operation {
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
pub const V1_OPERATIONS: &[V1Operation] = &[
    V1Operation::new("POST", "/payment_intents"),
    V1Operation::new("GET", "/payment_intents"),
    V1Operation::new("GET", "/payment_intents/{id}"),
    V1Operation::new("POST", "/payment_intents/{id}/confirm"),
    V1Operation::new("POST", "/payment_intents/{id}/cancel"),
    V1Operation::new("GET", "/events"),
    V1Operation::new("GET", "/account_holders"),
    V1Operation::new("GET", "/events/{id}"),
    V1Operation::new("POST", "/checkout/sessions"),
    V1Operation::new("GET", "/checkout/sessions"),
    V1Operation::new("GET", "/checkout/sessions/{id}"),
    V1Operation::new("POST", "/checkout/sessions/{id}/expire"),
    V1Operation::new("POST", "/refunds"),
    V1Operation::new("GET", "/refunds"),
    V1Operation::new("GET", "/refunds/{id}"),
    V1Operation::new("POST", "/refunds/{id}"),
    V1Operation::new("POST", "/refunds/{id}/cancel"),
    V1Operation::new("POST", "/customers"),
    V1Operation::new("GET", "/customers"),
    V1Operation::new("GET", "/customers/{id}"),
    V1Operation::new("POST", "/customers/{id}"),
    V1Operation::new("DELETE", "/customers/{id}"),
    V1Operation::new("POST", "/invoices"),
    V1Operation::new("GET", "/invoices"),
    V1Operation::new("GET", "/invoices/{id}"),
    V1Operation::new("POST", "/invoices/{id}"),
    V1Operation::new("PATCH", "/invoices/{id}"),
    V1Operation::new("DELETE", "/invoices/{id}"),
    V1Operation::new("POST", "/invoices/{id}/finalize"),
    V1Operation::new("POST", "/invoices/{id}/void"),
    V1Operation::new("POST", "/invoices/{id}/mark_uncollectible"),
    V1Operation::new("POST", "/invoices/{id}/pay"),
    V1Operation::new("POST", "/invoice_items"),
    V1Operation::new("GET", "/invoice_items/{id}"),
    V1Operation::new("POST", "/invoice_items/{id}"),
    V1Operation::new("PATCH", "/invoice_items/{id}"),
    V1Operation::new("DELETE", "/invoice_items/{id}"),
];
