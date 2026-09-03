# Restore from backup, and the quarterly drill

**Nobody has done this, and there is nothing to do it to.** No vpay database
has ever been backed up, no backup exists, and no restore has ever been
performed. [ADR-0013](../adr/0013-database-backups-and-retention.md) is
**proposed**, not implemented. This runbook is written so that the drill it
describes can be run the first time somebody has a backup to run it against.

What *has* been done, on 2026-09-03: **every SQL statement below was executed
against a scratch `postgres:16-alpine` with all 21 migrations applied**, on a
fixture built to contain one torn ledger transaction. The queries run, and
§4's ledger check found the torn transaction and then reported clean once it
was repaired — so it is checking the data and not itself. That is the whole
of the evidence. See §7.

---

## 1. When to use this

| Situation | Use this runbook? |
|---|---|
| Data loss or corruption in the primary database | Yes — §3 onward, against a scratch database first |
| The quarterly drill ADR-0013 §6 requires | Yes — §2, then §4, then §6 |
| A rolled-back deploy | **No.** Migrations do not roll back; see [release.md](release.md) §5 |
| A retired signing key crash-looping the server | **No.** See [rotate-signing-key.md](rotate-signing-key.md) |

## 2. Restore into a scratch database. Always.

**Never restore over a live database.** Restore to a *new* instance or a new
database name, verify it with §4, and only then decide whether to cut over.
A restore that turns out to be to the wrong instant, over the live data, is
the failure this rule exists to prevent — and vpay has no down-migrations to
undo it with.

The restore command itself belongs to whoever operates the database, and is
deliberately not written here:

- **A managed provider**: its own PITR restore, which produces a *new*
  instance. Restoring in place is the option not to take.
- **CloudNativePG**: a `Cluster` with a `bootstrap.recovery` stanza naming
  the `barmanObjectStore` and a `recoveryTarget` — a new `Cluster`, not the
  existing one.

Pick the target instant deliberately. ADR-0013 proposes an RPO of ≤ 5 minutes,
so the instant you want is usually "immediately before the event", not
"latest".

## 3. Before you connect anything to it

**Both binaries run migrations at boot** (`vpay_db::run_migrations`). A
restored database is already at the schema version its snapshot was taken at;
starting a *newer* image against it migrates it forward, in place, with no
way back. Decide the image version before you start anything, and pin it by
digest ([release.md](release.md) §4).

**Do not start the worker yet.** ADR-0013 §5: restored `jobs` rows are work
that may already have run. `poll_charge` and `scan_live_charges` re-run
harmlessly — they ask a rail for status. `resubmit_charge` is a call that can
move money. Start the server first, check §4 and §5, then the worker.

## 4. The two assertions

Run these against the **restored scratch database**. Both are "expect zero
rows"; anything else means the restore is torn and must not be promoted.

### 4a. Every ledger transaction balances, per currency

```sql
SELECT t.id AS transaction_id,
       e.currency_code,
       SUM(e.amount) FILTER (WHERE e.direction = 'debit')  AS debits,
       SUM(e.amount) FILTER (WHERE e.direction = 'credit') AS credits
FROM ledger_transactions t
JOIN ledger_entries e ON e.transaction_id = t.id
GROUP BY t.id, e.currency_code
HAVING COALESCE(SUM(e.amount) FILTER (WHERE e.direction = 'debit'), 0)
    <> COALESCE(SUM(e.amount) FILTER (WHERE e.direction = 'credit'), 0)
ORDER BY t.id;
```

A transaction whose entries do not balance is one the restore cut in half.
Migration `0005` explains why no database constraint catches this: the
invariant is an aggregate over several rows, and a row-level `CHECK`
evaluates one row at a time. This query is the only thing that checks it on
restored data.

And the transactions that lost *all* their entries, which the query above
cannot see because it joins:

```sql
SELECT t.id, t.charge_id, t.created_at
FROM ledger_transactions t
LEFT JOIN ledger_entries e ON e.transaction_id = t.id
WHERE e.id IS NULL
ORDER BY t.created_at;
```

**What this does not check.** ADR-0013 says it and it bears repeating here:
`account_kind` has no per-merchant dimension (migration `0005`'s own GAP
note), so "balance(`merchant_payable`) per merchant" is not computable from
these tables. Per-transaction balance is what the schema supports checking,
and it is what this drill claims.

**Both tables are empty on any database restored today**, because no code in
this repository posts to the ledger. The queries are correct and they will
find nothing, which is not the same as passing. Re-read this section the day
ledger persistence lands.

### 4b. `one_charge_per_intent` holds, and still fires

Present:

```sql
SELECT indexdef FROM pg_indexes
WHERE schemaname = 'public' AND indexname = 'one_charge_per_intent';
```

Not violated:

```sql
SELECT payment_intent_id, COUNT(*) AS charges
FROM charges GROUP BY payment_intent_id HAVING COUNT(*) > 1
ORDER BY payment_intent_id;
```

And — the part worth doing, because the two queries above pass on a database
whose index was silently dropped by a restore that rebuilt the table —
**prove it still refuses**, inside a transaction you roll back:

```sql
BEGIN;
INSERT INTO charges (id, payment_intent_id, provider_code, provider_reference_id,
                     state, amount, currency_code)
SELECT 'ch_restore_probe', c.payment_intent_id, c.provider_code, gen_random_uuid(),
       'submitting', c.amount, c.currency_code
FROM charges c LIMIT 1;
ROLLBACK;
```

The expected outcome is an **error**:

```
ERROR:  duplicate key value violates unique constraint "one_charge_per_intent"
DETAIL:  Key (payment_intent_id)=(...) already exists.
```

A successful `INSERT` here is the finding. `ROLLBACK` either way.

## 5. What the restore leaves you to clean up

None of these are errors. They are consequences of a correct restore, and
nothing reports them.

| What | Query | What to do |
|---|---|---|
| **Charges left mid-flight** | `SELECT state, COUNT(*) FROM charges WHERE state IN ('submitting','submitted','pending','unresolved') GROUP BY state;` | These are payments whose outcome the snapshot does not know. The poll ladder will chase them once the worker starts |
| **Rail calls with no answer recorded** | `SELECT charge_id, provider_reference_id, operation, attempt, sent_at FROM provider_requests WHERE status_code IS NULL ORDER BY sent_at;` | The ambiguous set: the rail may have acted. [unresolved-charges.md](unresolved-charges.md) is the procedure, and `provider_reference_id` is what you search the rail by |
| **Jobs that will re-run** | `SELECT kind, dedupe_key, attempts, run_at FROM jobs WHERE run_at <= now() AND locked_at IS NULL ORDER BY run_at;` | Read this list **before** starting the worker. `resubmit_charge` rows are the ones to think about (§3) |
| **A revoked client is un-revoked** | `SELECT client_id, disabled_at, reason FROM disabled_clients ORDER BY disabled_at;` | Compare against why each was disabled. A revocation made after the restore instant is **gone**, and the client is live again — see [rotate-rail-credentials.md](rotate-rail-credentials.md) §3 |
| **The replay window re-opened** | `SELECT COUNT(*) FROM oauth_client_assertion_jtis WHERE expires_at > now();` | Nothing to run; a fact to know. Assertions spent after the restore instant are spendable again until their own `exp` passes (ADR-0013). Merchant assertion lifetimes are short, so this closes on its own — it is not detectable while it is open |
| **Which signing key the snapshot believes is active** | `SELECT kid, active, expires_at FROM oauth_signing_keys ORDER BY created_at;` | Must agree with the Secret you are about to mount. §6 |

Also: `idempotency_keys` older than the restore instant are gone with it, so a
merchant retrying a `POST /v1` from before the snapshot is treated as a first
attempt, not a replay. `one_charge_per_intent` still stops a second charge on
the same intent; a second *intent* is a second payment.

## 6. The signing key is the second restore input

A restore needs the database **and** the Kubernetes Secret holding the RSA
private PEM. `oauth_signing_keys` holds public halves only (migration `0010`).

Compare the `kid` in §5's last query against the key in the Secret you intend
to mount. The `kid` is the RFC 7638 thumbprint of the public JWK — a function
of the key itself, not of the file or the process — so this is a real
comparison, not a naming convention.

- **They agree** → boot normally.
- **The Secret's key is newer** → `ensure_active_signing_key` rotates the
  restored record on boot. Fine.
- **The Secret's key is one the restored database has already retired** →
  `vpay-server` **exits 78** with `DbError::SigningKeyRetired`, naming the
  `kid` and its retirement instant. It is a crash loop and it is deliberate.
  See [rotate-signing-key.md](rotate-signing-key.md).

## 7. The drill

Quarterly, per ADR-0013 §6.

1. Restore the most recent backup to a scratch database (§2).
2. Run §4a and §4b. Record what they returned.
3. Run §5's queries. Record the counts — they are what a real restore's
   follow-up work would be.
4. Confirm you can name the signing-key Secret that was current at the
   backup instant (§6). If nobody can, that is the drill's finding.
5. **Record the wall-clock time from decision to a verified database.** That
   number is what makes ADR-0013's proposed RTO of ≤ 60 minutes real or
   false.
6. Destroy the scratch database.

### What was actually run, 2026-09-03

Not a drill — there was no backup. A scratch `postgres:16-alpine` container
with **all 21 migrations applied in order**, loaded with a fixture holding two
charges on two intents, one balanced ledger transaction (5000 debit
`payer_clearing`; 4900 credit `merchant_payable` + 100 credit
`platform_fee_revenue`), one deliberately **torn** transaction (3000 debit,
2900 credit — the shape a restore to the wrong instant produces), one
unanswered `provider_requests` row and one overdue `poll_charge` job.

```
== A1. Ledger balance, per transaction per currency (expect ZERO rows) ==
 transaction_id | currency_code | debits | credits
----------------+---------------+--------+---------
 ltx_torn       | XAF           |   3000 |    2900
(1 row)
```

After inserting the missing 100 credit leg, the same query returned `(0 rows)`.
That is the negative control: the check fails on torn data and passes on
whole data, so it is reading the rows and not asserting itself.

```
== B1. one_charge_per_intent index is present (expect exactly one row) ==
 CREATE UNIQUE INDEX one_charge_per_intent ON public.charges USING btree (payment_intent_id)
(1 row)

== B2. Intents carrying more than one charge (expect ZERO rows) ==
(0 rows)

== B3. the duplicate-charge probe (§4b) ==
BEGIN
ERROR:  duplicate key value violates unique constraint "one_charge_per_intent"
DETAIL:  Key (payment_intent_id)=(pi_ok) already exists.
ROLLBACK
```

§5's queries returned one `pending` charge, one `provider_requests` row with
`status_code IS NULL`, one claimable `poll_charge` job, and no signing keys.
The `jobs` dedupe bound in ADR-0013 §5 was checked the same way: re-enqueueing
`poll:ch_two` with `ON CONFLICT (dedupe_key) DO NOTHING` reported
`INSERT 0 0`, leaving exactly one row — `jobs_dedupe_key` is a real unique
index on the real schema.

**What that proves and what it does not.** It proves the SQL in this runbook
is valid against the schema these migrations produce, and that §4a
distinguishes torn data from whole data. It proves **nothing** about a
backup, a PITR restore, a provider's tooling, the RTO, or any step in §2, §3
or §6 — none of which was exercised, because none of it exists.
