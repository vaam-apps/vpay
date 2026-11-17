# Runbook: personal data in telemetry

**What this page is.** The operator's statement of which personal data a
running vpay can put into its telemetry — logs, metrics, traces, error
responses, webhook payloads, and the diagnostic columns that sit beside them
in Postgres — in what form, and who owns its retention. It is the runbook
[RFC-0002](../rfc/0002-gdpr-policy-and-operator-decisions.md)'s PR 4 names for
[issue #147](https://github.com/vaam-apps/vpay/issues/147).

**What it is not.** A retention policy. Every retention period and every
retention owner on this page is either a fact about where vpay stops (vpay
does not operate your log aggregator, your Prometheus or your backups) or an
**open decision**, written as "undecided (RFC-0002 Dn)". Nothing here was
decided by writing it down, and no period on this page is a commitment. The
engineering inventory's `retention:` and `owner:` labels
([schemas/privacy-inventory.yaml](../../schemas/privacy-inventory.yaml)) are
classification labels for the gate, **not** approved periods or owners — RFC-0002
D1 and D9 are open.

**Status.** Written 2026-11-17 from the code at that date, after the
telemetry sweep recorded in
[status/verification/2026-11-17-telemetry-sweep.md](../status/verification/2026-11-17-telemetry-sweep.md),
which lists every `tracing` site read, the ones changed and the ones left with
the reason. **Never followed against a deployment** — no deployment exists
([../status.md](../status.md)). The webhook payload minimisation (RFC-0002 PR
5, blocked on D2) and the webhook URL policy (D4) are **not built**; their
consequences are stated below rather than hidden.

## The rule the code follows

Correlation is by vpay's own opaque identifiers, which are logged raw:
`pi_…`, `ch_…`, `evt_…`, `re_…`, `cs_…`, `cus_…`, `in_…`, job and delivery
UUIDs, merchant and client ids, `stf_…` staff ids, the `x-request-id` request
id, and the rail references vpay generated (`provider_reference_id`,
`provider_txn_id`; the inventory's `sys_provider_ref`, `subject: none`). An
operator starts from one of those and reads the row it names; the row is
where the personal data lives, under the erasure and retention controls
the inventory records.

Everything that can name a payer, an account holder or a staff member, and
every credential, is either never emitted or emitted in one of the fixed
representations below.

## Surface by surface

### Logs (stdout/stderr, `tracing-subscriber`)

| Data                                                                            | Can it appear?             | Form                                                                                                                                                                                                                                                                                                                           |
| ------------------------------------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Payer / payee MSISDN                                                            | **masked, three lines**    | `+2376••••NNN` (country code, the leading `6`, four fixed bullets, last three digits) on exactly three `info`/`warn` lines: `account-holder lookup served`, `account-holder lookup failed at the rail` and `instructing a rail to return money`. Nowhere else.                                                                 |
| Account-holder name                                                             | no                         | —                                                                                                                                                                                                                                                                                                                              |
| Customer name, email, phone, address, GPS point                                 | no                         | hand-written `Debug` on every row type that holds one (`debug_protections`)                                                                                                                                                                                                                                                    |
| Rail failure text (`rail_failure_text` — the rail's own words for a decline)    | no                         | dropped from `ProviderError`'s `Display` on 2026-11-17; kept in `charges.failure_raw` / `refunds.failure_raw`. A decline's line carries the failure code and, when the adapter wrote one, its **own** sentence — `rail rejected the request: provider_account_blocked — mtn_momo: the rail refused our credentials (HTTP 403)` |
| Rail request URL (MTN's account-holder URL carries the MSISDN in its path)      | **origin only**            | `scheme://host[:port]/` in an error's source chain (`vpay_provider::http::redact_url`); `SafeUrl(scheme://host)` in a `Debug`                                                                                                                                                                                                  |
| A rail body serde could not decode                                              | **position only**          | `a data error, value withheld at line L column C`, or `missing field \`x\`` (a field name of vpay's own wire struct)                                                                                                                                                                                                           |
| A rail's non-2xx error body                                                     | **documented fields only** | the error code when it has a code's shape (`vpay_provider::http::rail_token`: 1–64 chars of `[A-Za-z0-9_.-]`, at most four digits), Orange's `message` only when it is an HTTP reason phrase (`error code 503 (Service Unavailable)`), and the body's length; never a `description` or any other free text                     |
| A rail's `base_url` in an adapter configuration error                           | **origin only**            | `SafeUrl(…)`; an unparseable one as `SafeUrl([unparsed])`                                                                                                                                                                                                                                                                      |
| Merchant webhook endpoint URL                                                   | **origin only**            | `SafeUrl(scheme://host[:port])` on the `exhausted the retry ladder` alert; the endpoint id beside it finds the configuration                                                                                                                                                                                                   |
| Webhook endpoint URL refused **at boot** (livemode, non-`https` or a stub host) | **yes, as written**        | `ConfigError::InsecureHost` / `StubHostInLivemode` print the URL the operator wrote, query included — see "Known gaps"                                                                                                                                                                                                         |
| vpay's own callback URL                                                         | yes                        | in full, on the callback-registration lines; it is the deployment's public address, not a credential and not personal                                                                                                                                                                                                          |
| Staff member                                                                    | id only                    | `stf_…`; never an email, a name, a password, a TOTP code or a token                                                                                                                                                                                                                                                            |
| Tokens, client secrets, signing keys, webhook signing secrets, rail credentials | no                         | `Secret` (`[redacted]`) in every registered `Debug`; never a `tracing` field                                                                                                                                                                                                                                                   |
| Publishable key (`pk_…`)                                                        | yes                        | public by design (it ships in the payer's browser); the checkout-session route bounds its length first, because the unauthenticated caller chose it                                                                                                                                                                            |
| Idempotency key                                                                 | 8-character hint           | `key_hint`                                                                                                                                                                                                                                                                                                                     |
| Request path                                                                    | yes, path only             | the `request` span records `method`, `path` (never the query string — the account-holder `msisdn` is a query parameter) and `request_id`                                                                                                                                                                                       |

**Whether the masked MSISDN may stay in logs is open.** The mask predates
RFC-0002's register: it is
[flows/account-holder-lookup.md](../flows/account-holder-lookup.md) § 3's rule,
the shape `charges.payer_ref_masked` is documented to hold. RFC-0002 **D3**
decides, per surface, whether an identifier is permitted raw, masked,
pseudonymised or forbidden, and it has not been answered. Until it is, these
three lines are the only place a payer's number appears in a log, in this form
only; they are not an approved disclosure. `vpay_core::privacy::Masked` and
`Pseudonymous` have no shipping consumer, for the same reason.

**Retention and owner of the log stream: undecided (RFC-0002 D13).** vpay
writes to stdout/stderr and keeps nothing; the aggregator, its retention, its
access control and its jurisdiction are the deployment's. Until D13 names an
owner, treat the log store as holding the masked numbers and ids above for as
long as the aggregator keeps anything.

### Metrics (`/metrics` on `--observability-bind`)

**No personal data.** Every label is drawn from a closed vocabulary: the
matched route **template** (`/v1/payment_intents/{id}`, never a concrete id),
the method, the status, a rail code from configuration, an outcome or verdict
constant, a job kind, an error category/code/severity, a charge state, and the
build's version and git sha. `vpay_account_holder_lookups_total` has one label,
`outcome`, with four values;
`every_outcome_is_counted_and_no_label_carries_the_number_or_the_name` renders
it through the shipping exporter and asserts no number or name appears.

**Retention and owner of the metric store: undecided (RFC-0002 D13).**

### Traces

**None leave the process.** There is no OTLP exporter
([../status.md](../status.md): deferred). Spans exist only as context on log
lines — the `request` span above — so everything said under Logs applies.

### Error responses (`/v1`, `/v1/browser`, `/dash/v1`, `/svc/v1`)

The envelope's `message` is `Classify::public_message`, never `Display`. It
never carries rail text, an MSISDN, a holder's name or a credential:

- a decline renders `The payment was declined (<failure_code>).`;
- a rail that could not be reached renders its category's generic sentence;
- a phone or MSISDN that does not validate is refused with a fixed sentence
  naming the **parameter**, never the value
  (`malformed_wrong_region_and_non_mobile_numbers_are_all_refused_without_an_echo`);
- an unknown id echoes the id the caller sent (bounded), as Stripe does; an
  idempotency key is never echoed past its hint.

What the merchant's own systems do with a response is the merchant's.

### Webhook payloads

**The payload is the complete rendered API object of the event's type** —
`events.data`, the inventory's `stored_api_body` — so a `customer.*` event
carries the customer's identifiers and a `payment_intent.*` event what the
intent object renders. That is **not** minimised: per-event projections are
RFC-0002 PR 5, blocked on **D2** (the minimum field set per event family),
and this page does not decide D2. Customer erasure rewrites the stored copies
([flows/customers.md](../flows/customers.md)); it cannot reach a copy a
merchant's endpoint already received.

The endpoint URL itself is stored in full in `webhook_deliveries.url` and in
the configuration. Whether a URL may carry a query string or fragment at all
is **D4**, open; until it is answered vpay accepts one and therefore stores
it. What changed on 2026-11-17 is only that vpay no longer **repeats** it in a
log line or a failure excerpt.

**Retention of `events` and `webhook_deliveries`: undecided (RFC-0002 D9).**

### Diagnostic columns in Postgres

These are telemetry that lives in the database, so they are listed here too:

| Column                                       | What it can hold                                                                                                                                                                                                                                                             | Inventory element                                                                                     |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `jobs.last_error`                            | a job failure's `Display` and source chain. Since 2026-11-17: no rail text and no rail URL beyond its origin                                                                                                                                                                 | `sys_job` (`subject: none`)                                                                           |
| `webhook_deliveries.response_excerpt`        | either the first bytes of **the merchant receiver's own answer** (whatever it chose to write), or, when nothing answered, `no response: <cause>` — since 2026-11-17 naming the endpoint's origin only, or the egress guard's classified reason, which never names an address | `rail_failure_text` (`control: redact`)                                                               |
| `charges.failure_raw`, `refunds.failure_raw` | the rail's own words for a decline — rail prose that can quote the payer                                                                                                                                                                                                     | `rail_failure_text` (`control: redact`)                                                               |
| `provider_requests.error_kind`               | a closed vocabulary of error kinds                                                                                                                                                                                                                                           | `rail_failure_text` — its redaction statement is an open maintainer decision (see the inventory page) |

**Retention of every one of them: undecided (RFC-0002 D9).**

## Copies written before 2026-11-17

The sweep stops new copies; it does not rewrite old ones. A deployment that
ran an earlier build may hold:

- **in its log store**: rail decline text after `rail rejected the request:
<code> —`, and a full rail URL — MTN's `…/accountholder/msisdn/<number>/basicuserinfo`
  among them — after `for url (` in an `api error` line's `source_chain` or a
  worker's `job failed:` line; a full webhook endpoint URL in the
  `url=` field of the `webhook delivery exhausted the retry ladder` alert;
- **in `jobs.last_error`**: the same rail text and rail URLs;
- **in `webhook_deliveries.response_excerpt`**: `no response: error sending
request for url (<the full endpoint URL>)…`, query included.

To find them (read-only; run as the database owner against the primary or a
replica):

```sql
-- Rail URLs with a path, and decline text, in job failures.
SELECT id, kind, run_at
FROM jobs
WHERE last_error ~ 'for url \([a-z]+://[^/)]+/[^)]'
   OR last_error LIKE '%rail rejected the request: % — %';

-- Endpoint URLs with a path or query in failed-attempt excerpts.
SELECT id, endpoint_id, sent_at
FROM webhook_deliveries
WHERE response_excerpt ~ 'for url \([a-z]+://[^/)]+/[^)]';
```

**What to do with what they find is not decided here.** Rewriting or deleting
them is a retention and erasure question (RFC-0002 **D9** for the database,
**D13** for log stores, PITR/WAL and backups, which an `UPDATE` does not
reach), and who decides is **D1**. Escalate through your deployment's
data-protection process with the row ids; do not edit `jobs` rows a worker may
still be leasing.

## Known gaps (each is open, none is hidden)

- **No static scanner over `tracing!` sites** (the inventory's `logs`
  surface, `enumerable: false`). The 2026-11-17 sweep read all of them once by
  hand; a new `tracing::info!(msisdn = %raw)` would pass every gate. The
  registered `Debug` impls and the `Display`-side redactions above are the
  structural guard; review is the rest.
- **Boot-time webhook URL errors print the URL as written** (above). They go
  to the operator who wrote it, on stderr, once; whether a query may be there
  at all is D4.
- **`Masked`/`Pseudonymous` adoption, keyed digests in logs**: D3.
- **Webhook payload minimisation**: PR 5, D2.
- **Every retention period and owner on this page**: D1, D9, D13.

## How to know it is still true

```bash
cargo nextest run -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money \
  -p vpay-worker -p vpay-api
cargo nextest run -p vpay-tests-integration --test webhooks \
  -E 'test(an_exhausted_delivery_never_logs_or_stores_the_endpoints_query)'
```

The canary tests those run — each asserting both that the protected literal is
absent and that the correlation an operator needs is present — are listed in
the verification page above with the mutation each one was proven against.
