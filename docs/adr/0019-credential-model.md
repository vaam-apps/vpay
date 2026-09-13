# ADR-0019: Credentials are their own object, generic over kind and subject

- **Status:** Accepted
- **Date:** 2026-09-13
- **Deciders:** vpay maintainers (directed, 2026-09-13; amended the same day
  with the federation requirement)
- **Amends:** [ADR-0017](0017-staff-authentication.md) decision 1. What that
  ADR decided about _how_ a staff member proves who they are — argon2id with a
  deployment pepper, RFC 6238 with a sealed secret, the strictly-increasing
  replay guard, mandatory enrolment, the one-time password — is unchanged in
  every particular. What changes is _where the material lives_ and _how many
  shapes of it the schema can hold_.
- **Does not touch:** [ADR-0018](0018-cross-tenant-admin-reads.md).
  `merchant_id` and `is_admin` stay on `staff_members`; see decision 7.

## Context

`model StaffMember` carries three unrelated concerns in one row, and the
maintainer's instruction on 2026-09-13 was to separate the third:

| Concern         | Columns                                                                                          |
| --------------- | ------------------------------------------------------------------------------------------------ |
| Identity        | `id`, `email`, `display_name`, `status`, `created_at`, `updated_at`, `last_sign_in_at`           |
| Authorisation   | `merchant_id`, `is_admin`                                                                        |
| **Credentials** | `password_hash`, `password_change_required`, `totp_secret`, `totp_enrolled_at`, `last_totp_step` |

Five of fourteen columns are credentials and their _names_ hardcode exactly
two authentication methods. A third method — a magic link, an emailed code, a
WebAuthn key, an IdP link — is more columns on a table that is not about
credentials, every one of them `NULL` for every member who does not use that
method. That is the shape a table takes just before somebody reaches for
`jsonb`.

The instruction was explicit that this is for staff **now** and customers
**later** ("because then we might use it later with customers"), and explicit
about what it is not: not oauth2-proxy, not the auth plugin itself, not a
relaxation of any sign-in rule.

**Amended the same day, before implementation, with a requirement that
changes the shape rather than extending it: the model must be future-proof
for OIDC and SSO.** The reason is not "one more kind". It is that **a
federated identity is not a secret.** A `password` row holds material you
verify locally. An OIDC link holds `(iss, sub)` — a _public_ identifier —
and verification means "a token signed by that issuer validates against its
JWKS and its `sub` equals this row's". A model that assumes every credential
has secret material to check does not fit federated identity, and what
happens next is that somebody crams an issuer into a `password_hash` column.
So the amendment arrived in time to be designed for rather than bolted on,
which is the whole reason it is recorded here as context and not as a later
consequence.

### The three properties a redesign can lose without any gate noticing

Named in the maintainer's brief and repeated here because each is load-bearing
below:

1. **The persistence layer is credential-agnostic.**
   `backends/crates/vpay-db/src/staff.rs` states it: it "never hashes, never
   verifies and never decrypts — `password_hash` and `totp_secret` are opaque
   strings on the way in and on the way out". Verification lives above the
   data layer because it needs deployment secrets `vpay-db` has no business
   holding.
2. **No `bytea`, no `jsonb`, no native enum on these tables.** ADR-0017
   designed all three out of `staff_members`, `staff_sessions` and
   `oauth_authorization_codes` on purpose, which is why those three tables
   contribute **zero** `column ... type differs` lines to the drift report.
   `cratestack-migrate`'s `map_scalar` maps six Postgres types and neither
   `bytea` nor `jsonb` is among them, so a column of either is excluded from
   the drift comparison **outright**, in both directions. A WebAuthn public
   key is therefore base64url `TEXT`, exactly as the sealed TOTP secret
   already is.
3. **The replay counter's `NOT NULL` argument.** `last_totp_step` is
   `NOT NULL` seeded to `0` because `NULL < step` is `NULL` in SQL — not
   false — so a nullable counter makes the compare-and-swap match zero rows
   and refuse **every subject's first code, forever**. Whatever column carries
   a counter in the new model inherits that argument verbatim.

## Decision

### 1. One table, `credentials`, one row per credential

A credential row is: a **subject**, a **kind**, optional opaque **material**,
optional **federated identity**, and a small amount of per-kind state. It is
born on CrateStack as `model Credential` in `schemas/vpay.cstack`, in the
shape migration 0035 established — no `DEFAULT` on any column a writer names,
no `seq`, no native enum, no `bytea`, no `jsonb` — so that every repository
method can run through the generated data layer.

**`material` is `String?`, not `String`, and that nullability is the
amendment's whole footprint on the column list.** Before the amendment
`material` would have been `NOT NULL`, because every kind then in view
(password, TOTP, HOTP, magic link, OTP, WebAuthn) has something secret or
opaque to store. A federated link has nothing: `(iss, sub)` is public and the
verification material is the issuer's JWKS, which is fetched, not stored.
Making the column nullable and letting a CHECK decide per kind is the
difference between a model that admits federation and one that forces a
non-secret into a secret-shaped hole.

### 2. Fork 1 — a credential names its subject by a **nullable FK per subject type**, not by `subject_kind` + `subject_id`

`credentials.staff_member_id TEXT REFERENCES staff_members (id) ON DELETE
CASCADE`, nullable in the DDL, with
`CHECK (num_nonnulls(staff_member_id) = 1)` — which today is a laborious
spelling of `NOT NULL` and is written that way deliberately, because the day
`customer_id` is added the constraint becomes
`num_nonnulls(staff_member_id, customer_id) = 1` and nothing else about the
column list changes.

**The argument for the FK.** Referential integrity is a statement the
database makes, and the alternative asks a convention to make it. This schema
already has the shape twice over: `staff_sessions.staff_id` and
`oauth_authorization_codes.staff_id` are both real FKs onto `staff_members`
with `ON DELETE CASCADE`, for exactly the reason that applies here — "a staff
row that goes away takes its sessions with it, which is the only sane reading
of 'this account no longer exists'". A credential that outlives its subject is
worse than a session that does: it is an authenticator with nobody to
authenticate.

**What the loser costs, stated rather than waved away.** `subject_kind` +
`subject_id` is one shape forever: adding customers needs no migration, and
"every credential for this subject" is one index regardless of subject type.
Choosing the FK means adding customers costs a migration — one nullable
column, one FK, one replaced CHECK, and one replaced partial unique index per
uniqueness rule — and it means the per-subject read is written per column
rather than generically. That is a real cost and it is accepted, because vpay
has exactly **two** subject types in view, ever. Paying a permanent loss of
referential integrity to save two migrations is a bad trade.

**A correction to the brief's own framing, which decides this fork rather
than merely supporting it.** The brief says the FK "makes customer erasure
cascade". **It makes half of it cascade**, and the half it misses is the
larger one. `Customers::erase_in_tx` (issue #111, closed in #143) has two
shapes and only one is a `DELETE`:

- a customer **nothing references** is hard-deleted, and the row is gone — a
  cascade takes their credentials with it;
- a customer an intent, a session or an invoice references **cannot be**
  deleted. Those foreign keys are `NO ACTION` on purpose, "so a payment is
  never detached from the payer it was taken from". That customer is
  **anonymised in place**: the row stays, `anonymized_at` is stamped, and
  every identifier column becomes the `REDACTED` marker.

Any customer who has ever paid is in the second branch. **No cascade fires
for them**, so when `customer_id` arrives, `erase_in_tx` must carry an
explicit `DELETE FROM credentials WHERE customer_id = $1` in the same
transaction, beside the rewrites it already does to event bodies, `payer_ref`
and stored responses.

This is an argument **for** the FK and not against it, and the reason is the
asymmetry: under the FK exactly one of the two branches needs code, and the
other is enforced by Postgres whatever a future writer forgets. Under
`subject_kind` + `subject_id`, **both** branches need code, nothing enforces
either, and an orphaned credential row is not merely possible but invisible —
there is no constraint whose absence a test could notice. A design where
forgetting is caught half the time by the database beats one where forgetting
is never caught at all.

**The CHECK is multi-column** the moment `customer_id` exists, and
`num_nonnulls(staff_member_id)` is single-column today, so `cratestack-migrate`
can see the one and will never see the other. Neither is expressible in the
grammar — there is no `@@check` — so `backends/migrations/*.sql` stays
authoritative, as it already is for ten other cross-column CHECKs, and
`postgres_smoke.rs` asserts the ones the drift report cannot.

### 3. Fork 2 — uniqueness is per kind, expressed as partial unique indexes, and the rules are not one rule

"One password per subject" and "many federated links per subject, at most one
per issuer" are **different rules**, and the amendment is right that a single
uniqueness statement cannot carry both. Four indexes, in the migration:

| Rule                                       | Index                                                                     | Kinds                                              |
| ------------------------------------------ | ------------------------------------------------------------------------- | -------------------------------------------------- |
| One per subject                            | `UNIQUE (staff_member_id, kind) WHERE kind IN ('password','totp','hotp')` | `password`, `totp`, `hotp`                         |
| At most one link per issuer per subject    | `UNIQUE (staff_member_id, issuer) WHERE kind = 'oidc'`                    | `oidc`                                             |
| **`(issuer, subject)` is globally unique** | `UNIQUE (issuer, subject) WHERE kind = 'oidc'`                            | `oidc`                                             |
| No rule — many are correct                 | _(none)_                                                                  | `webauthn`, `magic_link`, `email_otp`, `phone_otp` |

**Where the rule lives, and how it is proven.** In
`backends/migrations/0044_create-credentials.sql`, and nowhere else.
`cratestack` cannot see an undeclared index — one of the two drift kinds it
structurally cannot close — so the `.cstack` model says nothing about
uniqueness and **no compile-time gate will ever notice if an index is
dropped**. What proves each rule is a container-backed test that writes the
second row and asserts `PersistenceError::Unique`, and the mutation that makes
each test worth having is dropping the index and watching it go red. The
mutations are recorded in the implementation notes; none of these rules is
claimed on the strength of the DDL alone.

**The third rule is the amendment's, and it is the federated equivalent of two
people sharing a password hash.** Two subjects claiming one IdP identity means
whoever signs in as `(iss, sub)` gets whichever row the query returns first.
It is global — not scoped to a subject and not scoped to a merchant — because
an IdP identity denotes **one human**, and a deployment where two staff rows
both claim `https://accounts.google.com` + `1234567890` has a bug no
per-tenant scoping makes safe. Scoping it per merchant would mean exactly that
bug is legal as long as the two rows belong to different tenants, which is the
cross-tenant account-takeover shape ADR-0018 spends a whole document keeping
out.

### 4. The kind vocabulary is eight, and **declaring a kind is not implementing it**

`password`, `totp`, `hotp`, `magic_link`, `email_otp`, `phone_otp`,
`webauthn`, `oidc`. Closed twice, exactly as `staff_members.status` is: by
`credentials_kind_is_known` at the database and by
`vpay_db::credentials::CredentialKind::parse` refusing anything else — `TEXT`
plus a hand-named CHECK, never a native enum, for the reason migration 0035's
header gives and migration 0032 had to undo.

**Two are implemented: `password` and `totp`.** The other six are values the
CHECK admits, the parse decodes, the repository stores and reads — and which
**no code path in this deployment can reach**, because nothing mints them and
`vpay_api::staff_auth` has a verifier for exactly two. That is stated here,
in `docs/status.md`, and in the model's own comment, because the failure mode
this repository's `CLAUDE.md` names first is making something look more
finished than it is. A vocabulary entry is a shape the schema can hold, not a
feature.

The repository is where the agnosticism is real: it stores opaque material and
**interprets none of it**. `staff.rs`'s property carries over unchanged —
`vpay_db::credentials` never hashes, never verifies, never decrypts and never
fetches a JWKS. The one credential rule that stays in the data layer is the
replay guard, and it stays because it is a compare-and-swap on a row (decision
6).

### 5. Federated identity is keyed on `(iss, sub)`, never on email, and issuer trust is configuration

**`iss` + `sub`, and any email in a token is display data.** An OIDC `sub` is
"locally unique and never reassigned" within its issuer; an email is mutable,
may arrive unverified, and inside a tenant is reassignable — a departing
employee's address handed to their replacement is normal IT hygiene and would
be an account takeover if it were the join key. So the match is
`(issuer, subject)` and an `email` claim is written nowhere and matched on
never.

**Lengths, checked against the specification and against real issuers rather
than guessed.** `subject` is bounded at **255**, which is not a round number
picked for comfort: OpenID Connect Core 1.0 §2 says the `sub` value "MUST NOT
exceed 255 ASCII characters in length". Real values sit far below it — Google
a 21-digit numeric string, Entra ID a ~43-character base64url value, Okta
`00u` plus 17, Keycloak a 36-character UUID, Auth0 a provider-prefixed
`google-oauth2|…` in the thirties — so 255 is the spec's own ceiling and not
an estimate from the sample. `issuer` is bounded at **512**, matching
`staff_members.email`: an Issuer Identifier is an `https` URL with no query or
fragment, the specification sets no length, and the longest real shapes
(`https://login.microsoftonline.com/<tenant-guid>/v2.0`, a Keycloak
`…/realms/<realm>`) are comfortably under a hundred.

**Which issuers a deployment accepts is configuration, not data, and this is
the half that is a security decision rather than a modelling one.** If the
allow-list were a table a credential row could point at freely, then a row
naming an attacker's issuer would be a **valid credential** — the attacker
signs their own token, their own JWKS validates it, and vpay believes it. So:
the accepted issuers live in the vpay configuration document beside the rest
of `staff_auth`, `jwks_cache.rs` fetches keys for those and only those, and a
`credentials` row whose `issuer` is not in the configuration is a row that
**verifies nothing**. The row is data; the trust is configuration; the
database never adjudicates trust.

**Per-deployment or per-merchant is left open, and deliberately.** Everything
else in `staff_auth` is per-deployment, and one IdP for one deployment is what
a first implementation wants. But `DashboardClient::merchant_id` already binds
a dashboard client to a tenant, and "different merchants use different IdPs"
is exactly the case a payment platform meets in its second year. Because the
`credentials` row carries the issuer string either way, **both shapes are
reachable from this schema without a migration**, which is what lets this ADR
leave it open honestly: it is a configuration-shape decision, it changes no
column, and it is listed under Reserved decisions below rather than settled in
passing.

### 6. Per-kind state: one counter, one expiry, one must-change flag — and what is deliberately absent

- **`counter BIGINT NOT NULL`, seeded `0`, `CHECK (counter >= 0)`.** This is
  `last_totp_step` renamed and generalised — the RFC 6238 time step of the
  last accepted code for `totp`, the RFC 4226 counter for `hotp` — and it
  inherits property 3 above **verbatim**: `NOT NULL` because `NULL < step` is
  `NULL` in SQL, so a nullable counter would make the compare-and-swap match
  zero rows and refuse every subject's first code forever. `BIGINT` and not
  `INT` because `int4` is one of the six types `map_scalar` does not map, and
  `currencies.exponent` had to be widened by migration 0032 for exactly that
  reason.
- **`expires_at TIMESTAMPTZ`**, with
  `CHECK (kind NOT IN ('magic_link','email_otp','phone_otp') OR expires_at IS NOT NULL)`.
  No writer today. It earns its column because the CHECK makes it
  load-bearing the moment a transient kind is implemented: a magic link with
  no expiry is a permanent bearer credential in an inbox, and the constraint
  refuses one before any reviewer has to notice.
- **`must_change BOOLEAN NOT NULL`**, with
  `CHECK (NOT must_change OR kind = 'password')`. `password_change_required`
  moved, and it moved because it is a property of the credential — "this
  material was printed by an operator and must be replaced by material the
  person chose" — not of the person.

**`totp_enrolled_at` is gone, and its disappearance is a strengthening.**
Migration 0035 needed `staff_members_totp_is_paired` —
`(totp_secret IS NULL) = (totp_enrolled_at IS NULL)` — to stop a code path
leaving a secret with no enrolment date or an enrolment with no secret, and
`Staff::enrol_totp` needed a `totp_enrolled_at IS NULL` guard to stop a second
enrolment silently replacing a working second factor. Under the split,
**enrolment is the existence of the row** and `created_at` is when it
happened, so the pair invariant has nothing to be untrue about, and the
re-enrolment guard is the partial unique index refusing the second `INSERT`
rather than a `WHERE` clause a future edit could drop. One CHECK and one
hand-written guard are replaced by one index and the shape of the table.

**`last_used_at` is deliberately absent.** It has no writer, no constraint
depends on it, and `staff_members.last_sign_in_at` already answers "when did
this person last complete a sign-in". Adding a per-credential column nothing
writes would be scaffold, and this ADR's own decision 4 is the reason not to.

**`last_sign_in_at` stays on `staff_members`**, against the brief's column
table, and the argument is the one the brief itself uses to keep `is_admin`
where it is. `last_sign_in_at` is "when this person last completed a full
sign-in" — a fact about the **person**, across every credential they hold and
across the two factors a single sign-in presents. Moving it would mean either
duplicating it on every credential row or picking one kind's row to be the
canonical one, and both are worse than leaving a column about a person on the
table about people.

### 7. A subject may hold zero secret-bearing credentials, and nothing anywhere may assume otherwise

Just-in-time SSO provisioning creates a staff member who has never had a
password and never will. So there is **no** invariant that a staff member has
a credential, no invariant that they have a password, no `NOT NULL` and no
CHECK expressing either, and the migration does not create one. Two
consequences are real today, before any federated code exists:

- **`POST /staff/password` answers "no such credential", not "no such
  person"**, for a member with no password row. Today every member has one —
  the migration backfills one per row and `staff add` writes one — so the
  branch is unreachable in this deployment; it is written as a refusal rather
  than as an `expect` because the first federated member makes it reachable.
- **The login path's constant-time property is preserved through the
  split.** `POST /staff/login` verifies against a dummy hash when no account
  matches, so that "no such address" and "wrong password" take the same path.
  A member who exists but holds no password credential is a **third** case
  that the split creates, and it takes the dummy path too — otherwise the
  response time distinguishes an SSO-only member from a nonexistent one, which
  is an account-enumeration oracle the pre-split code could not have had.

`is_admin` and `merchant_id` stay on `staff_members`. They are who the person
may read as, not how they proved who they are, and PR #166 is unaffected in
substance.

### 8. Redaction is per field, not per row, and the split is what makes that expressible

`StaffRow`'s hand-written `Debug` redacts `password_hash`, `totp_secret`,
`email` and `display_name`. `CredentialRow` gets a hand-written `Debug` on the
same principle and with a **per-field** answer, because the amendment is right
that a blanket rule is wrong in both directions:

| Field                                              | In `Debug`   | Why                                                                                                                                                                                               |
| -------------------------------------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `material`                                         | `[redacted]` | An argon2id digest is an offline cracking target; a sealed TOTP seed is a second factor; a WebAuthn key is neither, but the column is one column and the rule has to hold for its worst occupant. |
| `issuer`, `subject`                                | **shown**    | Public identifiers. An operator debugging a broken SSO link needs exactly these two, and redacting them is how that debugging session ends in someone printing the whole row by hand.             |
| `id`, `kind`, `counter`, `must_change`, timestamps | **shown**    | None is a secret and each is what an operator actually reads.                                                                                                                                     |

Note what the table does **not** do: it does not make redaction depend on
`kind`. `material` is redacted for every kind including the ones whose
material is not very secret, because a per-kind exception is a branch, and a
branch that decides whether to print a secret is one mis-edit away from
printing a seed. The amendment warns against exactly that, and the answer is
that the **column** decides, not the row's kind.

`@sensitive` goes on `material` in the `.cstack` — what
`cratestack-macros`' `is_sensitive_field` reads for audit redaction — and not
on `issuer`, `subject`, `counter` or the timestamps, for the same split.

### 9. Demo and e2e are **unchanged**, and that is the decision rather than an omission

`just demo-staff`, `compose.e2e.yml` and `dashboard.cy.ts` sign in with
password + TOTP today and sign in with password + TOTP after this change.
Nothing about the HTTP surface moves: the request bodies, the status codes,
the session states and the two-leg flow are identical, and the only thing that
changed is which table the material was read out of.

**That identity is the acceptance criterion, not a happy accident.** A split
that changed the sign-in flow would be two changes wearing one commit, and the
integration suite (`staff_sign_in.rs`) and the Cypress run are what say it
did not: they were written against the pre-split behaviour and they pass
unedited. **A test edited to accommodate this change would be evidence
destroyed**, so none was.

What changes when a third kind is implemented is a question this ADR can only
half-answer: `staff add` would need to say which kind it is creating, and a
demo that provisions an SSO member needs an IdP the demo controls. That is
work for the slice that implements a third kind, and naming it here is
cheaper than discovering it there.

### 10. The migration moves every existing row, atomically, with no window

`backends/migrations/0044_create-credentials.sql`, one file, and `sqlx::migrate!`
runs it in a transaction — so there is no instant at which the material has
left `staff_members` and not arrived in `credentials`. Four steps:

1. `CREATE TABLE credentials`, its CHECKs and its four indexes.
2. One `password` row per `staff_members` row: `material = password_hash`,
   `must_change = password_change_required`, `counter = 0`, timestamps copied.
3. One `totp` row per `staff_members` row **with `totp_secret IS NOT NULL`**:
   `material = totp_secret`, `counter = last_totp_step`,
   `created_at = totp_enrolled_at`.
4. `ALTER TABLE staff_members DROP COLUMN` for all five, which takes
   `staff_members_totp_is_paired` and `staff_members_totp_step_is_not_negative`
   with them.

**The material is copied byte for byte and no secret is re-derived**, which is
what makes "nobody is locked out" true rather than hoped: the argon2id PHC
string verifies under the same unchanged `staff_auth.password_pepper`, and the
sealed TOTP secret opens under the same unchanged
`staff_auth.totp_encryption_key`. A migration that re-hashed anything would be
a migration that logged everyone out, and there is nothing in this one that
could.

**The backfilled ids are derived rather than random, and the derivation is
chosen so the shape check still passes.** `'cred_' || substr(md5(id || ':password'), 1, 24)`
— `md5` is core Postgres, needs no extension, and its hex alphabet
`0123456789abcdef` is a **subset** of `vpay_core::ids`' base32 alphabet
`0123456789abcdefghjkmnpqrstvwxyz`, so 24 hex characters are 24 legal id
characters and `ids::is_well_formed(CREDENTIAL_PREFIX, …)` accepts a
backfilled row exactly as it accepts a minted one. `md5` is doing id
derivation here and no security work whatsoever; the alternative — a random id
per row — would make the migration non-deterministic for no gain, since a
credential id is not a secret and is never presented by a caller.

**The drop is in the same migration, and that is deliberate.** Leaving the
five columns behind as a "safety net" would leave two sources of truth for a
password hash, and the one nobody reads is the one that goes stale and then
gets read by accident. A migration is never edited afterwards
(`just verify-migrations` refuses changed bytes), so the rollback story is a
_new_ migration either way; keeping dead credential columns would not improve
it and would meanwhile put a live argon2id digest in a column no code
maintains.

## Reserved for the maintainer

Four decisions this ADR deliberately does not take. Each is written here
rather than defaulted, because the repository's rule is that a choice marked
as a maintainer's is surfaced and not quietly resolved.

### R1 — May a live payment deployment accept a single-factor credential?

**This is the policy question behind "in dev, a simple email can be enough",
and it is not the same question.** The maintainer's sentence is about which
_kinds_ a deployment enables and is plainly right about development. Whether a
**livemode** deployment may enable, say, `magic_link` alone — one emailed link
and you are inside a dashboard that reads a merchant's payment intents — is a
different question with a different blast radius, and the two get conflated
precisely because one sentence can be read as answering both.

The case **for** allowing it: a magic link to a corporate mailbox behind the
company's own SSO is often stronger in practice than a password a person
reuses, the factor has simply moved to the mail provider; refusing it pushes
small deployments toward a shared password, which is worse than either.

The case **against**: the mail provider becomes an unaudited dependency of
vpay's authentication with no way for vpay to know what it enforces; email is
the channel account recovery already uses, so one compromised mailbox is total
rather than partial; and this is a payment system, where the surface being
protected is other people's money.

**It is not settled here, and it is not live yet either.** No code path in
this deployment can reach a third kind, so nothing is enabled or disabled
today and no default has been smuggled in. It becomes live the moment a third
kind gets a verifier, and it should be decided **before** that, not during it.
The shape of the answer is likely `staff_auth.enabled_credential_kinds` with a
livemode boot refusal for sets the policy forbids — which is deliberately
described and **not implemented**, because implementing the mechanism is how
the default gets chosen by whoever writes it.

### R2 — May an IdP claim drive `is_admin`?

`is_admin` stays on `staff_members` — it is authorisation, and decision 7 is
firm about that. What is **not** decided is whether a group or role claim in
an IdP token may **write** it.

"The IdP says this person is an admin" and "vpay's own row says so" are
different trust models, and the difference is who can grant a cross-tenant
read. Under ADR-0018 that grant is deliberately narrow and operator-issued;
letting a claim drive it means anyone who can edit a group in the customer's
directory can grant it, and vpay would have no record of the grant beyond a
token that has since expired. The opposite reading is just as coherent: an
organisation that has centralised identity expects to manage roles there too,
and a vpay-local flag becomes the thing that is always stale.

The decision changes what #166 means, so it belongs to the maintainer. This
ADR's position is only that nothing in this slice does it, and that the
`credentials` table is the wrong place for the answer either way: a role is
not a credential.

### R3 — Is the issuer allow-list per deployment or per merchant?

Decision 5 argues both shapes are reachable without a migration and leaves the
choice open. It should be taken when the first IdP is actually integrated,
against a real customer requirement, rather than guessed at now.

### R4 — Does `staff add` grow a `--kind`, and what provisions an SSO member?

Decision 9 names it. It is only answerable alongside the slice that implements
a third kind.

## Consequences

**The drift report grows, and every new line is a hand-named CHECK or an
undeclared index.** `credentials` is a modelled table born in migration 0035's
shape, so it costs no `column ... type differs`, no
`column ... default value differs`, and no entry in
`EXPECTED_UNMAPPABLE_COLUMNS` — the two kinds `cratestack` structurally cannot
close are the only two it contributes. The exact numbers are **measured
against a freshly migrated Postgres and recorded in
`postgres_smoke.rs`**, never derived by adding predicted deltas; that file's
own comment demands it and the measurement is in the implementation notes.

**Sign-in costs one more round trip.** The login handler reads the staff row
and then the password credential; the TOTP leg reads the staff row and then
the TOTP credential. Both are primary-key or single-index reads on a table
with one row per credential per member, and both are already behind the rate
limiter, so this is a latency cost and not a new denial-of-service surface. It
could be collapsed into a join later; it is not collapsed now, because a join
would put the credential read and the "is this account still active?" re-read
into one statement, and ADR-0017 keeps those separate on purpose so that a
disabled account is re-checked rather than trusted from a join.

**`Staff::enrol_totp`, `Staff::record_totp_step` and `Staff::set_password` are
gone from the `Staff` trait**, and with them the `staff_members` columns they
wrote. Their replacements are on `Credentials` and are credential-agnostic:
`create`, `find_for_staff_member`, `advance_counter`, `replace_material`. The
`Staff` trait keeps `create`, `find_by_email`, `find` and `record_sign_in` —
which is what a trait about people should have had all along.

**A missing `@@allow` arm on `model Credential` is as dangerous as it is on
`model StaffMember`, and in the same asymmetric way.** `read` and `update`
fail **silently** — the policy compiles into the statement's `WHERE`, so an
empty allow list renders `FALSE`, and `advance_counter` returning `Ok(false)`
is indistinguishable from a replayed code. `advance_counter` **is** the replay
guard, so this is the same "most dangerous edit in the crate" that
`Staff::record_totp_step` carried, moved. It is pinned the same way, by a
container-free test naming the four slots.

**Nothing sweeps `credentials`.** There is no periodic delete of expired
transient credentials, because there are no transient credentials: nothing
writes `expires_at`. When something does, it needs a sweep; this ADR records
that requirement rather than implying the cleanup already exists.

**Six of the eight kinds have no implementation, and the schema does not
pretend otherwise.** They are reachable by no code path in this deployment.
`docs/status.md` carries the gap; this paragraph exists so that a reader who
finds `'webauthn'` in a CHECK constraint does not conclude vpay supports
WebAuthn.

**Federated identity is designed for and not built.** `issuer` and `subject`
are columns with constraints, three indexes and no writer. What is bought by
having them now rather than later is precisely what the amendment asked for:
the next person to add SSO finds a column shaped like an issuer instead of a
`password_hash` column with an issuer in it. What is **not** bought is any
working SSO, and `jwks_cache.rs` remains a cache with no federated caller.
