<!-- The narrative half of `backends/crates/vpay-core`'s documentation. The
     rustdoc says WHAT each item is and links here; this page says WHY it is
     shaped that way. Neither is a flow document: docs/flows/ describes
     processes, this describes one crate's decisions. -->

# `vpay-core` reference

`vpay-core` is the domain crate: ids, money, the two state machines, the
failure taxonomy, the error classification, and the metric vocabulary. It
depends on no payment rail, no HTTP framework and no database, and if a
rail-specific name (`msisdn`, `pay_token`, `subscription key`) ever appears in
it that is a defect ([ADR-0002](../adr/0002-provider-port.md)).

Each module's rustdoc says what a thing is and links to the section here that
says why. Where a decision already has a home — an ADR, a flow document — this
page points at it rather than restating it.

| Module | rustdoc says | This page says |
|---|---|---|
| `ids` | the four generators and the shape check | why Crockford base32, why the shape check is not an existence oracle, where a client secret's entropy comes from |
| `money` | minor units, the two provider encodings | why there are two encodings, what the bounded echo protects |
| `state` | the lifecycle enums and `next_status` | why the wire labels are written out twice, why `Transition` is only the merchant's three verbs |
| `failure` | the closed taxonomy | why the policy lives on the code |
| `settlement` | `settle` and `contradiction` | why this is a sibling of `state` and not part of it |
| `error` | `Classify`, `Category`, `source_chain` | the three tiers and the five questions ([ADR-0011](../adr/0011-error-modelling.md)) |
| `metrics` | the twelve names and their labels | why a library describes but does not install, and the caveats on four of the series |

---

## ids

An id is a *merchant-visible, permanent* name. It appears in URLs, in logs, in
support tickets, in a merchant's own database, and — because
[docs/api/README.md](../api/README.md) promises Stripe's shape — in code
merchants wrote against Stripe. That fixes four properties, and each one is a
test in `vpay_core::ids`:

* **A prefix says what it names.** `pi_` on a charge id is a bug an operator
  can see at a glance instead of one they have to look up.
* **The body is `[a-z0-9]` only,** so the id survives a URL path segment, a
  query string, a form body, a shell argument and a filename unchanged. The
  crate's own test proves `encodeURIComponent` (what both SDKs escape path
  segments with) is the identity on it. A `+` or a `/` from a base64 id would
  be re-encoded by one client and not another, and the two would then address
  different URLs.
* **It carries no information.** Not a sequence, not a timestamp, not a
  merchant id: an id that leaks how many payments a deployment has taken is a
  business fact given away to anyone holding one id, and a guessable id is an
  enumeration attack against a tenant-scoped API. This is why `Uuid::new_v4`
  (OS CSPRNG) rather than v7, which embeds a timestamp.
* **It fits.** 3 or 4 prefix characters plus 24 body characters is 27–28
  characters, comfortably inside the `CHECK (char_length(id) BETWEEN 1 AND 64)`
  the schema puts on every id column, with room for a longer prefix later.

24 body characters is 24 × 5 = 120 bits, taken from the top 120 bits of a v4
UUID. Six of those are the version and variant bits RFC 9562 fixes, so an id
carries **114 bits of randomness**: a deployment would need on the order of
2^57 (≈1.4 × 10^17) ids before a collision became an even bet, against a system
that will issue perhaps 10^9 in its life. The 8 dropped low bits are simply the
difference between 128 bits and a whole number of base32 characters; they are
dropped rather than folded in because a fold would be extra arithmetic nobody
could check by eye for zero practical benefit.

### Why Crockford base32 and not hex or base62

Hex would need 32 characters for the same entropy and reads as a hash, which
invites people to try to invert it. Base62 is mixed-case, and a mixed-case id in
a case-insensitive place (a Windows filename, an email subject line someone
lower-cased, a `LIKE` in a merchant's own database) becomes two different ids.
Crockford's alphabet is lower-cased here and drops `i`, `l`, `o` and `u`, so an
id read aloud or copied out of a screenshot cannot become a *different
valid-looking* id — which matters because these end up in support tickets.

The alphabet is Crockford's; the *encoding* deliberately is not. Crockford
specifies check symbols and case-insensitive decoding with `i`/`l` → `1` and
`o` → `0`; vpay decodes nothing (an id is an opaque key, looked up whole) so
none of that applies. Only the character set is borrowed.

### `is_well_formed` is a shape check and not an existence oracle

It says nothing about whether the object exists, belongs to the caller, or ever
existed — the merchant-scoped query is what answers that, and it must stay the
only thing that does.

What it *is* for is the case where a malformed id would otherwise produce a
silent wrong answer rather than an error. A caller-supplied id used as a list
cursor is the live example: an id the shape check would have caught resolves to
`NULL` in `vpay_db`'s cursor subquery and comes back as an empty page, so
without a boundary check a typo pages a merchant into silence instead of into a
`400`.

It is case-sensitive because the ids are: the alphabet is lower-cased, and an
uppercase copy of a real id is not that id anywhere else in the system either,
so accepting it here would be the one place that disagreed.

### Client secrets

`client_secret_suffix` is 32 alphabet characters — 160 bits, of which **148 are
unpredictable** (RFC 9562 fixes four version bits and two variant bits in each
draw's top 80, and the function does not fold them out, because folding would be
arithmetic nobody could check by eye for a fraction of a bit of collision
margin). 148 bits is far past the point where guessing is the attack anyone
would choose.

Deliberately more than an id's 120: an id names an object and is *meant* to be
quotable in a support ticket, while this is the credential that authorises a
stranger's browser to confirm a payment, and the only thing standing between a
guesser and a live intent is how many bits it holds. The floor is enforced in
the database by migration `0026`'s `client_secret_suffix_length` CHECK.

**Why two UUID draws.** One v4 UUID is 128 bits and this needs 160, so a single
draw could not supply them however it were sliced. Two draws contribute the top
80 bits each — an even split, which is the shape that makes "two independent
CSPRNG draws" true of the whole string rather than of a prefix of it.
`Uuid::new_v4` is `getrandom`-backed and is already how every id in the module
is minted; adding a second RNG crate to this dependency-light crate would be a
second thing to audit for the same guarantee.

`CLIENT_SECRET_INFIX` is a **wire contract**, not an implementation detail:
`@vaam-apps/vpay-stripe-js` splits a `clientSecret` on that exact string to recover the id
it must build a URL from (`sdks/stripe-js/src/client.ts`'s `SECRET_SEPARATOR`),
and Stripe spells its own client secrets the same way. `client_secret` is the
one place the two halves are joined, on the minting side and the checking side
both — `vpay_api::browser::authenticate` rebuilds the expected secret with it
and compares that, rather than parsing what the caller sent. A parser would have
to decide what to do with a value carrying two separators or none, and every
such decision is a place where "which secret did we actually compare?" stops
being obvious.

Since Step 9 the same function serves `checkout_sessions.client_secret_suffix`
(migration `0028`, an identical CHECK) as well, and `cs_…_secret_…` is spelled
by the same `client_secret` join. One generator and one join for two tables is
what makes "both credentials are 160 bits and both survive a URL" one fact
rather than two that could drift.

### `return_token` is a second *capability*, not a second secret

`return_token` has the same 32 characters, the same generator body and the
same CHECK. It is a separate function, a separate column and a separate
constant-time comparison because it authorises something strictly smaller, and
D6 of [the Step 9 plan](../plans/2026-09-04-step9-hosted-checkout.md) is the
reason it has to exist at all.

Every secret on a vpay-served page rides in a URL **fragment**, which the
browser never sends to any server. A fragment does not survive a rail's
redirect, though — a payer coming back from Orange's own checkout arrives at a
URL the *rail* built — so the return page's credential has to be a query
parameter. Query parameters are written to access logs, kept in browser
history and sent as `Referer` by some clients.

The answer is not to make that value weaker or stronger; it is to make it buy
less. `return_token` reads the session and its intent **without** the intent's
`client_secret`, which is enough to render an outcome and forward a payer and
is not enough to confirm anything. The session's own `client_secret` — the one
in the fragment — is what buys the intent's credential.

`secret_body` is shared so the *entropy* is one decision;
`client_secret_suffix` and `return_token` stay two names so the *authority* is
two. A call site that spelled the first while minting a return token would
read as if they were interchangeable, which is precisely what D6 says they are
not — and `a_return_token_is_thirty_two_characters_and_independent_of_the_secret_beside_it`
pins that they are drawn independently, so a future "optimisation" that
derived one from the other fails rather than quietly collapsing the two
capabilities into one.

`CHECKOUT_SESSION_PREFIX` is `cs_`, Stripe's own spelling for the same object.
That matters more here than for `pi_`/`ch_`: the prefix is the leading
characters of a `client_secret` a merchant pastes into `initEmbeddedCheckout`,
so a merchant who has integrated Stripe once recognises at a glance which of
the two credentials on their page they are holding.

---

## money

[docs/flows/money.md](../flows/money.md) is the rule: integer minor units, XAF
is zero-decimal, one conversion point. What follows is what the code adds to it.

### The two provider encodings

`Money::to_provider_string` and `Money::to_provider_minor` are the *same*
conversion in two encodings, not two conversions: both read the exponent from
the currency and neither scales anything. Orange Money's `webpayment` body sends
`"amount": 5000` while MTN's sends `"amount": "5000"` — one exponent lookup, two
renderings, which is what the flow document's "single conversion point" rule
means now that a rail needs the other encoding.

**The mistake `to_provider_minor` can be used to make.** It returns *minor*
units. Handing `5000` to a rail that expects major units is 100× on a
two-decimal currency and nothing can detect it downstream — the number is valid,
the currency is right, and the charge succeeds. An adapter that reaches for it
must have read its rail's documentation and found the word "minor" (or a
zero-decimal currency, where the question does not arise).
`eur_minor_units_are_not_the_major_amount` is the test that fails if someone
"simplifies" the function into something major-unit-shaped.

### The bounded currency echo

An ISO-4217 code is three characters; `Classify::public_message` echoes at most
eight, plus an ellipsis if anything was dropped. Eight is generous enough that a
merchant recognises what they sent (`"xaf "`, `"XAFF"`, `"xaf\n"`) and short
enough that the echo cannot become a reflection channel.

Without a bound, `Currency::from_code` would happily build an `UnknownCurrency`
around a megabyte of caller-supplied bytes and the public message would put all
of it in a response body — `from_code` is the real ingress, so the bound has to
live in `public_message` rather than in a caller that may forget. The truncation
is character-wise, not byte-wise: slicing at byte 8 would panic on a multi-byte
boundary, and [ADR-0007](../adr/0007-lint-policy.md) denies panics on a request
path.

`Display` keeps the whole string, because that half goes to an operator's log,
never into a response body.

### `Currency` is exempt from the `snake_case` convention

Step 7's serde convention is `#[serde(rename_all = "snake_case")]` on every type
that models vpay's own wire or config. `Currency` carries `"UPPERCASE"` instead
and is a deliberate exception: these are ISO-4217 codes rather than vpay field
names, and `"XAF"` is the spelling the database, both adapters and
`Currency::code` already agree on. `Money` itself carries the convention.

---

## state

[docs/flows/payment-lifecycle.md](../flows/payment-lifecycle.md) is the
lifecycle. Two things about how it is modelled are worth a paragraph.

### Two routes to one label

Each status's wire label is written out in `as_wire_str` *beside* the `serde`
rename, and the duplication is deliberate: the two paths that need it do not go
through `serde`. The `intent_status` Postgres enum is read and written as a
`String` (`vpay-db` binds strings, this crate parses them — Step 2's D4), and
`vpay-api`'s repository calls pass the *expected* and *new* label into a
compare-and-swap `UPDATE`.

A hand-rolled spelling that disagreed with `serde`'s would mean a status that
renders one way to a merchant and matches another way in a `WHERE` clause, so
`the_wire_spelling_is_the_same_by_both_routes` pins the two together for every
variant.

`from_wire` returns `Option` rather than implementing `FromStr` with an error
type: the only caller is the boundary reading a Postgres enum back, where an
unparseable label is not a caller's mistake but a schema/code mismatch that the
HTTP layer answers `500` for. Returning `None` lets that layer say so in its own
vocabulary instead of forcing a new public error type into this crate for a case
no merchant can cause.

### The merchant's three verbs

`Transition` is deliberately not "every edge in the lifecycle". The rail-driven
edges of the lifecycle diagram (`requires_action → processing` once the payer
has been redirected, `processing → succeeded|failed` when a status query
answers) are moved by the reconciler from an authenticated status query, never
by a request. Modelling them here would invite a handler to call `next_status`
and move an intent on a *callback*, which is exactly what
[docs/flows/provider-port.md](../flows/provider-port.md) forbids: `parse_callback`
returns identifiers only, never a status, and the authenticated status query is
the only thing that moves money. (The code comment this paragraph replaces cited
a `docs/flows/callbacks.md` that has never existed.)

So `next_status` answers `None` for them, meaning "not something this request
may do", not "impossible". `None` is not the same as "the intent is stuck".

`next_status` is one function on purpose: a handler that decided "cancel is fine
here" for itself is how `canceled` becomes reachable from `processing`, after
the rail already has the request and cannot be recalled. Both legal confirm
answers route through `ProviderFlow::status_after_confirm` rather than repeating
the push/redirect split, so the two cannot drift.

And a legal answer is *not* permission to write it: the write itself is a
compare-and-swap on the row's current status, because between the call and the
`UPDATE` another request may have moved the same row.

---

## failure

[docs/flows/failures.md](../flows/failures.md) is the taxonomy and how adapters
map into it. The one thing the code adds: `payer_actionable` and
`merchant_actionable` are *on the code*, not at a call site, so the answer to
"whose problem is this decline" is the same in the API response, the dashboard
and any future retry heuristic. A code is at most one of the two, and one that
is neither (`provider_account_blocked`) is the operator's own.

`ProviderError` — the unmapped arm — is never payer-actionable, and a rising
rate of it means an adapter's mapping table has drifted behind the rail. It is
always accompanied by the rail's raw reason, which is stored on the charge and
never branched on.

---

## settlement

This is the reconciler's half of the state machine, and it is deliberately *not*
part of `state::Transition`. That enum is "one of the three verbs a **merchant**
can apply", and `next_status` answers `None` for every rail-driven edge on
purpose: adding a variant for `processing → succeeded` would make that edge
reachable from an HTTP handler, which is the single thing that enum exists to
prevent. So the rail-driven edges live in a sibling module with their own
vocabulary, and the two cannot be confused at a call site.

The table is [docs/flows/reconciler.md](../flows/reconciler.md) plus the
recovery table of [docs/flows/crash-safety.md](../flows/crash-safety.md),
transcribed. `settle` is a `const fn` and total, with no wildcard in either
dimension, so a new `ChargeState` or `StatusKind` is a compile error rather than
a silent default.

`StatusKind` is a near-copy of `vpay_provider::ChargeStatus`, and that
duplication is deliberate: this crate knows nothing about any payment rail
(`vpay-provider` depends on *it*, never the other way round), so the port's type
cannot appear in the signature. The caller maps one onto the other — a four-arm
`match` in `vpay_worker::handlers` — and drops the two payloads no state
decision may read: the rail's transaction id (a reconciliation field, not an
input to a state machine) and, for a decline, the rail's raw reason string
(which belongs in `charges.failure_raw`, never in a branch). `Failed` keeps its
`FailureCode` because the decision *result* carries it: the taxonomy code is
written to the charge in the same statement that fails it.

`Settlement::Recover` exists so that "the rail has never heard of this" cannot
be spelled the same way as any state. A `NotFound` that fell into
`Settlement::Failed` would fail a charge a payer may already have paid, which is
the one conclusion the whole recovery design refuses to draw. On a charge the
rail has already acknowledged (`pending`, `unresolved`), the same answer means
the rail lost track rather than that we never sent it, so it is treated as
`Stay`: keep polling, never resubmit.

`Settlement::Live` carries a `ChargeState` rather than being one variant per
state, so the caller's write is one compare-and-swap parameterised by the value.
The type does not stop a terminal state being named there, but no arm of
`settle` produces one and the terminal edges have their own variants precisely
so they cannot be reached by accident —
`the_live_variant_never_names_a_terminal_state` is the assertion.

`Unresolved` deliberately does not fall back to `Pending` on a `Pending` answer:
the escalation is a fact about how long this charge has been outstanding, and
un-escalating it would drop the alert a human is working from.

### Contradictions

`settle` answers `None` for every terminal charge, which is right — no rail
answer moves one — but `None` folds two very different situations into one word.
Usually it means the charge settled a moment ago and this poll is simply late.
Sometimes it means the rail is telling us the money went the *other* way from
what we recorded and told the merchant.

That second case has to reach a human. vpay must not act on it — a charge is
settled once, and flipping `failed` to `succeeded` from a poll would make the
settlement transaction's compare-and-swap meaningless — but discarding it
silently is how a real double-charge, or a payment a merchant was told had
failed, goes unnoticed until the rail's monthly statement. So the caller keeps
the job finished and raises an alert
([docs/runbooks/unresolved-charges.md](../runbooks/unresolved-charges.md) is the
reconciliation this starts).

Only the two money-bearing disagreements count. `Pending` against a terminal
charge is a rail that has not caught up with itself, and `NotFound` is never on
its own grounds for any conclusion — neither says the money moved differently
than recorded, and alerting on them would bury the two that do. The test is
written as the full cartesian product rather than as two positive cases, because
the bug it guards against is an over-eager alert, and an alert that fires
constantly is an alert nobody reads.

---

## error

[ADR-0011](../adr/0011-error-modelling.md) is the decision;
[docs/flows/errors.md](../flows/errors.md) is the policy table, transcribed
row-for-row into `every_category_matches_the_policy_table_in_docs_flows_errors_md`
so the document and the code fail together.

vpay has many error *types* on purpose — `MoneyError`, `LedgerError`,
`ConfigError`, `DbError`, `ProviderError`, and the composites the API and worker
layers build from them — because a caller that can `match` on a closed enum can
react precisely, and a `String` cannot be matched on at all. What those types
share is not a base class but a *classification*: whichever concrete error
reaches a boundary, the boundary needs to answer the same five questions —

1. whose fault is it (**category**),
2. what HTTP status and Stripe-shaped `type` does it map to,
3. may it be retried, and by whom (**retry**),
4. how loudly should it be logged (**severity**),
5. what may a merchant be told about it (**public message**).

`Classify` is that seam. Every error enum in `backends/crates` implements it
(machine-checked by `cargo xtask verify-errors`), so the HTTP envelope, the
worker's retry decision and a binary's exit code are all *derived* from one
classification instead of hand-rolled per call site — the same discipline
`docs/flows/failures.md` already applies to the merchant-facing failure
taxonomy, applied to the system's own errors.

Three tiers:

* **Leaf** errors: one `thiserror` enum per crate concern, closed, with
  `#[source]` chains preserved and no secrets in `Display`.
* **Composite** errors: a layer's own enum that `#[from]`s the leaves it depends
  on and adds the layer's own variants (`vpay_api::ApiError`,
  `vpay_worker::JobError`). Its `Classify` impl delegates to the leaf; it never
  re-classifies, so a `DbError` is `Storage` whether it surfaces through the API
  or the worker.
* **Boundary**: `anyhow` in `backends/apps/*` only, for `.context(..)` chains at
  startup; the HTTP envelope; the worker's retry policy; the process exit code.
  Boundaries consume `Classify`, they never re-invent it.

This crate knows nothing about HTTP frameworks or `tracing`, so the mappings are
plain data (`u16`, `&'static str`, small enums) that a boundary translates into
its own vocabulary. Adding a `Category` variant is an ADR-level change: the
`match`es are exhaustive by design, so every boundary is forced to decide what
the new category means for it.

`find_in_chain` is typed rather than dynamic because `dyn Error` cannot be
downcast to `dyn Classify`: a binary names the leaf types it knows how to
classify, in order of specificity, and anything else falls through to
`Category::Internal` — exit `1`, severity `Page`, which is the honest outcome
for an unclassified startup failure in a payment binary.

`source_chain` is what makes `#[source]` worth carrying. A leaf that flattens
its cause into its own message contributes nothing to it, which is precisely why
[ADR-0011's 2026-09-03 amendment](../adr/0011-error-modelling.md) stopped
`ProviderError::{Transport, Malformed}` doing that.

### Metric labels are the Debug spelling

`Category::as_metric_label` and `Severity::as_metric_label` return the `Debug`
spelling, deliberately. `vpay_error_events_total{category="Internal"}` has to be
joinable by eye with the JSON log line that produced it, and that line carries
`category` through `tracing`'s `?category` — i.e. `Debug`. A snake_case label
would read better in PromQL and would mean an operator pivoting from an alert to
the logs had to translate; worse, the two spellings could drift without anything
failing. `the_metric_label_is_the_debug_spelling` pins them together for every
variant.

Both are `const fn` over an exhaustive match, so a thirteenth category is a
compile error rather than a series that silently never appears.

---

## metrics

### Why a library describes but does not install

Installing a process-wide recorder is an *application* decision, the same one
`install_crypto_provider()` is in both `main.rs` files: a library that calls
`metrics::set_global_recorder` takes it out of the binary's hands and makes two
linked libraries a startup panic. So this module owns the vocabulary — the
names, their units, their help text — and each binary owns the exporter it
renders them through. `describe_all` is the one call that connects the two, and
it is a no-op until a recorder exists, which is why every caller runs it
immediately *after* installing one.

`describing_without_a_recorder_is_harmless` pins that: it is exactly what
happens in every test binary in this workspace that links `vpay-core` and
installs nothing.

### Why the names are `const`s and not string literals at call sites

A typo in a metric name is invisible: nothing fails, a dashboard is simply
empty, and the gap is discovered during the incident the dashboard existed for.
A `const` makes the typo a compile error. The same reasoning applies to
`job_outcome`'s, `webhook_outcome`'s and `provider_operation`'s label *values*,
which are closed vocabularies an alerting rule matches on exactly.

The module doc's `text` block is the specification
(`docs/plans/2026-09-03-step6-deployment.md` §3, transcribed verbatim), and
`the_module_doc_list_and_the_all_constant_agree` reads *that file* to prove
`ALL` matches it — so a metric added to the code without a line in the doc, or a
line with no metric, fails the build.

### HTTP request labels

`route` is the axum path *pattern* (`/v1/payment_intents/{id}`), never the
concrete path: a label whose cardinality grows with the number of payment
intents would eventually be the largest thing in the metrics store. A request
that matched no route carries `route="unmatched"`, which is a bounded label for
an unbounded set of paths.

`method` is bounded the same way: the ten methods `http::Method` names (`QUERY`
included) verbatim, anything else `method="other"`. A method is a free-form
token in RFC 9110 §9.1 and `http::Method` parses an unknown one rather than
rejecting it, so an unauthenticated caller sending `M12345` would otherwise mint
a series per request — the route label's hole, on the next label along.

The duration histogram is measured around the *inner* service, so it excludes
the request-id and trace layers above it and includes routing, authentication,
the handler and the error renderer. That is the span an operator can act on.

### Port calls, not wire requests

`vpay_provider_requests_total` counts calls to the `ProviderAdapter` port, not
HTTP requests to a rail. One `submit` on Orange Money mints an access token and
then posts the payment — two HTTP requests, one increment. A `submit` refused
before the socket is opened (a missing credential, a payer-less push charge) is
also one increment, with that refusal's `error_kind`. The question the metric
answers is "how are calls to this rail going", which is a port-level question;
`provider_requests` in Postgres is the per-attempt record.

The latency histogram is deliberately not labelled by `error_kind`: a histogram
split by every failure mode is mostly empty buckets, and "is this rail slow"
does not depend on why a call failed.

### Charge transitions are counted after commit

`vpay_charge_transitions_total` is emitted through
`vpay_db::charges::record_transition`, for the six statements that can move
`charges.state` and for nothing else — three in `vpay_db::charges`, three in
`vpay_db::settlement`. The database layer rather than the worker's settlement
points, because *every* transition passes through these functions and only some
of them pass through the worker: a confirm opens and submits a charge inside
`vpay-api`, and a metric mounted on the worker would silently miss it.

**Counted after the transition commits, never inside the transaction that made
it.** `vpay_db::settlement`'s three own their transaction and record after their
own `COMMIT`; `vpay_db::charges`' three run inside a *caller's* transaction, so
they return their row and the caller records after `tx.commit()`. Nothing inside
a transaction can know whether it will be committed, and a counter claiming a
charge that a `ROLLBACK` erased is worse than one that is a moment late.

Every label is read back off the row the database returned, never off the
caller's copy, so a transition that did not actually fire — a compare-and-swap
that matched nothing — cannot be counted. The one caveat is on `from` alone, in
`vpay_db::settlement::apply_succeeded`/`apply_failed`: those two read the
previous state through a sub-select in `RETURNING`, which sees the statement's
snapshot, so under a concurrent live-state move the label can name the rung the
charge was on a moment earlier. `to` and `provider` are exact in every case.

### The queue gauge goes negative

`vpay_jobs_oldest_claimable_age_seconds` is backed by
`vpay_db::jobs::oldest_runnable_run_at`, which is `SELECT min(run_at) FROM jobs
WHERE locked_at IS NULL AND run_at < 'infinity'` — every unleased, unparked row,
*including ones scheduled in the future*. So on a healthy idle deployment, whose
only queued work is the hourly `sweep_expired`, this reads about `-3500`: the
next job is nearly an hour away. Observed directly on `just demo`
(`vpay_jobs_oldest_claimable_age_seconds -540.01`).

The name is transcribed verbatim from the Step 6 design and is not changed, but
"age of the oldest claimable row" is the wrong reading of it. The right one is
**"seconds until (negative) or since (positive) the next piece of queued work
was due"**, which is the same quantity the worker's `queue_behind_seconds` log
field has carried since Step 4. A `> 300`-style alert is unaffected — it is the
positive tail that means a backlog — but a dashboard that renders this as an
"age" will show negative bars on a perfectly healthy queue, and a `min()`/`abs()`
applied to make that look tidier would hide exactly the case the metric exists
for.

It is **zero** when the queue holds nothing runnable, which is deliberately
*not* what the worker's `job loop gauge` log line does — that leaves the field
null, because "nothing to do" and "caught up to the second" are different facts.
A Prometheus gauge has no null: it holds its last value until something writes
another one. Leaving it unwritten on an empty queue would mean the value from the
last backlog stays on the series forever, and an alert thresholded on it would
page indefinitely after the backlog cleared. Zero is the lesser inaccuracy, and
it is the one that cannot invent an incident. It is left *unwritten* only when
the read itself failed, because then the answer is genuinely unknown; the worker
logs a warning in that case.

### Four job outcomes, not three

`docs/plans/2026-09-03-step6-deployment.md` §3 writes
`terminal|retry|dead_letter`; that list was written before Step 4 landed
`vpay_worker::Disposition::Lost` — the case where a worker's lease was reaped
mid-job and its answer thrown away. (Named in prose and not linked from the
rustdoc: `vpay-core` does not depend on `vpay-worker`, and it must not start
doing so for a doc link.) Folding `lost` into any of the other three would make a
real defect — a lease shorter than a handler — invisible, so it gets its own
value and this note rather than a quiet reconciliation.

### The git sha label

`option_env!` reads the environment **rustc was invoked with**, so
`VPAY_GIT_SHA=<sha> cargo build` bakes the value in with no code generation at
all. What that alone does not do is *rebuild*: cargo's fingerprint for the crate
does not include an environment variable it has never been told about, so
changing the sha and rebuilding would silently keep the old label.
`vpay-core/build.rs` exists for exactly one line —
`cargo::rerun-if-env-changed=VPAY_GIT_SHA` — which puts the variable in the
fingerprint. That is the whole mechanism, and it is why the build script emits no
`rustc-env` of its own: a value passed through twice can disagree with itself.

**It never shells out to `git`.** `backends/Dockerfile` builds in a context with
no `.git` directory (the image is `FROM scratch`; the build context is a `COPY`
of source trees), and a scratch or vendored build has no repository either. A
`git rev-parse` there either fails or — worse — succeeds against whatever tree
the build machine happens to be standing in, which is a sha that describes
nothing. `"unknown"` is the honest answer to "which commit is this" when nobody
told the build.

The label is therefore `unknown` on every local `cargo build` and every
`just demo`, and carries a real sha only where something passes one:
`backends/Dockerfile`'s `ARG VPAY_GIT_SHA` and `release.yml`'s
`build-args: VPAY_GIT_SHA=${{ github.sha }}`. The end-to-end half of that is
manual, and this is what it looks like:

```text
VPAY_GIT_SHA=deadbeef cargo build -p vpay-server
./target/debug/vpay-server … & curl -s localhost:9090/metrics | grep build_info
# vpay_build_info{version="0.1.0",git_sha="deadbeef"} 1
```

### The alert-events gap

`record_error_event` is one function rather than a macro call at each logging
site because the two counters must never disagree about what a page is, and
neither may disagree with the `alert = true` field an alerting rule reads out of
the JSON logs. Written twice, they would drift the first time someone added a
severity arm.

Three call sites, each the point where an error is logged *at its own
classification*: `vpay_api::ApiError::log`,
`vpay_worker::handlers::log_failure`, and the job loop's "the job queue is not
answering" arm.

Four other log lines in this workspace carry `alert = true` and do **not**
increment these counters, which is a real gap and is recorded rather than
papered over:

* `vpay_worker::run_loop::log_disposition` — it re-reports a failure
  `log_failure` has already counted, and at a *wider* severity net, so counting
  it would double some incidents and add others with no classification to label
  them with;
* the seed-singletons, release-leases and settlement-contradiction lines, which
  flag `alert = true` unconditionally and carry no `Classify` value to derive
  `category`/`code` from.

So `increase(vpay_alert_events_total)` is a *subset* of "log lines with
`alert = true`", not the whole of it. Closing that gap means giving the worker's
ad-hoc alerts a classified error to carry, which is a change to the worker's
error model rather than to `record_error_event`.

---

## Status

This page is documentation of code that exists. Every claim on it is either
covered by a test named in the text, by a doctest in the item it describes, or
marked as manual (`the git sha label`'s end-to-end check). It describes no
unbuilt behaviour; [docs/status.md](../status.md) remains the record of what is
and is not built.
