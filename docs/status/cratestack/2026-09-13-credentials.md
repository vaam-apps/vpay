# CrateStack — `credentials`, the model that is generic over kind and subject (2026-09-13)

Migration `0044` creates `credentials` and `schemas/vpay.cstack`'s
`model Credential` lands in the **same commit**, which is `customers`' rule
since 0034 and `staff_members`' since 0035. Decided in
[ADR-0019](../../adr/0019-credential-model.md).

## What moved

Five of `StaffMember`'s fourteen columns — `password_hash`,
`password_change_required`, `totp_secret`, `totp_enrolled_at` and
`last_totp_step` — were credentials whose **names hardcoded exactly two
authentication methods**. They are rows now: one per credential, a vocabulary
of eight kinds, and implementations for two.

Three of `vpay_db::Staff`'s six methods moved with them and were generalised
on the way, because none of them said anything about a password or a time step
that was not already "opaque material" and "a strictly increasing counter":

| Was                       | Is                              |
| ------------------------- | ------------------------------- |
| `Staff::enrol_totp`       | `Credentials::create`           |
| `Staff::record_totp_step` | `Credentials::advance_counter`  |
| `Staff::set_password`     | `Credentials::replace_material` |

`Staff` keeps `create`, `find_by_email`, `find` and `record_sign_in`, and
gained one — `Staff::delete`, which exists for a single caller and is a
**compensation**; see "What `staff add` now risks" below.

**Every method on the new module runs through the generated data layer**, so
the CrateStack statement registry grows by four and the table count by one.
That is a property of migration 0044 rather than of ambition: it repeats
0035's shape to the letter — no `jsonb`, no `bytea`, no native enum, no
`DEFAULT` on any column a writer names, no `seq` cursor.

## The measurement

Read off a freshly migrated `postgres:16-alpine` on 2026-09-13, not derived by
adding deltas.

| Constant                      | Before | After   |
| ----------------------------- | ------ | ------- |
| `EXPECTED_DRIFT_CHANGES`      | 179    | **190** |
| `EXPECTED_DRIFTED_RELATIONS`  | 24     | **25**  |
| `EXPECTED_UNMAPPABLE_COLUMNS` | 19     | **19**  |

**The +11 is fully accounted for.** `credentials` contributes **twelve** lines
and `staff_members` loses **one**.

The twelve, every one of them a hand-named CHECK or an undeclared index — the
two kinds `cratestack` structurally cannot close:

- **eight CHECKs**: `credentials_counter_is_not_negative`,
  `credentials_has_exactly_one_subject`, `credentials_id_length`,
  `credentials_issuer_length`, `credentials_kind_is_known`,
  `credentials_kind_length`, `credentials_staff_member_id_length`,
  `credentials_subject_length`;
- **four indexes**: `credentials_federated_identity_key`,
  `credentials_one_federated_link_per_issuer`,
  `credentials_one_singleton_kind_per_staff_member`,
  `credentials_staff_member_idx`.

The minus one is `staff_members_totp_step_is_not_negative`, which went with the
column it constrained.

**Migration 0044 declares eleven CHECKs and the report sees eight**, and the
three missing ones are the three that are **multi-column**:
`credentials_federated_carries_identity_and_no_material`,
`credentials_transient_kinds_expire` and
`credentials_only_a_password_may_require_change`. Introspection filters
`array_length(c.conkey, 1) = 1`, so it cannot see them in either direction.
`a_malformed_credential_is_refused_by_the_database` asserts all three against a
real Postgres, and `postgres_smoke.rs`'s multi-column CHECK inventory lists
them, because **the drift report is not the guard for those three and never can
be**.

**Not one `column ... type differs`, not one
`column ... default value differs`, not one
`column ... is declared in the schema but does not exist`.** That is what
"migration 0035's shape, to the letter" claims and this is the evidence for it.

**`EXPECTED_UNMAPPABLE_COLUMNS` did not move, and that is the assertion that
makes "no `bytea`, no `jsonb`" mean something.** `credentials` adds eleven
columns and not one lands in the unmeasured block. That is a decision, not
luck: per-kind material in a `jsonb` blob is the obvious shape for a table with
eight kinds, and a WebAuthn public key is the obvious `bytea`. Both would have
been **excluded from the comparison outright**, with drift on them unmeasured
and `EXPECTED_DRIFT_CHANGES` saying nothing at all about it. `material`,
`issuer` and `subject` are `TEXT`; `counter` is `BIGINT` and not `INT`, because
`int4` is one of the six `map_scalar` does not map and `currencies.exponent`
had to be widened by migration 0032 for exactly that reason.

## What a CHECK stopped being

`staff_members_totp_is_paired` —
`(totp_secret IS NULL) = (totp_enrolled_at IS NULL)` — is **gone, and it
dissolved rather than being dropped.** "Enrolled" was one fact spread over two
columns, and it needed a constraint to stop a writer leaving a sealed secret
with no enrolment date. Under the split, enrolment is **the existence of a
`totp` row** and `created_at` is when it happened, so there is no second column
for the first to disagree with.

`Staff::enrol_totp`'s `totp_enrolled_at IS NULL` guard went the same way, and
its replacement is stronger: a second enrolment is refused by
`credentials_one_singleton_kind_per_staff_member`, a partial unique index. A
`WHERE` clause can be dropped by an edit; an index cannot be dropped by one.

## What `staff add` now risks, and what closes it

`vpay-server staff add` was **one** insert and is **two** — a `staff_members`
row and a `credentials` row. Two inserts that are not one statement have a
window, and a failure in it would leave a staff member **who can never sign in
and whose address is taken**, because the email unique index refuses a second
`staff add` for them. The operator could not even retry.

So it compensates: on a failed credential insert it calls `Staff::delete`,
whose cascades take the sessions, the codes and any credential with it.

**A real transaction would be better and is not available.** `TxRepositories`
is a hand-curated trait of raw `sqlx` statements, and putting these two inserts
in it would take both tables off the generated data layer — which is the
property migrations 0035 and 0044 were both shaped to buy. The residual is
stated where it is paid: if the compensating delete **also** fails, the
operator is shown both errors and the `stf_…` to remove by hand.

## What is declared and not implemented

**Eight kinds; two work.** `password` and `totp` are reachable;
`hotp`, `magic_link`, `email_otp`, `phone_otp`, `webauthn` and `oidc` are
values the CHECK admits, the parse decodes and the repository stores, and which
**no code path in this deployment can reach**. Nothing mints them and
`vpay_api::staff_auth` has a verifier for two.

`issuer` and `subject` are columns with constraints, three indexes and **no
writer**. `expires_at` is a column with a CHECK and no writer. Nothing sweeps
`credentials`, because there is nothing to sweep — when something writes a
transient kind, it needs a sweep, and that absence is recorded here rather than
left to be discovered.

A reader who finds `'webauthn'` in a CHECK constraint must not conclude vpay
supports WebAuthn.
