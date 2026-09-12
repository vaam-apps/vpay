# Verification log — 2026-09-12, the customer write path's `Debug`, and issue #113's two open questions

Last verified: 2026-09-12, on `claude/issue-113`, for
[issue #113](https://github.com/vaam-apps/vpay/issues/113).

## What this claims

It claims one thing about behaviour: `vpay_db::CustomerAddress`, `NewCustomer`
and `CustomerPatch` no longer print a payer's name, email, phone, street or GPS
point through `Debug`, and a test fails if any of them starts again.

Everything else on this branch is documentation, and the rest of issue #113 is
**deliberately unbuilt** — see
[../../plans/issue-113-notes/decision.md](../../plans/issue-113-notes/decision.md).

## What it does **not** claim

- **`just ci` was not run**, and neither was `just verify`, `just test-rust` or
  any workspace-wide build. Other agents were building on this host and
  concurrent `cargo` builds have OOM-killed it before, so this branch was capped
  at crate-scoped commands on purpose. **Nothing here predicts the twelve
  gates.** The two the definition of done named were run, and they are below.
- **No capability moved.** No route, no adapter, no column, no migration, no
  wire field. `docs/status.md`'s banner, gate table and `NotImplemented`
  declaration are untouched, which is why this branch adds no row to that page.
- **Nothing was measured about conversion, consent or regulatory exposure.**
  The decision write-up reasons about all three and says so where it has no
  number; in particular vpay has never taken a real payment, so **no figure for
  what a permission prompt costs at checkout exists and none was invented**.
- The coordinate's behaviour in the **retention sweep** is still asserted only
  transitively. See "the thin spot" below.

## The defect, and why it counts as one

`CustomerRow` has had a hand-written redacting `Debug` since 2026-09-10, and
`vpay_api::model::{CustomerObject, AddressObject}` gained one on 2026-09-11 out
of [issue #70](https://github.com/vaam-apps/vpay/issues/70). The write path had
neither: `CustomerAddress`, `NewCustomer` and `CustomerPatch` still
**derived** `Debug`, so a single `{:?}` printed the payer in full.

Those are the values `insert_in_tx` and `update_in_tx` hold in the frame that
runs the statement — the frame an `anyhow` chain or a `tracing` event carries
when a CHECK fires or a pool times out. And `CustomerRow`'s own guard was
narrower than it looked: it counted the address components itself over
`self.address.*` rather than delegating, so it protected the row while
everything else holding the same `CustomerAddress` printed it whole.

`#[derive(Debug)]` was removed from all three; the component count now lives in
exactly one place, `CustomerAddress`'s own impl, which `CustomerRow` delegates
to. `CustomerPatch` prints its three states as `absent` / `cleared` /
`[N chars redacted]`, so the one question an operator brings to an update is
still answerable with no personal data in the answer.

## Gates run

| Command                                                        | Result                                              |
| -------------------------------------------------------------- | --------------------------------------------------- |
| `cargo xtask verify-links`                                     | **exit 0** — see the run below                      |
| `just fmt-check-web` (`pnpm exec prettier --check .`)          | **exit 0**, on Node 22.23.2 (`.nvmrc`), pnpm 9.15.0 |
| `cargo clippy -p vpay-db --all-targets -- -D warnings`         | **exit 0**, no warnings                             |
| `cargo nextest run -p vpay-db` (real Postgres, testcontainers) | **188 tests run, 188 passed, 0 skipped**, 338 s     |

`just fmt-check-web` failed first, on `decision.md` and `backend.md`, and was
fixed with `prettier --write` rather than by editing around it.

## The test, and the five mutations that show it is load-bearing

`no_customer_type_ever_prints_a_payers_identifiers_street_or_gps_point` in
`backends/crates/vpay-db/src/customers.rs` builds one fixture — a payer with a
name, an email, a canonical MSISDN, a street, a city, a country and the Douala
point (`4_061_000`, `9_786_000`) — renders all **four** types from it, and
asserts in both directions: that none of seven literals appears, the
coordinate's digits included, and that each rendering still reports four
address components, so an impl printing nothing fails too.

Each mutation was applied to this branch's own head, run, and restored.

| #   | Mutation                                                                  | The assertion that caught it                                                                                                                     |
| --- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| M1  | `CustomerAddress`'s `Debug` prints the pair it just counted               | "`4061000` is in `CustomerAddress { redacted: {4 component(s) redacted}, latitude_microdeg: Some(4061000), longitude_microdeg: Some(9786000) }`" |
| M2  | `NewCustomer` prints `self.phone` instead of its length                   | "`237600000200` is in `NewCustomer { … phone: Some(\"237600000200\") … }`"                                                                       |
| M3  | `CustomerPatch`'s three-state renderer echoes the value                   | "`Ada Ngo Bikai` is in `CustomerPatch { name: Ada Ngo Bikai, email: cleared, phone: absent … }`"                                                 |
| M4  | `CustomerRow` stops delegating and reaches into `self.address.line1`      | "`Rue Njo-Njo, Bonapriso` is in `CustomerRow { … address: Some(\"Rue Njo-Njo, Bonapriso\") … }`"                                                 |
| M5  | `CustomerAddress`'s `Debug` prints **nothing at all** (the positive half) | "`CustomerAddress` has to say an address is present and how much of one, without saying what it is: `CustomerAddress`"                           |

M5 is the one worth keeping: it passes every substring search, which is exactly
how a redaction test stops being a test.

The sixth direction needs no test. Restoring `#[derive(Debug)]` on any of the
four is `E0119` — a `cargo check` failure, not a green suite — which is the
same guard issue #70 recorded for `CustomerObject` and `AddressObject`.

## The thin spot, named and not closed

`the_sweep_deletes_an_idle_unreferenced_customer_and_anonymises_a_referenced_one`
in `backends/tests/integration/tests/customers.rs` creates its three fixtures
with a `name` and **no address**, so **no test drives a coordinate through
`erase_idle`**, the twelve-month retention sweep. The property almost certainly
holds — the sweep runs the same `anonymize` statement as `DELETE`, which sets
both columns `NULL`, and `anonymized_customers_carry_the_marker` would raise a
`23514` and roll the sweep's transaction back if it did not — so this is a thin
fixture rather than an unproven property, and it is recorded here rather than
quietly fixed.

It was not closed on this branch because it is a container-backed case in the
integration binary, whose build is exactly what this branch was capped from
running. **Adding a test that was never executed would be worse than naming
one.**

Smaller, in the same family: nothing asserts the two columns' Postgres **type**
directly the way `postgres_smoke.rs` does for `currencies.exponent`. A change to
`DOUBLE PRECISION` would surface only as a `type differs` line inside
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`.
