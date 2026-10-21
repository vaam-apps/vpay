# 2026-10-21 — the merchant SDKs refuse an empty or all-dots id

Branch `claude/determined-ptolemy-c39f3b`, from `master` at `e95dc3d8`
(#293), where everything below was run. `master` was then merged in at
`f7c7ae33` (#303), which touches Notch Pay code and docs only. The merge
conflicted in this directory's index alone. On the merged tree, `just
verify` passed again (`verify-doc-counts` read 83 pages then), `vpay-sdk`
191/191, `sdks/nodejs` 234/234, and `fmt-check-rust` and clippy on the SDK
and both Notch Pay crates were clean. It was merged in once more at
`5b4c583c` (#304, Notch Pay only), with the same one conflict, in the same
index. That index now counts 84 pages. The change itself: both merchant
SDKs, their tests, their READMEs and the status pages. **No route, no server
code, no migration, no gate and no `NotImplemented` token moved.**

## What was wrong

Both SDKs interpolate a caller's id into a URL path with
`encodeURIComponent`'s rule: `sdks/rust`'s private `path_segment` calls
`crate::form::percent_encode`, which reproduces it, and `sdks/nodejs`
called `encodeURIComponent` itself at 19 sites covering 21 methods. That
rule keeps `/`, `?` and `#` inside the segment, and leaves `.` alone. So
`""`, `.` and `..` were sent unchanged, and as whole segments the URL
parser each SDK sends through — the `url` crate under reqwest, WHATWG `URL`
under `fetch` — resolved them before the request left. Escaping the dots
could not help: both parsers read `%2E%2E` and `.%2e` as `..`. This is the
gap [`docs/reference/rails.md`](../../reference/rails.md) § "A path segment
the caller chose" named on 2026-10-20 and left unmeasured.

## What vpay answered, before the fix

Measured first, against a compose stack of this tree (`compose.yml`,
`compose.e2e.yml` and `compose.demo.yml`, project `vpay-dotseg` on port
18080, the six services `just sdk-live` brings up, merchant `demo-merchant`
from `just gen-demo-keys`). Each SDK was built from `master` at `e95dc3d8`
before any change: a throwaway example binary linking `sdks/rust`, and
`sdks/nodejs`'s `dist`. Neither program was committed. Each called all
**21** id-taking methods with `""`, `.`, `..` and `...`: 84 calls per SDK,
168 in all.

**The two SDKs requested the same path and got the same answer on every one
of the 84**, by a `diff` of the two outputs (method, resolved path, status,
error `code`, message). The path below is the one the server recorded. Its
request span logs `path`, and the 162 `api error` lines it wrote name every
path in the table except the six 405s, which axum answers without reaching
the error envelope.

| Method                               | `""`                                                                  | `.`                                                                                 | `..`                                           | `...`                                                                    |
| ------------------------------------ | --------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------ |
| `payment_intents.retrieve` (GET)     | `/v1/payment_intents/` — 404 `unknown_route`                          | `/v1/payment_intents/` — 404 `unknown_route`                                        | `/v1/` — 404 `unknown_route`                   | `/v1/payment_intents/...` — 404 `resource_missing`, id `...`             |
| `payment_intents.confirm` (POST)     | `/v1/payment_intents//confirm` — 404 `resource_missing`, id empty     | `/v1/payment_intents/confirm` — **405**, empty body                                 | `/v1/confirm` — 404 `unknown_route`            | `/v1/payment_intents/.../confirm` — 404 `resource_missing`, id `...`     |
| `payment_intents.cancel` (POST)      | `/v1/payment_intents//cancel` — 404 `resource_missing`, id empty      | `/v1/payment_intents/cancel` — **405**, empty body                                  | `/v1/cancel` — 404 `unknown_route`             | `/v1/payment_intents/.../cancel` — 404 `resource_missing`, id `...`      |
| `checkout.sessions.retrieve` (GET)   | `/v1/checkout/sessions/` — 404 `unknown_route`                        | `/v1/checkout/sessions/` — 404 `unknown_route`                                      | `/v1/checkout/` — 404 `unknown_route`          | `/v1/checkout/sessions/...` — 404 `resource_missing`, id `...`           |
| `checkout.sessions.expire` (POST)    | `/v1/checkout/sessions//expire` — 404 `resource_missing`, id empty    | `/v1/checkout/sessions/expire` — **405**, empty body                                | `/v1/checkout/expire` — 404 `unknown_route`    | `/v1/checkout/sessions/.../expire` — 404 `resource_missing`, id `...`    |
| `customers.retrieve` (GET)           | `/v1/customers/` — 404 `unknown_route`                                | `/v1/customers/` — 404 `unknown_route`                                              | `/v1/` — 404 `unknown_route`                   | `/v1/customers/...` — 404 `resource_missing`, id `...`                   |
| `customers.update` (POST)            | `/v1/customers/` — 404 `unknown_route`                                | `/v1/customers/` — 404 `unknown_route`                                              | `/v1/` — 404 `unknown_route`                   | `/v1/customers/...` — 404 `resource_missing`, id `...`                   |
| `customers.del` (DELETE)             | `/v1/customers/` — 404 `unknown_route`                                | `/v1/customers/` — 404 `unknown_route`                                              | `/v1/` — 404 `unknown_route`                   | `/v1/customers/...` — 404 `resource_missing`, id `...`                   |
| `invoices.retrieve` (GET)            | `/v1/invoices/` — 404 `unknown_route`                                 | `/v1/invoices/` — 404 `unknown_route`                                               | `/v1/` — 404 `unknown_route`                   | `/v1/invoices/...` — 404 `resource_missing`, id `...`                    |
| `invoices.update` (POST)             | `/v1/invoices/` — 404 `unknown_route`                                 | `/v1/invoices/` — 404 `unknown_route`                                               | `/v1/` — 404 `unknown_route`                   | `/v1/invoices/...` — 404 `resource_missing`, id `...`                    |
| `invoices.del` (DELETE)              | `/v1/invoices/` — 404 `unknown_route`                                 | `/v1/invoices/` — 404 `unknown_route`                                               | `/v1/` — 404 `unknown_route`                   | `/v1/invoices/...` — 404 `resource_missing`, id `...`                    |
| `invoices.finalize` (POST)           | `/v1/invoices//finalize` — 404 `resource_missing`, id empty           | `/v1/invoices/finalize` — 404 `resource_missing`, id `finalize`                     | `/v1/finalize` — 404 `unknown_route`           | `/v1/invoices/.../finalize` — 404 `resource_missing`, id `...`           |
| `invoices.void` (POST)               | `/v1/invoices//void` — 404 `resource_missing`, id empty               | `/v1/invoices/void` — 404 `resource_missing`, id `void`                             | `/v1/void` — 404 `unknown_route`               | `/v1/invoices/.../void` — 404 `resource_missing`, id `...`               |
| `invoices.mark_uncollectible` (POST) | `/v1/invoices//mark_uncollectible` — 404 `resource_missing`, id empty | `/v1/invoices/mark_uncollectible` — 404 `resource_missing`, id `mark_uncollectible` | `/v1/mark_uncollectible` — 404 `unknown_route` | `/v1/invoices/.../mark_uncollectible` — 404 `resource_missing`, id `...` |
| `invoices.pay` (POST)                | `/v1/invoices//pay` — 404 `resource_missing`, id empty                | `/v1/invoices/pay` — 404 `resource_missing`, id `pay`                               | `/v1/pay` — 404 `unknown_route`                | `/v1/invoices/.../pay` — 404 `resource_missing`, id `...`                |
| `invoice_items.retrieve` (GET)       | `/v1/invoice_items/` — 404 `unknown_route`                            | `/v1/invoice_items/` — 404 `unknown_route`                                          | `/v1/` — 404 `unknown_route`                   | `/v1/invoice_items/...` — 404 `resource_missing`, id `...`               |
| `invoice_items.update` (POST)        | `/v1/invoice_items/` — 404 `unknown_route`                            | `/v1/invoice_items/` — 404 `unknown_route`                                          | `/v1/` — 404 `unknown_route`                   | `/v1/invoice_items/...` — 404 `resource_missing`, id `...`               |
| `invoice_items.del` (DELETE)         | `/v1/invoice_items/` — 404 `unknown_route`                            | `/v1/invoice_items/` — 404 `unknown_route`                                          | `/v1/` — 404 `unknown_route`                   | `/v1/invoice_items/...` — 404 `resource_missing`, id `...`               |
| `refunds.retrieve` (GET)             | `/v1/refunds/` — 404 `unknown_route`                                  | `/v1/refunds/` — 404 `unknown_route`                                                | `/v1/` — 404 `unknown_route`                   | `/v1/refunds/...` — 404 `resource_missing`, id `...`                     |
| `refunds.update` (POST)              | `/v1/refunds/` — 404 `unknown_route`                                  | `/v1/refunds/` — 404 `unknown_route`                                                | `/v1/` — 404 `unknown_route`                   | `/v1/refunds/...` — 404 `resource_missing`, id `...`                     |
| `refunds.cancel` (POST)              | `/v1/refunds//cancel` — 404 `resource_missing`, id empty              | `/v1/refunds/cancel` — 404 `resource_missing`, id `cancel`                          | `/v1/cancel` — 404 `unknown_route`             | `/v1/refunds/.../cancel` — 404 `resource_missing`, id `...`              |

Read across, that is:

- **Nothing succeeded.** 94 `404 unknown_route`, 68 `404 resource_missing`
  and 6 `405`s with an empty body. One token request per SDK run. Afterwards
  `pg_stat_user_tables` showed inserts into no resource table, and the fresh
  stack held no resource to change or delete. Axum does not fold a trailing
  slash, so `customers.update("")`, a `POST /v1/customers/`, did not become
  a create.
- **`.` sends a sub-resource call to a different handler.**
  `invoices.finalize(".")` is a `POST /v1/invoices/finalize`, which the
  router serves as **`invoices.update` on an invoice whose id is
  `finalize`**. The same goes for `void`, `mark_uncollectible`, `pay` (with
  the URLs in its body) and `refunds.cancel`. Each answered 404 only because
  vpay's ids are prefixed and nothing is named `finalize`.
  `payment_intents.confirm`/`cancel` and `checkout.sessions.expire` landed on
  a `GET`-only route and got a bodiless `405`. Both SDKs surface that as an
  unexpected-response error, not an API error.
- **`""` addresses an empty id**: `POST /v1/invoices//finalize` was matched
  with `{id}` empty and answered `No such invoice:` with nothing after the
  colon. Any hop that merges `//` would turn it into the `.` case.
- **`..` climbs out of the resource**, to `/v1/`, or to `/v1/confirm`,
  `/v1/finalize` and the like. `customers.del("..")` was a `DELETE /v1/`.
- **`...` is an ordinary segment**, as the standards say, and every method
  answered `404 resource_missing` naming it.

**The one cost that outlived a request: the `Idempotency-Key`.** Every
`POST`/`DELETE` that reached a route, the 56 `resource_missing` answers
among the 168, left a `complete` row in `idempotency_keys` holding that
404, and the row lives 24 hours (`expires_at` defaults to `now() + 24h`).
The 404s that hit no route (`unknown_route`) stored nothing. With the key
each SDK generates, that is a dead row. With a **pinned** key, the
corrected retry is refused. `sdks/rust`'s `RequestOptions` doc recommends a
pinned key, derived from the merchant's order id, for a retry of "the
_same_ logical operation" across restarts, and the Node SDK takes one as
`idempotencyKey`. Measured with the pre-fix Node SDK on a fresh draft
invoice:

```text
1. finalize("") with key K: Api 404 invalid_request_error code=resource_missing msg="No such invoice: "
2. finalize("in_zt5r9xnrwn7454xf1hse09wa") with the same K: Api 400 idempotency_error code=idempotency_key_in_use msg="The Idempotency-Key beginning order-do… was already used with a different request body."
3. finalize("in_zt5r9xnrwn7454xf1hse09wa") with a fresh key: OK invoice … status=open
```

The same sequence through the fixed SDK, on another fresh draft:

```text
1. finalize("") with key K: TypeError: invalid request parameter id: an id that is empty or only dots would address a different endpoint once the URL is parsed; nothing was sent
2. finalize("in_fzt1fcfdw57vqa1gfs8t3tty") with the same K: OK invoice in_fzt1fcfdw57vqa1gfs8t3tty status=open
```

So the harm to a merchant is small and real: misleading errors, a request
that reaches a handler for a different operation, and, with a pinned key, a
legitimate retry locked out for a day. Nothing worse was reachable on this
router. That rests on two facts nothing guarantees: that vpay's ids can
never be `finalize`, and that no hop in front of vpay folds `//` or a
trailing slash.

**After the fix, against the same stack:** both SDKs built from this branch
ran the same 84 calls each. All 168 came back as the local refusal: 84
`Error::InvalidParams` and 84 `TypeError`, each with the fixed message.
`docker compose logs vpay-server --since` the start of that run printed
**0** lines, and `idempotency_keys` gained no row.

## The change

- **`sdks/rust`.** `path_segment` returns `Result<String, crate::Error>` and
  refuses an id whose bytes are all `.` — the empty id included — as
  `Error::InvalidParams { param: "id", message }`, with one fixed message
  that never quotes the input. Every one of the 21 id-taking methods calls
  `path_segment(id)?` while building its path, which is before `Client::get`,
  `post` or `delete`, so a refused id mints no token either. **No public
  signature changed**: every method already returned `Result<_, Error>`,
  `Error` is `#[non_exhaustive]`, and `InvalidParams` already existed. The 21
  methods' `# Errors` sections, `InvalidParams`' own doc and the README's
  errors table and § Status say so.
- **`sdks/nodejs`.** `src/validate.ts` gains `pathSegment(id)`, which throws
  a `TypeError` for the same ids — the prefix `invalid request parameter id:`
  and the same sentence, byte for byte — and otherwise returns
  `encodeURIComponent(id)`. All 19 interpolation sites call it. **No
  signature changed**: every method was already `async`, so the throw is a
  rejected promise. It is a `TypeError` and not a `VpayError`, like the
  amount refusal; `docs/sdks/parity.md`'s dated gap for that is widened to
  name both rather than leaving the package with two refusal types.
- **`...` and longer runs are refused by decision**, the one the server-side
  port took on 2026-10-20. They are not dot segments, and both parsers keep
  `...` as an ordinary segment (pinned by the premise test below). Every id these
  methods take is a `pi_`, `cs_`, `cus_`, `in_`, `ii_` or `re_` id
  (`backends/crates/vpay-core/src/ids.rs`), so nothing legitimate is all
  dots; and between a merchant and vpay sit the merchant's egress proxy and
  vpay's ingress, whose normalisers nobody here has read.
- **The payer surfaces were read and are not exposed.** `sdks/stripe-js`'s
  `parseClientSecret` (`src/client.ts`) and the Flutter plugin's
  `_tryParseClientSecret` (`lib/src/browser_client.dart`) take the id out of
  a `pi_…_secret_…` or `cs_…_secret_…` secret and require it to start with
  that prefix and be longer than it, so the segment can never be empty or
  all dots. The Tauri plugin builds no path of its own: its guest-JS calls
  stripe-js, and its Rust, Kotlin and Swift hosts only open URLs.
  `sdks/stripe-compat` is a private test harness whose paths the official
  `stripe` package builds. No test in any of them names a dot segment, so
  the protection is a property of the secret's shape, not a guard.

## What ran, on 2026-10-21 (macOS arm64, rustc 1.98.0, cargo-nextest 0.9.137, Node 24.14.1)

```text
$ cargo nextest run -p vpay-sdk              191 tests run: 191 passed, 0 skipped
                                             (187 on master at e95dc3d8; +4)
$ pnpm exec vitest run   (sdks/nodejs)       Tests 234 passed (234), 9 files
                                             (230 on master at e95dc3d8; +4)
$ cargo nextest run -p vpay-sdk --features live-stack --test live_invoices --test live_refunds
                                             5 tests run: 5 passed, 0 skipped
$ pnpm --filter @vaam-apps/vpay-sdk test:live
                                             Tests 6 passed (6), 2 files
$ just fmt-check-rust                        exit 0
$ just clippy                                exit 0   (cargo clippy --workspace --all-targets -- -D warnings)
$ pnpm --filter @vaam-apps/vpay-sdk lint     exit 0   (eslint . --max-warnings 0)
$ pnpm --filter @vaam-apps/vpay-sdk typecheck exit 0
$ just verify                                verify: ok — the fifteen gates above passed; the verify-docs report is advisory
                                             verify-sdk-parity: ok — 778 proving test(s) named in docs/sdks/parity.md all exist,
                                               45 dated gap(s), 35 SDK method(s) enumerated across 41 row(s)
                                               (769 / 45 / 35 / 41 on master at e95dc3d8)
                                             verify-links: ok — 2445 repository link(s) in 463 tracked markdown file(s)
                                             verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s)
$ just test-doc                              130 passed, 0 failed, 1 ignored (16 doctest runs; no doctest added)
$ just verify-ignored                        0 ignored (expected 0), 53 test binaries (expected 53), 2322 total (minimum 1080)
$ cargo nextest run --workspace --no-fail-fast -E 'not (<the four staff_sign_in loopback cases>)'
                                             Summary [1856.313s] 2318 tests run: 2318 passed (2 slow), 4 skipped
```

The live suites ran against the same compose stack as the measurement, after
it, with both SDKs built from this branch. No dot id appears in them; they
are here because every one of the 21 methods changed how it builds its path,
and a refusal that misfired on a real id would show there first.

The four new tests in each SDK, by the names `docs/sdks/parity.md` cites:

| What it pins                                                                                                                                         | `sdks/rust` (`tests/resources.rs`)                                   | `sdks/nodejs` (`src/client.test.ts`)                                                    |
| ---------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `""`, `.`, `..`, `...`, `....` refused on all 21 methods, and the stub server saw **no** request, the token endpoint included                        | `an_empty_or_all_dots_id_is_refused_before_any_request`              | `an empty or all-dots id is refused on every method that takes one, before any request` |
| one fixed message, naming `id`, for every input                                                                                                      | `the_dot_id_refusal_is_one_fixed_message_naming_id`                  | `the dot-id refusal is one fixed message naming id`                                     |
| `a..b`, `.a`, `a.`, `..a`, `cus_1.`, `%2E%2E`, `.%2e`, a dot and a space, `./`, `..\` are still sent, each as one segment, with `%` escaped to `%25` | `an_id_with_dots_that_is_not_only_dots_is_still_sent_as_one_segment` | `an id with dots that is not only dots is still sent, as one segment`                   |
| the premise: the parser resolves `""`, `.`, `..`, `%2E%2E`, `.%2e`, and keeps `...`                                                                  | `the_url_parser_resolves_a_dot_segment_id_before_the_request_leaves` | `the URL parser resolves a dot-segment id before the request leaves`                    |

The first test in each SDK also counts the id-taking methods off the source
text. Rust counts `pub async fn`s in `src/resources.rs` whose signature takes
`id: &str`. Node counts public `async` methods in `src/resources/*.ts` whose
first parameter is `id: string`; the private `#transition` is not counted.
The test fails if its list is shorter than that count, and it pins the count
at **21**.

### Mutations, each reverted after the run

| Mutation                                                                                      | Caught by                                                                                                                                    |
| --------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Rust M1: no refusal (`if id.is_empty() && false`)                                             | `an_empty_or_all_dots_id_is_refused_before_any_request`, `the_dot_id_refusal_is_one_fixed_message_naming_id`                                 |
| Rust M2: refuse only `""`, `.`, `..` (the standard's set)                                     | the same two; the first reports `payment_intents.retrieve("...") was not refused: Err(UnexpectedResponse { status: 404, body_prefix: "" })`  |
| Rust M3: `invoices.void` interpolates the raw id                                              | `an_empty_or_all_dots_id_is_refused_before_any_request`, reporting `invoices.void("") was not refused`                                       |
| Rust M4: also refuse any id containing `..`                                                   | `an_id_with_dots_that_is_not_only_dots_is_still_sent_as_one_segment`                                                                         |
| Rust M5: `refunds.cancel` dropped from `every_id_taking_call`                                 | `an_empty_or_all_dots_id_is_refused_before_any_request` (`left: 20`, `right: 21`)                                                            |
| Node M1: no refusal (`if (false)`)                                                            | `an empty or all-dots id is refused on every method that takes one, before any request`, `the dot-id refusal is one fixed message naming id` |
| Node M2: refuse only `""`, `.`, `..`                                                          | the same two                                                                                                                                 |
| Node M3: `#transition` (finalize, void, markUncollectible) uses `encodeURIComponent` directly | `an empty or all-dots id is refused on every method that takes one, before any request`, reporting `invoices.finalize("")`                   |
| Node M4: also refuse any id containing `..`                                                   | `an id with dots that is not only dots is still sent, as one segment`                                                                        |
| Node M5: `refunds.cancel` dropped from `everyIdTakingCall`                                    | `an empty or all-dots id is refused on every method that takes one, before any request` (length 20, expected 21)                             |

## The companion change

- **`vaam-apps/vpay-skills`**: `vpay-sdks` gains a section on the refusal.
  It says a new id-taking method must call the helper and get a line in both
  test lists, that `...` is refused by decision, and that the Node
  `TypeError` is deliberate. `references/parity.md` records the four rows and
  the 769 → 778 move.

## What did not run

- **No deployed vpay and no reverse proxy.** The measurement is one compose
  stack on one machine, with nothing between the SDK and `vpay-server`. Which
  of these requests a real ingress (or a merchant's egress proxy) would have
  normalised differently, for instance by folding `//` or a trailing slash,
  was not measured. The fix makes that question moot for these inputs. It
  does not answer it.
- **The pre-fix Rust run used a throwaway example binary**, built from
  `master` at `e95dc3d8` and not committed. The pre-fix Node run used that
  commit's `dist`. Neither program is in the tree. Their output and the
  server's logs are summarised above, not reproduced in full.
- **The payer surfaces' safety was established by reading, not by a test.**
  No test in `sdks/stripe-js`, the Flutter plugin or the Tauri plugin sends
  a dot-segment secret. Adding one is a follow-up, not part of this change.
- **The Node refusal is still not a `VpayError`.** That dated gap is widened
  here, not closed.
- `just fmt-check-web` fails on this machine, on one file:
  `.claude/settings.local.json`. That is a local harness file excluded by the
  user's global gitignore, so CI never sees it. Every tracked file passes
  `prettier --check`. The seven Markdown files this change touches pass
  `npx markdownlint-cli2@0.20.0` under super-linter's default config.
- `check-schema` warned that `cratestack` 0.15.1 is on `PATH` while the
  repository pins 0.15.0, and checked the schema under 0.15.1. That is this
  machine's install, and unrelated to this change.
- **Four `staff_sign_in` cases** were excluded from the workspace run. They
  need macOS loopback aliases (`just loopback-aliases`, which needs `sudo`).
  CI's Linux `rust` job runs them. They do not touch either SDK.
