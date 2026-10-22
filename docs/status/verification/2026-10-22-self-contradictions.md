# 2026-10-22 — fourteen self-contradictions vpay carried at v0.10.2

Branch `claude/nervous-williams-fdaa3b`, from `master` at `4f3344e`.
vaam-apps/vpay-docs re-read all of its pages against v0.10.2
([vaam-apps/vpay-docs#15](https://github.com/vaam-apps/vpay-docs/pull/15)) and
listed the places where vpay disagreed with its own code, or with itself. Each
was re-checked here on `master`. Wording only, as dated corrections in the
style of the page each sits on, plus one code comment. **No code path, value,
route, gate, migration, test or `NotImplemented` token moved**, and no status
cell moved except the one row below whose evidence is nine release runs.

## Corrected

| Place                                                                                                                              | It said                                                                                       | What is true, and the evidence                                                                                                                                                                                                                                                                              |
| ---------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [../../runbooks/release.md](../../runbooks/release.md) § 9 and [../infrastructure.md](../infrastructure.md), the `notify-docs` row | `notify-docs.yml` has never run on a real tag                                                 | It has run on every `v*` tag since it merged, v0.5.0 to v0.10.2, and every run succeeded; each was followed by a `repository_dispatch` run of vpay-docs' `release-parity` within two and a half minutes. Runs below. The infrastructure row moves from 🟡 to ✅ on that evidence                            |
| [../../../AGENTS.md](../../../AGENTS.md), "Documentation"                                                                          | vpay-docs' `release-parity` "opens one issue per release"                                     | It opens one draft PR per release. Issues were only its first version, replaced the same day, 2026-09-23 (vpay-docs `c9fa0ad` → `70778fe`)                                                                                                                                                                  |
| [../frontend.md](../frontend.md), the dashboard SSO row                                                                            | the SSO callback 303s to `/payments`                                                          | `src/server/sso.ts` redirects to `HOME_PATH`, which is `/overview` since #318 (2026-10-21)                                                                                                                                                                                                                  |
| [../frontend.md](../frontend.md), the dashboard row, and [../../flows/dashboard.md](../../flows/dashboard.md)                      | status chips on every list                                                                    | on every list but customers: `CUSTOMERS_SCREEN` in `src/dash/list-screens.ts` declares no filter                                                                                                                                                                                                            |
| [../../flows/dashboard/status-styling-and-demo.md](../../flows/dashboard/status-styling-and-demo.md), Lane C                       | `GET /dash/v1/payment_intents` is the only payments list the frontend can reach               | narrowed: since #318 the Overview calls `searchPaymentIntents` too (`src/dash/overview.ts`); the `/payments` list still reads `GET /dash/v1/payment_intents`                                                                                                                                                |
| `frontends/apps/dashboard/README.md`, `/login/password`                                                                            | there is no "current password" field                                                          | wrong since 2026-09-10 (`e9e9a845`): the page asks for it, and `vpay-api`'s `change_password` refuses an absent one with the same `401` as a wrong one                                                                                                                                                      |
| `frontends/apps/dashboard/README.md`, the BFF section and "Not proven"                                                             | the payments screens' later reads, paging and refetches go through `/api/dash`                | true from 2026-09-12 until #318, which set `staleTime: Infinity` and `gcTime: 0` in `DashProviders`; the next page of payments is one server read, and a twelve-step click-through made no BFF request ([2026-10-21-dashboard-md3e.md](2026-10-21-dashboard-md3e.md))                                       |
| `frontends/apps/dashboard/src/server/session.ts`, `HOME_PATH`'s comment                                                            | the Overview has "six exact counts"                                                           | twelve: all payments, one per payment status, and six that need a look (`readOverview` in `src/dash/overview.ts`)                                                                                                                                                                                           |
| [../../flows/adapter-orange-money.md](../../flows/adapter-orange-money.md), Status                                                 | "All **ten** to confirm items"                                                                | eleven: #319 added item 11, whether the notification carries `order_id`. The page's own note from 2026-09-16 records the same slip for item 10                                                                                                                                                              |
| [../../runbooks/README.md](../../runbooks/README.md)                                                                               | `live-sandbox-test.md` is "the one page whose commands have been run against a real rail"     | the first: the next row records `notchpay-sandbox-test.md` run against Notch Pay on 2026-10-21                                                                                                                                                                                                              |
| [../../flows/payment-lifecycle.md](../../flows/payment-lifecycle.md), "What still has never happened"                              | every settlement observed so far came from a WireMock host                                    | wrong since 2026-09-15: MTN's sandbox settled charges through the worker that day ([2026-09-15.md](2026-09-15.md)), and Notch Pay's on 2026-10-21, a success and a failure ([2026-10-21-notchpay-step5.md](2026-10-21-notchpay-step5.md)). Now "against a production rail"                                  |
| [../../flows/adapter-notchpay.md](../../flows/adapter-notchpay.md) and [../../flows/reconciler.md](../../flows/reconciler.md)      | the poll ladder settles every charge without callbacks                                        | it reads every charge. Step 5's short collection was read for fifteen minutes and never settled, and by the code it becomes `unresolved` at the horizon                                                                                                                                                     |
| [../../flows/failures.md](../../flows/failures.md)                                                                                 | no Notch Pay status "has been read from Notch Pay itself"; no real rail has declined a charge | step 5's reads mapped to `Pending`, `Succeeded` and, from `failed`, `Failed { provider_error }`, and a short `complete` was refused as `Malformed`. That `failed` is a rail-reported failure, but `provider_error` is the catch-all, so no real rail has yet declined a charge with a reason the tables map |
| [../../sdks/parity.md](../../sdks/parity.md), Flutter, "Exercised against a real rail"                                             | quotes the banner "no HTTP call to a real rail has ever been made"                            | that clause was retired on 2026-09-15 and narrowed again on 2026-10-21; neither sandbox run drove the Flutter plugin, so the row's ⛔ stands and only a dated note was added                                                                                                                                |

The `notify-docs` runs, from `gh run list -R vaam-apps/vpay --workflow
notify-docs.yml` and `gh run list -R vaam-apps/vpay-docs --workflow
release-parity.yml --event repository_dispatch`, read 2026-10-22:

| Tag     | `notify-docs` run (vaam-apps/vpay)                                        | Started              | `release-parity` dispatch run (vaam-apps/vpay-docs)                                      | Started              |
| ------- | ------------------------------------------------------------------------- | -------------------- | ---------------------------------------------------------------------------------------- | -------------------- |
| v0.5.0  | [35844839387](https://github.com/vaam-apps/vpay/actions/runs/35844839387) | 2026-09-23T09:45:44Z | [vpay-docs/35844855673](https://github.com/vaam-apps/vpay-docs/actions/runs/35844855673) | 2026-09-23T09:45:53Z |
| v0.6.0  | [37899303149](https://github.com/vaam-apps/vpay/actions/runs/37899303149) | 2026-10-09T07:29:24Z | [vpay-docs/37899317602](https://github.com/vaam-apps/vpay-docs/actions/runs/37899317602) | 2026-10-09T07:29:33Z |
| v0.7.0  | [39112751766](https://github.com/vaam-apps/vpay/actions/runs/39112751766) | 2026-10-18T08:46:33Z | [vpay-docs/39112772420](https://github.com/vaam-apps/vpay-docs/actions/runs/39112772420) | 2026-10-18T08:46:53Z |
| v0.8.0  | [39392894109](https://github.com/vaam-apps/vpay/actions/runs/39392894109) | 2026-10-20T08:22:11Z | [vpay-docs/39393189063](https://github.com/vaam-apps/vpay-docs/actions/runs/39393189063) | 2026-10-20T08:24:37Z |
| v0.9.0  | [39505617949](https://github.com/vaam-apps/vpay/actions/runs/39505617949) | 2026-10-20T22:06:24Z | [vpay-docs/39505645715](https://github.com/vaam-apps/vpay-docs/actions/runs/39505645715) | 2026-10-20T22:06:38Z |
| v0.9.1  | [39571456359](https://github.com/vaam-apps/vpay/actions/runs/39571456359) | 2026-10-21T08:43:37Z | [vpay-docs/39571474280](https://github.com/vaam-apps/vpay-docs/actions/runs/39571474280) | 2026-10-21T08:43:46Z |
| v0.10.0 | [39609650769](https://github.com/vaam-apps/vpay/actions/runs/39609650769) | 2026-10-21T13:37:42Z | [vpay-docs/39609909270](https://github.com/vaam-apps/vpay-docs/actions/runs/39609909270) | 2026-10-21T13:39:34Z |
| v0.10.1 | [39668289852](https://github.com/vaam-apps/vpay/actions/runs/39668289852) | 2026-10-21T20:12:34Z | [vpay-docs/39668318477](https://github.com/vaam-apps/vpay-docs/actions/runs/39668318477) | 2026-10-21T20:12:46Z |
| v0.10.2 | [39715612085](https://github.com/vaam-apps/vpay/actions/runs/39715612085) | 2026-10-22T03:12:59Z | [vpay-docs/39715623547](https://github.com/vaam-apps/vpay-docs/actions/runs/39715623547) | 2026-10-22T03:13:07Z |

Every run in both columns concluded `success`. v0.10.2 has a second pair,
[39715665930](https://github.com/vaam-apps/vpay/actions/runs/39715665930) and
[vpay-docs/39715685881](https://github.com/vaam-apps/vpay-docs/actions/runs/39715685881).
The dispatch run for v0.10.2 is the one that opened vpay-docs' parity PR for
that release (vaam-apps/vpay-docs#14 names it). The tags before v0.5.0 predate
the workflow.

## Re-checked and left as they are

Four things the vpay-docs re-read listed turned out not to be contradictions:

- **`scan:live` "hourly"** in [../../flows/reconciler.md](../../flows/reconciler.md)
  was already corrected after v0.10.2, by #326.
- **`worker_recovery`'s "23"** in
  [../../flows/crash-safety.md](../../flows/crash-safety.md) sits inside two
  dated "what was re-run to say so" paragraphs, of 2026-09-07 and 2026-09-08.
  It was the count then. The file has 28 `#[tokio::test]` cases now.
- **`sdks/nodejs/README.md`'s `TypeError`** paragraph of 2026-10-21 already
  carries its own same-day correction pointing at `VpayInvalidParamsError`.
- **`docs/status.md`'s gate table**, "`verify-status` … 1 unimplemented item",
  is a column the page dates itself: what the gates printed on 2026-09-18 at
  `bd5c85d1`, when there was one token. Today's run prints 2 (below); the
  whole column is that dated snapshot, so it was left alone rather than
  refreshed one cell at a time.

## What ran, on 2026-10-22 (macOS arm64)

```text
$ just verify                                   # before the change, on 4f3344e
verify-status: ok — 2 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify: ok — the fifteen gates above passed; the verify-docs report is advisory
$ just verify                                   # after
verify-status: ok — 2 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-links: ok — 2603 repository link(s) in 480 tracked markdown file(s) resolve to a tracked path (anchors and http(s) URLs are not checked)
verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s) agree with what this tree measures
verify: ok — the fifteen gates above passed; the verify-docs report is advisory
$ pnpm exec prettier --check <every changed file>          # prettier 3.9.6
All matched files use Prettier code style!
$ npx markdownlint-cli2@0.20.0 --config <super-linter's default> <every changed .md but AGENTS.md> <a misaligned control file>
Summary: 2 error(s)        # both in the control file; 0 in the 15 changed files
$ just docs-check-citations                     # exit 1, on one id this change does not touch
  - run 4673…3454 does not exist (HTTP 404), cited at docs/runbooks/live-sandbox-test.md:178, docs/runbooks/live-sandbox-test.md:236, docs/status/verification/2026-09-15.md:30
```

Every id this change cites resolves. 268 ids resolved, and the one failure is
`4673…3454`, which is MTN's stock sandbox test MSISDN and not a run. It is
eleven digits and standalone, so the check reads it as a run id; it fails the
same way on `master`, in ~~three files~~ three places this change does not
edit. The check is opt-in, so no CI job has caught it. Left for its own change,
which would list ~~those three~~ those `(file, id)` pairs as not claims.

_(Corrected 2026-10-22, the same day, by the change that landed that fix —
[2026-10-22-msisdn-citation.md](2026-10-22-msisdn-citation.md). The three
places are in **two** files, since the runbook writes the number twice, so the
fix listed two pairs, not three. And the number above was printed in full until
then, which made **this page** a third file citing it: `verify-citations` cannot
tell a quotation of its own output from a claim. Two middle digits are dropped,
as [2026-09-16-privacy-inventory-gate.md](2026-09-16-privacy-inventory-gate.md)
already does for the same output, rather than this page being exempted too —
a page recording a dangling citation must not become one.)_

`verify-citations` reads any standalone eleven-digit number as a run of this
repository. The vpay-docs runs above are therefore written
`vpay-docs/<id>`, which it skips as attached, the same way it skips
`authkestra#287`: written bare, each was resolved against vaam-apps/vpay and
reported as a missing run, on the first run of the check.

## What did not run, and what was not done

- **Not `just ci`.** One code comment changed, in
  `frontends/apps/dashboard/src/server/session.ts`; nothing else that compiles
  did. No test was added, because nothing a test can observe moved.
- **No vpay-skills change here.** None of these corrections adds or removes a
  flow page, mounts a route, retires a token, moves a gate or a pin, or moves a
  path a skill cites, so the parity rule does not require one. Two skills do
  repeat an old claim, read at vpay-skills `c54bad3`:
  `vpay-tooling/references/ci.md` says `notify-docs` "has not run on a real
  tag", and `vpay-dashboard/references/read-seam-and-bff.md` lists the
  dashboard README's "no current password field" as still wrong. Both want a
  dated note once this merges.
