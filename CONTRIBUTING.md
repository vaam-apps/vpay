# Contributing to vpay

## Safe first change

Start with a small, scoped change: a documentation correction, test improvement,
isolated UI change, or bug with an existing failing test. Read
[docs/status.md](docs/status.md), the code you will change, and its adjacent
tests.

Before changing rails, authentication, payment state transitions, migrations,
schema changes, or public wire types, read [AGENTS.md](AGENTS.md).

## Rules that always apply

- Do not make unfinished behavior look successful. Keep
  `ProviderError::NotImplemented(...)` and its matching `docs/status.md`
  declaration until behavior is proven.
- Do not add mocks, fakes, or stubs reachable from `vpay-server`. Rail tests use
  WireMock over HTTP.
- Do not edit an existing migration. Add a new one.
- Do not use floating-point money, environment-specific code paths, or
  provider-code branching outside adapters.
- Add or update a test that fails when the behavior regresses.

## Verify

Run before review:

```bash
just fmt
just ci
```

Also run `just test-storybook` for checkout, story, or theme changes; `just
test-e2e` for browser-flow changes; `just helm-check` for Helm changes; and
`just docs-check-citations` when adding or changing a GitHub PR, issue, or CI
run citation.

## Documentation

Update documentation only when a public contract, operator procedure, or
cross-component invariant changes. Keep ordinary implementation detail in code
and tests. Update `docs/status.md` only when its current-state claim or its
machine-checked `NotImplemented` declaration changes.

## Go deeper

Read [AGENTS.md](AGENTS.md) before changing money, persistence, rails,
authentication, public API or wire types, the UI system, or dependencies.
