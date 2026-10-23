# RFC-0013: vpay registers its own callback endpoints

- **Status:** Draft. **Delivery steps 2 and 3 were built on 2026-10-22, with
  Notch Pay as the first registered rail, and step 4 for Notch Pay without
  the `PUT` and `DELETE` it names.** § "As built" below says what was built
  and where it differs from this design. Stripe and PayPal have no adapter,
  so nothing here runs for them. **No endpoint has been registered with a
  real Notch Pay account by this code** (step 5): the integration suite runs
  it against a WireMock stub. _(This said "Nothing in this document is
  built. What exists is two calls in the Notch Pay client,
  `Client::list_webhooks` and `Client::create_webhook`, … and nothing calls
  them" until 2026-10-22. The second half had been false since 2026-10-20,
  when the Notch Pay adapter came to depend on the client.)_
- **Author:** vpay maintainers
- **Date:** 2026-10-20
- **Related:** [RFC-0012](0012-notchpay-adapter.md) § 6 and open question 4
  (the maintainer's decision this RFC designs),
  [RFC-0010](0010-stripe-adapter.md) §§ 2, 3, 6, 8, 9 and open question 4,
  [RFC-0011](0011-paypal-adapter.md) §§ 5, 6, 9 and open question 6,
  [RFC-0009](0009-provider-accounts-per-merchant.md),
  [ADR-0002](../adr/0002-provider-port.md),
  [ADR-0019](../adr/0019-credential-model.md),
  [ADR-0022](../adr/0022-surface-isolation-and-independent-scaling.md),
  [ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
  D2, [reconciler.md](../flows/reconciler.md),
  [crash-safety.md](../flows/crash-safety.md),
  [adapter-mtn-momo.md](../flows/adapter-mtn-momo.md),
  [adapter-orange-money.md](../flows/adapter-orange-money.md)
- **Sources, all read on 2026-10-20:** MTN's
  [callback page](https://momodeveloper.mtn.com/api-documentation/callback),
  its [API user and key page](https://momoapi.mtn.com/api-documentation/api-description),
  its developer portal's
  [sandbox provisioning listing](https://momodeveloper.mtn.com/developer/apis/sandbox-provisioning-api/operations?api-version=2022-04-01-preview),
  its [best practices](https://momoapi.mtn.com/content/html_widgets/j2w89.html)
  and one post by an MTN community manager, marked where used; Orange
  Developer's [Web Payment overview](https://developer.orange.com/apis/om-webpay)
  and [FAQ](https://developer.orange.com/apis/om-webpay/faq), and third-party
  guides, marked where used; docs.stripe.com, and one support.stripe.com page,
  marked; developer.paypal.com's Markdown pages and the Webhooks v1
  [OpenAPI schema](https://developer.paypal.com/api/webhooks/v1/schema.json);
  Notch Pay's [`openapi.yaml`](https://developer.notchpay.co/openapi.yaml)
  **version 2.1.0** and its
  [webhooks API reference](https://developer.notchpay.co/api-reference/webhooks).
  Nothing in this RFC was learned by calling any of them. A vendor fact that
  no vendor page confirms is a "To confirm" item in § 12, never resolved by
  choosing the convenient reading.

**Words.** In vpay a _webhook_ is what vpay sends a merchant
([webhooks.md](../flows/webhooks.md)). This RFC is about the other direction.
A _callback_ is a notification a rail sends to
`{public_base_url}/provider/{rail}/callback`. A _callback endpoint_ is the
object a provider stores to know where to send it — what Stripe, PayPal and
Notch Pay each call a webhook.

## Problem

The maintainer wrote on 2026-10-20:

> This case is interesting. If vpay knows it public url, it can itself
> manually register webhooks: Create webhook on notchpay at startup; MTN and
> Orange shall have something similar I guess; Stripe and PayPal should have
> something similar. Goal is never to miss a webhook

vpay does know its public URL. `deployment.public_base_url`
([application.yml](../../config/application.yml), line 18), or a per-rail
override (`ProviderHost::callback_url`, `vpay-config/src/config.rs:157`),
gives the effective callback URL (`effective_callback_url`,
`config.rs:373-378`). Boot already refuses it under livemode unless it is
`https` and names no stub (`config.rs:1161-1167`). The question is what to do
with it.

### What a missed callback costs: one rung of the poll, never an outcome

This comes first, because it changes what the goal is.

- **A callback is a hint.** The route writes one thing: the position of the
  charge's `poll_charge` job, which it enqueues if missing and pulls forward
  to now (`backends/crates/vpay-api/src/provider_callback.rs`, lines 11–24;
  the handler, 252–324). It reads identifiers only and discards `ref_extra`
  (lines 49–55). "The authenticated status query is the only thing that moves
  money" (AGENTS.md, "Callbacks are hints").
- **Every charge has a poll from birth.** The confirm commits the charge and
  its `poll_charge` job in one transaction
  (`vpay-api/src/v1/payment_intents.rs:1966-1971`;
  [reconciler.md](../flows/reconciler.md), line 102).
- **The poll never runs out.** 10, 20, 30, 45, 60 and 90 s, then 120 s to the
  twentieth attempt, then 15 minutes (`vpay-worker/src/lib.rs:66-73`; "It
  never runs out", line 51). Past 24 hours, hourly and with an alert
  (`vpay-worker/src/recovery.rs:94`, `lib.rs:89`). A ten-minute scan
  re-enqueues a live charge whose job went missing (`handlers.rs:151` and
  `1313-1359`).
- **It is proven without a callback.**
  `backends/tests/integration/tests/worker_e2e.rs` takes a charge from
  confirm to `succeeded` against a WireMock rail with no callback in the path.
  And "no rail has ever called the callback route"
  ([reconciler.md](../flows/reconciler.md), line 236).

So a missed callback costs the wait for the next rung: up to 90 s in a
charge's first four minutes, up to 2 minutes until about its 32nd minute, and
15 minutes after that. That is arithmetic on the ladder: the six short rungs
total 255 s, the fourteen 120 s rungs 1 680 s. On PayPal, whose status read
also captures (RFC-0011 § 3.2), the ladder captures within one rung, inside
PayPal's three-hour window (RFC-0011 § 5). A callback that arrives after the
answer finds a terminal charge and does nothing (`handlers.rs:329-332`).

"Never miss a webhook" is therefore not what protects a payment. The poll
does, today. And on MTN no registration could deliver it: "the Wallet
Platform sends the callback only once. There is no retry mechanism"
([callback](https://momodeveloper.mtn.com/api-documentation/callback)).

What self-registration buys is two things, and on Stripe a third:

1. **Latency.** On a rail whose URL is registered rather than sent, an outcome
   arrives at callback speed instead of at the next rung.
2. **No manual step to get wrong.** RFC-0010 § 9 and RFC-0011 § 9 give
   registration to the merchant, by hand, in a dashboard, and RFC-0012 § 9
   does until this RFC lands. A wrong
   URL, a missing event type or an endpoint deleted months later raises no
   error anywhere. The deployment keeps working, slower, and looks healthy.
   That is the failure worth removing.
3. **On Stripe, a pinned event shape.** An endpoint created through the API
   takes `api_version`: "Events sent to this endpoint will be generated with
   this Stripe Version instead of your account's default Stripe Version"
   ([create](https://docs.stripe.com/api/webhook_endpoints/create)). A
   Dashboard endpoint cannot: "you must create a webhook endpoint using the
   API to use other API versions in production"
   ([Dashboard webhooks](https://docs.stripe.com/development/dashboard/webhooks)).
   RFC-0010 § 3 pins `2026-09-30.endive`. Only a self-registered endpoint
   sends events in that version.

The goal, restated: **on every rail that registers its callback URL, vpay
makes sure its own endpoint exists and is right, and notices when it stops
being so — without making a payment, a boot or anyone else's endpoint depend
on it.**

### What there is to register, rail by rail

| Rail                      | How the rail learns the URL                                                                                                                                                                                                                               | What there is for vpay to register                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mtn_momo` (shipping)     | Per request: `X-Callback-Url` on `requesttopay` (`vpay-adapter-mtn-momo/src/lib.rs:94`, `:803`). Its host "must be hosted on the same domain specified when creating the API user" ([callback](https://momodeveloper.mtn.com/api-documentation/callback)) | **Nothing at startup.** The host is a _domain_ per API user, `providerCallbackHost`. In production it is registered "on the portal when creating your API keys" ([callback](https://momodeveloper.mtn.com/api-documentation/callback)); an MTN community manager wrote "You cannot auto-generate users in Production via the API" ([forum](https://momodevelopercommunity.mtn.com/how-to-59/understanding-momo-open-api-keys-sandbox-and-production-455)). In the sandbox it is set once, by `POST /v1_0/apiuser`, and read back by `GET /v1_0/apiuser/{id}`; the provisioning API has three operations and none changes it ([listing](https://momodeveloper.mtn.com/developer/apis/sandbox-provisioning-api/operations?api-version=2022-04-01-preview)) |
| `orange_money` (shipping) | Per request: `notif_url` in the `webpayment` body (`vpay-adapter-orange-money/src/lib.rs:481`, `wire.rs:46`)                                                                                                                                              | **Nothing.** Orange's public overview and FAQ say nothing about `notif_url`, and its reference is gated, as [adapter-orange-money.md](../flows/adapter-orange-money.md) lines 6–10 already record. Third-party guides describe a per-request field and no registration ([Paynote](https://www.paynote.africa/comment-deployer-lapi-orange-money/)) — To confirm (§ 12, item 17)                                                                                                                                                                                                                                                                                                                                                                          |
| `stripe` (RFC-0010)       | Registered: a Checkout Session has no callback field (RFC-0010 § 6)                                                                                                                                                                                       | `POST /v1/webhook_endpoints` (below)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `paypal` (RFC-0011)       | Registered (RFC-0011 § 6)                                                                                                                                                                                                                                 | `POST /v1/notifications/webhooks` (below)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `notchpay` (RFC-0012)     | Registered: the initialize call's `callback` is a browser redirect, and a push charge has no browser (RFC-0012 § 6)                                                                                                                                       | `POST /webhooks` (below)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

The three that register differ in exactly the ways that shape § 4:

| Property            | Stripe                                                                                                                                                                                                                                                                            | PayPal                                                                                                                                                                                                                                                                                                              | Notch Pay                                                                                                                                                                                                                                                         |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Create              | `POST /v1/webhook_endpoints`: `url`, `enabled_events`, `api_version`, `description`, `metadata`, `connect` ([create](https://docs.stripe.com/api/webhook_endpoints/create))                                                                                                       | `POST /v1/notifications/webhooks`: `url`, `event_types` ([webhooks-post](https://developer.paypal.com/api/webhooks/v1/webhooks-post))                                                                                                                                                                               | `POST /webhooks`: `url`, `events`, optional `description` (openapi.yaml 2.1.0)                                                                                                                                                                                    |
| List                | No filter; `limit` 1 to 100 ([list](https://docs.stripe.com/api/webhook_endpoints/list))                                                                                                                                                                                          | No URL filter ([schema](https://developer.paypal.com/api/webhooks/v1/schema.json))                                                                                                                                                                                                                                  | Paginated: `page`, `limit` (default 30); `current_page`, `last_page` (openapi.yaml 2.1.0)                                                                                                                                                                         |
| Update, delete      | Both; update takes `url`, `enabled_events`, `disabled`, `description`, `metadata` ([update](https://docs.stripe.com/api/webhook_endpoints/update)); `api_version` "can't change … after you create the endpoint" ([object](https://docs.stripe.com/api/webhook_endpoints/object)) | Both; `PATCH` supports only `replace`, and a patch that changes nothing is an error, `WEBHOOK_PATCH_REQUEST_NO_CHANGE` ([schema](https://developer.paypal.com/api/webhooks/v1/schema.json))                                                                                                                         | `PUT` and `DELETE` on `/webhooks/{endpoint}` exist; the client implements neither, "deliberate" ([vpay-notchpay.md](../reference/vpay-notchpay.md), "What it deliberately omits")                                                                                 |
| A label vpay writes | `metadata` and `description`                                                                                                                                                                                                                                                      | **None**: the object is `id`, `url`, `event_types`, `links` ([schema](https://developer.paypal.com/api/webhooks/v1/schema.json))                                                                                                                                                                                    | `description` only                                                                                                                                                                                                                                                |
| Idempotent create   | **Yes**: "all `POST` requests accept idempotency keys", and "Retries with the same key return the saved response" ([idempotent requests](https://docs.stripe.com/api/idempotent_requests)). A duplicate URL is not documented either way (§ 12, item 1)                           | No key: `PayPal-Request-Id` is not listed for this call, and "Not all APIs support this header" ([idempotency](https://developer.paypal.com/api/rest/reference/idempotency)). **But a duplicate URL is refused**: `WEBHOOK_URL_ALREADY_EXISTS` ([schema](https://developer.paypal.com/api/webhooks/v1/schema.json)) | **No**: no idempotency header and no duplicate-URL refusal is documented (`vpay_notchpay::Client::create_webhook`'s docs)                                                                                                                                         |
| Secret              | `whsec_…`: "Only returned at creation" ([object](https://docs.stripe.com/api/webhook_endpoints/object))                                                                                                                                                                           | None. Verification needs the `webhook_id`, which "does not come in the header or body of the message" ([webhooks/rest](https://developer.paypal.com/api/rest/webhooks/rest/)); list returns it                                                                                                                      | The `Webhooks.Webhook` schema has none (`id`, `url`, `description`, `events`, `created_at`, `updated_at`); the reference says "Each webhook has a secret that you can find in your dashboard or when creating a webhook via the API" — To confirm (§ 12, item 10) |
| Limit               | "up to 16 webhook endpoints on each Stripe account" ([Dashboard webhooks](https://docs.stripe.com/development/dashboard/webhooks)), per mode ([how events work](https://docs.stripe.com/events/how-events-work))                                                                  | "Up to 10 webhook URLs may be subscribed per app" ([webhooks](https://developer.paypal.com/api/rest/webhooks/))                                                                                                                                                                                                     | None documented                                                                                                                                                                                                                                                   |
| Who may create      | A restricted key needs `webhook_write`, "a sensitive permission because it allows subscribing to events across your entire account"; `webhook_read` lists ([permissions](https://docs.stripe.com/stripe-apps/reference/permissions))                                              | The REST app's token, with the webhooks scope; webhooks are per app: "Only events associated with the specific app will be received" ([webhooks](https://developer.paypal.com/api/rest/webhooks/))                                                                                                                  | The private key, in `X-Grant` ([API authentication](https://developer.notchpay.co/api-reference/authentication))                                                                                                                                                  |
| Endpoint state      | `status`: `enabled` or `disabled`; Stripe disables one that fails "for several days" ([support.stripe.com](https://support.stripe.com/questions/why-is-stripe-trying-to-reach-my-webhook-endpoints))                                                                              | None documented                                                                                                                                                                                                                                                                                                     | None in the schema                                                                                                                                                                                                                                                |

**The one rail where a registration can stop a payment is the one where vpay
cannot register anything.** MTN validates the callback host at submit: vpay
reads `INVALID_CALLBACK_URL_HOST` as one of its three `CONFIGURATION_CODES`,
"permanent until a human edits configuration"
(`vpay-adapter-mtn-momo/src/mapping.rs:186-193`), so a charge on a
misregistered host fails as `ProviderError::Config` and pages. That is a
manual portal step in production, and the first charge already names it.

## Proposal

### 1. The rule

For each rail whose adapter declares `CallbackUrlMode::Registered` (§ 8), and
each provider account on it, vpay ensures there is **one callback endpoint it
created**, at the effective callback URL, subscribed to the rail's charge
events, and enabled. **It never modifies or deletes an endpoint it did not
create.**

A rail that declares `PerRequest` — `mtn_momo` and `orange_money` — is never
asked anything. What it does today does not change.

The design changes how soon a hint arrives. It does not change what a hint
can do: `parse_callback`, the route, its four answers and the poll ladder are
untouched.

### 2. Where it runs: a worker job, never the boot path

**Not in `vpay-server`'s boot.** Three reasons:

- **A boot that waits on a provider's API is a deployment the provider can
  take down.** A Stripe outage during a rollout would hold every new pod; a
  boot that fails on a refusal would crash-loop it. Boot today touches only
  configuration and the database ([configuration.md](../flows/configuration.md),
  "Boot sequence").
- **The server is the surface that autoscales.** ADR-0022's table gives
  `<release>-server` the `/v1`, `/v1/browser` and `/provider` routes and an
  HPA. Every scale-out would rerun registration against every provider
  account, concurrently.
- **It answers RFC-0012's old objection instead of dropping it.** Until the
  maintainer decided, RFC-0012's open question 4 recommended against
  registering because "a boot that writes to a provider is a side effect no
  other rail has". That is right about boot, and this design does not use
  boot.

**A worker singleton job.** Proposed kind `ensure_callbacks`, dedupe key
`ensure:callbacks`, seeded beside the five singletons `seed_singletons`
already seeds (`vpay-worker/src/run_loop.rs:488`), due at once, and
rescheduling itself the way `scan_live_charges` does (`handlers.rs:1359`). So
it runs at every worker start, and then on a schedule — proposed hourly, open
question 3. The schedule is what catches drift: an endpoint deleted in a
dashboard, an event type removed, an endpoint Stripe disabled.

**It never fails a process, and it never dead-letters.**

- A seed that fails "is logged with `alert` and the loop starts anyway"
  (`run_loop.rs:668-683`). The sixth seed inherits that.
- The handler always answers with a reschedule, never with an error. This is a
  rule, not a preference. A dead-lettered job is parked at
  `run_at = 'infinity'` and "keeps its `dedupe_key` occupied" (migration
  `0023_jobs-scan-deliveries.sql`, lines 30–33). The
  `ON CONFLICT DO NOTHING` in `seed_singletons` could then never revive it,
  and registration would stop for good, silently.
- Nothing waits on it. A charge on a rail with no endpoint is polled exactly
  as today.

| The rail answers                                                                  | The pass                                                                                                                     |
| --------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `401` or `403`: a revoked key, or a Stripe restricted key without `webhook_write` | Logs at `error` with `alert`, naming the rail, the account and the permission. Reschedules at the normal interval            |
| `429`, `5xx`, a timeout                                                           | Reschedules sooner (proposed five minutes). On a rail with no idempotency key, the create is not repeated in this pass (§ 4) |
| A body that does not parse                                                        | `Malformed`: logs, reschedules                                                                                               |
| `NotImplemented`: an adapter that declares `Registered` and has not written it    | Logs at `warn`; `docs/status.md` already lists the token                                                                     |

**And a command that runs the same pass.** Proposed `vpay-server callbacks
ensure`, a sibling of `vpay-server staff` (`vpay-config/src/cli.rs:450-472`).
It runs one pass in the foreground under the same lock, prints one line per
rail and account, and exits non-zero if any registered rail is left with no
endpoint at vpay's URL. `--check` lists and reports without writing.

The command **alone** was weighed as the whole design. For it: nothing writes
to a provider account without a person in front of it, and on Stripe the
deployment's resident key could keep RFC-0010 § 2's three permissions, with
`webhook_write` given only to a key the operator passes to the command.
Against it: it brings back the manual step the maintainer wants gone — once
per provider account, on every RFC-0009 onboarding pull request — and nothing
notices drift. So **both, on one code path**: the job is the default; the
command is for first setup, for a deployment that runs the job in `check`
mode, and for runbooks.

**One setting per rail.** Proposed `providers[].callbacks.register`:
`ensure` (the default, open question 1), `check` (list and report, never
write; on Stripe `webhook_read` is enough), or `off`. A `callbacks` block on a
`PerRequest` rail refuses to boot, because a setting that does nothing is one
somebody believes in.

### 3. Recognising vpay's own endpoint

Every list is read whole: Notch Pay's every page to `last_page` — the client
already says "stopping at the first would miss one and register a duplicate"
(`vpay_notchpay::wire::WebhookPage`'s docs) — Stripe's 16 at most in one page of
`limit=100`, PayPal's in one call. No provider filters a list by URL or label,
so the classification is vpay's:

| Class                          | How vpay knows                                                                                                                     | What vpay may do                                                                                |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| **vpay's, current**            | this deployment's marker, at the effective callback URL                                                                            | put it right: re-enable it, restore its events                                                  |
| **vpay's, superseded**         | this deployment's marker at another URL: a `public_base_url` that changed, or a Stripe endpoint replaced for an `api_version` bump | delete it, and only after a current one is present (open question 4)                            |
| **someone else's, at our URL** | vpay's URL with no marker                                                                                                          | **nothing.** Not modified, not deleted, and no second endpoint beside it. Reported as `foreign` |
| **someone else's**             | anything else, another deployment's marker included                                                                                | **nothing.** Its URL and label are compared and nothing more is read                            |

**The marker** is `vpay:{deployment.name}:{rail}:{provider account id}`.
Stripe carries it in `metadata[vpay_endpoint]` and in `description`; Notch Pay
in `description`, its only label: the client already documents that field
as "what lets vpay recognise an endpoint it created itself"
(`vpay_notchpay::wire::CreateWebhook`). The deployment's name is in the marker
so that two deployments sharing one sandbox account never take each other's
endpoints for superseded ones.

**PayPal has no label, so on PayPal vpay can never prove an endpoint is its
own.** It recognises one by exact URL alone, never modifies or deletes one,
and reports an endpoint at an old URL rather than retiring it. PayPal's
duplicate refusal makes the URL unique on the app, which is what makes
URL-only recognition safe for a create. It is not enough for a delete.

**Why a foreign endpoint at vpay's URL is left alone, rather than adopted or
doubled.** It is most likely the one a merchant registered by hand under
RFC-0010, -0011 or -0012's interim instructions, and it already delivers to
the right place. A second endpoint beside it would double every delivery:
harmless, because the pull-forward is idempotent, but it spends one of 16 or
10 slots and a merchant's dashboard shows two. Adopting it would mean
modifying something vpay did not create. The cost is stated in the report:
on Stripe its events arrive in the account's default API version rather than
the pinned one, and vpay never learns its secret. Deleting it lets vpay
create its own.

### 4. One pass, for one rail and one account

1. **Take the lock, or skip.** A transaction opens with
   `pg_try_advisory_xact_lock` on a new key in `vpay_db::lock_keys` —
   proposed `vpaycbk`, on that module's mnemonic scheme
   (`backends/crates/vpay-db/src/lock_keys.rs`) — beside the two it holds,
   `SIGNING_KEY_ROTATION` and `CONFIG_RECONCILE`. If another worker or the
   command holds it, this pass skips the account and the next pass catches it.
2. **List** the account's endpoints, whole (§ 3).
3. **Decide.** Present and right: done. Present and drifted: put it right —
   vpay's own only. Foreign at vpay's URL: report and stop. Absent: create,
   unless the mode is `check`.
4. **Create once.** Where the rail takes an idempotency key (Stripe), a fresh
   UUID per pass, so a create retried after a timeout inside the pass is a
   replay. Where it does not (Notch Pay), **a create whose answer was lost is
   never repeated in the same pass**: the pass ends, and the next one lists
   first and finds the endpoint by its marker. On PayPal a repeated create
   meets `WEBHOOK_URL_ALREADY_EXISTS`, and the pass lists.
5. **Retire** vpay's superseded endpoints, once step 3 or 4 has a current one.
6. **Commit**, which releases the lock. One log line, one gauge per rail and
   state.

Why each piece is there:

- **List before create**, because Notch Pay's create is not idempotent and
  neither Stripe nor Notch Pay documents refusing a duplicate URL. Without the
  list, every worker restart could add an endpoint.
- **A lock**, because several replicas start at once and the singleton job is
  not one. Its row is claimed under a five-minute lease
  (`vpay-worker/src/recovery.rs:93`) that the reaper can hand to a second
  worker while a slow pass still runs, and the command runs outside the queue.
  Two passes that both list before either creates would both create on Notch
  Pay. The lock closes that window; the list makes one pass correct.
- **Transaction-scoped, not session-scoped.** It is released by commit, by
  rollback, or by a dropped connection, so a worker killed mid-pass leaves
  nothing held, and no code path can hand a still-locked connection back to
  the pool. The cost: one pooled connection held open for one account's pass —
  a list, at most one create and one retirement, each bounded by
  `DEFAULT_REQUEST_TIMEOUT`, 20 s (`vpay-provider/src/lib.rs:1400`). The
  singleton bounds that to one pass per worker process, inside the pool
  arithmetic [crash-safety.md](../flows/crash-safety.md) records under
  "Worker concurrency and the pool".
- **No table, in the first build.** The provider's own list is the durable
  record, and the marker is what makes it readable as vpay's. "Write first,
  network second" ([crash-safety.md](../flows/crash-safety.md)) holds because
  nothing vpay needs exists only in the create's response — as long as vpay
  discards Stripe's secret (§ 6). The day it keeps the secret, that stops
  being true, and a table arrives with it.

### 5. Rail by rail

**`stripe`** (RFC-0010).

- Create: `POST /v1/webhook_endpoints`, form-encoded like every RFC-0010 call,
  with `Idempotency-Key`; `enabled_events[]` the four `checkout.session.*`
  types RFC-0010 § 6 parses; `api_version=2026-09-30.endive`, RFC-0010 § 3's
  pin; `description` and `metadata[vpay_endpoint]` the marker; `connect`
  false (model A).
- Drift on vpay's own: `status: disabled` is re-enabled with `disabled=false`
  and an alert (open question 5); a missing event type is restored with an
  update; an `api_version` other than the pin means a replacement, because the
  field cannot change, and the old endpoint is retired. A replacement rotates
  the secret.
- The key: `webhook_write` in `ensure` mode, `webhook_read` in `check` mode.
  RFC-0010 § 2's three permissions become four, and the fourth is the one that
  lets a stolen key subscribe the thief's URL to every event on the account.
  That is open question 1.
- v1 and not v2 event destinations: see Alternatives.

**`paypal`** (RFC-0011).

- Create: `POST /v1/notifications/webhooks` with `url` and the eight
  `event_types` RFC-0011 § 6 subscribes. The token must carry the webhooks
  scope, whose name two PayPal pages give differently (§ 12, item 8), so the
  adapter reads it from the token response's `scope` rather than hard-coding
  either.
- A callback URL not on port 443 is refused before any call, as
  `ProviderError::Config` naming it: the listener must be on "HTTPS port 443"
  ([webhooks/rest](https://developer.paypal.com/api/rest/webhooks/rest/)).
- Drift is reported, never repaired: without a label, no endpoint is provably
  vpay's (§ 3).

**`notchpay`** (RFC-0012).

- Create: `Client::create_webhook` with `url`, `description` the marker, and
  `events` the five in `PAYMENT_EVENTS` (`vpay_notchpay::wire::PAYMENT_EVENTS`).
- List: `Client::list_webhooks(page)` until `current_page` equals
  `last_page`. The client sends `page` and takes the default `limit` of 30.
- Drift: the schema has no enabled state to read. Restoring vpay's events
  needs `PUT /webhooks/{endpoint}` and retiring needs `DELETE`; the client has
  neither, on purpose. Both would be added, and called only on an endpoint
  that carries this deployment's marker. _(Retiring will not be added: on
  2026-10-23 the maintainer made it an operator's task, open question 4.)_
- None of this moves RFC-0012's gate. The Notch Pay adapter is not buildable
  until its delivery step 0 is met, which since 2026-10-20 means Notch Pay's
  written answer on rule 3 (RFC-0012, "The gate"). _(False since later on
  2026-10-20: [ADR-0031](../adr/0031-a-status-read-authenticated-by-a-server-side-key.md)
  amended rule 3, and the adapter was built the same day. Noted 2026-10-22.)_

### 6. Secrets acquired at runtime

- **vpay's rail secrets are configuration today, never database state.** They
  are `${VAR}` placeholders resolved at boot
  ([application.yml](../../config/application.yml), lines 4–13; for example
  `ORANGE_CLIENT_SECRET`, line 125). Nothing in vpay writes a rail secret
  anywhere.
- **Stripe's `whsec_…` arrives once**, in the create response, and the API
  never shows it again. Keeping it needs **a sealed-at-rest store**: a row,
  sealed under a deployment key, the way `vpay_api::staff_auth::secret_box`
  seals TOTP secrets — AES-256-GCM, `base64url(nonce ‖ ciphertext ‖ tag)` as
  `TEXT` (`backends/crates/vpay-api/src/staff_auth/secret_box.rs`, lines 1–31
  and 76). It would need its own key, not `staff_auth.totp_encryption_key`
  (`vpay-config/src/config.rs:610`), and a home the worker can link, since
  that module belongs to staff authentication. Custody and rotation of a key
  for rail secrets is the decision RFC-0009 deferred: storing provider
  accounts as rows "needs a key-custody decision for rail secrets".
  **This store is the largest new piece of the design** — larger than the
  pass.
- **And nothing could read the secret yet.** `parse_callback` takes
  `body: &[u8]` and no headers (`vpay-provider/src/lib.rs:1518`), so no rail's
  signature can be checked until the port grows a headers argument: RFC-0010
  open question 4. RFC-0012 § 2 refused to configure Notch Pay's hash key on
  exactly this ground: "a secret that nothing reads is a secret held for no
  reason". _2026-10-20: [RFC-0014](0014-verifying-rail-callbacks.md)
  designs the reader: a defaulted `verify_callback` with the headers. For an
  endpoint vpay creates itself, verifying needs this section's sealed store
  (RFC-0014 § 5). Neither is built._
- **So the first build discards it** (open question 2). The adapter keeps the
  endpoint id and drops the secret, and a `Debug` canary proves it never
  reaches a log. When the port can verify, vpay replaces each endpoint it
  created — create, seal the new secret, retire the old, which is § 3's own
  rule — and the deterministic idempotency key that a stored secret needs
  arrives with the table. What discarding costs: one replacement per Stripe
  account on that day.
- **PayPal's `webhook_id` is not a secret**, and listing returns it, so it
  needs no store. RFC-0011 § 6 and open question 6 treat it as configuration;
  with registration it comes from the API instead. Verifying a PayPal
  callback still needs the same port change.
- **Notch Pay's secret is unconfirmed.** The schema has no field for it; the
  reference says creation returns it (§ 12, item 10; RFC-0012 § 10, item 13).
  If it does, it is Stripe's case.

### 7. Per provider account

- **Today there is one account per rail.** "Provider credentials exist once
  per deployment" (RFC-0009, "What the tree does"), so a pass per registered
  rail, with the rail-level credentials.
- **With RFC-0009, N accounts on a rail are N passes**, each with that
  account's merged `ProviderConfig`, each making its own endpoint in its own
  provider account, each with its own marker. Before RFC-0009, the account id
  in the marker is the reserved id RFC-0009 § 4 records on a one-merchant
  deployment's charges.
- **All N point at the same URL**, `{public_base_url}/provider/{rail}/callback`,
  with no account in the path. The route finds the charge by rail and by the
  reference in the body (`get_by_provider_reference`,
  `provider_callback.rs:281-283`), and RFC-0009 § 3
  resolves the account from the **charge**. An account in the URL would be an
  input the caller chooses on an unauthenticated route, and anything that
  trusted it — a verifier choosing a secret, a poll choosing credentials —
  would let a callback about one merchant's charge act with another's. When
  signatures are checked, the secret is the charge's account's.
- **Model B registers per platform, not per account.** A Stripe Connect
  platform registers one endpoint with `connect=true`, and "Each event for a
  connected account contains a top-level `account` property"
  ([Connect webhooks](https://docs.stripe.com/connect/webhooks)). A PayPal
  partner needs a webhook lookup as well
  (`POST /v1/notifications/webhooks-lookup`,
  [schema](https://developer.paypal.com/api/webhooks/v1/schema.json)). Out of
  scope until RFC-0010 step 6
  and RFC-0011 step 7 (open question 6).

### 8. The port shape

```rust
// Illustrative. Names are proposals; none of this exists.

/// How a rail learns where to send its callbacks. Not persisted, on
/// `refund_destination`'s terms.
pub enum CallbackUrlMode {
    /// The URL travels on every submit (`X-Callback-Url`, `notif_url`).
    /// The core never asks this rail about endpoints.
    PerRequest,
    /// The URL is registered with the provider account, out of band.
    Registered,
}

// `Capabilities` gains one field:
//     pub callback_url_mode: CallbackUrlMode,

/// One pass's question. The URL is `ProviderConfig::callback_url`.
pub struct EndpointCheck<'a> {
    /// `vpay:{deployment}:{rail}:{account}`: what makes an endpoint vpay's.
    pub marker: &'a str,
    /// `false` in `check` mode: list and report, never write.
    pub write: bool,
}

pub enum EndpointState {
    /// vpay's endpoint is present and right, or was just put right.
    Present { endpoint_id: String, repaired: bool },
    /// Created by this call.
    Created { endpoint_id: String },
    /// An endpoint at vpay's URL that vpay did not create, left as found.
    Foreign { endpoint_id: String },
    /// No endpoint at vpay's URL, and `write` was false.
    Missing,
}

// `ProviderAdapter` gains one method:
//
//     /// Called only when `callback_url_mode` is `Registered`.
//     async fn ensure_callback_endpoint(
//         &self,
//         check: &EndpointCheck<'_>,
//         config: &ProviderConfig,
//     ) -> Result<EndpointState, ProviderError> {
//         Err(ProviderError::Unsupported)
//     }
```

- **One field and one method.** The field is not persisted, as
  `supports_account_holder_lookup` and `refund_destination` are not
  (`vpay-provider/src/lib.rs:110-118`, `139-145`). The method defaults to
  `Unsupported`, the shape of `refund` and `account_holder_name` (lines
  1729–1746 and 1776–1797). `EndpointState::Created` carries no secret until
  step 6 of Delivery gives it a reader.
- **A rail that declares `PerRequest` is untouched.** `mtn_momo` and
  `orange_money` add one line each to their `Capabilities` literal, because
  the compiler demands it, and nothing else. The core never calls the method
  on them, and their submits keep sending the URL as they do today.
- **The adapter knows the wire; the core knows when.** The schedule, the
  lock, the accounts and the marker are the core's, as the poll ladder is.
  What a rail's list looks like, where the marker goes and what counts as
  drift are the adapter's.
- **A coherence rule.** `Registered` requires `delivers_callbacks: true`, a
  line in `Capabilities::is_coherent`, which boot already enforces
  (`vpay-api/src/v1/boot.rs:140-158`) and `vpay-server` asserts over every
  linked adapter (`backends/apps/vpay-server/src/lib.rs:70`).
- **The conformance suite reads the capability.**
  `the_submit_tells_the_rail_where_to_call_back`
  (`backends/tests/conformance/tests/adapter_conformance.rs:1046`) asserts the
  URL in the submit on `PerRequest`, and nowhere in it on `Registered`. That
  turns RFC-0010 § 8's per-row `callback_url_pattern: Option` into a rule, as
  `every_shipping_rails_refund_destination_agrees_with_its_parser` turned a
  census into one. A second rule: the method answers `Unsupported` exactly on
  `PerRequest` rails.
- _(2026-10-22: the first consumer was Notch Pay, not Stripe. The bullet
  below is kept as written.)_
- **It lands with its first consumer, not before.** ADR-0029 D2: "each lands
  with its first consumer, never ahead of one". That is not this RFC's
  acceptance and not the Notch Pay client. It is the first adapter to declare
  `Registered`: on today's state Stripe, because RFC-0010 step 2 depends only
  on its landed step 1, while Notch Pay's adapter waits on RFC-0012 step 0 and
  PayPal's on RFC-0010 steps 1–3. An adapter that declares `Registered` before
  writing the method overrides it with a `NotImplemented` token, which
  `docs/status.md` then lists, as the port requires of every unbuilt method.

### 9. Catch-up after downtime

A separate benefit of a registered rail is that some providers keep their
events.

| Provider  | Redelivery                                                                                                      | Reading missed events                                                                                                                                                                                                                                                                                                                                                                                           |
| --------- | --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Stripe    | "for up to three days with an exponential back off in live mode" ([webhooks](https://docs.stripe.com/webhooks)) | `GET /v1/events?delivery_success=false`, events "failed delivery to at least one of your endpoints", "created in the last 30 days" ([manage endpoints](https://docs.stripe.com/events/manage-webhook-endpoints)). **No resend API**: the Dashboard resends for 15 days, the CLI for 30. Events created while an endpoint is disabled are never resent ([Workbench](https://docs.stripe.com/workbench/webhooks)) |
| PayPal    | "up to 25 times over the course of 3 days" ([webhooks](https://developer.paypal.com/api/rest/webhooks/))        | `GET /v1/notifications/webhooks-events` by time window and type, with no delivery-status filter, and `POST …/webhooks-events/{id}/resend` ([schema](https://developer.paypal.com/api/webhooks/v1/schema.json)). The API states no retention; the Dashboard says "The range must be in the last 30 days" ([events dashboard](https://developer.paypal.com/api/rest/webhooks/events-dashboard))                   |
| Notch Pay | "several times with increasing delays" ([webhooks API](https://developer.notchpay.co/api-reference/webhooks))   | None in openapi.yaml 2.1.0                                                                                                                                                                                                                                                                                                                                                                                      |
| MTN       | None: "sends the callback only once" ([callback](https://momodeveloper.mtn.com/api-documentation/callback))     | None                                                                                                                                                                                                                                                                                                                                                                                                            |
| Orange    | Not documented                                                                                                  | None documented                                                                                                                                                                                                                                                                                                                                                                                                 |

**Recommendation: vpay does not use them for charges.** Walk the three ways
vpay can be down:

- **The API surface down, the worker up.** Callbacks fail, and the rails that
  retry, retry. The poll resolves every charge in the meantime, and a retry
  that lands later finds a terminal charge (`handlers.rs:329-332`).
- **The worker down.** Every poll that came due is claimable the moment a
  worker returns, because a claim takes any job due by the database's `now()`
  ([ADR-0026](../adr/0026-the-database-clock-schedules-jobs.md)). A replayed
  event would pull forward a poll that is already due.
- **Both down.** The same as the worker down.

In none of the three does an event list hold anything a status read does not.
What would change that is an event about something vpay does not poll: a
dispute, a refund settled at the rail (there is no refund status read,
RFC-0003 open question 8), a refund made in a dashboard. When the first of
those is designed, events become data rather than hints, they must be
verified first (RFC-0010 open question 4), and catch-up belongs to that
design.

One read from this table is worth taking now, and § 5 takes it: Stripe's
endpoint `status`. A disabled endpoint loses its callbacks for good, and the
pass sees it within one interval.

### 10. Proving it

- **Unit tests in each adapter**, no container: § 3's four classes row by row,
  another deployment's marker included and never touched; `check` never
  writes; on Notch Pay a create whose answer is lost is not repeated; on
  PayPal a URL off port 443 is `Config`; a `Debug` canary proving Stripe's
  secret never reaches a log.
- **WireMock mappings** beside each rail's charge mappings, transcribed from
  the vendor's reference, with WireMock scenarios so that a list after a
  create returns what was created.
- **Integration cases against a real Postgres**: two workers and the command
  racing on one database against the Notch Pay stub make one create in the
  stub's journal, and the mutation that removes the lock makes two. A create
  whose response the stub drops is not repeated in that pass, and the next
  pass makes none. A foreign endpoint at vpay's URL draws no write call at
  all. Every registration call answering `503` leaves the worker loop running
  and `worker_e2e`'s charge settling.
- **Boot**: a `callbacks` block on `mtn_momo` refuses to boot, with a
  `ConfigError` variant and a fixture, as every boot refusal has.
- **The first sandbox registration on each rail**, on a dated verification
  page, with § 12 answered or left open by name — the 2026-09-15 MTN
  precedent.

### 11. Findings about vpay's own documentation

The research for this RFC found these. They are recorded here, not fixed
here; Delivery step 0 fixes them as dated corrections.

_(Updated 2026-10-21: Delivery step 0 has landed for items 1–3 and not for
items 4 and 5. Item 1 is corrected in the three places it names and in every
other place that said the same thing, item 2 is "To confirm with MTN" item 1 in
[adapter-mtn-momo.md](../flows/adapter-mtn-momo.md), and item 3 is "To
confirm with Orange Cameroun" item 11 in
[adapter-orange-money.md](../flows/adapter-orange-money.md);
[the verification page](../status/verification/2026-10-21-rail-callback-retries.md)
lists every place. One detail in item 2 was narrower than it reads: a `GET`
on `/provider/mtn_momo/callback` **is** measured, as a bare `405`, by
`a_get_on_the_callback_path_is_a_405_and_not_the_404_envelope` in
`vpay-api`; a `PUT` is not.)_

_(Updated 2026-10-22: **Delivery step 0 is complete.** Item 4 is corrected in
[reconciler.md](../flows/reconciler.md) and in the two other pages that said
"hourly", [crash-safety.md](../flows/crash-safety.md) and the worker-queue
runbook. Item 5 is corrected in `vpay-config` and in every other place that
said MTN has, or allows, an IP-allowlisted callback host (the chart, its
README, `docs/reference/rails.md`, an MTN WireMock mapping); and the MTN
adapter's `requires_ip_allowlist: true` now says why it may be right, the
best practices' Disbursements entry. See
[the verification page](../status/verification/2026-10-22-rfc-0013-step-0-rest.md).)_

1. **"Both rails retry a non-2xx" is contradicted by MTN.** vpay says it in
   three places: `backends/crates/vpay-api/src/provider_callback.rs:81-85` ("A
   rail that gets anything other than a 2xx retries — MTN and Orange both do,
   on their own schedules"), [reconciler.md](../flows/reconciler.md) line 56
   ("a rail retries anything that is not a 2xx") and
   [provider-callback.md](../reference/vpay-api/provider-callback.md) line 98
   ("Both rails retry a non-2xx on their own schedules"). MTN says the
   opposite, twice: "There is no retry mechanism if the partner system does
   not respond" ([callback](https://momodeveloper.mtn.com/api-documentation/callback))
   and "There is no retry on the callback if the Partner system does not
   respond" ([API user](https://momoapi.mtn.com/api-documentation/api-description)).
   No source says either way for Orange. The two indistinguishable `202`s
   still stand on their other argument, that the route must not be an oracle
   (`provider_callback.rs:85-88`). The sentence becomes true of the three
   registered rails.
2. **MTN may call back with `PUT`, and vpay mounts `POST` only.** MTN's
   callback page says "The callback will be a POST request" and also "Allow
   PUT & POST on your callback listener host"; its API user page says "The
   PUT method is used by the Open API when sending callbacks". The route is
   `post(callback)` (`provider_callback.rs:189`). A `PUT` to a known path
   would get axum's wrong-method answer, which
   [vpay-api.md](../reference/vpay-api.md) lines 167–168 document as a bare
   `405` for the OAuth router; it has not been measured on `/provider`. It is
   unobserved, because no real MTN callback has reached vpay, and its cost
   would be latency.
3. **Orange's real notification may not carry `order_id`, which
   `parse_callback` requires.** The adapter refuses a body with no `order_id`
   (`vpay-adapter-orange-money/src/lib.rs:608-612`) or no `notif_token`
   (`lib.rs:618-627`; `CallbackBody`, `wire.rs:106-111`). Two third-party
   sources describe the notification as a status, `notif_token` and `txnid`
   ([Paynote](https://www.paynote.africa/comment-deployer-lapi-orange-money/);
   a third-party project's [test report](https://github.com/SamK107/hayaflash/pull/33)).
   Neither is Orange's, so this is unverified; item 1 of
   [adapter-orange-money.md](../flows/adapter-orange-money.md)'s "To confirm"
   list already names the payload. If it holds, every real Orange callback is
   a `400`, and Orange settles by polling alone. The cost is latency.
4. **Smaller:** [reconciler.md](../flows/reconciler.md) line 106 calls
   `scan:live` "hourly"; it reschedules every ten minutes
   (`vpay-worker/src/handlers.rs:151`, `1359`).
5. **Smaller:** `vpay-config/src/config.rs:147-151` justifies the per-rail
   `callback_url` override by saying MTN requires "an IP-allowlisted host" for
   callbacks. No MTN page the research read says so: the best-practices page
   lists IP whitelisting for "Disbursements / Transfers", "Remittances /
   Transfers", "Collections / Pre-Approval" and "Dynamic Journey Services",
   not RequestToPay, and the callback page names no allowlist. Disbursements
   is what vpay's MTN refunds use, so `requires_ip_allowlist: true`
   (`vpay-adapter-mtn-momo/src/lib.rs:726`) may be right for that other
   reason.

### 12. To confirm

In the sandbox (S) or in writing (W).

#### Stripe

1. **(S)** Does `POST /v1/webhook_endpoints` refuse a second endpoint at a URL
   the account already has? No docs.stripe.com page says. A third-party
   report says the Dashboard allows it
   ([issue](https://github.com/woocommerce/woocommerce-gateway-stripe/issues/3326)).
2. **(S)** Does a test-mode create accept an `http://` or unreachable URL? One
   page says endpoints "must be publicly accessible HTTPS URLs"
   ([webhooks](https://docs.stripe.com/webhooks)); the v2 reference says
   "sandbox mode also supports HTTP"
   ([event destinations](https://docs.stripe.com/api/v2/core/event-destinations/create)).
3. **(W)** After how long does Stripe disable a failing endpoint? Only
   support.stripe.com says, and it says "several days".
4. **(S)** Do v1 endpoints and v2 event destinations share the 16, and does
   each list show the other's?
5. **(W)** The Dashboard's label for `webhook_write`, and whether a Stripe
   App's restricted key may request it, with RFC-0010 § 10 item 4.

#### PayPal

6. **(S)** Is `WEBHOOK_URL_ALREADY_EXISTS` per app or per account, and on
   which HTTP status? The schema maps no error to an operation.
7. **(S)** Is `PayPal-Request-Id` honoured on `POST /v1/notifications/webhooks`?
8. **(S)** Which scope the token carries for webhook management:
   `https://uri.paypal.com/services/applications/webhooks` (webhooks-post) or
   `https://uri.paypal.com/services/webhooks`
   ([apps, scopes and credentials](https://developer.paypal.com/apps-scopes-and-credentials)).
9. **(W)** Does PayPal ever disable a failing webhook, and how far back does
   the events API read?

#### Notch Pay

_(Items 10 and 11 were answered in Notch Pay's sandbox on 2026-10-21,
[page](../status/verification/2026-10-21-notchpay-sandbox-run.md), as
RFC-0012 § 10 items 13 and 14: **no** secret, in the create answers or the
list; and **no** refusal, the same URL twice made two endpoints, both `201`.
The same run found what this RFC did not ask: Notch Pay answers
`"url": null` for every endpoint, and `DELETE /webhooks/{id}` answered `404`.
Noted 2026-10-22.)_

10. **(S)** Does `POST /webhooks` return the endpoint's signing secret, as the
    reference says and the schema does not? (RFC-0012 § 10, item 13.)
11. **(S)** Does `POST /webhooks` refuse a URL the account already has?
    (RFC-0012 § 10, item 14.)
12. **(S)** Does a `202` count as delivered? The reference says to "Respond to
    webhooks with a 200 status code … This prevents Notch Pay from retrying";
    vpay answers `202` (`provider_callback.rs:323`).
13. **(W)** How many retries, over how long, and can an endpoint be disabled?
    The schema has no state field.

#### MTN

14. **(W)** Does production expose any read of an API user's
    `providerCallbackHost`?
15. **(S)** On which HTTP status does MTN send `INVALID_CALLBACK_URL_HOST`?
    The "500" is vpay's own reading (`vpay-adapter-mtn-momo/src/mapping.rs:163`);
    MTN's published responses for `requesttopay` are 202, 400, 409 and 500.
16. **(S)** Does MTN call back with `PUT` or `POST` (§ 11, item 2)?

#### Orange

17. **(W)** Is `notif_url` per request only, with no registration or
    allowlist? Does Orange retry? Does the notification carry `order_id`
    (§ 11, item 3)?

## As built (2026-10-22)

The maintainer decided: "the webhook registration should be done
automatically then, e.g via cli or at startup fo the providers that supports
it", then "Yes, start it". That is open question 1 answered `ensure`, and it
names § 2's two entry points. "At startup" is read as § 2 reads it: the job
is seeded at every **worker** start and is due at once. `vpay-server`'s serve
boot asks no rail anything.

What was built, against § 8's port shape and § 2–§ 4's pass:

- **The port.** `CallbackUrlMode`, `Capabilities::callback_url_mode`,
  `EndpointCheck`, `EndpointState` and `ProviderAdapter::ensure_callback_endpoint`
  defaulting to `Unsupported` (`backends/crates/vpay-provider/src/lib.rs`).
  `Measured` forwards and counts the method under the operation label
  `ensure_callback_endpoint`. `mtn_momo` and `orange_money` declare
  `PerRequest`; `notchpay` declares `Registered`.
- **The job.** `JobKind::EnsureCallbacks`, migration
  `0051_jobs-ensure-callbacks.sql` widening `kind_is_known`, the sixth seed
  in `seed_singletons`, and `vpay_worker::callbacks`: the plan, the pass and
  the handler. It reschedules hourly (open question 3, as proposed), five
  minutes after a rail that did not answer or a database that did not, and
  never answers `Err`.
- **The lock.** `vpay_db::lock_keys::CALLBACK_REGISTRATION` (`vpaycbk`),
  taken with `pg_try_advisory_xact_lock` by
  `TxRepositories::try_lock_callback_registration`.
- **The setting.** `providers[].callbacks.register`: `ensure` (the default),
  `check`, or `off`. A `callbacks` block on a `PerRequest` rail refuses to
  boot (`ConfigError::CallbacksOnPerRequestRail`).
- **The command.** `vpay-server callbacks ensure [--check]`. One line per
  rail on stdout, `{rail} {register} {state} {detail}`; non-zero exit when a
  rail is `missing`, `locked` or `failed`.
- **Notch Pay's implementation**
  (`backends/crates/vpay-adapter-notchpay/src/callbacks.rs`), with WireMock
  mappings beside its charge mappings (`webhooks.json`).
- **The two conformance rules.** `the_submit_tells_the_rail_where_to_call_back`
  now asserts its census against the capability, and
  `the_endpoint_check_is_unsupported_exactly_on_a_per_request_rail` is new.

Where it differs from this design, and why:

| This RFC                                                             | As built                                                                                                                                                                                                                                                                                                                                                                           |
| -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| § 3: vpay's endpoint is the marker **at the effective callback URL** | On Notch Pay, the marker **and a tag of the URL in the description**: `vpay:{deployment}:notchpay:default url:{16 hex digits of SHA-256(url)}`. Notch Pay answers `"url": null`, so the URL cannot be read back. An echoed URL is still preferred when there is one                                                                                                                |
| § 3: "someone else's, at our URL" is reported as `foreign`           | **Cannot be seen on Notch Pay**, for the same reason. A merchant's hand-registered endpoint at vpay's URL is invisible, and vpay creates its own beside it: callbacks arrive twice, which the pull-forward absorbs (§ 3)                                                                                                                                                           |
| § 4 step 3: a drifted endpoint of vpay's own is put right            | **Reported, not repaired**, as a new `EndpointState::Drifted`. The Notch Pay client has no `PUT`, and § 5's "both would be added" was not done                                                                                                                                                                                                                                     |
| § 4 step 5: superseded endpoints are retired (open question 4)       | **Never retired on Notch Pay.** `DELETE /webhooks/{id}` answered `404` for endpoints its own list had just returned (2026-10-21). Superseded and duplicate endpoints come back in a new `EndpointReport::retirable` list and are logged at `warn` for an operator to delete, which since 2026-10-23 is the maintainer's decision, not a gap. Open question 4 stays open for Stripe |
| § 8: `ensure_callback_endpoint` answers `EndpointState`              | It answers `EndpointReport { state, retirable }`                                                                                                                                                                                                                                                                                                                                   |
| § 8: the coherence rule is "a line in `Capabilities::is_coherent`"   | A separate `Capabilities::callbacks_are_coherent`, refused at boot as `ConfigError::IncoherentCallbackMode`. `is_coherent` holds only the rules a database CHECK mirrors, and `callback_url_mode` is not persisted                                                                                                                                                                 |
| § 4 step 6: "one gauge per rail and state"                           | A **counter**, `vpay_callback_endpoint_checks_total{provider,state}`, states `present`, `created`, `drifted`, `foreign`, `missing`, `locked`, `failed`. The singleton runs on whichever worker claims it, so a per-process gauge would keep each process's last answer, however old                                                                                                |
| § 4: one transaction per account's pass                              | One transaction per **rail**. With one account per rail (RFC-0009 phase A) these are the same. Two concurrent passes may split the rails between them, each reporting the others `locked`                                                                                                                                                                                          |
| § 2: every configured rail                                           | Every **enabled** configured rail. A rail with `enabled: false` is not asked: an operator who turned it off has not asked vpay to write to its account                                                                                                                                                                                                                             |

§ 10's integration cases, against a real Postgres and the Notch Pay stub
(`backends/tests/integration/tests/callback_registration.rs`): one create,
then found by its description; **two racing passes make one create, and with
the lock removed they make two**, measured by hand; `check` and `--check`
never write, `off` asks nothing, `mtn_momo` is never asked; the seeded job
registers the endpoint and reschedules hourly; a refused key reschedules at
the hour and an unreachable rail at five minutes. Not built from § 10: a
foreign endpoint (undetectable, above), a create whose answer is dropped
(WireMock cannot record a request and then drop its answer), and "every
registration call answering `503` leaves the worker loop running and
`worker_e2e`'s charge settling" — the handler's reschedule is tested, the
full loop with a failing rail is not.

Not done: step 5, the first registration against Notch Pay's sandbox. It
needs the maintainer's approval to call Notch Pay; no public URL is needed
to register one, as the 2026-10-21 run showed.

## Delivery

| Step | Contents                                                                                                                                                                                                                                                                                                                                                                                                                         | Depends on                                                                             |
| ---- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| 0    | § 11's corrections to vpay's own documentation, as dated corrections. No code                                                                                                                                                                                                                                                                                                                                                    | nothing                                                                                |
| 1    | § 12's sandbox items for the first registered rail, on a dated verification page. No code                                                                                                                                                                                                                                                                                                                                        | a test-mode account; may run before 2                                                  |
| 2    | **The first consumer, and with it the port shape**: `CallbackUrlMode`, `Capabilities::callback_url_mode`, `ProviderAdapter::ensure_callback_endpoint` defaulting to `Unsupported`, the coherence rule, `mtn_momo` and `orange_money` declaring `PerRequest`, the two conformance rules of § 8; and the first registered rail's implementation with its WireMock mappings — Stripe's, inside RFC-0010 steps 2–4, on today's state | RFC-0010 step 2 (or RFC-0012 steps 0 and 2, or RFC-0011 step 3, whichever lands first) |
| 3    | The pass: `JobKind::EnsureCallbacks` and the migration widening `jobs.kind`'s `kind_is_known`, the sixth seed in `seed_singletons`, the `vpaycbk` lock key, `providers[].callbacks.register` and its boot refusal, `vpay-server callbacks ensure`, the gauge, § 10's integration cases                                                                                                                                           | 2, in the same pull request                                                            |
| 4    | Each further registered rail's `ensure_callback_endpoint`: PayPal's; Notch Pay's on `vpay-notchpay`'s two calls, plus a `PUT` and a `DELETE` scoped to vpay's marker                                                                                                                                                                                                                                                             | that rail's adapter                                                                    |
| 5    | The first registration against each provider's sandbox, on a dated verification page                                                                                                                                                                                                                                                                                                                                             | 3 or 4 for that rail; a sandbox account                                                |
| 6    | Signature verification: headers reach `parse_callback`, a sealed store for rail secrets (a table, a key and its custody, the AEAD in a crate the worker links), `Created` carrying the secret, and vpay's own endpoints replaced so that each one's secret is held                                                                                                                                                               | RFC-0010 open question 4 decided; 3                                                    |
| 7    | One pass per provider account                                                                                                                                                                                                                                                                                                                                                                                                    | RFC-0009 step 2                                                                        |
| 8    | Model B: Stripe's Connect endpoint, PayPal's webhook lookups                                                                                                                                                                                                                                                                                                                                                                     | 7; RFC-0010 step 6; RFC-0011 step 7                                                    |

Steps 2 and 3 are one pull request against `master` and one against
`vpay-skills`, because together they add a capability, a job kind and a
command a skill must describe. Their wiring includes
[provider-port.md](../flows/provider-port.md)'s capability table,
[reconciler.md](../flows/reconciler.md)'s callback section,
`docs/reference/vpay-worker.md`, a runbook line for the Stripe permission,
and the "Registering the webhook" rows of RFC-0010 § 9 and RFC-0011 § 9,
which move from the merchant to vpay, and RFC-0012 § 9's "until then".

## Alternatives considered

- **Register in `vpay-server`'s boot**, as "at startup" reads literally.
  Refused in § 2: the provider could hold or crash-loop a rollout, and the
  autoscaled surface would register on every scale-out.
- **The operator command alone.** Weighed in § 2 and kept as a companion: it
  brings back the manual step and notices no drift.
- **A table of endpoints, claimed by a lease on its row,** as `jobs` are.
  It holds no connection open during the calls. But a lease can expire under
  a slow pass and admit a second creator, which is the exact failure the lock
  exists to prevent on Notch Pay. And a table holds nothing the provider's
  list does not, until there is a secret to keep (§ 6).
- **Delete and recreate vpay's endpoint at every start.** The simplest
  "ensure". It rotates Stripe's secret on every restart, drops callbacks in
  between, and a crash loop would spend the 16 or 10 slots.
- **One callback URL per provider account.** Refused in § 7: the account in
  the path would be an input the caller chooses.
- **A startup check of MTN's `providerCallbackHost`.** Possible only in the
  sandbox, where `GET /v1_0/apiuser/{id}` exists. A check that can succeed in
  only one environment is a code path production never runs, and the first
  submit already names a mismatch as `ProviderError::Config`.
- **Stripe's v2 event destinations.** Stripe's guide steers new work there,
  and v2 keeps idempotency keys for 30 days rather than v1's 24 hours. But
  RFC-0010 is written against v1's form encoding and snapshot events; a v2
  list redacts the URL unless `include` names it, and v2 returns the secret
  only when asked. v1 carries no deprecation notice. Revisit with RFC-0010's
  next version bump.
- **Adopting a foreign endpoint at vpay's URL.** Refused in § 3: it is
  modifying something vpay did not create.
- **Keeping Stripe's secret from the first build.** Open question 2.
- **Catch-up from event lists.** Refused for charges in § 9.

## Open questions

_(Questions 1, 3 and 4 were settled for Notch Pay on 2026-10-22; see § "As
built". 2, 5 and 6 are open.)_

1. **`ensure` or `check` by default?** `ensure` gives the Stripe key a fourth,
   sensitive permission; `check` keeps the manual step for creation and keeps
   drift detection. (Maintainer.)
2. **Discard Stripe's secret until the port can verify, or build the sealed
   store first?** Recommended: discard (§ 6). (Maintainer.)
3. **The interval.** Hourly is proposed: one list per registered rail and
   account an hour. (Maintainer.)
4. **Retire vpay's superseded endpoints automatically, or only from the
   command?** Recommended: automatically, after the current one is present.
   An endpoint left at a domain the operator gave up sends payment events to
   whoever holds that domain next. (Maintainer.) _(Answered for Notch Pay on
   2026-10-23: neither. The maintainer wrote "#12 is nice but an operator's
   task only, not code". vpay logs each superseded or duplicate endpoint by
   id at `warn`, and an operator deletes it in Notch Pay's dashboard. It is
   still open for Stripe.)_
5. **Re-enable a vpay-created Stripe endpoint that Stripe disabled**: once,
   automatically, with an alert, or alert only? Stripe disabled it because
   deliveries failed; re-enabling without the cause fixed buys another
   failure. (Maintainer.)
6. **Model B's registration unit**, the platform rather than the account
   (§ 7). (Maintainer, with RFC-0009.)

7. **A rail that registers its URL but only in a dashboard.** Flutterwave,
   re-measured on 2026-10-20, documents no webhook-registration endpoint on
   either API version
   ([evaluation](../plans/2026-09-23-card-provider-evaluation.md)). If it gets
   an adapter, it is `Registered` and `ensure_callback_endpoint` stays
   `Unsupported`, so the pass can only report that the URL is the merchant's
   to register. Is a third mode worth naming for that, or is `Unsupported` on
   a `Registered` rail the honest answer already?

## Impact on existing invariants

- **Rails behind the port** holds. The core branches on
  `callback_url_mode`, never on a code, and each rail's wire stays in its
  adapter.
- **Callbacks are hints** holds, unchanged. Registration changes how soon a
  hint arrives, not what it can do.
- **Crash safety** holds. The provider's list is the durable record, read
  before every create, and a create without an idempotency key is never
  repeated blind.
- **Boot never waits on a rail** holds. Nothing is added to boot.
- **No environment branching** holds. The pass runs the same way in a
  sandbox and in production.
- **No mocks in shipping processes** holds. The stubs are WireMock hosts.
- **Rail secrets are configuration** holds in the first build, which stores
  none. Delivery step 6 is where it stops holding, and that needs RFC-0009's
  deferred key-custody decision.
- **Least privilege changes on Stripe.** In `ensure` mode the restricted key
  needs `webhook_write`, a fourth permission beyond RFC-0010 § 2's three.
- **Who registers changes.** RFC-0010 § 9 and RFC-0011 § 9 give it to the
  merchant, and RFC-0012 § 9 does until this RFC lands. Under this RFC it is
  vpay's.
- **`verify-status`**: an adapter that declares `Registered` before its
  method is written carries a `NotImplemented` token and a `docs/status.md`
  line.
- **Pass-through, one charge per intent, integer money** are untouched. No
  money moves.
