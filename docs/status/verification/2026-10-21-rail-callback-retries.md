# 2026-10-21 — No rail is documented to retry a callback, and two callback questions are written down

Branch `docs/rails-callback-retry-claims`, rebased onto `master` at `e393116`.
[RFC-0013](../../rfc/0013-vpay-registers-its-own-callbacks.md) § 11 found
these and its Delivery step 0 asked for them as dated corrections; this lands
step 0 for items **1–3**, not for items 4 and 5 (`reconciler.md`'s "hourly",
and `vpay-config`'s IP-allowlist reason). Documentation, doc comments, the
messages of two test assertions and the text of one `info` log line. **No
route, status code, adapter behaviour, Helm value, migration or
`NotImplemented` token moved.** No MTN or Orange callback has ever reached
vpay, and none was sent for this.

## What was wrong

Several live pages and doc comments said that **both rails — MTN and Orange —
retry a provider callback that is not answered with a 2xx**. The design leaned
on it: it was the first of two reasons `vpay_api::provider_callback` answers
`202` to a notification naming a reference this deployment has no charge for.
None of these places cited a source.

MTN's own documentation says the opposite, on three pages that agree:

- the callback page,
  <https://momodeveloper.mtn.com/api-documentation/callback>: the platform
  sends the callback only once, there is no retry mechanism if the partner
  does not respond, and partners are strongly recommended to poll the
  transaction status with `GET`;
- the "API Methods" section of
  <https://momodeveloper.mtn.com/api-documentation/api-description>: the
  callback is sent once, with no retry, and a partner that misses it can use
  `GET`;
- <https://momodeveloper.mtn.com/api-documentation/use-cases>: a
  `requesttopay` callback is sent once, with no retry.

No official Orange Money Web Payment source says either way. Orange's public
pages (`developer.orange.com/apis/om-webpay`, its overview and FAQ) carry
nothing about the notification, and the getting-started page third parties
cite for it answered `404` on 2026-10-21. This repository's Orange knowledge
was already a reconstruction from community SDKs
([../../flows/adapter-orange-money.md](../../flows/adapter-orange-money.md)).
Third-party claims about whether Orange retries contradict each other and
cite nothing.

**The design did not move.** The other reason was always enough: an
unauthenticated route that answered differently for "this charge exists"
would be an oracle for exactly that question. vpay already does what MTN
recommends — the poll ladder settles every charge whether or not a callback
arrives — so a callback MTN sends once and vpay misses costs latency, not
money.

**Notch Pay is the exception, and it is not new here.** It became a linked
rail on 2026-10-20 (vaam-apps/vpay#295), after the claims above were written.
Its reference asks for a `200` so that it does not retry, and whether it
treats vpay's `202` as delivered is RFC-0013 § 12, item 12, still open. The
corrected texts say so where it changes their meaning (the module docs,
`reconciler.md`, `provider-callback.md` and the Helm comment).

## Every place corrected

Each keeps the old wording in a dated note, in the style of the page it is on.

| Place                                                                                          | What it said                                                                                | What it says now                                                                                                                   |
| ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `vpay_api::provider_callback`, module docs, "The four answers"                                 | two reasons, the first "MTN and Orange both do, on their own schedules"                     | one reason, the oracle; the MTN URL and what it says; Notch Pay's open question                                                    |
| the same module, `callback`'s `# Errors`                                                       | a failed enqueue "is the one case the rail _should_ retry"                                  | the one case a resent notification would help, and MTN never resends one                                                           |
| the same module, the unknown-reference `info` line                                             | "accepting it anyway so the rail stops retrying"                                            | "accepting it anyway, with the same 202 a known reference gets"                                                                    |
| [../../flows/reconciler.md](../../flows/reconciler.md)                                         | "a rail retries anything that is not a 2xx"                                                 | the oracle alone; a dated note, and a line in § Status                                                                             |
| [../../reference/vpay-api/provider-callback.md](../../reference/vpay-api/provider-callback.md) | "Both rails retry a non-2xx on their own schedules"                                         | the oracle alone, and a dated note                                                                                                 |
| [../../api/README.md](../../api/README.md), the callback route                                 | "a rail must not be told to retry forever"                                                  | the oracle, and a dated note                                                                                                       |
| [../backend.md](../backend.md), the "Rail callbacks" row                                       | the same clause                                                                             | struck through, with a dated correction linking here                                                                               |
| `deploy/helm/vpay/values.yaml`, `providerCallback.limitBurstMultiplier`'s comment              | the burst "exists because both rails retry on a non-2xx and their retries arrive together"  | headroom for several charges settling at once and for any Notch Pay retries, as unmeasured as the rate; **the value is unchanged** |
| `vpay_db::jobs`, `pull_forward_in_tx`'s guards                                                 | duplicate callbacks "which both rails send"                                                 | which anyone who can reach the unauthenticated route can send                                                                      |
| `vpay_worker::jobs::poll_dedupe_key`                                                           | "a callback arrived, and so did a retry of it"                                              | "… a copy of it"                                                                                                                   |
| `backends/tests/integration/tests/provider_callback.rs`                                        | one doc comment ("both rails retry a non-2xx") and two assertion messages about retry loops | the oracle, and what each assertion actually checks                                                                                |

[../../flows/adapter-mtn-momo.md](../../flows/adapter-mtn-momo.md) § "The
callback is unsigned and unauthenticated" now states the fact where a reader
of that adapter will look for it: MTN sends the callback once. RFC-0013 § 11
carries a dated note that step 0 landed for items 1–3, and corrects one
detail of its item 2: a `GET` on `/provider/mtn_momo/callback` **is**
measured as a `405`; only a `PUT` is not.

**Left as written, on purpose:** `docs/plans/2026-09-03-step8-production-gate.md`
and `docs/plans/step8-notes/lane-c.md` repeat the same reasoning ("the rail
must not retry forever"). Both are point-in-time working documents whose
header says the status pages are the record and the plan is history, and
this page is that record.

**One small correction made on the way, unrelated to callbacks:**
[../../flows/adapter-orange-money.md](../../flows/adapter-orange-money.md)
§ "What this rail therefore cannot say" sent readers to "item 9" of its "To
confirm" list for `FAILED` sub-reasons; that is item 10.

## The two open questions, recorded and not acted on

Neither changed code. Each is a "To confirm" item in its adapter's flow doc,
with its sources.

1. **Which HTTP method MTN sends the callback with** (RFC-0013 § 11, item 2;
   § 12, item 16) —
   [../../flows/adapter-mtn-momo.md](../../flows/adapter-mtn-momo.md),
   "To confirm with MTN", item 1. The callback page says the callback will be
   a `POST`, and its own setup steps tell partners to allow `PUT` and `POST`
   on the listener; the "API Methods" section says the Open API sends
   callbacks with `PUT`. `vpay_api::provider_callback` mounts `POST` only, so
   a `PUT` meets axum's bare `405`, as a `GET` does
   (`a_get_on_the_callback_path_is_a_405_and_not_the_404_envelope` pins the
   `GET`; no test sends a `PUT`). If MTN sends `PUT`, every MTN callback is
   refused and, since MTN sends each once, lost. That costs latency only.
2. **Whether Orange's notification carries `order_id` at all** (RFC-0013
   § 11, item 3; § 12, item 17) —
   [../../flows/adapter-orange-money.md](../../flows/adapter-orange-money.md),
   "To confirm with Orange Cameroun", item 11. `parse_callback` requires it.
   RFC-0013's two third-party sources say the body is `status`, `notif_token`
   and `txnid` and nothing else:
   [SamK107/hayaflash#33](https://github.com/SamK107/hayaflash/pull/33)
   (Mali; a body received from a real payment on 2026-09-27) and
   [Paynote's guide](https://www.paynote.africa/comment-deployer-lapi-orange-money/)
   (not fetchable on 2026-10-21). A third agrees:
   [pretix-orange-money-mdg](https://github.com/mikaoelitiana/pretix-orange-money-mdg/blob/e9d206d58c97b08a4b3203e00f6a27b5fc1f8d4c/pretix_orange_money_mdg/views.py)
   (Madagascar; a comment citing Orange's "Transaction Notification"
   section), and so does
   [izifacture's Orange webhook](https://github.com/malibs/izifacture/blob/14cfeaae0363e70490d795fb7a2d1b0d67e70d63/app/api/payments/webhook/orange/route.ts).
   [afrotools' spec for Orange Mali](https://github.com/afrotools/afrotools/blob/a02a60573d17bad4b17cd6897a50fc6939a9c1fd/specs/payment/orangemoney-mali/webhook_payment_completed/schema.json)
   lists `order_id` and cites nothing. None that was read is Orange
   Cameroun; Paynote's could not be. If they are right for Cameroon, every
   real Orange callback gets a `400` and a `warn`, and the poll ladder
   settles the charge anyway.

## How each source was read

| Source                                                            | Read how, and when                                                                                                                                                                                |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| MTN callback page                                                 | Read by the maintainer on 2026-10-20. Re-read on 2026-10-21 through the HTML widget the page embeds (`/content/html_widgets/c0m2f.html`, HTTP 200), because the page itself is a JavaScript shell |
| MTN "API Methods" (`api-description`)                             | The same way, widget `ouv7x.html`, HTTP 200, 2026-10-21                                                                                                                                           |
| MTN use cases                                                     | The same way, widget `1t1oe.html`, HTTP 200, 2026-10-21                                                                                                                                           |
| `developer.orange.com/apis/om-webpay-dev/getting-started`         | `404` to `WebFetch` on 2026-10-21 (`403` to plain `curl`). A Wayback Machine snapshot from 2023-01-27 exists and was **not** read: the archive refused the tools available                        |
| `developer.orange.com/apis/om-webpay` (overview, FAQ)             | Read by a research sub-agent; nothing on the notification body or on retries                                                                                                                      |
| pretix `views.py`, izifacture `route.ts`, afrotools `schema.json` | Fetched raw at the pinned commits linked above on 2026-10-21; the field lists stated here were checked against the files                                                                          |
| SamK107/hayaflash#33                                              | `gh pr view` on 2026-10-21: merged, created 2026-09-27, the three-field body quoted in its description                                                                                            |
| Paynote's guide                                                   | **Not read.** `WebFetch`, the browser pane and `curl` all failed on 2026-10-21; what is said of it here is RFC-0013's reading                                                                     |

## What ran, on 2026-10-21 (macOS arm64)

On the branch rebased onto `6d9dfbf`, with Docker 29.8.0 for the containers:

```text
$ cargo fmt --all -- --check                                   # exit 0
$ cargo clippy -p vpay-api -p vpay-db -p vpay-worker \
    -p vpay-tests-integration --all-targets -- -D warnings     # exit 0
$ cargo nextest run -p vpay-api -p vpay-db -p vpay-worker
     Summary [ 199.900s] 771 tests run: 771 passed, 0 skipped
$ cargo nextest run -p vpay-tests-integration --test provider_callback
     Summary [  40.735s] 11 tests run: 11 passed, 0 skipped
$ cargo test --doc -p vpay-api -p vpay-db -p vpay-worker
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
$ just verify
verify-links: ok — 2545 repository link(s) in 473 tracked markdown file(s) resolve to a tracked path
verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s) agree with what this tree measures
verify: ok — the fifteen gates above passed; the verify-docs report is advisory
$ just helm-check
helm-check: ok — lint, 4 renders, 24 guards, rate limit (both paths), rail callback (both paths), …
$ npx prettier@3.9.6 --check <every changed .md and .yaml>
All matched files use Prettier code style!
$ npx markdownlint-cli2@0.20.0 --config <super-linter's default> <every changed .md>
Summary: 0 error(s)
```

The `provider_callback` suite is the one whose assertion messages and doc
comment changed, and it runs against a real Postgres and a real WireMock rail
(ADR-0006). `verify-doc-counts` measures the directory, so the page count is
checked rather than asserted.

Rebased once more onto `e393116` (the 0.10.0 release, vaam-apps/vpay#315 and
#317, none of which touches a file here but the status index), and re-ran
what that could move: `just verify` (exit 0; `verify-links` 2553 links in 475
files, `verify-versions` 0.10.0, `verify-doc-counts` ok), the same `clippy`
(exit 0) and the same prettier check (clean).

## What did not run, and what was not done

- **No rail callback.** Neither question above is answered; both wait on the
  first real callback from each rail, on a deployment whose callback host the
  rail can reach.
- **No code was changed for either question.** Mounting `PUT`, or finding an
  Orange charge by something other than `order_id`, are decisions that
  should follow an observation.
- **RFC-0013 § 11 items 4 and 5 are not done.** `reconciler.md` still calls
  `scan:live` "hourly", and `vpay-config/src/config.rs` still gives MTN's
  IP allowlist as the reason for the per-rail `callback_url` override.
- **The Wayback snapshot of Orange's old getting-started page, and Paynote's
  guide, were not read.** Someone with an ordinary browser can open both;
  the snapshot's section 3.3 is the closest thing to Orange's own text this
  repository has located.
- **Not the whole of `just ci`.** `test-rust` ran for the three crates whose
  doc comments changed and for the one integration binary whose messages
  did, not workspace-wide; `lint-web`, `test-web`, `audit-web` and `deny`
  did not run, because nothing under their reach changed.
- **The burst multiplier in `values.yaml` keeps its value.** Its stated reason
  is gone and the new one is no more measured than the old.
