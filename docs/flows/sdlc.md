# Secure Development Life Cycle

The written form of the repository's engineering workflow as an ISO/IEC
27001:2022 control. It answers, for an ISMS reader and for a contributor, the
one question the control names: **what is the required path from a change to
production, and what evidence does each step leave behind?**

This is a flow, not an ADR. Nothing here records a new decision; it writes
down, as a policy an auditor can reference, the change-to-production practice
the repository already enforces in `just ci`, `just verify`, the CI workflows
and the security inventory. The machine-readable view of the same control is
`schemas/security-inventory.yaml`'s `A.8.25` row, and the human asset-and-control
view is
[`docs/reference/security-asset-inventory.md`](../reference/security-asset-inventory.md).

Scope: this is the repository's contribution to the organisation's ISMS. It
does not, and cannot, replace the organisational risk treatment or the
Statement of Applicability — certification applies to the organisation and its
scoped management system, not to this repository (ADR-0034). What this page
provides is the code-grounded, repeatable evidence the organisation's ISMS
references.

## What a change must pass, in order

`just ci` is the recipe a contributor runs before opening a pull request, and
the `ci.yml` workflow runs the same set on every pull request. In order:

| Step (recipe)                       | What it refuses                                                                                     | Evidence of it passing |
| ----------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------- |
| `fmt-check`                         | rustfmt drift                                                                                       | green `just ci`        |
| `clippy`                            | a warning, a denied lint, or `unsafe`/`unwrap`/`expect`/`panic`/`todo`                              | green `just ci`        |
| `verify`                            | the seventeen invariant gates (below)                                                               | green `just ci`        |
| `test-rust`                         | a failing Rust test (`cargo nextest run --workspace`)                                               | green `just ci`        |
| `test-doc`                          | a failing doctest (`cargo test --doc --workspace`)                                                  | green `just ci`        |
| `verify-ignored`                    | a test skipped without a reason                                                                     | green `just ci`        |
| `lint-web`                          | a TypeScript/Prettier lint failure                                                                  | green `just ci`        |
| `test-web`                          | a failing Vitest case, including the jsdom axe suites and the Storybook a11y gate lock              | green `just ci`        |
| `audit-web`                         | a `pnpm audit` finding at `moderate`, `high` or `critical` in the workspace or its production graph | green `just ci`        |
| `deny`                              | a `cargo deny check` finding (bans, licenses, sources, advisories)                                  | green `just ci`        |

Beyond `just ci`, CI runs `helm-check` and `test-alert-rules` on the `deploy`
job, `test-storybook` on the `web` job, and the supply-chain scans (CodeQL,
super-linter, Trivy) on the `quality` job. The gates that need a network or a
binary this workspace does not build are run by CI even though they are not in
`just ci`, for the reasons each recipe's own header states.

### The `verify` gates

`just verify` is seventeen gates and one advisory report, all listed and dated
in [docs/status.md](../status.md)'s gate table. The ones a security reader
should know by name are the security and supply-chain invariants:

- `verify-no-mocks` — no test double reachable from a shipping binary;
- `verify-serde` — every serialisable type spells the wire convention;
- `verify-repositories` — repository implementations are reached only through
  `vpay-db`'s traits;
- `verify-toolchain` — the Dockerfile compiler matches the toolchain pin;
- `verify-migrations` — an applied migration's bytes have not changed;
- `verify-versions` — every release-please-owned version agrees;
- `verify-privacy-inventory` — every migrated column is classified, and every
  redacting type stays undebuggable;
- `verify-security-inventory` — the security inventory (assets, trust
  boundaries, secrets, controls, gaps) is consistent with the code in both
  directions;
- `verify-mcp-parity` — every `/v1` route has an MCP tool or a dated gap.

## Security-relevant changes: the review and threat-assessment trigger

A change is **security-relevant** when it touches a trust boundary, a secret, a
credential, an external integration, an authentication or authorization
decision, a new inbound or outbound route, a data store of personal data, or
the dependency/supply-chain posture. In concrete terms for this repository,
that is any change that:

- adds, renames or moves a surface listed under a service in
  `schemas/security-inventory.yaml` (a new `/v1`, `/dash/v1`, `/svc/v1`,
  `/v1/browser` or `/provider/{code}/callback` route);
- adds a secret class, a credential, a rail or an external integration;
- changes how a request is authenticated or authorized (envelope, scope,
  console key, OAuth, webhook signature);
- changes a trust tier, a NetworkPolicy, an egress rule or the SSRF guard;
- adds or ignores an advisory (`deny.toml` `[advisories]`, `pnpm-workspace.yaml`
  `auditConfig`), or changes a dependency floor;
- changes the ledger, the settlement path, or anything that moves money.

When a change is security-relevant:

1. **A threat model precedes the code.** The
   [security README](../security/README.md) requires a threat model, in
   `docs/security/threat-models/`, before a new external interface, a new
   store of secrets or personal data, or a new inbound route is opened. A
   threat model that lands with its code is late.
2. **The security inventory moves with the change.** Adding a service, table,
   secret class, rail or control without updating
   `schemas/security-inventory.yaml` fails `verify-security-inventory` in the
   same pull request.
3. **The change gets review from a reviewer who is asked to assess the
   threat.** PR review is not waived for a security-relevant change; the
   reviewer checks the change against the trust boundary it sits on and the
   threat model that precedes it.

## Dependency and advisory scanning: the required gate and its threshold

Advisory scanning is a required CI gate at an approved severity threshold, on
both dependency graphs:

- **Web:** `just audit-web` runs `pnpm audit --audit-level=moderate` over the
  whole workspace and over the production graph only. Moderate, high and
  critical findings fail the build. It is in `just ci` since 2026-09-11 and in
  CI's `web` job. GAP-103's original complaint — that it ran at `high`, was
  not in `just ci`, and stayed green over a moderate advisory for two days —
  is resolved: the level is `moderate`, it is in `just ci` and in CI.
- **Rust:** `cargo deny check` runs in CI's supply-chain job and locally via
  `just deny`, refusing banned, unlicensed, unvetted-source and advisory
  findings.

### The exception and acceptance path

A finding that cannot be fixed is **risk-accepted, not ignored** — through a
dated, reasoned, removal-conditioned allowlist entry that a reviewer reads in
the same way they read any other security decision:

- a web advisory is accepted by id under `auditConfig.ignoreGhsas` in
  `pnpm-workspace.yaml`, each with a dated decision, the reason, the scope of
  exposure and a `REMOVE WHEN` condition;
- a Rust advisory is accepted by id under `[advisories] ignore` in
  `deny.toml`, each with a dated decision and the reasoning.

The current state of this exception path is recorded in the `A.8.7` control and
the `GAP-103` gap in `schemas/security-inventory.yaml`. On the **web** graph
one advisory is accepted today: `GHSA-vfj7-8cjw-p6xm` (braces, high, stack
exhaustion on deeply nested patterns), ignored by id under the maintainer
decision of 2026-10-09 because no fixed release exists and it is reached only
through a lint-time glob matcher. On the **Rust** graph `deny.toml` accepts
three advisories (`RUSTSEC-2023-0071`, `RUSTSEC-2024-0449`, `RUSTSEC-2026-0387`),
each with a dated decision and reasoning. Every other advisory — including any
new one against the same packages — still fails the gate.

## Vulnerability remediation: severity, owner, target, and record

A vulnerability finding gets, at intake:

- **Severity** — the advisory's own CVSS/label (pnpm and cargo-deny carry it);
- **Owner** — the maintainer who merges the fix is the accountable owner;
- **Remediation target** — the SLA below;
- **Exception or acceptance path** — if it cannot be fixed in target, the
  dated, reasoned allowlist entry described above.

Remediation targets, as policy:

| Severity   | Fix or risk-accept within |
| ---------- | ------------------------- |
| Critical   | 7 days                    |
| High       | 30 days                   |
| Moderate   | 90 days                   |
| Low        | next release              |

The target is the engineering commitment. A finding that reaches its target
without a fix and without a dated allowlist entry is a CI failure waiting to
happen, because `audit-web` and `deny` will not stop failing while it is open.

## CI evidence: retained and reproducibly queryable

- **Retained:** every CI run is a GitHub Actions workflow run, retained by the
  repository, with the failing step named and the gate's output attached.
- **Reproducible:** `just ci` and `just verify` run the same gates locally, so
  any claimed evidence can be re-derived from a clean checkout without the
  network (the network-only steps — `audit-web`, `deny`'s advisory fetch, the
  supply-chain scans — are named as such in their recipes).
- **Release-attached:** the release workflow builds, signs and publishes from
  `master`, and the gates above must be green before a merge, so a release's
  provenance is the merged history plus the signed artifact.

## Security test coverage

Security tests cover the authentication and authorization boundaries and
representative abuse and failure paths:

- the **conformance suite** (`backends/tests/conformance`) is one suite
  parameterised over every rail and proves the wire contract against real
  WireMock hosts, including redirect-refusal, refund-to-payee and
  callback-as-hint behaviour;
- the **integration suite** (`backends/tests/integration`) drives the real
  router, the real database and the real authorization servers, covering
  tenancy, scopes, both audience directions, replay caches, idempotency,
  cross-tenant isolation, and the worker's SSRF guard (`vpay_worker::ssrf`);
- the **unit suites** in `backends/crates/*` pin the fine-grained invariants
  (envelope verification, nonce replay, signing-key lifecycle, config
  rejection of literal secrets in livemode).

The named tests each control cites are enumerated in
`schemas/security-inventory.yaml`'s `material_controls` and checked to exist by
`verify-security-inventory`.

## Emergency changes

A change that must reach production faster than the normal path follows
[docs/flows/emergency-changes.md](emergency-changes.md), which carries the
auditable record and the retrospective review requirement. The normal path
above is never silently skipped; an emergency change is a documented exception,
not an unwritten one.

## Status

**This page is the policy; the enforcement it describes is built and gated.**

- The `just ci` / `just verify` path, `audit-web` at `moderate`, `deny`, the
  security inventory and the threat-model-before-code rule all exist and are
  machine-checked. See [docs/status.md](../status.md) for what each gate
  printed.
- What is **not** built, and is out of this repository's scope: the
  organisational ISMS risk treatment, Statement of Applicability, and the
  cross-release CVE history ledger. Those are organisation-level records that
  an ISMS reader maintains outside this tree; this page gives them the
  repository's evidence to cite.
- The remediation SLA above is policy written down here; nothing machine
  enforces the calendar target, because the gates already fail while a finding
  is open — the target is what makes that failure mean something.
