# ADR-0036: The console is a signed message under a paired key

- **Status:** Accepted. **Supersedes** [ADR-0017](0017-staff-authentication.md)
  and [ADR-0030](0030-staff-sign-in-federates-to-an-oidc-provider.md) entirely
  (password, TOTP, session, authorization code, bearer token and OIDC SSO are
  deleted, not kept as a second door) and
  [ADR-0008](0008-dashboard-scope.md)'s "OIDC session" credential. It keeps
  [ADR-0018](0018-cross-tenant-admin-reads.md)'s tenancy rule unchanged.
- **Date:** 2026-10-26
- **Deciders:** the owner. The rulings are his: _"NEVER we should use access
  or refresh tokens. … We'll use ONLY keypair for signing."_ and _"vpay and vsms
  too — keypairs everywhere."_ (2026-09-29); _"Admin rights only via admin phone
  pairing"_ (2026-10-09); and, on 2026-10-26, **Q1** phone pairing only, so
  Keycloak SSO and password + TOTP sign-in are deleted; **Q2** break-glass is
  command-line key enrolment; **R2** a paired console user is a plain member bound
  to `dashboard.merchant_id` with `is_admin = false`, never an administrator
  through pairing; **R3** a console key expires 30 days after pairing.

## Context

`/v1` became a signed message in [ADR-0035](0035-v1-is-a-signed-message-not-a-token.md)
(VP-3). `/dash/v1` still authenticated a person with a password and a TOTP code,
a server-side session, an authorization code exchanged for an RS256 bearer
token, or an OIDC provider's ID token ([ADR-0017](0017-staff-authentication.md),
[ADR-0030](0030-staff-sign-in-federates-to-an-oidc-provider.md)): eight tables
and columns, a rate limiter and an OP client existed to carry them, and every
one of them is a token or a shared secret the ruling forbids. The vaam
operator console is a browser, and a browser can hold a P-256 key that cannot
be read back (WebCrypto, `extractable: false`).

## Decision

1. **Every `/dash/v1` request is a COSE_Sign1 message** (`ESP256`, ECDSA P-256)
   under a **console key**: a key a person's browser generated, registered in
   `console_keys`, and answered by a message sealed under the server's key. The
   layer is `EnvelopeMode::Required`; there is no optional mode and no token
   fallback. The same holds for CrateStack's procedure transport under
   `/dash/v1/$procs/…`, which has its own generated layer.
2. **A key is paired two ways, and only two.**
   `POST /svc/v1/console_keys` is called by **vaam** after its admin phone has
   vouched for a person, with the key and a proof that the browser holds its
   private half (ECDSA-SHA256 over `"vpay.console-pair.v1\n" + publicKey + "\n" +
issuedAt`, within 300 s of the server's clock). `vpay-server staff pair-key`
   is the **break-glass**: an operator with database access enrols a key for an
   existing member. Both go through one function (`console_keys::store_key`).
   Revocation has two doors the same way: `DELETE
/svc/v1/console_keys/{thumbprint}` and `vpay-server staff revoke-key
--thumbprint`, both through `console_keys::revoke`, so a deployment never has
   to reach for SQL to end a key.
   **The proof binds the key and the instant, not an audience.** It is not
   addressed to this deployment or to a vaam user: it reaches vpay relayed by the
   trusted service, which is the only party that can present it, and a proof
   lifted from that hop could only register the key its signer already chose,
   for the person the service already names. It cannot move a key to another
   person (the thumbprint is unique) and it is stale after 300 s.
3. **A person is linked to vaam by `(issuer = vaam, subject = <vaam user id>)`**
   in `credentials` (`kind = oidc`, the row ADR-0019 designed for exactly this).
   A first pairing provisions the member just in time: **`is_admin = false`,
   bound to `dashboard.merchant_id`**, from configuration and never from the
   request. An existing member's flag is never touched.
4. **Pairing never creates or upgrades an administrator.** The request body has
   no role field and rejects unknown ones; a person who is already an
   administrator is refused with a `403` (and a log line saying why) — a service holding
   `console_pairing` can name any linked person, so this route must not be a way
   to a key that speaks as an administrator. Administrators are made by `staff
add --admin` and keyed by `staff pair-key`, both run by an operator
   (_"admin rights only via admin phone pairing"_ is the vaam half: the phone
   decides who may ask; vpay's half is that a service route cannot grant it).
   **The command line can key an existing administrator, and that is stated, not
   incidental:** `staff pair-key` is the break-glass, an operator who can run it
   against the database already holds more authority than any pairing grants, and
   it neither creates a member nor changes a flag. **Revocation is asymmetric the
   other way, on purpose:** `DELETE /svc/v1/console_keys/{thumbprint}` and `staff
revoke-key` revoke _any_ member's key, an administrator's included. Revoking
   is the incident-response power; a service that could not cut off the most
   privileged keys would be useless in exactly the incident it exists for, and
   revoking grants nothing.
5. **A key lives exactly 30 days** (720 hours) from `paired_at`, enforced by a
   `CHECK` (`console_keys_lives_exactly_thirty_days`) as well as by the writer.
   It is never extended: a console pairs again with a new key. A **revoked or
   expired key is never registered again** — the row is never deleted, so its
   thumbprint stays taken.
6. **Revocation takes effect on the next request.** The resolver reads
   `console_keys` on every request, with no cache; a revoked or expired key,
   an unknown one and a bad signature are the same coarse unsigned `401`. After
   the signature verifies, the member is re-read on every request: a disabled
   member, or one who belongs to another merchant than the dashboard's, is a
   sealed `403`.
7. **Tenancy is fixed by the binding.** A non-administrator reads
   `dashboard.merchant_id` and nothing the request says (the `?merchant_id=`
   parameter is not parsed for them); an administrator may name another
   registered merchant ([ADR-0018](0018-cross-tenant-admin-reads.md)).
8. **Services are a separate registry from merchants.**
   `service_clients[]` in the YAML configuration carries `client_id`,
   `capabilities` (a closed set; today `console_pairing`) and `cose_keys`, in the
   shape of `merchant_clients[].cose_keys` and read by the same code. A merchant
   key is not a key on `/svc/v1`, and a service key is not a key on `/v1` or
   `/dash/v1`; boot refuses one public key present in both lists. **A capability
   is a field of the YAML and nothing else** — no HTTP route, no database row and
   no environment variable can grant one — so granting `console_pairing` is a
   reviewed configuration change, the way adding a merchant is. A service is
   disabled with the merchants' kill switch (`disabled_clients`).
9. **`dashboard:` replaces `dashboard_client:`.** It names the one tenant the
   console reads (`merchant_id`) and nothing else: there is no client id, no
   redirect URI, no scope and no secret to configure.
10. **A first pairing is one transaction.** The just-in-time member, the
    `(vaam, <user id>)` link and the key commit together (`pair_console_key_in_tx`
    beside `provision_federated_staff_in_tx`), so a refusal leaves no member and no
    link. Two refusals are told apart: a taken or revoked key is `KeyTaken`
    (`409`, "already registered"), and a synthetic address already held by another
    account (two vaam ids differing only in letter case) is `IdentityTaken` (`409`,
    "cannot be provisioned").
11. **Which surface `/svc/v1` is on.** The business surface
    (`deployment.surfaces` containing `business`), not the management one.
    `/dash/v1` is what a person's browser reads; `/svc/v1` is what a registered
    service calls, as a merchant calls `/v1`. In a split release
    ([ADR-0022](0022-surface-isolation-and-independent-scaling.md)) `-server` is
    business-only and `-management` admits only the dashboard, so serving
    `/svc/v1` from `-management` would have left it without a caller: vaam dials
    the one in-cluster origin it already knows, `http://vpay:8080`, which is the
    `-server` Service. The alternative, routing `/svc/v1` to `-management` and
    admitting vaam to that tier, would have changed vaam's origin and widened the
    tier that is meant to be reached by one workload. Consequences: a
    `surfaces: [management]` process answers `404` on `/svc/v1`; the chart
    admits the caller to `-server` through `networkPolicy.serviceIngress.peers`
    (a guard refuses a peer that selects everything); and no HTTPRoute or
    Ingress publishes `/svc/v1` (`helm-check` asserts all three). With no
    `dashboard:` neither `/svc/v1` nor `/dash/v1` is mounted, whatever the surface.
12. **The per-request key lookup is indexed.** The resolver reads `console_keys`
    by `thumbprint LIKE '<kid>%'` before it verifies the signature, so any
    well-formed `kid` costs a read. Migration 0057 adds a `text_pattern_ops` index
    (partial on `revoked_at IS NULL`) because the unique btree answers a prefix
    `LIKE` only under the C collation.
13. **Hard cutover.** The migration drops `oauth_authorization_codes`,
    `staff_sessions` and `staff_sso_attempts` and deletes the `password` and
    `totp` rows of `credentials`; everyone is signed out. The `oidc` link rows
    stay (a public identifier, and how a vaam person is found again).

## Consequences

- **The dashboard application cannot sign in until VP-6b ships the console's
  own pairing and signing.** Until then it renders one notice page (its sign-in,
  BFF and session code is deleted) and the way in is `vpay-server staff
pair-key` and a client that signs requests. `docs/status.md` says so, and
  `docs/status/frontend.md` lists the end-to-end cases VP-6b restores.
- A deployment that still names `dashboard_client` or `staff_auth`, even as a
  null, does not boot (`ConfigError::RetiredKey`, exit 78); `docs/flows/dashboard-auth.md`,
  "Upgrading from before VP-6a", is the checklist. With no `dashboard:` the
  console is simply off (owner ruling R4).
- vaam needs a signing key registered under `service_clients` and a pairing
  screen on the admin phone; that is VP-6b and vaam's own work.
- The five dashboard settings of ADR-0017 (client id, redirect URI, scope, the
  password pepper and the TOTP key), the SSO block and its Keycloak test are
  gone from configuration, the Helm chart and the secret inventory.
- A lost browser profile is a revocation (`DELETE
/svc/v1/console_keys/{thumbprint}` or `vpay-server staff revoke-key`), not a
  password reset.
- The first administrator of a fresh deployment is `vpay-server staff add
--admin` followed by `staff pair-key`, run by an operator with database
  access. That is deliberate: it is the only path, and it is the audit trail.
- One more auth mechanism fewer: the only tokens left in vpay are the OP's own
  (`/v1/oauth/*`, deleted in VP-4) and the rails'.
