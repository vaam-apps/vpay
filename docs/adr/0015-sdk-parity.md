# ADR-0015: The merchant SDKs are held to parity, per capability, machine-checked

- **Status:** Accepted
- **Date:** 2026-09-03
- **Deciders:** vpay maintainers

## Context

`sdks/rust` and `sdks/nodejs` are two independent implementations of the
same wire contract (`docs/flows/stripe-sdk-compat.md`, ADR-0010). They were
built by different lanes on different days (Step 2's demo, then whichever
lane touched each SDK next), and nothing before this ADR compared them to
each other. In practice they had already drifted in ways a merchant would
notice only by hitting them:

- `sdks/nodejs` exports `createStripeAuthenticator`, letting the official
  `stripe` package talk to vpay (ADR-0010's 2026-09-03 amendment).
  `sdks/rust` has no equivalent — `async-stripe` has no per-request async
  hook, so the same result there means custom transport middleware, and
  nobody had scoped that work.
- Neither SDK calls `GET /v1/events/{id}`, though the server serves it.
- The Rust SDK never reads a token response's `token_type`, so a `DPoP` or
  `MAC` response would be accepted and presented as `Bearer`. The Node SDK
  refuses it.
- The Node SDK's amount-range validator throws a bare `TypeError`, not a
  `VpayError`, so the one narrowing the package documents
  (`err instanceof VpayError`) misses it.
- Neither SDK surfaces the `request-id` response header, and neither reads
  `stripe-should-retry` — both are already emitted by the server
  (`vpay_api::STRIPE_REQUEST_ID_HEADER`, `Classify::retry`) and already read
  by real `stripe-node`, which is exactly what `sdks/stripe-compat` proves.
- `sdks/nodejs`'s `PaymentIntent` is a plain interface: `console.log(intent)`
  prints a live `client_secret`. `sdks/rust`'s redacts it in `Debug`.

None of these are hidden — each is visible by reading the two trees side by
side — but nothing forced that reading to happen, and nothing stopped a new
capability from landing in one SDK without landing in the other. A merchant
who reads `sdks/nodejs`'s README and then reaches for the equivalent Rust
call has no way to know, short of trying it, whether the two behave the
same, behave differently, or one of them doesn't exist. For a payments SDK
that is worse than an admitted gap: it is an unadmitted one.

The user's rule, verbatim: *"vpay sdk parity: each should share the very
same kind of features. We need a matrix for that."*

## Decision

**1. Parity is per capability, with the same wire semantics — not per
method name or per file.** A capability is a single, testable claim about
behaviour on the wire: "the token is cached and refreshed at `expires_in`
minus the margin," not "the SDK has a `TokenManager` class." Two SDKs are at
parity on a capability when the same request produces the same claim in
both, not when both merely expose a same-named method. This is what makes
the matrix's rows meaningful across two languages with different idioms.

**2. A capability lands in every merchant SDK in the same PR, or it is
recorded as a dated, named gap in the matrix.** "Merchant SDK" means
`sdks/rust` and `sdks/nodejs` — the two clients a server-side merchant
integration uses to call `/v1`. A PR that adds `events.retrieve` to one and
not the other does not merge silently; it either brings both along or it
updates the matrix row to `⛔` with today's date, the reason, and an owner,
so the gap is a decision on record rather than an accident nobody noticed.
This is a statement about how *new* capabilities land, not a demand to close
every gap this ADR found on adoption — see "What this does not require"
below.

**3. The matrix is machine-checked in `just verify`, the same way
`docs/status.md` is (`cargo xtask verify-status`, ADR-0011's sibling
convention).** A parity claim that only a human enforces decays the same way
an unchecked `NotImplemented` list would. `cargo xtask verify-sdk-parity`
reads [`docs/sdks/parity.md`](../sdks/parity.md) on every `just verify` and
fails the build the moment the document and the trees disagree.

**4. `@vpay/stripe-js` is a separate surface with its own rows, not a third
merchant SDK.** It authenticates a **payer's browser** with a publishable
key and a per-intent `client_secret` (ADR-0010's Step 5c amendment), speaks
`/v1/browser`, and shares no capability with `/v1`'s `client_credentials` +
`private_key_jwt` handshake. Comparing it row-for-row against the merchant
SDKs would force capabilities that cannot mean the same thing — "token
lifecycle" has no browser analogue — into the same table. It gets its own
table in the same document, for the same tests-prove-it discipline, without
pretending it is the third column of the merchant matrix.

**`sdks/stripe-compat` is evidence, not an SDK, and gets no rows.** It
drives the real `stripe@22.6.1` package through `@vpay/sdk/stripe` against a
live compose stack. It exists to *prove* capabilities the merchant SDKs
claim (it is where `request-id` and `stripe-should-retry` are actually
observed on the wire), not to claim any of its own — a row here would be
graded against code this repository does not ship to a merchant.

### What counts as a capability

The matrix's rows are drawn from what a merchant integration actually
depends on, not from an SDK's internal structure:

- the `private_key_jwt` auth handshake (RS256 assertion shape, `kid`,
  `jti`, `exp`, the `aud`/audience distinction, key rejection)
- token cache and refresh margin (reuse, the 30s-or-half-`expires_in` rule,
  single-flighting concurrent first calls)
- every `/v1` resource operation: `payment_intents.{create,retrieve,confirm,
  cancel,list}`, `refunds.create`, `events.{list,retrieve}`,
  `balance.retrieve`
- idempotency-key handling (generated when absent, replayed byte-identical
  on the 401 re-auth retry)
- error mapping, including the `request-id` header and the
  `stripe-should-retry` advisory, and whether a transport failure is
  distinct from an HTTP one
- webhook signature verification and its tolerance window (rotation via a
  second `v1=`, the literal-`t`-text HMAC input, malformed-header handling)
- `client_secret` exposure on `create`/`retrieve` and its absence from list
  items
- `Debug`/inspect redaction of the private key, a cached token, and
  `client_secret`
- transport concerns: timeouts, TLS trust roots, `User-Agent`, URL
  normalisation
- retry behaviour beyond the mandatory single 401 re-auth
- the Stripe authenticator (`createStripeAuthenticator` and its Rust
  equivalent, whenever one exists): host-binding, shared token cache

A capability absent from *both* SDKs is still recorded `⛔`/`⛔` — the two
are at parity with each other and both short of the server, which is a
different, weaker statement than "done," and it is recorded rather than
left blank so the difference stays visible (see "What this matrix does not
claim" at the end of the document).

### How a gap is recorded

A `⛔` cell is not "TODO." It carries three things, on one line, in the cell
itself:

- the date it was found (`YYYY-MM-DD`) — the check enforces this
  mechanically;
- the reason, specific enough that fixing it is unambiguous (not "not done
  yet" but "the package calls the global `fetch` and configures no trust
  store, so its roots are whatever the host Node was built with");
- who owns closing it (a role, not a promise of a date — this repository has
  no sprint to attach one to).

Every `⛔` also appears once more in the document's "Gap ledger," an index
over the same cells, so a reader can see every open gap without walking
every table. The ledger is not separately authoritative — a mismatch between
a cell and the ledger is a documentation bug, not a parity finding, and the
check reads the tables, not the ledger.

### How the check reads the matrix

`cargo xtask verify-sdk-parity` treats every markdown table in
`docs/sdks/parity.md` whose first header cell is literally `Capability` as
one parity table (the gap ledger and the legend are prose and other tables
around it, and are ignored). For each such table:

- the remaining header cells, read as code spans, name repository-relative
  directories (`sdks/rust`, `sdks/nodejs`, `sdks/stripe-js`) — a column that
  is not a real directory fails the build outright;
- every cell must start with `✅` or `⛔`; nothing else is a valid answer, and
  a blank cell fails;
- a `✅` cell names one or more tests in backticks. Every one of them must
  exist under that column's directory as a live test: a Rust function
  preceded (skipping blank lines and doc comments) by a `#[test]` or a
  `#[tokio::test]`-shaped attribute with no `#[ignore]` in the same
  attribute block, or a TypeScript `it("…")`/`test("…")` call (not
  `it.skip`/`it.each`, and not a `submit(...)`-style false match on a
  trailing substring) — found by walking the column's directory, skipping
  `node_modules`, `dist`, `target`, `.git` and `coverage` so a vendored
  dependency's own test cannot satisfy someone else's claim;
- a `⛔` cell must contain a `YYYY-MM-DD`-shaped run of characters somewhere
  in its text (shape only — this is a "was this dated" check, not a
  calendar).

The check is proven against synthetic matrices and a synthetic SDK tree
(`xtask`'s `sdk_parity_tests` module) independent of this repository's own
`docs/sdks/parity.md`, and separately against the real matrix
(`the_repositorys_own_matrix_passes`), so a change to either the parser or
the document is caught by the corresponding half.

## Alternatives considered

- **A parity table with no machine check, reviewed by hand at each SDK
  PR.** Rejected for the reason `docs/status.md` was given
  `verify-status`: an unchecked table is accurate on the day it's written
  and silently wrong the first time someone renames a test or adds a method
  to one SDK without the other. This repository already has one working
  instance of "the document a human reads is the document the build checks"
  (`verify-status`); this ADR reuses the pattern rather than inventing a
  second one.
- **Parity enforced by a shared conformance test suite run against both
  SDKs (a single spec, executed by both languages' runners).** Attractive in
  the abstract, rejected for now: the two SDKs do not share a request layer
  or even a test framework, and building a cross-language runner is a
  larger project than the gap it would close. The matrix gets most of the
  same benefit — a forced, dated, named comparison — for a fraction of the
  cost, and does not block adopting a shared suite later for the rows that
  would benefit most (the byte-identical wire-encoding rows already are, in
  effect, a hand-maintained version of this).
- **One row per SDK method instead of one row per capability.** Rejected:
  method-name parity is nearly meaningless across Rust and TypeScript (a
  builder pattern vs. a plain options object) and would either force
  artificial method-name symmetry or produce a matrix nobody could map back
  to a merchant-visible behaviour. Capability rows are what a merchant
  actually depends on.
- **Grade `@vpay/stripe-js` and `sdks/stripe-compat` in the merchant
  matrix's own columns.** Rejected per Decision 4: the browser surface's
  credential model has no merchant-SDK analogue, and `stripe-compat` proves
  claims rather than making its own; grading either in the same table as
  `sdks/rust`/`sdks/nodejs` would misrepresent what both actually are.
- **Block this ADR's adoption on closing the gaps it found.** Rejected: the
  point of the rule is that gaps become visible and owned, not that they are
  zero on day one. Closing 24 gaps to land one ADR would also mean shipping
  that work unreviewed, which is the opposite of what "an accurately
  reported gap is worth more than a green you cannot trust" asks for.

## What this does not require

- **Not that every capability is present in both SDKs today.** As of
  2026-09-03 the matrix records 24 dated gaps across the merchant SDKs and
  the browser surface (`docs/sdks/parity.md`'s gap ledger). This ADR is
  adopted with those gaps open, on record, and owned — closing them is
  follow-up work, not a precondition.
- **Not that a `✅` cell means the capability is bug-free**, only that a
  named test fails when it regresses.
- **Not that the server itself offers every capability the SDKs might
  someday need** — `/v1/refunds` and `/v1/balance` server-side gaps, where
  they exist, are tracked in `docs/status.md`, not here.

## Consequences

- A new PR that adds a capability to one merchant SDK and not the other
  either brings the second SDK along or edits the matrix, in the same PR —
  `just verify` fails otherwise once the new capability's row exists, and a
  reviewer can ask for the row if it doesn't.
- Renaming or deleting a test a `✅` cell names is caught immediately,
  because the check reads the SDK's actual sources, not the document's
  claim about them. This is the same "the check re-runs the original
  failure, not just 'does it build'" property `verify-status` has.
- The matrix becomes the map a maintainer reads before touching either SDK:
  "what does the other one do here" has one place to look instead of two
  trees and institutional memory.
- `docs/status.md` gains a summary row pointing at the matrix and the check,
  the same way it already summarises `verify-no-mocks` and `verify-errors`.
- Extending the matrix to a third merchant-facing language (should one ever
  ship) is additive: a new column, one directory to walk, no change to the
  check's rules.
