# 2026-09-16 — the refund surface in both merchant SDKs (wave 3 / arm G)

Branch `refunds/w3-sdks`, from `refunds/w3-routes` (`5aaac9ca` — the four
routes, arm F). Arm F's own page is
[2026-09-16-w3-refund-routes.md](2026-09-16-w3-refund-routes.md).

## The headline, first

**No refund this surface can create has ever moved money, and this arm does
not change that.** `orange_money::refund` is a declared `NotImplemented`
token; `mtn_momo::refund` is MTN's Disbursements `transfer`, and MTN's
Disbursements product has still never been called from this repository, in
sandbox or anywhere else — the `202` the live run below observed came from a
`wiremock/wiremock` container answering a stub this repository wrote from
MTN's documentation. And **nothing settles a `pending` refund**: there is no
refund poll ladder (RFC-0003 open question 8), so every refund these SDKs
create stays `pending` until an operator moves it. Both SDKs' doc comments say
all three, and `live_refund_lifecycle` / "creates a refund with a registered
payee…" both assert `pending` after a successful create precisely so that a
future change claiming otherwise fails here.

## What this arm closed

The dated ⛔/⛔ row arm F left in `docs/sdks/parity.md`: neither
`CreateRefundParams` had a `destination`, both rails declare
`RefundDestination::Required`, and so `refunds.create()` from either SDK had
become **a `400` where it used to be a `404`** — an SDK user's call made worse
by a route being mounted. That row is now ✅/✅ and names four proving tests
per column.

Also landed: `refunds.update`, `refunds.list` and `refunds.cancel` in both
SDKs, a parity row each, a row for the two refund event types (which
`vpay_api::v1::refunds` became the first writer of the same day), and a row
for the payee's number being on no object, no event body and no `Debug`
output.

## The wire shape, and the rule that bites

    POST /v1/refunds
      payment_intent=pi_...
      amount=2000                          # omit for a full refund
      reason=requested_by_customer
      destination[mtn_momo][msisdn]=%2B237600000200
      metadata[order_id]=1234

The **rail's code is the outer key**. The server strips it and hands the
interior to that rail's own adapter, so an SDK that flattened the envelope or
wrote one rail's code whatever rail the charge was on would break the
boundary. Both SDKs build the envelope from the `payment_method_type` the
caller supplies, in one function each, and a test per SDK drives both rails
and asserts the key.

`RefundTarget::mobile_money` requires a leading `+` and refuses the bare
national form, because `vpay-provider` is not Cameroon-specific. **Neither SDK
canonicalises a payee's number**, deliberately and identically — the same
decision `docs/sdks/parity.md` records for `account_holders` — and a test per
SDK sends four spellings, including the one the server refuses, and asserts
the bytes that reach the wire.

## Gates run

`just ci` was **not** run — the arm's brief forbids it on this host.

| Gate                                                                          | Result                                                                                                                                                                                                                                                                          |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-sdk`                                               | **178 passed, 0 skipped, 0 ignored** — ten more than arm F's tree, counted from the diff (`#[test]`/`#[tokio::test]` added under `sdks/rust/tests/`)                                                                                                                            |
| `cargo clippy -p vpay-sdk --features live-stack --all-targets -- -D warnings` | clean                                                                                                                                                                                                                                                                           |
| `cargo fmt -p vpay-sdk -- --check`                                            | clean                                                                                                                                                                                                                                                                           |
| `cargo clippy -p vpay-server --all-targets -- -D warnings`                    | clean                                                                                                                                                                                                                                                                           |
| `cargo fmt -p vpay-server -- --check`                                         | clean                                                                                                                                                                                                                                                                           |
| `pnpm --filter @vaam-apps/vpay-sdk test`                                      | **222 passed, 0 skipped** — eleven more than arm F's tree, counted the same way (`it(` added under `sdks/nodejs/src/`)                                                                                                                                                          |
| `pnpm --filter @vaam-apps/vpay-sdk typecheck`                                 | clean                                                                                                                                                                                                                                                                           |
| `pnpm --filter @vaam-apps/vpay-sdk lint`                                      | clean                                                                                                                                                                                                                                                                           |
| `pnpm exec prettier --check 'src/**/*.ts'`                                    | clean                                                                                                                                                                                                                                                                           |
| `cargo xtask verify-sdk-parity`                                               | **ok — 600 proving tests, 35 dated gaps, 35 SDK methods across 39 rows**. Arm F's tree, measured on this host by checking `sdks/` and `docs/sdks/` out at `5aaac9ca` and running the same gate: **559 / 37 / 32 / 36**. The two gaps closed are the destination row's two cells |

`cargo test --doc -p vpay-sdk`: **8 passed, 0 failed, 1 ignored** — unmoved by
this arm, which added no doctest.

Not run: `just verify`, `just ci`, and every gate outside these two packages.
`cargo xtask verify-links` and `verify-status` were run for the documentation
above: **ok — 1703 links in 372 files** and **ok — 1 unimplemented item**.

## The live run — a real `vpay-server`, over a socket

This is the part `verify-sdk-parity` cannot do. It proves a **name** exists
and not that the SDK sends the field (issue #122), and a fixture once invented
a `client_secret` the server never sends while unit tests, review, mutation
testing and CI all passed.

Stack: `compose.yml` + `compose.e2e.yml` + `compose.demo.yml`, project
`vpay-w3sdks`, published on 18080, built from this branch's head. Postgres,
`wiremock-mtn`, `wiremock-orange`, `wiremock-webhook`, `vpay-server`,
`vpay-worker`.

```text
cargo nextest run -p vpay-sdk --features live-stack --test live_invoices --test live_refunds
  Summary [0.598s] 4 tests run: 4 passed, 0 skipped

pnpm --filter @vaam-apps/vpay-sdk test:live
  Test Files  2 passed (2)
       Tests  5 passed (5)
```

Both live suites **fail rather than skip** with no stack: the Rust binary is
behind the `live-stack` feature and panics naming the missing variable, and
the vitest project's `globalSetup` throws.

What the run observed that no stub could:

- the `destination[<rail>][msisdn]` envelope both SDKs write is one the real
  server strips, hands to the real `mtn_momo` adapter, and accepts;
- the server's own log line, from the process the suites drove:
  `instructing a rail to return money … rail=mtn_momo destination="+2376••••200"`
  — masked, four times, once per refund;
- `wiremock-mtn`'s request journal: **4 requests to
  `POST /disbursement/v1_0/transfer`, all answered `202`**, each carrying
  `payee: {"partyIdType": "MSISDN", "partyId": "237600000200"}` — the SDK sent
  `+237600000200`, the server canonicalised it, and the rail stub saw the
  digits-only `partyId` the adapter is documented to spell;
- the three refusals, each a real `400` naming `destination`: no payee at all,
  a payee with no `+`, and a payee the rail has no holder for. The third is
  the one that proves the interior really reached the adapter rather than
  being shape-checked by the core;
- none of the three refusals cost a row — asserted by listing the intent's
  refunds afterwards, because a refusal that reserved an amount would silently
  shorten the merchant's next full refund;
- a cancel really releases its reservation: the case cancels a 2 000 partial
  and then creates a full refund of the intent's whole 5 000.

## The bug the live run found

**The demo profile's MTN block carried no Disbursements credentials at all**,
so `POST /v1/refunds` on the e2e/demo stack was
`ProviderError::Config` — "credentials.disbursement_subscription_key is
required" — a `500` before the request ever reached the rail stub. `providers`
is a **list**, so `gen-demo-keys`' overlay _replaces_ `config/application.yml`'s
block outright and a key the base file carries and the overlay omits is simply
absent, with no merge to fall back on. Fixed in the `gen-demo-keys` heredoc
(three keys) and in `compose.e2e.yml`, whose `MTN_DISBURSEMENT_SUBSCRIPTION_KEY`
now spells the exact value `wiremock/mtn/mappings/transfer.json` matches on —
a change that file's own comment had deferred to "the day a refund walk
exists".

Nothing but a live run finds this. Every fixture test in both SDKs passed
throughout.

_(A second hazard, recorded because it cost a cycle: the `gen-demo-keys`
heredoc is **unquoted**, so a backtick in a comment is command substitution
and the word between a pair of them vanishes from the generated YAML. The
added comments carry none, and say so.)_

## The false claim this arm corrected

`backends/apps/vpay-server/src/main.rs`'s boot banner said **"Refunds are not
routed: nothing calls the `refunds` writer"**. It had been false since arm F
mounted the routes and was the one in-code claim of its kind that arm F's
"eleven in-code claims" commit did not move. It was not reasoned about: a real
boot of this binary printed it into the log **while the live suite was
creating, updating, listing and cancelling refunds through the very process
that printed it**. The replacement names the five routes and keeps every true
half — no refund has ever moved money, and nothing settles a pending one.

## One pre-existing flake, named rather than left to be rediscovered

`a_refused_connection_to_the_token_endpoint_is_a_transport_error`
(`sdks/rust/tests/errors.rs`) failed once during this arm's final gate run and
passed five consecutive re-runs immediately after. It is **not this arm's**:
nothing here touches that file, and the test's own comment says why it is
racy — it binds an ephemeral port, reads its number, drops the listener, and
then expects the connection to that port to be refused. On a host running
several Docker stacks and several agents, something else can claim the port
inside that window and the SDK gets a connection instead of a refusal. This
page records it because `docs/status/merchant-sdks.md` already carries a
similar note about `a_second_concurrent_401_does_not_discard_the_token_the_first_one_just_fetched`
on the same host, and two independent flakes in one binary are worth one line
somebody can find.

## Mutation testing

Each mutation was applied to shipping source, the suite run, and the source
restored.

| Mutation                                                    | Caught by                                                                                        |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Node: hardcode `mtn_momo` as the destination's outer key    | `refunds.create scopes the destination to the rail it was given, not a constant`                 |
| Node: strip a leading `+237` from the payee's number        | three cases, including `refunds.create never normalises a payee's number on its way to the wire` |
| Rust: hardcode `"mtn_momo"` in `RefundDestination::to_form` | `the_destinations_outer_key_is_the_rail_the_refund_is_on`                                        |
| Rust: print the payee in `CreateRefundParams`' `Debug`      | `a_create_refund_params_debug_output_never_contains_the_payees_number`                           |
| `docs/sdks/parity.md`: rename one proving test              | `verify-sdk-parity`, naming the row and the file it looked under                                 |

## What this arm did NOT do

- **No `just ci`**, ~~no `cargo test --doc`~~, no `just verify`. CI is the
  gate. (Corrected on review, 2026-09-16: `cargo test --doc -p vpay-sdk` **was**
  run — its numbers are in the table above — so this line contradicted that
  table. Only the workspace-wide `just test-doc` was not run.)
- **No Flutter plugin.** `sdks/flutter` has no refund surface and this arm did
  not add one; it is not in `docs/sdks/parity.md`'s columns either, so no
  dated gap was owed. Stated here rather than left to be discovered.
- **No `stripe-compat` case.** A Stripe SDK's `refunds.create` has no
  `destination` parameter at all, so what a compat case would assert is a
  question about the compat surface and not about this one.
- **No webhook delivery case.** That the two refund events are _delivered_
  (signed, retried, stored) is `vpay-worker`'s subject and is not observed
  here; what is observed is that both SDKs know the types and decode their
  payload.
- **No claim that any rail refunded anybody.** See the headline.

## Adversarial review, 2026-09-16 — what was re-measured, and what moved

Branch `review/w3g-sdks`, from `refunds/w3-sdks` (`ea21b658`). Every number
below was measured on this host, on that head, and not taken from the sections
above. `just ci` was not run here either.

### The live proof is live

Both suites were run first **with no stack and no environment at all**, which
is the claim that mattered most, and both **fail**:

| Suite, with nothing running                                                                    | Result                                                                                                                   |
| ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `cargo nextest run -p vpay-sdk --features live-stack --test live_invoices --test live_refunds` | **4 tests run: 0 passed, 4 failed, 0 skipped**, exit 100. Each panic names `VPAY_BASE_URL`                               |
| `pnpm --filter @vaam-apps/vpay-sdk test:live`                                                  | exit 1 — the `globalSetup` throws out of `readLiveEnv` before collection. No case is reported as passed, skipped or todo |

Then a stack of this branch's head was built and run — project
`vpay-w3greview`, published on **19080/19082/19083** so the `vpay-demo` stack
already on 8080 was never touched — and both suites were run against it
**twice**: once through `just sdk-live`, and once more directly after
confirming `git status --porcelain -- sdks` was empty, so that no mutation from
the review's own mutation testing could be in the binary.

| Suite, against `vpay-w3greview`   | Run 1                                | Run 2 (clean tree)                   |
| --------------------------------- | ------------------------------------ | ------------------------------------ |
| `live_invoices` + `live_refunds`  | **4 tests run: 4 passed, 0 skipped** | **4 tests run: 4 passed, 0 skipped** |
| `vitest -c vitest.live.config.ts` | **2 files, 5 tests, 5 passed**       | **2 files, 5 tests, 5 passed**       |

### Read off the running stack rather than reported

- `wiremock-mtn`'s journal, fetched from inside the compose network: **8
  `POST /disbursement/v1_0/transfer`, every one answered `202`** — four per
  live run, which is the number this page claims — and every one carrying
  `payee: {"partyId": "237600000200", "partyIdType": "MSISDN"}`. **No `+`
  reached the rail**, and no other payee spelling appears.
- `/collection/v1_0/accountholder/msisdn/237600000404/basicuserinfo` was called
  **4 times** (twice per run), which is the half that proves the envelope's
  interior really reached `mtn_momo`'s adapter rather than being shape-checked
  by the core.
- `vpay-server`'s own log: **8 `instructing a rail to return money` lines**,
  each `destination="+2376••••200"`, and `docker logs … | grep -c 600000200`
  is **0** — the payee's digits appear nowhere in the log at all.
- The boot banner this binary printed is the corrected one, naming all five
  refund routes. The claim it replaced really was false: `v1::mod`'s route
  table mounts `/refunds`, `/refunds/{id}` and `/refunds/{id}/cancel` as of arm
  F, and arm F's own diff **edited that same banner** and left "Refunds are not
  routed" standing in it.

### Counts reproduced, including the parity baseline

| Gate                                                                  | Re-measured                                                     |
| --------------------------------------------------------------------- | --------------------------------------------------------------- |
| `cargo nextest run -p vpay-sdk`                                       | **178 passed, 0 skipped**                                       |
| `pnpm --filter @vaam-apps/vpay-sdk test`                              | **222 passed, 9 files**                                         |
| `cargo test --doc -p vpay-sdk`                                        | **8 passed, 0 failed, 1 ignored**                               |
| `cargo xtask verify-sdk-parity`, arm G's head                         | **600 / 35 / 35 / 39** — exactly as claimed                     |
| the same gate with `sdks/` and `docs/sdks/` checked out at `5aaac9ca` | **559 / 37 / 32 / 36** — the baseline was measured, not guessed |

All four mutations this page lists were applied again, independently, and all
four were caught by the test it names: the Rust outer-key constant fails
`the_destinations_outer_key_is_the_rail_the_refund_is_on` (1 of 75 in
`--test resources`), the Rust `Debug` leak fails
`a_create_refund_params_debug_output_never_contains_the_payees_number` (1 of 8
in `--test debug_redaction`), the Node outer-key constant fails 1 of 222, and
the Node `+237`-stripper fails 3 of 222.

### What the review changed

1. **A parity row that over-claimed in one column.** "The payee's number is on
   no refund object, no event body and no `Debug`/`inspect` output" was ✅/✅,
   and the `inspect` half is **not true of `sdks/nodejs`**: a
   `CreateRefundParams` there is a plain object literal the merchant builds, so
   `util.inspect` prints `msisdn: '+237600000200'` in full — measured, not
   reasoned about — and neither Node proving test on that row checked it (both
   are about the delivered event body). This is issue #122's trap one level up:
   the gate proved the two Node test names exist, which they do, while the row
   title said something else. The row is now split — "on no refund object and
   no event body" stays ✅/✅, and a second row for the `Debug`/`inspect`
   guarantee is ✅ for Rust and a **dated ⛔ for Node**. `verify-sdk-parity`
   now reports **603 proving tests, 36 dated gaps, 35 SDK methods across 39
   rows**.
2. **One clause of the honesty banner, again.** The corrected banner still said
   "no deployment holds the credential", and this arm had given the e2e/demo
   stack a `disbursement_subscription_key` hours earlier — so the binary
   printed that sentence on a stack whose config held one, which is the same
   shape of false in-code claim the arm had just fixed. Narrowed in
   `main.rs` and in the matching sentence in both SDKs' doc comments: **no real
   MTN credential exists in this project**, the only one anywhere is the stub
   the e2e/demo stack points at WireMock, and the product has never been
   called. `docs/status.md`'s addendum already said exactly this; the three
   in-code copies did not.
3. **A contradiction in this page**, above: the gate table reported
   `cargo test --doc -p vpay-sdk` and "What this arm did NOT do" said it was
   not run.

Not changed, and reported rather than fixed: the same "no deployment holds"
sentence appears in `vpay-db`'s doc comments, `repositories.rs` and **two
migration comments**. Migration text is byte-checksummed
(`just verify-migrations`), so 0047 cannot be edited; that is itself the
argument for reading "deployment" as a real one, and for the narrower wording
used above rather than a repo-wide rewrite. Owner: the maintainer.

### The flake

`a_refused_connection_to_the_token_endpoint_is_a_transport_error` is
**pre-existing and not this arm's**, confirmed two ways: `git log` on
`sdks/rust/tests/errors.rs` shows its last commit is `a15df87f`, the original
SDK commit, and `git diff refunds/w3-routes..refunds/w3-sdks -- sdks/rust/tests/errors.rs`
is empty. `closed_port()`'s own comment says "Racy in principle (something else
could claim it)" — it binds `127.0.0.1:0`, reads the port and drops the
listener. Its stated reason for tolerating that ("nothing else in this test
binary binds ports") is what is no longer safe on a host running several Docker
stacks. It did not fail in any run during this review.
