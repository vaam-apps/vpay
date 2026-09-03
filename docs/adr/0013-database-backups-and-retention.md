# ADR-0013: Database backups, PITR and retention

- **Status:** **Proposed.** Nothing here is implemented. **No backup of any
  vpay database has ever been taken**, no restore has ever been performed,
  and no restore drill has ever run. This ADR records obligations the schema
  already creates and proposes a policy to meet them; every number in it is
  proposed, not measured.
- **Date:** 2026-09-03
- **Deciders:** vpay maintainers

## Context

There is no vpay deployment. Every database this repository has ever written
to has been an ephemeral testcontainer or a developer's compose stack, and
[docs/status.md](../status.md) says so at the top. Backup policy for a system
that has never held a real row could reasonably be deferred.

It is written down now for one reason: **the schema already commits vpay to
holding things whose loss cannot be recovered from anywhere else.** Some of
them are money, some of them are the only evidence an operator would have
about money, and one of them — replay protection — is a security control
whose loss is not visible in any dashboard. The obligations exist as soon as
the first real row does, which is before anyone will think to write this
document.

[Step-6 decision (9)](../plans/2026-09-03-step6-deployment.md) put Postgres
outside the Helm chart: `DATABASE_URL` comes from an existing Secret, and
`deploy/helm/vpay` templates no database. That decision is what makes this
ADR necessary in this shape — it can state obligations it cannot enforce,
because the machinery that would meet them belongs to whoever operates the
database, not to this repository.

### The obligations the schema has already created

Read from the migrations, not from the design documents. "Written by code
today" distinguishes a table that holds real rows from one that is schema
only — losing an empty table costs nothing *yet*, and that changes the day
something writes to it, not the day this ADR is revised.

*This ADR's own status is still **Proposed** and unreleased on this branch,
so the table below is updated in place as migrations land rather than
superseded — an **accepted** ADR would be superseded by a new one instead of
having its table edited.*

| Table | Migration | What it holds | Written by code today | What losing it costs |
|---|---|---|---|---|
| `payment_intents` | `0003`, `0014` | The merchant-facing object: amount, currency, status, `merchant_id`, `metadata`, and `seq` — the pagination order `GET /v1/payment_intents` walks | Yes | The merchant's own record of what they asked for. A merchant retrying against a restored database gets a `404` for an intent they hold an id for |
| `charges` | `0004`, `0014`, `0019`, `0021` | One attempt against one rail: `provider_reference_id` (the rail-facing idempotency key), state, `failure_code`/`failure_raw`, `provider_txn_id` | Yes | **The rail's reference.** Without it there is nothing to search a rail's dashboard or settlement statement by, so a payer who was debited cannot be matched to an intent at all. The unique index `one_charge_per_intent` is also what stops a second charge on an intent — see the restore caveat below |
| `provider_requests` | `0016`, `0020` | One row per *attempt* to call a rail: operation, attempt number, `status_code` (with `NULL` = no answer and `0` = answered without an HTTP status), `error_kind`, `sent_at`/`responded_at` | Yes | The only diagnosis path there is. [ADR-0004](0004-musl-mimalloc.md) put no shell in the runtime image precisely because "diagnosis happens through logs, traces and the `provider_requests` audit trail". Losing it deletes step 2 of [runbooks/unresolved-charges.md](../runbooks/unresolved-charges.md): whether the rail was ever asked, and whether it ever answered, becomes unanswerable |
| `oauth_client_assertion_jtis` | `0011` | Spent `private_key_jwt` assertion ids, with `expires_at` | Yes | **A replay window re-opens.** The primary key *is* the single-use guard (RFC 7523 §3 point 7). Restoring to an instant before an assertion was recorded makes every captured assertion whose `exp` has not yet passed spendable again, and nothing anywhere reports that it happened |
| `idempotency_keys` | `0015`, `0025` | The claim, request hash and stored response for every `POST /v1`, expiring after 24 h. `0025` adds `response_retry` — the verbatim `stripe-should-retry` value the stored response carried (or `NULL` if it carried none), written once by `store` and re-emitted unchanged by `replay` so the advisory is derived from `Classify::retry` in exactly one place (ADR-0011) | Yes | A merchant's retry after a restore is treated as a first attempt instead of a replay, so the work is re-executed rather than the stored response returned. Even a restore consistent enough to keep the stored response can lose `response_retry` — it reverts to `NULL`, its "unknown" state — and a replay of a stored refusal then emits no retry advisory, so a client that would otherwise know not to retry gets none |
| `oauth_signing_keys` | `0007`, reshaped by `0010` | **Public halves only** — `kid`, `public_jwk`, `active`, `expires_at`. Migration `0010` dropped `private_key_pem` deliberately; no private key material exists in any table | Yes | `/v1/oauth/jwks.json` loses the retired keys still inside their overlap window, so tokens already issued stop verifying. See "two restore inputs" below — this row is *half* of a restore |
| `disabled_clients` | `0012` | The operator kill switch: `client_id`, `disabled_at`, `reason`. Only ever subtracts access ([ADR-0010](0010-merchant-auth-private-key-jwt.md)) | Yes (read on every token request) | **A restore silently re-enables a revoked client.** Restoring to an instant before an `INSERT` here undoes a revocation that was deliberately made without a deploy, and nothing about a booting server looks wrong |
| `jobs` | `0021`, `0022`, `0023` | The worker's durable queue, enrolled in the same transaction as the write that produced the work. The closed `kind` vocabulary (`kind_is_known`) grew from four housekeeping/poll kinds at `0021` to include the webhook machinery: `fan_out_events` and `deliver_webhook` (`0022`), then `scan_deliveries` — the backstop that re-enqueues a `deliver_webhook` job whose own row was deleted or lost, but not one that was dead-lettered (`0023`) | Yes | Work in flight at the restore instant, including a queued `fan_out_events`, `deliver_webhook` or `scan_deliveries` job. See the re-run caveat below — this table is the one where restoring *too much* is the hazard, not restoring too little |
| `ledger_transactions`, `ledger_entries` | `0005` | The intended persistence shape for `vpay_ledger::Transaction`/`Entry` | **No** — no SQLx query in this repository writes to either table | Nothing today. Once posting exists, this is the record of what is owed to whom, and it is the one place a double-entry answer can be reconstructed from |
| `refunds` | `0017` | Refund objects | **No** | Nothing today |
| `events` | `0018`, `0024` | The event log behind webhook fan-out and `GET /v1/events`; `seq` is the delivery cursor. `0024` adds `fanout_attempts` (how many fan-out passes have failed on this event) and a third `fanout_state`, `failed`, set once `vpay_worker::webhooks::FANOUT_MAX_ATTEMPTS` (5) passes have failed on the row | **Partly** — the settlement transaction writes `payment_intent.succeeded` and `payment_intent.payment_failed`; the other five types are emitted by nothing. `/v1/events` is routed (`vpay_api::v1::events::list`/`retrieve`), and the fan-out drain does mark a row `fanout_state = 'done'` on success (`webhook_deliveries::mark_fanned_out_in_tx`) — `fanout_attempts` and `fanout_state = 'failed'` are written by `events::record_fanout_failure` | The `data` column is a snapshot of the object **as it was at emit time**, so it cannot be regenerated from current rows. Every unfanned-out row lost is a webhook a merchant will never receive, with nothing to notice it by. Losing `fanout_attempts` or a `fanout_state = 'failed'` row **re-arms an event the drain had deliberately abandoned**: restored to `pending`, it re-enters `events::pending_page` and the drain retries an event that had already exhausted its attempts and been alerted on once — the exact "one poisoned event blocks every page behind it" failure `0024` exists to stop |
| `webhook_deliveries` | `0022` | One row per (event, endpoint): the delivery record behind `docs/flows/webhooks.md` — `attempt`, `state`, `status_code`, `response_excerpt`, `sent_at`/`responded_at`, `next_attempt_at` (the retry-ladder position) and `payload_sha256` (the digest of the first attempt that rendered and signed a body). Created by the `fan_out_events` drain in the same transaction that marks the event `fanout_state = 'done'`; every column but `created_at` is overwritten by the row's most recent attempt — this is a *state* row, not an append-only attempt log | Yes — `create_in_tx`, `record_attempt`, `record_success` and `mark_fanned_out_in_tx` (`vpay_db::webhook_deliveries`); read by `pending_due`, whose only caller is the `scan_deliveries` backstop job (`0023`) | A receiver's delivery history and its position on the retry ladder — the `attempt` count and `next_attempt_at`. `events.data` still holds the snapshot each delivery was built from, so replaying a lost delivery by hand from `events` is possible; what a restore cannot recover is which attempts already went out. Losing that risks delivering a webhook the merchant already received — the exact duplicate `webhook_deliveries_event_endpoint`'s unique index exists to prevent — and nothing rebuilds that history automatically |

Two entries in the ledger row deserve their own sentence, because they limit
what any restore check can prove. Migration `0005` states both: the
per-transaction balance invariant is *aggregate* and no row-level `CHECK` can
express it, and `account_kind` has **no per-merchant dimension**, so
"balance(`merchant_payable`) per merchant" cannot be computed from these
tables at all. A restore can therefore verify that each transaction balances
(the drill does, see below); it cannot verify per-merchant balances, and
saying otherwise would be inventing evidence.

## Decision

**1. Recovery objectives — proposed, not measured.**

| Objective | Proposed value | What it means |
|---|---|---|
| RPO | **≤ 5 minutes** | At most five minutes of committed writes may be lost |
| RTO | **≤ 60 minutes** | At most sixty minutes from the decision to restore to serving traffic again |

Both are proposed. Neither has been measured against anything, because
nothing has been restored. The RTO in particular is a guess about a procedure
[runbooks/restore-from-backup.md](../runbooks/restore-from-backup.md)
describes and nobody has followed; the first real drill is what turns it into
a number. If a drill shows 60 minutes is not achievable, the honest response
is to change this ADR, not to leave the number and quietly miss it.

**2. Continuous WAL archiving with point-in-time recovery.** Nightly
`pg_dump` alone cannot meet a 5-minute RPO and is not an acceptable
substitute. Two implementations are sanctioned, and which one applies is a
property of the deployment, not of this repository:

- **A managed Postgres provider's own PITR.** The default, following step-6
  decision (9). The provider owns WAL archiving, base backups and the restore
  path.
- **CloudNativePG's `barmanObjectStore`**, for an in-cluster database. CNPG
  is documented in `deploy/helm/vpay/README.md` and deliberately **not**
  templated by the chart, for exactly the reason this ADR exists: a chart
  that templates a database implies it owns the obligations above.

**3. Retention — proposed.** A **30-day PITR window** (WAL retained so any
instant in the last 30 days is recoverable) and **90-day full-backup
retention**. 30 days is chosen to cover a rail's settlement and dispute
cycle, which is the longest window in which someone credibly asks "what did
this charge do"; 90 days is chosen to survive a corruption that is not
noticed inside the first month. Neither has been validated against a legal or
scheme requirement, and no such requirement has been identified for XAF mobile
money in Cameroon — that is a gap in this ADR, not a finding of "none apply".

**4. A restore needs two inputs, from two custodians.** This is the one
consequence of [ADR-0010](0010-merchant-auth-private-key-jwt.md) and migration
`0010` that is easy to miss until it is being missed at 3am:

- the **database backup**, and
- the **signing-key Secret** holding the RSA private PEM.

`oauth_signing_keys` stores public halves only. A restored database that
believes `kid_A` is active, plus a Secret holding the PEM for `kid_B`, is not
a degraded boot: `vpay-server` **exits 78** if `kid_A` has since been retired
(`DbError::SigningKeyRetired`), and rotates the database record if it has
not. A backup schedule that does not also record which signing-key Secret was
current at the backup instant has backed up half the system. Whoever holds
the Secret and whoever holds the backup may be different people; the restore
runbook assumes they are.

**5. The `jobs` re-run caveat is bounded by `dedupe_key`, not eliminated by
it.** Restoring `jobs` to an earlier instant restores work that has since
completed, and the next worker to boot will claim and run it again. The
`jobs_dedupe_key` unique index means a work item cannot exist twice — a
re-enqueue of `poll:<charge_id>` collapses onto the restored row with
`ON CONFLICT DO NOTHING`, verified against a real database in the restore
drill — but it says nothing about whether that row has already been *run*.
`poll_charge` and `scan_live_charges` are safe to re-run: they query a rail
for status and are read-only against it. `resubmit_charge` is the one to
think about before starting a worker against a restored database, because a
resubmit is a call that can move money. The restore runbook therefore starts
the server before the worker, deliberately.

**6. A quarterly restore drill**, following
[runbooks/restore-from-backup.md](../runbooks/restore-from-backup.md):
restore into a **scratch** database, never over a live one, and assert two
properties that a torn or wrongly-timed restore breaks — that every ledger
transaction balances per currency, and that `one_charge_per_intent` holds and
still fires. The drill records the wall-clock time it took, which is how the
RTO above stops being a guess.

## Consequences

**This ADR states obligations it cannot enforce.** Nothing in this repository
takes a backup, checks that one was taken, or fails if none exists. There is
no CI job that could: the database is external by decision (9). The only
mechanism here is documentation, and documentation does not page anyone.

**"No backup has ever been taken" is a true statement about this project on
2026-09-03**, and it stays true until somebody deploys a database and
configures a provider. If you are reading this next to a running vpay, the
first thing to check is whether that is still true.

**The obligations table will drift.** It was read off migrations `0001`–`0025`
on 2026-09-03. Adding a table without adding a row here makes this document
quietly wrong; there is no `xtask` check for it, and adding one was not in
scope for this step.

**Restoring is not neutral for security or for money.** Two of the rows above
are consequences a well-executed restore *causes* rather than prevents: the
assertion-replay window re-opens, and a revoked client is re-enabled. Both
are listed in the restore runbook as steps to take afterwards, and neither is
detectable from the outside.

**The retention numbers are unanchored.** 30/90 days is a defensible starting
point and nothing more. A real requirement — a scheme rule, a regulator, a
merchant contract — should replace them, and should supersede this ADR rather
than edit it.

## What this ADR does not decide

- **Where backups are stored, and in which jurisdiction.** An object store in
  the wrong region is a compliance problem this document does not address.
- **Encryption at rest for the backup itself**, and who holds that key. A
  backup of a payments database is as sensitive as the database.
- **Whether the signing-key Secret is backed up at all, and by whom.** §4
  says a restore needs it; it does not say who keeps a copy, and a Secret
  that exists only in one cluster's etcd is not backed up. That is a
  maintainer decision, not one to default.
- **How long an incident may run before restoring is the right call.** The
  RTO says how long a restore takes, not when to start one.
