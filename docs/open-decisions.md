# Open maintainer decisions

This page records choices that implementation must not make by default. A
decision belongs in an ADR once it is taken.

- **Merchant-token lifetime and revocation:** choose the `/v1` access-token TTL,
  the response to Authkestra's lack of a revocation endpoint, and the signing-key
  overlap that must cover that TTL. See
  [ADR-0009](adr/0009-dashboard-oidc-provider.md) and
  [the signing-key rotation runbook](runbooks/rotate-signing-key.md).
- **OIDC provider evaluation:** Authkestra remains the accepted provider, but
  the Keycloak/ZITADEL comparison has not been completed. See
  [ADR-0009](adr/0009-dashboard-oidc-provider.md).
- **Merchant assertion and ingress controls:** decide whether `jti` values can
  be scoped per merchant and how ingress rate limits `/v1/oauth/token` and
  `/v1`. See [merchant-auth limits](flows/merchant-auth/verification-and-limits.md).
- **Account-holder lookup controls:** decide the rate-limit shape, audit-log
  policy, and dedicated scope for the lookup endpoint. See
  [account-holder lookup](flows/account-holder-lookup.md).
- **Dashboard implementation:** decide whether to use CrateStack's refine
  integration for the dashboard. This affects the dashboard boundary in
  [ADR-0008](adr/0008-dashboard-scope.md).
- **Provider references:** decide whether `charges.provider_reference_id` must
  be unique.
- **SSRF address policy:** decide whether the egress guard must reject all of
  `2001::/23`, rather than only narrower reserved ranges.
- **Checkout configuration failure:** decide whether `checkout_not_configured`
  should map to `503`; this changes the error-category policy in
  [ADR-0011](adr/0011-error-modelling.md).
- **Checkout deployment boundary:** decide whether `checkout.public_base_url`
  uses a separate host or a path under the API host, and whether a checkout
  session may create a PaymentIntent inline. See
  [the hosted checkout flow](flows/hosted-checkout.md).
- **Staff sign-in limit:** deployments may set the sign-in rate-limit budget;
  the default is not a policy decision. See
  [ADR-0017](adr/0017-staff-authentication.md).
- **Privacy and operator policy:** all decisions D1-D13 remain open in
  [RFC-0002](rfc/0002-gdpr-policy-and-operator-decisions.md). Do not implement
  an answer without its named owner.
