# 2026-09-15 — MTN Disbursements refund (RFC-0003 § 5, wave 2 / arm D)

Branch `refunds/w2-mtn`, based on `claude/orange-refund-transaction-2b95e5`
(`4caf7648`).

## The headline, first

`mtn_momo::refund` is **written**. MTN's Disbursements product has **never
been called from this repository** — not in production, not against MTN's
sandbox, not once — and **no deployment holds a Disbursements subscription
key**. The `ProviderError::NotImplemented("mtn_momo::refund")` token is
retired and `verify-status` prints zero items; that is a fact about a token,
not about a rail. Nothing on this page should be read as "MTN refunds work".

## Gates run

`just ci` was **not** run: this arm's brief forbids it (five concurrent local
builds have OOM-killed this host). CI is the gate. What was run locally:

| Command                                                                                                       | Result                               |
| ------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| `cargo nextest run -p vpay-adapter-mtn-momo`                                                                  | **87 tests, 87 passed, 0 skipped**   |
| `cargo nextest run -p vpay-tests-conformance` (real containers)                                               | **67 tests, 67 passed, 0 skipped**   |
| `cargo nextest run -p vpay-config`                                                                            | **115 tests, 115 passed, 0 skipped** |
| `cargo nextest run -p vpay-provider`                                                                          | see the run below                    |
| `cargo clippy -p vpay-adapter-mtn-momo -p vpay-tests-conformance -p vpay-config --all-targets -- -D warnings` | clean                                |
| `cargo test --doc -p vpay-adapter-mtn-momo -p vpay-provider`                                                  | see the run below                    |
| `cargo xtask verify-status`                                                                                   | `ok — 0 unimplemented item(s)`       |
| `cargo +nightly fmt --all --check`                                                                            | clean                                |

**`cargo nextest run -p vpay-server` was NOT among them, and it was red.**
Found by the review of 2026-09-15: adding three `${MTN_DISBURSEMENT_*}`
placeholders to `config/application.yml` made
`the_repositorys_own_configuration_passes_the_adapter_join` exit 78 on an
unresolved placeholder (47 passed, 1 failed). Fixed on
`review/w2d-mtn`. The general lesson is on the review page: a change to
`config/application.yml` is a change to every crate that loads it, not only to
the crate that wanted the key.

The conformance suite needs `DOCKER_HOST=unix:///run/user/1000/docker.sock`
on this host (rootless Docker). **Nothing was skipped**: the suite reports `0
skipped`, and `just verify-ignored` holds that at zero by construction.

## The mutation this arm exists to run

The brief's sharpest item. The Collections token stub matched
`{"contains": "client_credentials"}` — a pattern the **form-encoded** body
`grant_type=client_credentials` also satisfies. PR #177 had just fixed exactly
that bug against MTN's real sandbox (a 200 "Request Rejected" HTML page), so
the question was whether the suite could have caught it.

The adapter's token mint was changed to post the form-encoded grant. It is
spelled by hand —

```rust
.header(CONTENT_TYPE, "application/x-www-form-urlencoded")
.body("grant_type=client_credentials")
```

— rather than with `reqwest`'s `.form(..)`, because **`.form(..)` does not
compile in this workspace**: the root `Cargo.toml` pins reqwest with
`default-features = false` and no `urlencoded` feature. That is itself a small
accidental safety net and is worth knowing. The bytes on the wire are
identical.

| Adapter                 | Token stub matcher                           | Conformance result                             |
| ----------------------- | -------------------------------------------- | ---------------------------------------------- |
| JSON grant (as shipped) | `equalToJson` + `Content-Type` (as shipped)  | **67 passed, 0 failed**                        |
| **form-encoded grant**  | `equalToJson` + `Content-Type`               | **43 passed, 24 failed** — every MTN wire case |
| **form-encoded grant**  | **old** `{"contains": "client_credentials"}` | **67 passed, 0 failed** — fully green          |

The third row is the finding. **With the old matcher, the exact regression PR
#177 fixed passes this suite green**, and it did so for as long as the matcher
stood. Both mutations were reverted and the suite re-run to 67/67 before
commit.

The tightening is applied to the Collections mint **and** to the new
Disbursements mint, in `wiremock/mtn/mappings/token.json`.

## The other mutation

`Credentials::fingerprint` hashes `Product::path_segment()` alongside the
three credential strings, so that a deployment configuring both products with
identical values still mints and caches two separately-scoped bearers.

Removing `self.product.path_segment()` from the fingerprint fails
`a_collections_bearer_is_never_served_to_a_disbursement` — **and nothing
else** in the crate (86 of 87 still pass).

**The consequence stated here was wrong, and is corrected by the review of
2026-09-15 (see
[2026-09-15-mtn-disbursements-refund-review.md](2026-09-15-mtn-disbursements-refund-review.md)).**
There is no "single cache lookup": `Adapter` holds two named cache fields and
`Adapter::slot` matches on the product, so removing the discriminator does
**not** put a Collections bearer on a `transfer`. Reproduced and measured —
the mutation leaves all 67 conformance cases green. The fingerprint's product
field is defence in depth for the single-slot or map design this adapter
rejected; the two slots are the primary guard, and on 2026-09-15 they were
held by no test at all (collapsing them was green in both suites). The review
added `a_products_bearer_is_stored_where_only_that_product_can_read_it` for
them. Neither guard is reachable from the conformance suite, which gives each
product different credentials and so never constructs the copy-pasted
configuration both exist for.

## What is proven, and against what

Seven conformance cases run against a real `wiremock/wiremock` container
(each × 2 rails; the Orange half asserts `ProviderError::Unsupported` rather
than skipping):

- `a_refund_on_a_rail_that_refunds_reaches_the_rail_and_is_accepted` — the
  stub answers `202` only for a bearer minted from `/disbursement/token/` and
  the per-product subscription key, so an `Ok` proves the adapter read the
  **Disbursements** configuration keys and asked the **Disbursements** token
  endpoint. _(Narrowed on review, 2026-09-15: this said an `Ok` "**is** the
  proof of scope" and that the leading `submit` made the wrong-scope bug
  reachable. Measured, it does not — the two products have different
  credentials in this suite, so their cache keys differ however the
  fingerprint is built, and the adapter's two cache slots keep the bearers
  apart regardless. The cross-product case is held by two unit tests and by
  nothing in this suite.)_
- `the_refund_is_addressed_to_the_payee_the_merchant_nominated` — asserted
  against WireMock's own request journal, on `$.payee.partyId` and the
  reference, because the return value cannot tell a 202 for the right payee
  from a 202 for the wrong one.
- `a_refund_to_a_payee_the_rail_rejects_is_a_decline_and_never_an_accepted_transfer`
- `a_duplicate_refund_reference_is_accepted_and_never_paid_twice`
- `a_refund_never_puts_the_payees_number_in_a_log_line` — a scoped `tracing`
  subscriber over the call, greped for **both** spellings of the number, with
  a non-empty-capture assertion so it cannot pass vacuously.
- `a_rail_without_the_refund_capability_answers_unsupported` (unchanged in
  substance, reworded)
- `a_required_rail_parses_its_own_destination` (unchanged)

Twelve new unit tests in `vpay-adapter-mtn-momo`, including the transfer
body's `payee`-not-`payer`, the partial amount, the reference, the whole
`refund_outcome` status table, the three per-key credential refusals, and the
no-payee `Config` answer.

**What none of it proves is that MTN behaves this way.** The stub is a
transcription of MTN's published `Transfer` operation and was written from the
same source as the adapter, so it cannot disagree with it.
`docs/flows/adapter-mtn-momo.md` § "Not proven" lists what a first real call
has to check.

## One stale comment that cannot be corrected, by design

`backends/migrations/0031_refunds-fee.sql`'s header says "`mtn_momo::refund`
is the one remaining `NotImplemented` token". That is now false. It was
corrected here and then **reverted**, because `cargo xtask verify-migrations`
refused it: applied migrations are immutable down to the byte (issue #76), and
every database that applied the original refuses to boot once the SHA-256
changes. Writing a new migration to fix a comment would be worse. The file
stays as it is; `docs/status.md` and
[../../flows/adapter-mtn-momo.md](../../flows/adapter-mtn-momo.md) are the
current record, and this paragraph is the pointer for anyone who reads that
header and believes it.

## Decisions taken here that a maintainer may want to revisit

1. **RFC-0003 open question 6** (a `Required` rail handed `destination:
None`) → `ProviderError::Config`. The RFC delegated this to "the first
   adapter to make a real transfer call"; this is it. Reasoning is on
   `Adapter::refund`.
2. **Which reference the transfer carries.** The port hands `refund` one
   `ChargeRef` and a refund needs its own rail reference. The adapter uses
   the reference it is given, and **that is only correct if the Wave-3
   handler passes the refund's `provider_reference_id`.** If it passes the
   charge's, every partial refund after the first is answered `409` and
   reported accepted with no money moved. Left open in RFC-0003 rather than
   decided here.
3. **Three explicit Disbursements credential keys, no fallback** to the
   Collections values. Prudence if MTN issues per-product credentials,
   friction if it does not — unverified either way.
4. **A `202` is accepted, not settled**, and the port has no refund status
   read. `Ok(Refunded)` must not be turned into `refunds.status = succeeded`
   by whoever writes the write path.
