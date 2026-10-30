# Dashboard authentication

## The invariant

> **No `/dash/v1` request is handled unless it is a COSE_Sign1 message signed
> by a console key that is registered to an active member of the dashboard's
> merchant, was paired less than 30 days ago, has not been revoked, is addressed
> to this deployment, and has not been seen before. Every answer to such a request
> is a COSE_Sign1 message signed by this deployment.**

**That is the invariant of a deployment that does not serve `human_login`, which
is every deployment until it writes the method's two blocks (`staff_auth` and
`dashboard_client`), and vaam's for good.** On one that does
([ADR-0037](../adr/0037-authentication-methods-are-deployment-configuration.md),
VPA-3) `/dash/v1` also takes a staff access token from a password-and-TOTP
sign-in, or from a single sign-on through an OpenID Connect provider when
`staff_auth.sso` is written (VPA-4), one credential per request, and the sentence
above holds for every request that carries a signature:
[dashboard-auth/human-login.md](dashboard-auth/human-login.md) and
[dashboard-auth/sso.md](dashboard-auth/sso.md).
Where the method is off, the rest of this paragraph is exact.

There is no password, no TOTP code, no session, no authorization code, no bearer
token and no federated sign-in on this surface. A person's identity on the wire
is a **P-256 key their browser generated and cannot export** (WebCrypto,
`extractable: false`), registered in `console_keys`; vpay's is one of the two
Ed25519 response keys that `/v1` is sealed with too. The mechanism is the same
cratestack signed transport [signed-transport.md](signed-transport.md) describes
for `/v1`; this page is what differs: who the signer is, how a key gets
registered, and how long it lives.

[ADR-0036](../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)
records the decision and the owner rulings behind it, and what it supersedes
([ADR-0017](../adr/0017-staff-authentication.md) and
[ADR-0030](../adr/0030-staff-sign-in-federates-to-an-oidc-provider.md) entirely).
[ADR-0008](../adr/0008-dashboard-scope.md) still decides what the dashboard is
for, and [ADR-0018](../adr/0018-cross-tenant-admin-reads.md) still decides which
tenant a request reads.

> **The dashboard application signs a person in with a password again (VPA-5a),
> only where vpay serves `human_login`; it cannot sign in with a paired key
> until VP-6b, and with single sign-on until VPA-5b.** The server side of a
> password sign-in is back (VPA-3, behind `human_login`) and so is the server
> side of a single sign-on (VPA-4, [sso.md](dashboard-auth/sso.md)). The
> application's half of the password sign-in is back too: `/login` asks vpay
> which methods it serves (`GET /.well-known/vpay-auth`) before it asks for
> anything, then one question per screen — the email, the password, the
> authenticator code (a first sign-in enrols one), and, while the one-time
> password is still in force, the new password — then lands on an account page
> and signs out by revoking the session. A deployment that does not serve
> `human_login` shows "This console signs in with a paired key" and no field.
> What the application does **not** have is any page that reads `/dash/v1`
> (the payments, refunds, deliveries and customers screens are a later ticket),
> a button for single sign-on (VPA-5b), and the signed reader for a paired key
> (VP-6b); its BFF and middleware stay deleted, and which credential wins when a
> browser holds both a session and a key is VP-6b's to decide. The way in with
> a key meanwhile is the break-glass below (`vpay-server staff pair-key`) and a
> client that signs requests; [`docs/status.md`](../status.md) says so, and
> [`docs/status/frontend.md`](../status/frontend.md#the-dashboard-application-signs-in-with-a-password-vpa-5a)
> has what VPA-5a built and measured.

## Upgrading from before VP-6a

> **From the release that carries VPA-3, `dashboard_client` and `staff_auth` are
> accepted again.** VP-6a refused both keys at boot, even as a null
> (`ConfigError::RetiredKey`, a variant since deleted with its empty table); VPA-3
> took them out of that table, because they
> are the two blocks of the `human_login` method of `auth.methods`
> ([ADR-0037](../adr/0037-authentication-methods-are-deployment-configuration.md)).
> The refusal below is the behaviour of the releases **between** VP-6a and VPA-3,
> and a deployment that skips straight to VPA-3 meets the rules in step 1
> instead.

A deployment whose configuration still names the two keys VP-6a deleted, on a
release cut between VP-6a and VPA-3, does not boot: it exits `78` with
`ConfigError::RetiredKey`. That was the cutover, not a fault to work around, and
the refusal was by design a hard one: a key the binary silently dropped would
leave an operator believing a sign-in exists that does not.

1. **On a release that carries VPA-3, keep the two blocks only if you want the
   sign-in, and write both.** `staff_auth` (the pepper and the TOTP key) and
   `dashboard_client` are `human_login`'s: with the method permitted (the
   default) and both blocks written it is served; one without the other, or
   either with `auth.methods` listing methods but not `human_login`, is a boot
   refusal that names the block (`AuthMethodNotConfigured`,
   `AuthMethodDisabled`). `staff_auth.sso` is accepted again from the release
   that carries VPA-4, as a sub-block of `staff_auth`, and is served with
   `human_login` ([sso.md](dashboard-auth/sso.md)); between VPA-3 and VPA-4 it is
   refused as an unknown key. **On a release cut between VP-6a and VPA-3, delete
   both from the base file and from every profile overlay, even as a null.**
   `dashboard_client: ~` is refused exactly as a populated block is: naming the
   key at all is the refusal.
2. **To serve a console, add `dashboard:` and `service_clients:`** (the YAML in
   "Services, and who may pair" below): `dashboard.merchant_id` names the one
   tenant the console reads and must be a `merchant_clients` entry; each
   `service_clients` entry is a service, vaam, with its capability and its
   Ed25519 key. The base `config/application.yml` carries neither. When
   `dashboard_client` is written too, it must name the same tenant.
3. **To run without a console, add nothing.** With no `dashboard:` and no
   `human_login` blocks the process mounts neither `/dash/v1` nor `/svc/v1` (both
   answer the honest `404`), and every `vpay-server` mode boots (owner ruling R4).
4. **Keep the password pepper and the TOTP key, and do not rotate them.**
   They are the only way the stored password hashes and sealed TOTP seeds ever
   verify again. On a release that carries VPA-3 they stay where they were, in
   `staff_auth`; on one cut between VP-6a and VPA-3 they come out of the
   configuration file (the block must go) and wait in the secret store. The
   migration signs everyone out (it drops `oauth_authorization_codes`,
   `staff_sessions` and `staff_sso_attempts` and deletes the `password` and
   `totp` credentials); the deletion can be undone only from an export, see
   [Keeping staff passwords across 0056 and 0058](../runbooks/deploy-and-rollback.md#keeping-staff-passwords-across-0056-and-0058).
   The SSO client secret (`staff_auth.sso.client_secret`) is `staff_auth`'s
   again from the release that carries VPA-4, and waits in the secret store until
   then. Members who signed in through SSO kept their `oidc` credential through
   migration 0056 and resolve to the same member again by `(issuer, subject)`
   ([sso.md](dashboard-auth/sso.md#members-kept-through-0056)).
5. **Helm**: `dashboard.clientId`, `dashboard.redirectUri` and `dashboard.scope`
   are gone from the chart values. A split release (`management.enabled`) serves
   `/svc/v1` from `-server`, so list the calling service in
   `networkPolicy.serviceIngress.peers` when `networkPolicy.enabled`.

The VP-6a change is a breaking one for release notes: the commit that landed it
carries a `BREAKING CHANGE:` footer (the changelog is release-please's,
generated from commits, and is not edited by hand).

## Pairing a key

A key reaches `console_keys` by exactly two doors, and both end in the same
function (`vpay_api::console_keys::store_key`).

```mermaid
sequenceDiagram
    autonumber
    participant B as Console (browser)
    participant P as Admin phone (vaam)
    participant V as vaam (service)
    participant S as /svc/v1 (vpay-server)
    participant D as Postgres

    B->>B: generate a non-extractable P-256 key
    B->>B: sign "vpay.console-pair.v1\n" + publicKey + "\n" + issuedAt
    B->>P: show the pairing request
    P->>V: the admin approves; vaam knows who the person is
    V->>S: POST /svc/v1/console_keys, COSE_Sign1 under vaam's Ed25519 key<br/>{vaam_user_id, public_key, possession_proof, issued_at}
    S->>S: the key is a service_clients key holding console_pairing?
    S->>S: shape of every field, then |now - issued_at| ≤ 300 s, then the proof
    S->>D: find (vaam, vaam_user_id)
    S->>S: linked: an administrator? disabled? another merchant's? refuse
    S->>D: INSERT console_keys (expires_at = paired_at + 720 h)
    S->>D: not linked: ONE transaction, a plain member of dashboard.merchant_id + the link + the key
    S-->>V: 201 {thumbprint, staff_member_id, expires_at, …} (200 for a repeat)
```

The second door is the break-glass (owner ruling Q2): an operator with database
access runs `vpay-server staff pair-key --staff <email|stf_…> --public-key <key>
[--label …]` for an **existing, active** member. It needs no proof of possession,
because the operator already holds more authority than a pairing grants; it
creates no member, changes no flag, and prints one line (`<staff id> <key id>
<thumbprint> <expires_at>`). It is also the **only** way a key reaches an
administrator, and `vpay-server staff add --admin` the only way one is made. Its
twin, `vpay-server staff revoke-key --thumbprint <t>`, runs the same function
`DELETE /svc/v1/console_keys/{thumbprint}` does (`console_keys::revoke`), so the
door that can pair can also revoke, with no database access beyond the
command's own.

What a pairing never does (owner rulings R2 and 2026-10-09):

- **create or upgrade an administrator.** The body has no role field and
  `deny_unknown_fields` rejects one; the member provisioned is `is_admin =
false`, bound to `dashboard.merchant_id` from configuration; an existing
  member's flag is never written; and an already-linked administrator is refused
  (`403`, logged as such), because a service holding `console_pairing` can
  name any linked person;
- **name a tenant.** There is no field for one;
- **move or reinstate a key.** A thumbprint is unique for ever: another member's
  key is a `409`, and a revoked or expired key is the same `409`
  — the row is never deleted. A refused first pairing leaves no member and no
  link behind (they commit with the key, in one transaction), and an address
  collision between two vaam ids that differ only in letter case is its own
  `409` ("cannot be provisioned"), not a taken key.

## One call

```mermaid
sequenceDiagram
    autonumber
    participant C as Console (signs with its key)
    participant L as EnvelopeLayer (vpay-server)
    participant K as console_keys
    participant N as envelope_nonces
    participant H as dash_scope, then the handler

    C->>L: HTTP request, body = COSE_Sign1 (even for a GET)
    L->>K: live keys for this kid (not revoked, expires_at > now)
    alt unknown, revoked or expired key, bad signature, wrong audience, replay
        L-->>C: 401, unsigned, coarse (the same bytes for every cause)
    else verified
        L->>N: record (kid, cti)
        L->>H: the request, with the verified signer attached
        H->>K: re-read the key (revoked between the two reads → 403)
        H->>H: re-read the member: active, and this merchant's? else 403
        H->>H: tenancy: dashboard.merchant_id (an admin may name another, ADR-0018)
        H-->>C: sealed answer
    end
```

Everything in the layer is [signed-transport.md](signed-transport.md)'s: what a
signature binds (method, route, parameters, query, payload type, audience), the
Postgres replay cache, and the coarse refusal. Three things differ.

- **Three layers, not one.** `/dash/v1`'s hand-written reads, CrateStack's
  procedure transport under `/dash/v1/$procs/…` (a layer the generated module
  builds, because its route table and contract digests are private to
  `vpay-db`), and `/svc/v1` each have their own `EnvelopeLayer`, their own
  contract domain tag in `vpay-wire` (`vpay/dash-route-contract/v1`,
  `vpay/svc-route-contract/v1`) so one operation's digest is never another surface's,
  and a JSON payload type. A merchant's key is not a key on any of them: `/dash/v1`
  resolves signatures against `console_keys`, `/svc/v1` against `service_clients`,
  `/v1` against `merchant_clients`, and boot refuses one public key in two lists.
- **Revocation takes effect on the next request.** The resolver reads
  `console_keys` on every request and caches nothing. `DELETE
/svc/v1/console_keys/{thumbprint}` or `vpay-server staff revoke-key`
  therefore ends a key at once, and either may revoke an administrator's key
  (incident response: revocation is never refused for the most privileged
  keys); a key ends itself 30 days after `paired_at`, enforced by a `CHECK` and
  by the read, with no job to run.
- **A database that cannot answer is a `500`, never a miss.** Treating a failed
  key lookup as "no key" would make an outage look like a revocation, and treating
  it as "yes" would make it a bypass.

### What a refusal looks like

| Cause                                                                                                      | Answer                                                                                                              |
| ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| no signature, an unknown key, a revoked key, an expired key, a bad signature, the wrong audience, a replay | the layer's coarse `401` (`UNAUTHORIZED`), unsigned, identical bytes                                                |
| a write method (`POST`, `PUT`, `PATCH`, `DELETE`) on a read-only route                                     | the layer's own `405` with `Allow: GET` (a signed one: `415`) — before the router, whatever key signed it           |
| an anonymous bodiless `OPTIONS`                                                                            | the router's `405`; the layer lets a CORS preflight through and the scope step passes it on, so it is never a `500` |
| a verified key whose member is disabled, gone, or of another merchant                                      | sealed `403` — one answer for the three                                                                             |
| an administrator naming a merchant this deployment does not serve                                          | sealed `400` naming `merchant_id`                                                                                   |

## Where each piece lives

| Piece                                       | Where                                                                                         |
| ------------------------------------------- | --------------------------------------------------------------------------------------------- |
| the table and its 30-day `CHECK`            | `backends/migrations/0056_console-keys.sql`                                                   |
| the repository                              | `vpay_db::ConsoleKeys` (`backends/crates/vpay-db/src/console_keys.rs`)                        |
| parsing a key, the proof, pairing, revoking | `vpay_api::console_keys`                                                                      |
| the key resolver and the tenancy step       | `vpay_api::envelope::console`                                                                 |
| `/svc/v1` and the capability check          | `vpay_api::svc`, `vpay_api::envelope::service`                                                |
| the registered services                     | `service_clients` in the YAML ([`vpay-config.md`](../reference/vpay-config.md))               |
| the one tenant                              | `dashboard.merchant_id` in the YAML                                                           |
| the operations and their digests            | `backends/crates/vpay-wire`                                                                   |
| the break-glass                             | `vpay-server staff pair-key` and `staff revoke-key` (`backends/apps/vpay-server/src/main.rs`) |

## Services, and who may pair

`POST /svc/v1/console_keys` and `DELETE /svc/v1/console_keys/{thumbprint}` are
called by a **service**, never a person and never a merchant. They are served by
the **business** surface (`deployment.surfaces` including `business`), at the
`vpay` Service origin vaam already dials, and not by the management tier that
serves `/dash/v1`: ADR-0036, "Which surface `/svc/v1` is on". A service is an
entry of `service_clients` — `client_id`, `capabilities`, and `cose_keys` in the
shape of a merchant's — read by the same code as `merchant_clients`, from a
separate list. **A capability is a field of that file and nothing else**: no HTTP
route, database row or environment variable can grant one, so granting
`console_pairing` is a reviewed configuration change, the way adding a merchant
is. The route table (`SVC_ROUTES`) names the capability each route needs and the
scope step enforces the row it was built from; a handler then demands a
`PairingAuthority`, a type only that step can construct. A service is switched
off with the merchants' kill switch (`disabled_clients`), effective on its next
request.

```yaml
dashboard:
  merchant_id: acme-cameroon-tenant

service_clients:
  - client_id: vaam
    capabilities: [console_pairing]
    cose_keys:
      - public_key: 11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo   # Ed25519, unpadded base64url
        label: vaam-2026-10
```

## Status

Built in VP-6a (2026-10-26) and proven by `console_pairing.rs`,
`dashboard_read_surface.rs` and `dashboard_procedure_transport.rs` against a real
Postgres, with each guard broken in turn and its test seen to fail (see
[`docs/status/verification/2026-10-26-vp6a-console-keys.md`](../status/verification/2026-10-26-vp6a-console-keys.md)).

Not built, and not claimed:

- **The console's own sign-in (VP-6b)**: key generation in the browser, the
  pairing request, signing every request, the pairing screen on vaam's admin
  phone. Until it ships the dashboard application cannot reach this surface.
- **vaam's side of `/svc/v1`**: the registered service key and the call itself.
- **Listing a person's keys**: only pairing and revoking exist.
- **Rate limiting of `/svc/v1`.** The sign-in limiter went with the sign-in
  (`rate_limit_windows` and `RateLimits` remain as unused scaffolding); a
  pairing is a signed call from a registered service, not an anonymous one.
- **A run against a cluster.** Nothing here has been applied to one.
