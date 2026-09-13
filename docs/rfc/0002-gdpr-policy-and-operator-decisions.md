# RFC-0002: GDPR policy and operator decisions

- **Status:** Under review
- **Author:** vpay maintainers
- **Date:** 2026-09-13
- **Related:** [issue #149](https://github.com/vaam-apps/vpay/issues/149),
  [ADR-0018](../adr/0018-privacy-controls-and-evidence.md)

## Problem

[ADR-0018](../adr/0018-privacy-controls-and-evidence.md) decides how vpay's
privacy controls fit together. It deliberately does not choose the policy
values that determine what data may be used, disclosed or retained, who may
operate privileged workflows, or when an organisation must act under law.

Those choices block parts of the GDPR engineering-readiness epic. Choosing
plausible defaults in code would turn an implementation convenience into an
unreviewed legal or operational policy. This RFC puts every such choice in one
place, identifies the implementation stage it blocks and records a technical
recommendation without treating that recommendation as approval.

The organisation remains responsible for controller/processor determinations,
lawful bases, notices, contracts, international transfers, data-subject
identity verification, disclosure approval, breach assessment and regulatory
notification. Acceptance of this RFC may record who owns those processes; it
cannot automate their decisions.

## Proposal

Resolve the decision register below before the named implementation stage.
Each accepted answer must include:

- the selected option or precise value;
- the accountable owner;
- the legal, contractual, risk or operational requirement supporting it;
- the date approved;
- any deployment migration or compatibility plan;
- the evidence by which an operator can verify the control.

An unresolved answer remains `OPEN`. An implementation PR must not copy a
recommendation into code and then mark the row resolved after the fact.

### Decision register

| ID  | Decision                                                      | Blocks                         | Status |
| --- | ------------------------------------------------------------- | ------------------------------ | ------ |
| D1  | Product purposes, subject categories and legal-role ownership | #144 inventory approval        | OPEN   |
| D2  | Minimum field set for every event and webhook family          | #147 webhook projections       | OPEN   |
| D3  | Identifier masking and pseudonymisation policy                | #147 diagnostics, #56 audit    | OPEN   |
| D4  | Webhook endpoint query-string and fragment policy             | #147 URL validation            | OPEN   |
| D5  | Dedicated account-holder OAuth scope and rollout              | #56 authorisation              | OPEN   |
| D6  | Meaning of "per key", rate budget and refusal semantics       | #56 limiter                    | OPEN   |
| D7  | Account-holder audit facts and crash semantics                | #56 audit writes               | OPEN   |
| D8  | Operator principal, authentication, grants and surface        | #56 audit query, #146 and #148 | OPEN   |
| D9  | Retention classes, actions and periods                        | #56 journal, #145 enforcement  | OPEN   |
| D10 | Hold authority, scope, expiry and release                     | #145 holds                     | OPEN   |
| D11 | Subject identifiers, matching and export contract             | #146 search/export             | OPEN   |
| D12 | Security-event taxonomy and incident export contract          | #148 evidence                  | OPEN   |
| D13 | Owners and eventual-erasure obligations for deployment stores | #145 and #148 runbooks         | OPEN   |

### D1: Product purposes and legal-role ownership

Decide, for every category in the #144 inventory:

- data-subject category;
- product purpose and whether the field is required for that purpose;
- recipient and transfer owner;
- who decides controller/processor role and lawful basis;
- who approves changes to purpose or recipient.

**Recommendation, not a decision:** the engineering inventory should use
neutral product purposes and mark legal role or basis `unresolved` until the
named owner approves it. It should not make absence of legal input a build
failure that developers can only clear by guessing.

### D2: Minimum webhook contract

Decide the minimum approved fields for every emitted event family and whether
narrowing an existing payload requires a new API/event version or migration
window. The present implementation stores complete rendered API objects for
several event types; compatibility therefore has to be assessed before fields
are removed.

**Recommendation, not a decision:** approve an explicit field allowlist per
event family, preserving correlation, object identity, transition and time,
then version any incompatible reduction. Do not define "minimum" as "whatever
the API object currently contains".

### D3: Masking and pseudonymisation

For each surface, decide whether merchant, staff, customer, payment, charge,
provider, account-holder and credential identifiers are:

- permitted raw;
- masked but recognisable;
- pseudonymised with a domain-separated digest;
- pseudonymised with a keyed function;
- forbidden.

If a keyed scheme is selected, also decide key custody, rotation, overlap and
incident handling. State whether operators must be able to start from a raw
value and derive its search token.

**Recommendation, not a decision:** use existing opaque vpay object IDs raw
only where operations need them. A Cameroon MSISDN has an enumerable input
space, so an unkeyed digest is reversible by dictionary enumeration despite
domain separation. For account-holder and similarly low-entropy targets,
prefer a tenant-bound, domain-separated HMAC or another approved keyed PRF and
settle key custody and rotation before the first audit row is written. Never
place secret material in a correlation field.

### D4: Webhook endpoint URL policy

Configuration already rejects URL userinfo but permits query strings and
fragments. A query can carry a receiver credential, after which the full URL
is stored or logged as another secret copy.

Choose one:

1. forbid all query strings and fragments;
2. allow only a closed set of non-secret query keys;
3. permit them but store and render a canonical redacted URL while retaining
   the full value only in protected runtime configuration.

**Recommendation, not a decision:** option 1 is smallest and safest. Its
compatibility impact on configured merchants must be checked before adoption.

### D5: Account-holder scope and rollout

Decide the exact OAuth scope and whether any existing scope implies it.
Existing credentials use `payments:read` or `payments:write`; requiring a new
scope immediately will refuse them after deployment.

Choose the rollout:

1. immediate dedicated scope with explicit credential re-registration;
2. a dated compatibility interval in which old and new scopes work, followed
   by removal;
3. permanent implication from another scope.

**Recommendation, not a decision:** use `account_holders:read`, implied by no
payment scope, with a documented migration before enforcement. Permanent
implication defeats the least-privilege purpose of issue #56.

### D6: Rate-limit identity, budget and response

Issue #56 asks for a fixed window per key. Decide what a key means in the
authenticated request:

- OAuth client registration (`client_id` or token subject);
- signing key (`kid`);
- merchant tenant;
- another credential identity.

Also approve attempts per window, whether malformed or unsupported requests
spend the budget, the precise boundary behavior and whether `Retry-After`
reports seconds remaining or the complete window.

**Recommendation, not a decision:** count against the narrowest authenticated
credential identity that the verifier can propagate reliably, before any rail
call; fail closed if the shared counter cannot be written; return the actual
remaining fixed-window duration. Do not call a client-wide counter "per key".

### D7: Lookup audit semantics

Decide which facts an append-only account-holder audit retains:

- initiated and terminal rows, or one terminal row;
- raw, masked or pseudonymous account-holder target;
- found, not found, invalid, unsupported, rate-limited and provider-error
  outcomes;
- actor identity and request correlation;
- audit retention class.

An external MTN call cannot be atomic with the audit database. Decide whether
an initiated row with no terminal row is the required crash evidence.

**Recommendation, not a decision:** append an initiated fact before the rail
call and one terminal fact after it under one operation id. Persist the
tenant-bound keyed target representation approved under D3, not the raw
MSISDN, an unkeyed digest or the returned name.

### D8: Operator security boundary

Choose how a human receives permission to query audits, search/export subject
data and export incident evidence. The current staff model is intentionally
bound to one merchant and cannot represent a platform operator.

The decision must cover:

- principal source and provisioning;
- MFA or upstream OIDC requirements;
- tenant grants and any exceptional cross-tenant privilege;
- least-privilege roles for audit query, subject export, holds and incident
  export;
- revocation and session lifetime;
- CLI, HTTP/dashboard or both;
- audit of every privileged invocation;
- separation of approval from execution where required.

**Recommendation, not a decision:** create a separate operator principal and
surface. Default every operation to exactly one explicitly granted tenant.
Host access to a one-shot CLI alone does not demonstrate authenticated and
authorised invocation. ADR-0018 permits explicit operator grants on an
existing identity model, but ordinary merchant staff permissions cannot imply
those grants.

### D9: Retention matrix

Approve a class, action and period for every stored category:

- customer and account-holder identifiers;
- payment, charge, invoice, refund and checkout records;
- metadata, descriptions and rail-authored failure text;
- events and webhook state/attempts;
- idempotency and worker records;
- authentication, session, code, replay and rate-limit artifacts;
- audit and security evidence.

For each class choose delete, anonymise or pseudonymise, state the trigger and
identify records that must retain accounting or integrity links.

There is a current conflict to resolve explicitly:
`vpay-worker/src/handlers.rs` fixes customer inactivity at 365 days and says it
must not be configurable because it is a promise to the payer. Issue #145
requires periods to be explicit and configurable and forbids hard-coding a
legal period without approval. ADR-0013's proposed 30-day PITR and 90-day full
backup periods are also unvalidated proposals, not answers to this RFC.

**Recommendation, not a decision:** use a closed validated configuration with
no silent defaults for legally significant classes. Record the approved source
and owner beside each value; a missing required class should fail boot.

### D10: Hold model

Decide:

- who may place and release a hold;
- whether dual approval is required;
- target granularity: tenant, class, subject or record;
- mandatory reason/reference;
- expiry and renewal;
- whether release is a new append-only fact;
- which retention classes may be held.

**Recommendation, not a decision:** holds should be tenant-scoped and narrow,
with an expiry unless the approved policy forbids one. Placement and release
should be append-only evidence; a deployment-wide boolean should not suspend
all erasure.

### D11: Subject search and export

Approve:

- searchable identifiers and exact canonicalisation/matching rules;
- whether anonymised rows appear and how they are described;
- export version, media type and provenance/purpose vocabulary;
- approved record families and fields;
- output channel and whether generated files may be persisted;
- who verifies identity, approves disclosure and delivers it;
- behavior for no match, repeated export and several matching tenants.

**Recommendation, not a decision:** require one tenant and one exact approved
identifier per search; return stable references before export; stream a
versioned allowlisted JSON envelope without persisting another copy. Identity
verification and disclosure approval remain external human steps recorded in
the runbook.

### D12: Security events and incident evidence

Approve the privacy-relevant subset of the wider taxonomy being considered by
[issue #154](https://github.com/vaam-apps/vpay/issues/154), including required
actor, target, outcome, timestamp and correlation fields. Decide which
authentication failures, scope denials, client/key lifecycle events, provider
attempts, webhook attempts, holds, erasures and exports are durable evidence.

Also approve incident export version, maximum time range, subject and record
filters, roles, retention, and a measured collection-time objective for the
representative tabletop. A cross-tenant incident must require a distinct
privilege and workflow rather than broadening the default query. The objective
supports the organisation's Article 33 process; it does not decide whether an
incident is a breach or whether notification is required.

**Recommendation, not a decision:** use the ADR-0018 journal and #146 export
machinery; default to one tenant and a bounded time range. Keep
`webhook_deliveries` as current retry state and add an immutable row per
attempt rather than changing state history into an overloaded JSON field.

### D13: Deployment-owned retention and erasure

Name the accountable owner and verifiable obligation for:

- stdout/stderr log aggregation;
- traces and metric storage;
- PostgreSQL replicas;
- PITR/WAL and full backups;
- restored copies and post-restore re-erasure;
- any generated export files or object storage.

For each, approve retention, access, jurisdiction where applicable, deletion
or expiry mechanism, and evidence collected by the operator.

**Recommendation, not a decision:** state an eventual-erasure bound for copies
that cannot be changed in place and require retention enforcement to run after
a restore. Do not claim `vpay-server` can enforce a log or backup provider it
does not operate.

## Implementation plan

Implementation remains in the dependency order accepted by ADR-0018. The PR
boundaries below are targets, not a promise that unknown inventory findings
will fit without another small change.

### Stage 1: #144 inventory and gate

**PR 1: inventory and shared data-flow map**

- Add `docs/reference/personal-data-inventory.md` and a machine-readable
  companion consumed by tooling.
- Classify every migrated column and enumerate telemetry, event/webhook,
  browser, provider, merchant, observability and backup surfaces. Record
  purpose necessity, configured retention/deletion behavior and owner for
  every surface, including copies vpay does not store in Postgres.
- Give copies one stable data-element identity and a required necessity/control
  reference. Treat the necessity statement as reviewable evidence, not as a
  fact the gate can understand.
- Reuse the trust boundaries and stores for issue #150 rather than creating an
  ISO-only inventory.

**PR 2: two-directional drift prevention**

- Add `cargo xtask verify-privacy-inventory` under `.xtask/src/`.
- Wire it into `just verify`, CI, gate counts, `AGENTS.md` and status/gate
  documentation.
- Derive each non-database source set from a central registry or static code
  scan independent of the inventory; record any surface that cannot be
  enumerated as an unmet criterion rather than manufacturing a self-check.
- Prove a new migration column, stale row and registered non-database
  disclosure without a classification each fail. Add a field to a protected
  serializer or diagnostic type without its positive projection and prove it
  remains absent or the static guard fails. Do not claim that a parser can
  detect two semantically identical values assigned different inventory ids.

Primary areas: `backends/migrations/`, `schemas/vpay.cstack`, `.xtask/src/`,
`justfile`, `.github/workflows/ci.yml`, `docs/reference/`, `docs/flows/`,
`docs/status.md` and the relevant status gate documentation.

### Stage 2: #147 telemetry and webhooks

**PR 3: protected diagnostic representations**

- Add shared secret, masked, pseudonymous and safe-URL representations in
  `vpay-core` or the narrow owning crates.
- Replace unsafe derived `Debug` on registered secret- and personal-data types.
- Extend the privacy gate with two-directional exemptions where a mechanical
  rule cannot express intent.

**PR 4: telemetry and error-path cleanup**

- Audit shipping `tracing` calls, API error logging, startup errors, provider
  diagnostics, receiver excerpts, request IDs and metric labels.
- Implement D4's approved webhook URL validation, protected storage/rendering
  and compatibility rollout. A safe display helper alone is not enough if the
  full credential-bearing URL remains accepted and copied into persistence.
- Capture final sinks with unique secret, name, number, coordinate, URL and
  provider-message canaries; assert approved correlation survives and protected
  values do not.
- Update the operator-facing telemetry inventory and runbook with every
  personal-data representation still permitted, its retention owner and its
  deletion/escalation procedure.
- Create `docs/runbooks/privacy-telemetry.md` as that operator runbook and link
  it from `docs/runbooks/README.md`.

**PR 5: minimal event/webhook projections**

- After D2, introduce exact per-event payload projections.
- Update emitters, `EventObject`, webhook signing/retry tests, erasure rewriting
  and SDK contracts where the public wire changes.

Primary areas: `backends/crates/vpay-core/`,
`backends/crates/vpay-api/src/{lib.rs,error.rs}`,
`backends/crates/vpay-api/src/{staff,browser,v1}/`,
`backends/crates/vpay-provider/`, both adapter crates,
`backends/crates/vpay-worker/src/webhooks.rs`,
`backends/crates/vpay-db/src/{events,charges,refunds,webhook_deliveries}.rs`,
`backends/crates/vpay-config/`, `backends/tests/`, `sdks/`,
`docs/flows/webhooks/`, `docs/runbooks/privacy-telemetry.md`.

### Stage 3: #56 account-holder controls and audit foundation

**PR 6: route-specific merchant scope policy**

- Move scope requirements into `V1Route` metadata.
- Apply D5 to account-holder lookup without changing unrelated routes.
- Test every route has a policy and retain current read/write behavior
  elsewhere.

**PR 7: limiter vocabulary and append-only journal**

- Add immutable migrations for the approved rate scope and generic journal;
  update `MANIFEST.sha256` and `schemas/vpay.cstack`.
- Add repository traits and private implementations with append and
  tenant-scoped query operations only.
- Use generated CrateStack operations and explicit `@@allow` policies where
  they can express the operation. For every raw SQL exception, record the
  missing generated capability and test the policy/tenant behavior the raw
  query must preserve; a decorative model is not CrateStack adoption.
- Add typed configuration for D6 with no invented policy value.
- Require D9 to approve the journal's retention class before this PR. Add its
  bounded retention handler and the two-directional class-to-handler
  registration here, but do not schedule destructive enforcement until Stage
  4 installs D10's hold check in the same change. The interim is explicit
  over-retention under an approved class, never deletion of evidence before a
  hold can protect it.

**PR 8: wire account-holder controls**

- Enforce scope, durable limit, D7 audit sequence and `Retry-After` before the
  provider call can disclose a name.
- Implement the minimum D8 operator authentication, tenant grants and query
  surface in this PR; #56 is not complete with an append-only table that only
  ad-hoc SQL can read. Stage 5 extends this boundary for subject exports rather
  than creating it for the first time.
- Test found, absent, invalid, unsupported, provider failure, limited,
  persistence failure and cross-tenant query cases.

Primary areas: `backends/crates/vpay-api/src/v1/{mod.rs,account_holders.rs}`,
`backends/crates/vpay-api/src/{lib.rs,error.rs,resource_auth.rs}`,
`backends/crates/vpay-config/src/`,
`backends/crates/vpay-db/src/{rate_limits,repository}.rs`, new journal repository,
`backends/migrations/`, `schemas/vpay.cstack`, integration account-holder and
merchant-token tests, `docs/flows/account-holder-lookup.md`.

### Stage 4: #145 retention, erasure and holds

**PR 9: retention configuration and holds**

- Implement D9 as a closed validated class/action mapping.
- Resolve and remove or supersede the hard-coded 365-day customer rule.
- Add the D10 hold model, repositories and audit facts.
- Extend the D8 operator boundary with authorised hold placement, release and
  query operations; a hold table reachable only by ad-hoc SQL is not complete.

**PR 10: scheduled enforcement**

- Reuse bounded durable worker sweeps for approved transient,
  authentication, checkout, customer, payment, event, webhook and evidence
  classes.
- Add a two-directional registry from every applicable inventory retention
  class to one executable handler and schedule. Removing a handler or leaving
  a configured class with no executor must fail a gate/test.
- Generalise the existing customer erasure transaction rather than building a
  second redaction path.
- Document D13 backup, replica, restore and telemetry obligations.

Primary areas: `backends/crates/vpay-config/src/config.rs`,
`backends/crates/vpay-worker/src/{jobs,handlers,run_loop}.rs`,
`backends/crates/vpay-db/src/`, migrations and schema,
customer/worker/Postgres integration tests, `docs/flows/customers/`,
`docs/runbooks/restore-from-backup.md`, and a new erasure-verification runbook.

### Stage 5: #146 subject search and export

**PR 11: tenant-bound search and allowlisted export**

- Add a domain service and repository queries for D11 identifiers.
- Implement the D8 operator surface, explicit tenant grants and audit of
  initiation/completion/failure by extending the boundary delivered with #56.
- Export versioned allowlisted DTOs; never serialise database rows and subtract
  credentials afterward.
- Include the approved provenance and product-purpose metadata from the #144
  inventory and assert its exact keys and values in the export contract.
- Test found, absent, repeated, cross-tenant and secret-canary cases.

Primary areas: a new privacy service/repository,
`backends/crates/vpay-db/src/repository.rs`, the selected CLI or protected
router, integration tests, a subject-access runbook and backend/status
documentation.

### Stage 6: #148 incident evidence and export

**PR 12: security events, webhook attempts and incident tabletop**

- Add the D12 durable security events and immutable webhook-attempt history.
- Register every new evidence class with the retention and hold machinery from
  Stage 4, including its executable handler and schedule; Stage 6 must not make
  #145's closed coverage stale.
- Reuse #146's export envelope and output channel for tenant-, time-, subject-
  and record-bounded evidence. Test each bound across every included evidence
  family.
- Exercise a compromised credential from authentication through access,
  external delivery, disablement and export, with another tenant seeded as a
  leakage sentinel. Measure the complete collection time against D12's
  approved objective and record the result; a functionally correct export with
  no timing evidence does not meet the epic's incident-readiness criterion.
- Share the taxonomy and evidence mechanism with issue #154 without claiming
  that issue's monitoring and incident-management scope is complete.
- Require the incident-evidence runbook to map each product evidence source to
  the organisation's breach-response process while explicitly leaving breach
  classification, risk assessment and notification decisions to the
  responsible human role.

Primary areas: `backends/crates/vpay-api/` staff/resource authentication, the
journal repository, `backends/crates/vpay-db/src/webhook_deliveries.rs`,
`backends/crates/vpay-worker/src/webhooks.rs`, provider requests, the operator
surface, integration/tabletop tests and an incident-evidence runbook.

### Verification required for every stage

- Additive migrations only; update `backends/migrations/MANIFEST.sha256`.
- Real-Postgres tests for persistence, concurrency, tenant isolation and
  rollback behavior.
- One named mutation per control: remove a tenant predicate, audit append,
  limiter increment, hold check, allowlist field or time bound and observe the
  intended test fail.
- Update the relevant flow, runbook and status page only after the behavior is
  exercised.
- Run the repository's commands under its pinned toolchains:

```bash
just fmt
just ci
just docs-check-citations
```

The citation check is required when issue, pull-request or CI-run evidence is
added or edited. A skipped integration suite is not a pass.

## Alternatives considered

### Decide all policy in the implementation PRs

Rejected. Reviewers would see isolated values without the full effect on
telemetry, retention, export and incident evidence. It also makes different
stories likely to choose incompatible meanings for the same identifier or
operator.

### Put recommendations directly in ADR-0018

Rejected. An accepted architecture decision would make examples look approved.
Keeping this RFC mutable distinguishes technical advice from the accountable
human decision.

### Block the complete epic until every legal answer exists

Rejected. The inventory and much of its drift tooling can record unresolved
answers honestly, and that work is needed to present the right questions to
the policy owners. Only the implementation stage that depends on a choice is
blocked.

### Let ordinary merchant permissions imply operator access

Rejected by ADR-0018. It would collapse merchant and platform privileges.
Explicit operator grants on an existing identity model remain an option under
D8, alongside a new principal source, but neither may be inferred from normal
merchant access.

## Open questions

All rows D1-D13 are open. This RFC is accepted only when each row either:

- records an approved answer and owner; or
- explicitly remains an external dependency with a named owner, while the
  corresponding product behavior stays unbuilt and is not claimed complete.

## Impact on existing invariants

- **No feature is done without evidence.** This RFC and ADR-0018 change no
  capability. Status pages move only with implemented and exercised behavior.
- **Tenant isolation remains structural.** Operator workflows add explicit
  grants and keep tenant predicates in repository queries.
- **No test doubles enter shipping processes.** Rail scenarios continue to use
  configured HTTP stub hosts outside the process.
- **Migrations remain immutable.** New rate, journal, hold and attempt tables or
  constraints use new migration files and update the manifest.
- **Repositories remain traits with private implementations.** New persistence
  follows ADR-0016.
- **Errors remain typed and classified once.** Privacy control failures do not
  become stringly exceptions or success-shaped empty exports.
- **Money and crash-safety rules do not weaken.** Retention cannot detach or
  delete financial links merely to simplify erasure, and evidence writes share
  transactions where atomicity is possible.
