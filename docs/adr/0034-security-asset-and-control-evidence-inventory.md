# ADR-0034: Security assets, trust boundaries and ISO 27001 control evidence share one inventory architecture

- **Status:** Proposed
- **Date:** 2026-10-23
- **Deciders:** vpay maintainers (not yet accepted)
- **Related:** [issue #150](https://github.com/vaam-apps/vpay/issues/150),
  [issue #155](https://github.com/vaam-apps/vpay/issues/155),
  [ADR-0020](0020-privacy-controls-and-evidence.md)

## Context

The ISO/IEC 27001 engineering-readiness epic ([issue #155](https://github.com/vaam-apps/vpay/issues/155))
requires vpay to produce repeatable, auditable engineering evidence that the
organisation (VAAM) can reference from its Information Security Management System
(ISMS) risk assessment, risk treatment plans, and Statement of Applicability (SoA).

ISO/IEC 27001 certification applies to the organisation and its scoped management
system, not to a single repository. The repository's role is to provide a truthful,
code-grounded map of what exists, where trust boundaries lie, how secrets are handled,
which controls are operational, and which controls remain unbuilt.

Without a shared inventory architecture, security claims drift:

- documents describe services or tables that were renamed or deleted;
- new endpoints, provider rails, or background jobs land without security analysis;
- unbuilt capabilities are marked as "implemented" in prose to satisfy checklists;
- external auditors cannot verify claimed controls independently.

[ADR-0020](0020-privacy-controls-and-evidence.md) solved this for personal data
via a machine-readable specification ([`schemas/privacy-inventory.yaml`](../../schemas/privacy-inventory.yaml)),
a human-facing reference ([`docs/reference/personal-data-inventory.md`](../reference/personal-data-inventory.md)),
and an automated two-directional gate (`cargo xtask verify-privacy-inventory`).
Security assets, trust boundaries, and control evidence require the same rigor.

## Decision

### D1: Canonical Machine-Readable Companion

All security-relevant assets, trust boundaries, state stores, secret classes,
external integrations, CI/CD pipelines, and material controls are defined in
[`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml).
The human-readable view is published in
[`docs/reference/security-asset-inventory.md`](../reference/security-asset-inventory.md).

### D2: ISO/IEC 27001:2022 Control Mapping

Material controls implemented in the codebase are mapped to ISO/IEC 27001:2022
Annex A controls and themes (predominantly the A.8 Technological Controls and
A.5 Organizational Controls). Each control must cite:

1. its concrete implementation path (Rust module, configuration, or CI workflow);
2. a repeatable evidence source (unit test, conformance test, static analysis gate, or runbook).

### D3: Truthful Gap Representation

Unbuilt, partial, or unverified controls are never represented as implemented.
A control has one of three statuses: `implemented`, `partial`, or `gap`. Every
`gap` control names a gap, and every gap names exactly one tracker: an open
GitHub issue (e.g., [#103](https://github.com/vaam-apps/vpay/issues/103),
[#56](https://github.com/vaam-apps/vpay/issues/56),
[#151](https://github.com/vaam-apps/vpay/issues/151)–[#154](https://github.com/vaam-apps/vpay/issues/154)),
or a document such as [RFC-0014](../rfc/0014-verifying-rail-callbacks.md) when
the work is designed but no issue tracks it.

### D4: Drift Prevention via `cargo xtask verify-security-inventory`

Gate 16 of `just verify`, run by CI's `self-checks` job. It enforces
consistency between the inventory and the codebase:

- every deployable service path and named crate exists; every service names a
  declared trust tier;
- every registered database table exists in `backends/migrations/`, **and**
  every table a migration creates is named by a state store;
- every cited implementation path, evidence path, runbook, ADR, threat model,
  Dockerfile stage and workflow exists;
- every cited test name is declared by a Rust source under `backends/`;
- every secret `config_keys` entry is one of the closed set of `vpay-config`
  paths the gate knows;
- every published package manifest exists, names the package, and is not
  `private: true`;
- every control status is known, a `gap` names a gap, and every gap names
  exactly one tracker whose issue number is cited by a document under `docs/`.

The gate does not call GitHub and does not judge a control's semantic adequacy.
`cargo xtask verify-citations` checks issue and pull-request existence when the
network is available.

## Consequences

- An external ISMS can ingest `schemas/security-inventory.yaml` and reference real
  code evidence without manual transcription.
- Adding a new service, database table, external payment rail, secret class or
  published package requires updating the security inventory in the same pull
  request; `verify-security-inventory` fails otherwise.
- Auditors can reproduce claimed control evidence by executing specified tests and gates.
