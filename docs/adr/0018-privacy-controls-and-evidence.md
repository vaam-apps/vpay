# ADR-0018: Privacy controls share one inventory and evidence architecture

- **Status:** Accepted
- **Date:** 2026-09-13
- **Deciders:** vpay maintainers
- **Related:** [issue #149](https://github.com/vaam-apps/vpay/issues/149),
  [RFC-0002](../rfc/0002-gdpr-policy-and-operator-decisions.md)

## Context

The GDPR engineering-readiness epic covers six connected pieces of work:

- a personal-data inventory and retention map
  ([issue #144](https://github.com/vaam-apps/vpay/issues/144));
- telemetry and webhook minimisation
  ([issue #147](https://github.com/vaam-apps/vpay/issues/147));
- controls around account-holder lookup
  ([issue #56](https://github.com/vaam-apps/vpay/issues/56));
- retention, erasure and holds
  ([issue #145](https://github.com/vaam-apps/vpay/issues/145));
- data-subject search and export
  ([issue #146](https://github.com/vaam-apps/vpay/issues/146));
- privacy-incident evidence and export
  ([issue #148](https://github.com/vaam-apps/vpay/issues/148)).

They are not independent features. The inventory decides what telemetry may
carry, what retention must act on, what an export may include and what an
incident report must name. Account-holder audit records, erasure records,
subject exports and incident exports need the same actor, tenant, target,
outcome, time and correlation vocabulary. Building a table, export format or
classification for each issue would create several partial answers that drift
in different directions.

The repository already has useful pieces, but not the shared control:

- customer deletion either removes an unreferenced row or anonymises a
  referenced one and rewrites known copies in events, idempotency responses and
  rail-authored failure text;
- `rate_limit_windows` is a durable fixed-window counter shared by replicas;
- `events`, `provider_requests` and `webhook_deliveries` retain parts of an
  operational timeline;
- account-holder lookup binds a merchant tenant, masks the number in logs and
  persists nothing;
- `/v1` authorisation chooses scopes by HTTP method, so it cannot assign a
  dedicated scope to one `GET` route;
- staff identities belong to one merchant and there is no cross-tenant
  operator principal;
- webhook delivery keeps latest state rather than immutable attempt history;
- authentication and authorisation events are principally tracing records.

These facts are current implementation, not proof that the epic is complete.
No control introduced by this ADR exists merely because its architecture is
now decided. [docs/status.md](../status.md) remains the authority on what has
actually run.

The epic also leaves legal and operational policy to people outside the code:
controller or processor roles, lawful bases, retention periods, disclosure
approval, breach classification and regulatory notification are not software
defaults. [RFC-0002](../rfc/0002-gdpr-policy-and-operator-decisions.md) keeps
those choices open and blocks the implementation stages that require them.

## Decision

### 1. One exhaustive, machine-readable processing inventory

vpay will maintain one version-controlled inventory of its processing
surfaces. It will classify every persisted database column, including columns
judged non-personal, and enumerate non-database surfaces explicitly:

- logs, traces, metrics and error responses;
- event and webhook payloads;
- jobs and stored diagnostic text;
- browser storage, cookies, URLs and session state;
- provider, merchant, observability and database/backup recipients;
- credentials, derived identifiers and security evidence.

Every entry will identify its source, subject category, product purpose,
whether the value is necessary for that purpose, tenant boundary, copies,
recipients, configured retention/deletion behavior, retention owner and code
or configuration reference. Copies of one data element share a stable element
identifier; every additional copy names why it is necessary and which control
removes or protects it. Source-derived checks detect persisted columns and
registered disclosures that have no classification. Positive projections
prevent new fields from crossing registered disclosure boundaries by default.
No gate can determine that two differently named values identify the same
person or judge whether a stated purpose is necessary, so those remain review
questions rather than claims made by the automation. A legal basis or
controller/processor role that has not been approved will be recorded as
unresolved, never inferred.

The inventory will have a two-directional gate: a new mechanically enumerable
surface without a row and a stale row without a surface both fail. Database
completeness will be checked against a fully migrated database or the
migrations that create it, not only against `schemas/vpay.cstack`; that schema
deliberately models less than the whole database. Non-database surfaces must
either pass through a central registered boundary or have a static scanner
whose source set is independent of the inventory. Deleting an inventory row
from a self-authored list does not prove that new tracing, browser storage or
outbound recipients will be found. A surface that cannot yet be enumerated is
recorded as a gate gap and keeps #144 open until review or architecture makes
it enumerable.

The same inventory and data-flow map will serve the security asset work in
[issue #150](https://github.com/vaam-apps/vpay/issues/150). GDPR and ISO work
must not maintain competing lists of the same services, stores and recipients.

### 2. Disclosures are positive projections

A value crosses a telemetry, webhook or export boundary only through an
explicit positive projection approved for that boundary. Code will not
serialise an internal row or complete API object and then remove a list of
known secrets or personal fields.

This applies to:

- `Debug` and structured tracing fields;
- public error envelopes and private diagnostic chains;
- metric labels;
- stored events and delivered webhooks;
- data-subject exports;
- privacy-incident evidence exports.

Shared wrappers will represent secrets, masked values, pseudonymous values and
safe URL diagnostics. Raw rail or receiver prose is untrusted input: bounding
its length does not make it safe for a log or export.

Exact-key contract tests and canary-value leak tests will protect each
boundary. A webhook event gets a dedicated payload projection per event family
once RFC-0002 records the minimum contract; its payload is not defined by
whatever fields the corresponding API object happens to have.

### 3. One append-only audit and security-event vocabulary

Privacy-sensitive actions will write to one generic, privacy-minimal journal.
This journal is also the implementation path for ADR-0008's existing rule that
every dashboard write produces an `audit_log` row; it does not introduce a
second audit store. Its closed vocabulary will cover every dashboard write as
that surface is implemented, account-holder lookups, holds, erasure, subject
search/export, incident export and the authentication or authorisation events
approved for durable retention.

Every row will carry only the approved subset of:

- immutable event id and timestamp;
- tenant;
- actor or service identity;
- action and outcome;
- target class and approved target representation;
- request, event or operation correlation identifiers.

The journal will not be an unrestricted JSON copy of request or response
bodies. Its repository exposes append and bounded, tenant-scoped reads; it
does not expose an update operation. Deletion, if policy permits it, arrives
later as a separately authorised retention operation and never as ordinary
record maintenance.

Where one database transaction can contain the product change and its audit
row, both use the same `UnitOfWork`. An external provider call cannot be made
atomic with Postgres. Such workflows record the facts and crash semantics
approved in RFC-0002; that RFC still decides whether account-holder lookup uses
initiated and terminal rows or one terminal row. Whichever answer is approved
must state what evidence a crash can leave and must not describe an absent row
as proof that no provider call happened.

Webhook delivery gains immutable per-attempt evidence separately from the
existing `webhook_deliveries` retry-state row. This journal and incident
timeline will also serve the broader security evidence work in
[issue #154](https://github.com/vaam-apps/vpay/issues/154).

### 4. Retention is a closed product vocabulary with scoped holds

Every inventory entry that vpay stores will map to a closed retention class.
A class states an approved action: delete, anonymise or pseudonymise. Periods
come from validated, explicit configuration or another approved source; code
does not invent a legal period and a missing required class fails boot rather
than disabling enforcement silently.

Every hold is bound to a tenant and to the target granularity approved in
RFC-0002, which may be the tenant itself or a narrower class, subject or record.
It records who placed or released the hold, why, when and under which reference.
A deployment-wide switch with no tenant boundary is not a substitute for
scoped semantics.

Scheduled enforcement uses the existing durable worker and bounded pages.
The inventory-to-retention mapping is two-directional: every applicable class
names an executable handler and schedule, and no handler survives the class it
enforces. Every action is idempotent and auditable. A hold blocks only its
target, a second sweep changes nothing, and a persistence failure cannot be
reported as successful erasure.

Infrastructure that vpay does not own remains an explicit external
dependency. The inventory and runbooks name the owner and eventual-erasure
procedure for logs, replicas, PITR and full backups without claiming that this
repository enforces it.

### 5. Authorisation is route-specific and operator access is separate

The `/v1` route registry will carry each route's scope policy. Authentication
and tenant resolution remain one boundary, but authorisation will no longer
infer every route's privilege solely from its HTTP method. A route added
without a declared policy must fail a structural test or compilation.

This permits account-holder lookup to require the dedicated scope chosen in
RFC-0002 without changing every other `GET`. Its rate limit will reuse the
durable fixed-window counter after an additive migration extends the database's
closed scope vocabulary. The approved credential identity, budget and
`Retry-After` semantics remain RFC decisions.

Privacy search, export, audit queries and incident evidence are separate
operator capabilities and are never granted by ordinary merchant access.
RFC-0002 chooses whether they use a new principal source or explicit operator
grants on an existing identity model, plus the authentication and interface.
Every repository query still contains its tenant predicate; prior
authentication is not a reason to omit data-layer isolation.

Exports use versioned, allowlisted domain DTOs and a shared envelope. Subject
and incident exports reuse the same output and self-audit machinery, while
remaining different queries with different approved contents.

### 6. Delivery follows the dependency graph

Implementation follows the epic's order:

1. inventory and drift gate (#144);
2. telemetry and webhook minimisation (#147);
3. account-holder scope, limiter and audit foundation (#56);
4. retention, erasure and holds (#145);
5. subject search and export (#146);
6. incident evidence, export and tabletop (#148).

The work lands as small reviewable changes rather than one epic branch. A
later stage may prepare decisions while an earlier stage is in review, but it
does not implement against an unapproved RFC answer.

Every persistence change is additive: existing migration bytes remain
immutable, the migration manifest is updated, new tables are born in
CrateStack shape, repository implementations remain private to `vpay-db`, and
tenant and failure behavior are tested against real Postgres.

Each stage names the mutation that would defeat its control and proves that
the corresponding test or gate fails. The final evidence is the repository's
own `just fmt` and `just ci`, plus `just docs-check-citations` when evidence
citations change. A skipped database suite is not evidence.

## Alternatives considered

### Implement each issue independently

Rejected. It creates duplicate inventories, audit tables and export formats,
and makes retention depend on whichever issue most recently classified a
field. The issues describe one processing system and need one source of truth.

### Treat logs as the audit journal

Rejected. This repository does not own log storage, access control or
retention, and application logs are not naturally tenant-scoped query models.
They remain operational telemetry, while durable product evidence is stored in
an append-only database vocabulary.

### Serialize internal rows and redact known fields

Rejected. A new credential or personal field is exposed by default until
someone remembers to subtract it. Positive projections fail in the safer
direction: a new field is absent until deliberately included.

### Let ordinary merchant access invoke operator workflows

Rejected. Existing staff identities belong to one merchant by ADR-0017, and a
normal merchant permission must not silently become a privacy-export or
incident-investigation permission. RFC-0002 may choose explicit operator grants
on an existing identity model or a new principal source, but either choice is a
separate authorisation boundary.

### Put retention periods in this ADR

Rejected. The current 365-day customer rule and ADR-0013's proposed backup
periods are engineering history, not approved GDPR retention requirements.
The policy owners and evidence for each number are RFC-0002 questions.

## Consequences

- #144 is the foundation and may expose data copies the later issue scopes did
  not anticipate. Those findings update the plan rather than being forced into
  an incomplete class.
- A new persisted field costs an inventory classification and, where
  applicable, an export and retention decision.
- Route metadata becomes the source of merchant scope requirements; method-only
  scope inference is retired when #56 lands, not by this document.
- The audit journal is itself personal or security data and must have a
  separately approved retention class.
- Event/webhook contracts may become narrower. Any compatibility impact is
  decided and versioned before implementation rather than hidden in a privacy
  refactor.
- No legal role, lawful basis, retention period, disclosure approval or breach
  notification decision is made here.
- This ADR records architecture, not completion. Every capability remains open
  until its issue acceptance tests, status update and verification evidence
  have landed.
