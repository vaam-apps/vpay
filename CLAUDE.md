# CLAUDE.md

Coding agents start with [CONTRIBUTING.md](CONTRIBUTING.md). Read
[AGENTS.md](AGENTS.md) before high-risk work: money, persistence, rails,
authentication, public API or wire types, UI-system changes, or dependencies.

Do not make the scaffold appear more complete than it is. Leave unimplemented
behavior explicit, ensure tests assert observable behavior, and report
verification honestly.

Run `just ci` before completing a change. Use the project recipe rather than a
reconstructed command, and state what you did not verify.
