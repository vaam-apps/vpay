# Configuration

Administration is YAML in git (ADR-0003). The dashboard cannot change any of it.

## There is no sandbox mode

Two statements that look contradictory and are not:

- **A sandbox *environment* — yes.** Two deployments: one talking to rail
  sandboxes and WireMock, one talking to real rails. Each has its own config
  file and its own database.
- **A sandbox *mode* — no.** No `if (sandbox)`, no code path that exists only
  outside production, no bean wired differently.

A profile selects a **configuration file**. It must never select a **code path**.
Same binary, same image digest, different YAML and different database.

Because Spring Boot is the idiom being borrowed, the trap it makes easy is worth
naming: `@Profile("!prod")`, `@ConditionalOnProperty` on business logic and
profile-specific bean overrides are all `if (sandbox)` wearing a
dependency-injection costume. Profiles may select *values*; never *beans that
behave differently*.

## Boot sequence

1. Load `application.yml`, overlay `application-{profile}.yml`.
2. Resolve `${}` placeholders. **An unresolved placeholder is fatal**, never an
   empty string — an empty subscription key otherwise fails much later and much
   more confusingly.
3. Validate (below).
4. Reconcile into the database in **one transaction**; record the config hash.
5. Only then bind the port.

**A validation failure exits non-zero without serving traffic.** A payment
gateway that boots half-configured is worse than one that does not boot.

## Rules that refuse to boot

| Rule | Why |
|---|---|
| Every merchant's rail host appears in that rail's allowlist | The host allowlist, checked before the FK |
| Every referenced provider exists and is enabled | A typo fails at boot, not at first payment |
| Currency exponent matches the canonical table | A 100× amount bug is otherwise silent |
| `livemode` ⇒ every host is `https://` | |
| `livemode` ⇒ no host labelled `wiremock`/`stub`/`mock`/`localhost` | **The most valuable rule here.** It is what makes "the code cannot tell a stub from a real rail" safe to live with |
| `livemode` ⇒ secrets come from `${}`, not literals | Stops a real key reaching git |
| `partial-refunds` ⇒ `refunds` | Mirrors the DB CHECK |

The last three plus the stub-host rule are implemented and tested in
`vpay-config`.

## Config changes and in-flight payments

**Safe to mutate:** credentials (rotation works on in-flight transactions),
prompt TTL, rate limits, webhook endpoints, capability flags.

**Identity-defining, refused while any non-terminal charge references the
config:** host, currency, payee identifier, or the merchant/provider pairing. A
charge submitted to host A must be *polled* at host A; silently repointing it
means recovery asks the wrong server and gets `NotFound` forever.

## Status

The guard rules are implemented and tested. **Figment layering and database
reconciliation are not started** — see [../STATUS.md](../STATUS.md).
