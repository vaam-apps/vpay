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
10. **Hard cutover.** The migration drops `oauth_authorization_codes`,
    `staff_sessions` and `staff_sso_attempts` and deletes the `password` and
    `totp` rows of `credentials`; everyone is signed out. The `oidc` link rows
    stay (a public identifier, and how a vaam person is found again).

## Consequences

- **The dashboard application cannot sign in until VP-6b ships the console's
  own pairing and signing.** Until then the way in is `vpay-server staff
pair-key` and a client that signs requests. `docs/status.md` says so.
- vaam needs a signing key registered under `service_clients` and a pairing
  screen on the admin phone; that is VP-6b and vaam's own work.
- The five dashboard settings of ADR-0017 (client id, redirect URI, scope, the
  password pepper and the TOTP key), the SSO block and its Keycloak test are
  gone from configuration, the Helm chart and the secret inventory.
- A lost browser profile is a revocation (`DELETE
/svc/v1/console_keys/{thumbprint}` or an operator's SQL), not a password reset.
- The first administrator of a fresh deployment is `vpay-server staff add
--admin` followed by `staff pair-key`, run by an operator with database
  access. That is deliberate: it is the only path, and it is the audit trail.
- One more auth mechanism fewer: the only tokens left in vpay are the OP's own
  (`/v1/oauth/*`, deleted in VP-4) and the rails'.
