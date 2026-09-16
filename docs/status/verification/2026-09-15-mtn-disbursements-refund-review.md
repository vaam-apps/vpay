# 2026-09-15 — adversarial review of the MTN Disbursements refund (wave 2 / arm D)

Branch `review/w2d-mtn`, from `refunds/w2-mtn` (`197d5df4`), diffed against
the merge base `4caf7648`. The arm's own page is
[2026-09-15-mtn-disbursements-refund.md](2026-09-15-mtn-disbursements-refund.md)
and the corrections below are also written into it, in place, so a reader of
that page is not told the wrong thing and then sent here.

## The headline, first

**`mtn_momo::refund` is written and MTN's Disbursements product has still
never been called from this repository** — not in production, not against the
sandbox, not once — and **no deployment holds a Disbursements subscription
key**. `cargo xtask verify-status` printing `0 unimplemented item(s)` is a
fact about a token and nothing else. Everything below assumes a reader who
might mistake an empty list for a finished system.

> **Merge note, added when this arm was merged into the refunds branch and
> _not_ a rewrite of what was measured.** Every number on this page is a
> measurement of arm D's own branch and is left exactly as it was recorded.
> Two of them are no longer the merged tree's: `verify-status` prints
> **1 unimplemented item**, not 0, because `orange_money::refund` became a
> token on the refunds branch the same day (RFC-0003 § 5); and the migration
> this page calls `0046_refunds-comments-mtn-refund-is-written.sql` is
> **`0047`** in the merged tree, because `0046_ledger-id-length.sql` had
> already taken that number — it had never been applied anywhere, so it was
> renumbered and its `MANIFEST.sha256` line rehashed. `verify-migrations`
> reports **47 files**. The merged tree's own gate run is in
> [2026-09-16-w2d-merge.md](2026-09-16-w2d-merge.md).

The arm's own honesty prose is good and this review found no sentence in
`docs/status.md`, `docs/flows/adapter-mtn-momo.md` or the arm's verification
page that could be read as "MTN refunds work". What it found is a different
failure: **the arm reported correcting "nine places" and there were twenty**,
and three of its evidence claims name mechanisms or tests that do not do what
is claimed.

## What was run

`just ci` was not run (the arm's brief forbids it; this host has been
OOM-killed by concurrent builds). `DOCKER_HOST=unix:///run/user/1000/docker.sock`
was set for every container-backed run. Nothing skipped anywhere.

| Command                                                                                 | On arrival                  | After this review                   |
| --------------------------------------------------------------------------------------- | --------------------------- | ----------------------------------- |
| `cargo nextest run -p vpay-adapter-mtn-momo`                                            | 87 passed, 0 skipped        | **88 passed, 0 skipped**            |
| `cargo nextest run -p vpay-tests-conformance` (real containers)                         | 67 passed, 0 skipped        | **67 passed, 0 skipped**            |
| `cargo nextest run -p vpay-server`                                                      | **47 passed, 1 FAILED**     | **48 passed, 0 skipped**            |
| `cargo test --doc -p vpay-provider -p vpay-adapter-mtn-momo`                            | 12 + 1 passed               | **12 + 1 passed**                   |
| `cargo xtask verify-status`                                                             | `0 unimplemented item(s)`   | `0 unimplemented item(s)`           |
| `cargo xtask verify-migrations`                                                         | 45 files match the manifest | **46 files match**                  |
| `cargo nextest run -p vpay-provider -p vpay-config`                                     | not re-measured             | **147 passed, 0 skipped**           |
| `cargo nextest run -p vpay-db`                                                          | not re-measured             | **234 passed, 0 skipped**           |
| `cargo nextest run -p vpay-api`                                                         | not re-measured             | **366 passed, 0 skipped**           |
| `cargo nextest run -p vpay-tests-integration -E binary(refunds)`                        | not re-measured             | **5 passed, 0 skipped**             |
| `cargo test --doc` (mtn / api / db / provider)                                          | not re-measured             | **1 / 18 / 8 / 12 passed**          |
| `cargo clippy` over all seven touched crates, `-D warnings`                             | —                           | clean                               |
| `cargo +nightly fmt --all --check`                                                      | clean                       | clean                               |
| `prettier@3.9.6 --check` over every changed `.md`/`.json`/`.yml`                        | —                           | clean                               |
| `cargo xtask verify-links` / `verify-no-mocks` / `verify-serde` / `verify-repositories` | —                           | ok                                  |
| **`cargo xtask verify-errors`**                                                         | **FAILS**                   | **FAILS — pre-existing, see below** |

Without `DOCKER_HOST` set, `cargo nextest run -p vpay-server` reports **18**
failures, 17 of which are `PermissionDenied` on the default Docker socket.
They fail rather than skip, which is the right behaviour, but the signal is
buried; run it with the variable set or the one real failure is invisible in
the noise.

## Finding 1 (blocking, fixed) — the branch was red

`config/application.yml` grew three `${MTN_DISBURSEMENT_*}` placeholders.
`vpay_config::config::resolve_string` has no default syntax: an unset name is
`ConfigError::UnresolvedPlaceholder`, exit 78, **before** the adapter join and
before `REQUIRED_RAIL_KEYS` is consulted. `the_repositorys_own_configuration_passes_the_adapter_join`
loads the real file and set only the six old names, so it failed:

```
vpay-server: loading and validating configuration (--config / VPAY_CONFIG, ADR-0003):
unresolved ${ENV} placeholder: environment variable MTN_DISBURSEMENT_API_KEY is not set
```

The arm's verification page lists four crates it tested and `vpay-server` is
not among them. **A change to `config/application.yml` is a change to every
crate that loads it**, not only to the crate that wanted the key.

The comment the arm wrote beside those keys asserted the opposite —
"an unset variable here is not a boot failure" — reasoning from their absence
in `REQUIRED_RAIL_KEYS`. Two different checks. `.env.example`, in the same
commit, says the true thing ("an unresolved `${VAR}` is exit 78").

Fixed: the test sets the three names; the comment is corrected; and the
placeholder list, which several places enumerate, went **seven to ten** in
`deploy/helm/vpay/templates/_validate.tpl`, `deploy/helm/vpay/values.yaml`,
`deploy/helm/vpay/README.md`, `docs/runbooks/rotate-rail-credentials.md` § 1
and the `README.md` quickstart `export` line (which as written exited 78).
A rails Secret upgraded across this change without the three new keys stops
both Deployments booting; that is now stated where an operator reads it.

## Finding 2 (honesty, fixed) — nine corrected places, eleven missed

The arm's last commit is titled "correct every stale
'`mtn_momo::refund` is `NotImplemented`' claim" and says "Nine places said
it." Grepping the tree finds eleven more, including **the MTN adapter's own
module header** — the most-read of them:

| Where                                                   | Note                                                     |
| ------------------------------------------------------- | -------------------------------------------------------- |
| `vpay-adapter-mtn-momo/src/lib.rs` module doc           | the adapter's own header                                 |
| `vpay-db/src/settlement.rs`                             |                                                          |
| `vpay-db/src/refunds.rs`                                | a second site; the module header was fixed, this was not |
| `vpay-api/src/v1/mod.rs`                                | the route test's doc; the routes-table comment WAS fixed |
| `vpay-db/tests/repositories.rs`                         | two sites                                                |
| `backends/tests/integration/tests/refunds.rs`           |                                                          |
| `docs/reference/vpay-db/cratestack-money-tables.md`     |                                                          |
| `docs/reference/vpay-db/events-refunds-and-webhooks.md` |                                                          |
| `docs/flows/merchant-auth/resource-contract.md`         |                                                          |
| `docs/roadmap/phase-4-rail-adapters.md`                 |                                                          |
| `docs/flows/errors.md`                                  | "the one remaining `NotImplemented` token is …"          |

Dated pages under `docs/status/` and `docs/plans/` are deliberately **not**
touched: they are a verbatim historical record and were true on their date.

## Finding 3 (honesty, fixed) — three migrations, and two of them are in the database

The arm found `0031_refunds-fee.sql`'s header, could not edit it
(`verify-migrations` refuses a byte change to an applied migration, issue
#76), and recorded that. Correct as far as it went. Two more carry the same
claim:

- `0017_create-refunds.sql` — header **and** `COMMENT ON TABLE refunds`
- `0042_invoices-amount-refunded.sql` — header **and**
  `COMMENT ON COLUMN invoices.amount_refunded`, which says in capitals
  "NO RAIL CAN REFUND YET (`ProviderAdapter::refund` is `NotImplemented` on
  MTN and `Unsupported` on Orange)"

The `COMMENT ON` halves are the part that matters: they are **not** stale text
in a file, they are text in every database that applied those migrations, and
an operator sees them in `\d+`. SQL can reach them, so the repository's own
rule applies unchanged — "to fix a mistake in an applied migration, write a
new migration that corrects it" — and there is a precedent for a migration
whose whole purpose is a comment, `0020_provider-requests-status-code-comment.sql`.

`0046_refunds-comments-mtn-refund-is-written.sql` does that. It changes no
data, no column and no constraint, and it corrects only the parenthetical
about which adapter can refund — both comments' substance stands, because
nothing still inserts a `refunds` row.

The three frozen **file headers** are the case the rule did not cover: no
statement addresses them. `backends/migrations/README.md` gains an
**§ Errata** table for exactly that, with a pointer from
`docs/runbooks/migrations.md` § 1, and all three are listed.

## Finding 4 (evidence, fixed) — the credential-separation claim named the wrong mechanism

The arm's headline for its second mutation:

> Removing `self.product.path_segment()` from the fingerprint fails
> `a_collections_bearer_is_never_served_to_a_disbursement` — and nothing else
> in the crate (86 of 87 still pass). **Without it the single cache lookup
> succeeds and a Collections-scoped bearer is sent on the Disbursements
> `transfer`.**

The first sentence reproduces exactly. The second is **false**, and the same
claim appears in `token.rs`'s module doc and `fingerprint` doc,
`docs/reference/rails.md`, `docs/flows/adapter-mtn-momo.md` and
`wiremock/mtn/mappings/transfer.json`.

There is no "single cache lookup". `Adapter` holds **two named cache fields**,
`Adapter::slot` is a `match` on the product, and both `bearer` and `mint` go
through it — so a Collections entry can never be in the slot a `transfer`
reads, whatever the fingerprint returns. The fingerprint's product
discriminator is **defence in depth**: it is what would carry the guarantee on
the single-slot or map design this adapter considered and rejected.

Measured, each mutation applied alone and then both together, adapter crate
and conformance suite:

| Mutation                                          | `vpay-adapter-mtn-momo` | `vpay-tests-conformance` |
| ------------------------------------------------- | ----------------------- | ------------------------ |
| none (as shipped)                                 | 87 passed               | **67 passed**            |
| `Product` dropped from `Credentials::fingerprint` | 86 passed, **1 failed** | **67 passed**            |
| `Adapter::slot` collapsed to one field            | **87 passed, 0 failed** | **67 passed**            |
| both of the above at once                         | —                       | **67 passed**            |

So: **the conformance suite cannot see either guard, and the second guard was
held by no test at all.** The reason is in the suite's own fixture — it gives
each product a _different_ subscription key and API key, so the two
fingerprints differ with or without the discriminator and the cache never
collides. The bug the guards exist for needs a deployment that pasted
_identical_ credentials into both halves, which that suite never constructs.

**Is one test enough for that blast radius? No.** There are now two, one per
mechanism, and the new one is decisive — collapsing `slot` fails
`a_products_bearer_is_stored_where_only_that_product_can_read_it` and nothing
else.

**Recommended and NOT done**, because it is a change to a suite three arms are
touching and the collision risk outweighs it today: a `Credentials::Shared`
variant in the conformance fixture that configures both products with one set
of strings, so the container suite can reach the case at all. Worth doing when
the refunds work merges and the suite is quiet.

The half of `transfer.json`'s claim that _is_ true was also checked: point
`token::mint` at `/collection/token/` and three conformance cases fail. That
half is kept and now says how many.

## Finding 5 (honesty, fixed) — a stub cited a test that does not exist

`wiremock/mtn/mappings/token.json` said:

> `a_refund_carries_a_disbursement_scoped_bearer` asserts the transfer's
> `Authorization` header against it, which is the only way to prove the
> adapter did not serve a Collections bearer to a money-out call.

**No test of that name exists anywhere in the tree, and none ever did.** The
case that drives the mapping is
`a_refund_on_a_rail_that_refunds_reaches_the_rail_and_is_accepted`, and what
it proves is narrower than the sentence claims (Finding 4). Corrected in
place, with the invented name quoted so the correction is legible.

Two other test names cited in prose were checked and both exist:
`a_collections_bearer_is_never_served_to_a_disbursement` and
`the_transfer_is_addressed_by_the_reference_the_core_supplied`.

## Finding 6 (design, fixed) — the port doc that asked to be replaced was not

`ProviderAdapter::refund` carried:

> What an adapter should do if the invariant is ever _broken_ … is
> deliberately **not settled here**. **No adapter in this workspace implements
> `refund` yet** … and **this paragraph is what must be replaced when it
> does.**

The arm settled RFC-0003 open question 6 (`ProviderError::Config`) and
recorded it in the RFC, in the adapter, in the flow page and on two status
pages — but not in the one place the trait's own instruction named, which is
also the place a Wave-3 handler author reads first. Replaced.

**On the decision itself: `Config` is right, and the honest thing about it is
that it does not fit.** No configuration _is_ wrong — the core broke an
invariant the capability declared. The arm's reasoning eliminates the
alternatives correctly (`Rejected` blames a rail never asked, `Malformed` is
about an answer that does not exist, `Unsupported`/`NotImplemented` are lies),
and the operative property is the classification: stops the poll ladder,
pages, never reaches a payer as a decline. The port doc now says both halves —
that it is the closest true sentence available and that the classification is
what was actually being chosen. A maintainer wanting a `Precondition` variant
on `ProviderError` has a clean case; it is a port change and was rightly not
taken unilaterally.

## Finding 7 (the money question) — a `202` is accepted, and the warning was in the wrong places

MTN's `transfer` answers `202 ACCEPTED` with an empty body, settles
asynchronously, and its outcome is read from
`GET /disbursement/v1_0/transfer/{referenceId}` — a call vpay does not make,
because `ProviderAdapter` has no refund status read and `Refunded` has no
status field. The adapter reports `Ok(Refunded)`.

**Is that safe today? Yes, by absence of a caller** — `POST /v1/refunds` is
unrouted and nothing inserts a `refunds` row, so no merchant learns anything.
The arm did not overstate this and documented it in four places.

**Is it safe as left? Not quite**, and the reason is where the warning lived.
All four places were the MTN adapter, the RFC and two status pages. A Wave-3
handler author writes against the _port_, and the port said nothing. Moving
`Ok` to `refunds.status = 'succeeded'` is one line and reads as obvious. The
warning is now on:

- `ProviderAdapter::refund` § "An `Ok` is an _acceptance_. It is not a
  settlement";
- the `Refunded` type itself, § "There is no status here";
- `vpay_db::settlement::apply_refund_succeeded` — **the method that would
  record the lie**.

`pending` is what an `Ok` supports. RFC-0003 open question 8 stays **open**:
closing it needs a refund poll ladder, which is work and not a doc comment,
and this review did not write one.

## The three-way token-grant table: reproduced exactly

The arm's headline. `.form(..)` does not compile in this workspace (reqwest is
pinned `default-features = false`, no `urlencoded`), so the identical wire
bytes were hand-built, as the arm did:

```rust
.header(CONTENT_TYPE, "application/x-www-form-urlencoded")
.body("grant_type=client_credentials")
```

| Adapter                 | Token stub matcher                          | Arm reported       | This review measured     |
| ----------------------- | ------------------------------------------- | ------------------ | ------------------------ |
| JSON grant (as shipped) | `equalToJson` + `Content-Type` (as shipped) | 67 passed          | **67 passed, 0 failed**  |
| form-encoded grant      | `equalToJson` + `Content-Type`              | 43 passed, 24 fail | **43 passed, 24 failed** |
| form-encoded grant      | old `{"contains": "client_credentials"}`    | 67 passed, green   | **67 passed, 0 failed**  |

**Row three is confirmed and it is the real finding of this arm.** The
regression PR #177 found against MTN's real sandbox — a form-encoded grant
answered with a 200 "Request Rejected" HTML page — passed this conformance
suite green, for as long as the `contains` matcher stood. The tightening is
right, it is applied to both mints, and it is the one change in this arm that
would have caught a bug that actually happened.

Both mutations were reverted and the suite re-run to 67/67 before commit.

## Not fixed, and why

- **`wire::Transfer` derives `Debug` and prints the payee's number.**
  `RefundTarget` has a redacting `Debug` precisely so a `tracing::debug!(?destination)`
  is harmless; a `tracing::debug!(?body)` would undo it. Nothing formats a
  `Transfer` today and `a_refund_never_puts_the_payees_number_in_a_log_line`
  would catch it if the adapter did. Left alone because `RequestToPay` has
  carried exactly the same latent gap for the payer's number since Step 3, and
  fixing one without the other is worse than fixing neither. Worth a small
  follow-up across both.
- **A `Credentials::Shared` fixture in the conformance suite** — see Finding 4.
- **`compose.e2e.yml`'s Disbursements stub values do not match the mappings**
  (`wiremock-stub-disbursement-subscription-key` versus the suite's
  `stub-disbursement-subscription-key`), so a transfer from that stack would 404. Nothing exercises it; the misleading comment is corrected rather than
  the values, because making them agree before a refund walk exists would be a
  tidy-up that reads as a capability.
- **RFC-0003 open questions 7 and 8 stay open.** Both need the Wave-3 handler
  or a refund poll ladder. Neither was decided here.

## A pre-existing red gate this review did not cause and did not fix

**`just verify-errors` fails on this branch, on its base, and on
`refunds/w2-mtn` as delivered.**

```
xtask: error-modelling violations:
  - backends/crates/vpay-provider/src/lib.rs: `InvalidMsisdn` has no
    `impl Classify` anywhere in `vpay-provider` (ADR-0011)
```

Confirmed pre-existing by `git stash`ing this review's whole working tree and
re-running against `197d5df4`: same failure. `InvalidMsisdn` was added by
`11b282dd` on `review/w1b-parse-destination` (wave 1, arm B — the canonicalising
`RefundTarget` constructor) and merged into
`claude/orange-refund-transaction-2b95e5` before arm D branched. So
**CI's `self-checks` job is red for the whole refunds line and has been since
that merge**; neither the w1b review page nor arm D records it.

Not fixed here, deliberately. It is another arm's type, and the fix is a
judgement rather than a mechanical one: `InvalidMsisdn` never crosses a port
boundary — both adapters convert it to `ProviderError::Malformed` — so the
right answer may be an ADR-0011 exemption rather than an `impl Classify`, and
choosing between them on someone else's type inside a review of a different
arm would be exactly the kind of silent scope creep this repository's rules
warn about. **It needs an owner before any of the refunds arms merge.**

## What this review did not check

The Orange arm, the ledger arm and the write-path arm, all of which touch the
same RFC. `just ci` on any head. Anything about MTN's actual Disbursements
behaviour, which remains, on this branch as on the arm's, entirely unobserved.
