# ADR-0038: Break-glass console keys are bounded and audited

- **Status:** Accepted. **Amends** [ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md):
  ruling **R3** ("a console key expires 30 days after pairing") becomes an
  **upper bound** for the ordinary vaam pairing, and the operator's break-glass
  command is given a shorter default life and a mandatory, journalled operator.
  Everything else ADR-0036 decides is unchanged.
- **Date:** 2026-11-19
- **Deciders:** the implementing change (PR #411). **There is no written owner
  ruling for the figures below.** The sources that exist are: the acceptance
  criterion the owner wrote on issue
  [#152](https://github.com/vaam-apps/vpay/issues/152) — "Break-glass/emergency
  access, if supported, is time-bounded and auditable" — and
  [ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md)'s
  owner ruling **R3** (2026-10-26, "a console key expires 30 days after
  pairing"), which this ADR turns into an upper bound. The **24-hour default**,
  the **720-hour cap** on a break-glass life, the mandatory `--operator` and the
  choice to journal in `audit_events` are design choices of this change, put to
  the owner for review; they are not quotations of a ruling.

## Context

[ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md) ruling
**Q2** made `vpay-server staff pair-key` the break-glass way in when no vaam
admin phone can pair a key, and ruling **R3** fixed every console key's life at
exactly 720 hours after pairing, enforced in the database by
`console_keys_lives_exactly_thirty_days`.

ISO/IEC 27001 readiness review (issue #152) found that break-glass door
wanting, and recorded it as `GAP-152-BREAK-GLASS` in
`schemas/security-inventory.yaml`. Three things, in the gap's own words:

> "A key paired that way lives the ordinary 30 days, needs no approval, and is
> recorded only by one `tracing` line on the process's output; no audit row, no
> shorter expiry, no second person."

The ordinary 30-day life is right for a key a person pairs onto their own
device: it bounds a stolen browser profile without forcing re-pairing. It is
wrong for an emergency: an incident ends in hours, and a key minted to repair
one should not outlive the week, let alone the month. And an emergency action
that leaves only a log line on a one-shot process is not auditable — the line
can be lost with the process's output, and it names no reporter.

## Decision

1. **A console key's life is a maximum, not an equality.** Migration `0063`
   replaces `console_keys_lives_exactly_thirty_days` with
   `console_keys_lives_at_most_thirty_days`: a row must satisfy
   `paired_at < expires_at` and `expires_at <= paired_at + 720 hours`. The
   `vpay_db::NewConsoleKey` a caller builds carries the `lifetime` it wants; the
   ordinary vaam pairing passes `CONSOLE_KEY_LIFETIME` (720 hours) exactly as
   before, so ruling R3 holds for every key a phone pairing creates. The vaam
   paths cannot be handed another life: the function they store through
   (`store_vaam_key`) takes no lifetime parameter, and only the break-glass
   `pair_from_operator` chooses one.

2. **The break-glass command defaults to 24 hours.** The command
   `vpay-server staff pair-key` takes `--ttl-hours` (default **24**, at most 720,
   `vpay_db::BREAK_GLASS_KEY_LIFETIME`). An operator finishing a longer incident
   asks for more explicitly; the default is a bound nobody has to remember.

3. **Every break-glass act names its operator and is journalled.** Both
   `staff pair-key` and `staff revoke-key` **require `--operator`**, and each
   writes one row to the append-only `audit_events` journal — `actor_kind`
   `operator`, `actor_id` the identity, `action` `console_key:pair` or
   `console_key:revoke`, `target_class` `console_key`, `target_repr` the key's
   RFC 9679 thumbprint (a public identifier, never PII) — **in the same
   transaction as the key write**, so a pairing or revocation made through the
   command never exists without its journal row and a failed journal write
   takes the key write down with it. Migration `0063` widens the journal's vocabulary
   (`audit_events_actor_kind_is_known`, `_action_is_known`, `_outcome_is_known`,
   `_target_class_is_known`) to admit the three new names; the table stays
   append-only.

An idempotent repeat of a member's own live key still writes no second audit
row, matching the command's existing idempotence.

The operator name is validated (non-empty, at most 128 characters, no control,
zero-width or direction-changing character) but it is **self-declared text**:
see below.

## What this does not do

- **No second person.** The gap also names "no approval / no second person".
  This ADR closes the _time-bounded_ and _auditable_ halves the acceptance box
  asks for; dual-control approval (a second operator confirming a pairing) stays
  open and is not built here.
- **No authenticated identity.** `--operator` is free text declared by whoever
  holds `DATABASE_URL`. Nothing proves it names the person at the keyboard, so
  the journal attributes an act by declaration, not by authentication. It is
  also personal data (examples use email addresses) on an append-only table with
  no retention or erasure handler; operators are told to pass a staff handle,
  and the privacy inventory classifies `audit_events.actor_id` as staff data.
- **Failed attempts are not journalled.** A pairing refused for an unknown or
  disabled member, a taken key or a bad lifetime writes no row; only a
  committed pair or revoke does.
- **Direct SQL bypasses the journal.** The same-transaction guarantee binds the
  two commands, not the table: a person who can run SQL against the database can
  still insert a `console_keys` row with no `audit_events` row. A database-side
  guard on `console_keys` inserts is not built.

These residual risks stay tracked as `GAP-152-DUAL-CONTROL` in
`schemas/security-inventory.yaml`, which keeps ISO 27001 control `A.8.2`
`partial`.

- **No reason field.** The journal records _who_, _what_, _when_ and _which key_
  — not a free-text justification, which would be an unclassified,
  potentially-personal column on an append-only table (RFC-0002 D9). The
  operator identity and the incident record carry that context.

## Consequences

- `console_keys_lives_exactly_thirty_days` no longer exists; it is
  `console_keys_lives_at_most_thirty_days`. Reader of the drift report and the
  multi-column CHECK list in `backends/tests/integration/tests/postgres_smoke.rs`,
  and of [ADR-0036](0036-the-console-is-a-signed-message-under-a-paired-key.md),
  take note.
- `audit_events` now carries `console_key:pair` / `console_key:revoke` rows
  alongside `account_holder:lookup`; an operator reads them the same way (the
  journal is tenant-scoped, and the pairing's tenant is the key owner's
  merchant).
- `GAP-152-BREAK-GLASS` in `schemas/security-inventory.yaml` is closed for the
  two properties issue #152 names (time-bounded, auditable). `A.8.2` stays
  `partial`: a new open gap, `GAP-152-DUAL-CONTROL`, carries dual control, an
  authenticated operator identity, journalling of refused attempts and the
  direct-SQL bypass.
