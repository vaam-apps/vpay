# 2026-10-21 — every local refusal in `sdks/nodejs` is a `VpayError`

Branch `claude/node-refusals-vpay-error`, from `master` at `73a5b5af` (#312),
where everything below was run. `master` was then merged in at `73c3a7cf`
(#309, #311, #318). The conflicts were only in this directory's index and in
`merchant-sdks.md`, where #309's payer row and this change's row landed in
the same place. Both rows are kept. On the merged tree `just verify` passed
again: `verify-sdk-parity` reads 790 proving tests (#309 added ten) at 44
gaps, and `verify-doc-counts` reads 90 pages. `sdks/nodejs` is 236/236. One SDK, its tests, its README and the
status pages. **No route, no server code, no wire byte, no migration, no
gate and no `NotImplemented` token moved.**

## What was wrong

`docs/sdks/parity.md` had carried a dated ⛔ since 2026-09-03 in the Node
column of "The refusal is one of **this SDK's own** error types, catchable
with the rest". Every refusal `sdks/nodejs` makes before sending threw a
bare `TypeError`. The README documents one catch-all,
`catch (err) { if (err instanceof VpayError) … }`, and it let every one of
them through. `sdks/rust` returns `Error::InvalidParams { param, message }`
for the same inputs. On 2026-10-21, vaam-apps/vpay#306 added a sixth such
site (the empty-or-all-dots id) and widened the gap rather than make the
package's refusals disagree with each other.

The six sites:

| Site                                               | Refuses                                                   | `param`                       |
| -------------------------------------------------- | --------------------------------------------------------- | ----------------------------- |
| `src/validate.ts` `assertIntegerAmount`            | an amount outside `0..=2^53-1`                            | `amount`, `unit_amount`, …    |
| `src/validate.ts` `pathSegment`                    | an empty or all-dots path id                              | `id`                          |
| `src/form.ts` `encodeNumber`, first guard          | a number that is not a safe integer                       | its form path, e.g. `amount`  |
| `src/form.ts` `encodeNumber`, second guard         | a number whose `String()` has an exponent (unreachable\*) | its form path                 |
| `src/form.ts` `flattenValue`                       | an object that is not plain (`Date`, `Map`, an instance)  | its form path, `metadata[x]`  |
| `src/resources/invoices.ts` `InvoicesResource.pay` | `success_url`/`cancel_url` beside `outOfBand`             | `success_url` or `cancel_url` |

\* The second guard is subsumed by the first as the language stands. Its own
comment says no test can kill it, and none does here.

## The change

- **`src/errors.ts`** exports `VpayInvalidParamsError extends VpayError`,
  with a `readonly param: string`. Its constructor takes `(param, detail)`
  and sets `message` to `invalid request parameter {param}: {detail}`, which
  is `sdks/rust`'s `Display` for `InvalidParams`. `src/index.ts` exports it.
- **All six sites throw it.** The dot-id message is byte-identical to
  Rust's, as it already was. The amount message keeps Node's own wording
  ("must be a non-negative safe integer in minor units, got …") behind the
  new prefix. It never matched Rust's sentence, and this change does not
  make it.
- **It is not a `TypeError`, and cannot be.** A class has one superclass.
  The only way to keep `instanceof TypeError` true would be to make
  `VpayError` itself extend `TypeError`, so that every error this package
  throws, a `VpayTransportError` included, would claim to be one. So a caller
  that tested `instanceof TypeError` for these refusals no longer matches.
  That is a **breaking change**, named in `sdks/nodejs/README.md` § Status
  and in the PR title's `!`. No signature changed, and no input that was
  sent before is refused now.
- Doc comments that promised a `TypeError` (`validate.ts`, `form.ts`,
  `invoices.ts`'s `pay` and both `invoiceItems` methods, two fields in
  `types.ts`) now name the class.

## What ran, on 2026-10-21 (macOS arm64, Node 24.14.1, rustc 1.98.0)

```text
$ pnpm exec vitest run   (sdks/nodejs)       Tests 236 passed (236), 9 files
                                             (234 on master at 73a5b5af; +2)
$ pnpm exec tsc -p tsconfig.json --noEmit    exit 0
$ pnpm exec eslint . --max-warnings 0        exit 0
$ cargo xtask verify-sdk-parity              ok — 780 proving test(s), 44 dated gap(s),
                                               35 SDK method(s) across 41 row(s)
                                               (778 / 45 / 35 / 41 on master at 73a5b5af)
$ cargo test -p vpay-sdk --doc               9 passed, 0 failed, 1 ignored
                                             (sdks/rust/README.md is a doctest include; it changed)
$ just fmt-check-rust                        exit 0
$ just lint-web                              exit 0   (build-sdk-node, then pnpm -r typecheck and pnpm -r lint,
                                               every consumer of @vaam-apps/vpay-sdk included)
$ just test-web                              exit 0   (pnpm -r test: sdks/nodejs 236, sdks/stripe-js 146,
                                               the Tauri plugin 71, examples/shop 108, checkout 556 + 1 skipped,
                                               dashboard 408 + 1 skipped, config 66, tokens 10, api-client 4)
$ just verify                                verify: ok — the fifteen gates above passed; the verify-docs report is advisory
                                             verify-links: ok — 2493 repository link(s) in 467 tracked markdown file(s)
                                             verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s)
```

**The tests.** Two are new:

- `every local refusal is a VpayInvalidParamsError naming its parameter, caught by instanceof VpayError, before any request`,
  in `src/client.test.ts`. It goes through the real request path, one call
  per kind: a negative amount on `paymentIntents.create`,
  `customers.del("..")`, a URL beside `outOfBand` on `invoices.pay`, and a
  `Date` in `metadata`. Each must be a `VpayError` and a
  `VpayInvalidParamsError`, must not be a `TypeError`, must carry the
  expected `param`, and must have a message that starts with the prefix
  `invalid request parameter {param}:` and a space. The stub server must have seen no
  request at all, the token endpoint included: the body is encoded, and the
  id checked, before the token is fetched.
- `refuses with a VpayInvalidParamsError carrying the wire name, not a TypeError`,
  in `src/validate.test.ts`. It pins the class shape and the exact message.

**26 existing assertions** that expected `TypeError` now expect the new
class, in `client.test.ts`, `form.test.ts`, `validate.test.ts` and the live
`refunds.live.test.ts`. **Ten test titles** that said "throws TypeError"
were renamed with them. One of them,
`throws VpayInvalidParamsError on an integer past MAX_SAFE_INTEGER`, is
cited by `docs/sdks/parity.md`'s amount row, and that cell moved in the same
change, which is why `verify-sdk-parity` still finds every name.
`the dot-id refusal is one fixed message naming id` now asserts the class,
`param === "id"` and the exact message, instead of `toThrow(new TypeError(…))`.

### Mutations, each reverted after the run

| Mutation                                                          | Caught by                                                                                                                                        |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `invoices.pay`'s refusal back to `throw new TypeError(…)`         | `every local refusal is a VpayInvalidParamsError naming its parameter, caught by instanceof VpayError, before any request`                       |
| `VpayInvalidParamsError extends TypeError` instead of `VpayError` | the same, and `refuses with a VpayInvalidParamsError carrying the wire name, not a TypeError`                                                    |
| `encodeNumber`'s first guard back to `throw new TypeError(…)`     | `throws VpayInvalidParamsError on a number that renders with an exponent`, `throws VpayInvalidParamsError on an unsafe integer or a non-integer` |

## The companion change

- **`vaam-apps/vpay-skills`**: `vpay-sdks`' `SKILL.md` said the Node
  `TypeError` was deliberate and that "turning one into a `VpayError` without
  the other leaves the package with two refusal types". Both are now true of
  the past only, and the page and `references/parity.md` say so, with the new
  class, its `param`, and the `instanceof TypeError` break.

## What did not run

- **`refunds.live.test.ts` was not run.** It needs a compose stack, and this
  change touched one assertion in it (`toBeInstanceOf(TypeError)` →
  `toBeInstanceOf(VpayInvalidParamsError)`). The same refusal is asserted
  against the stub in `client.test.ts`. CI's `e2e` job runs the live suite.
- **No consumer outside this repository was checked.** A merchant that
  matched `TypeError` breaks, and nothing here can find one. Inside the
  repository, no package catches a `TypeError` from this SDK. `git grep`
  finds `TypeError` in `examples/` only in `examples/tauri-checkout`, and
  those are `@vaam-apps/vpay-stripe-js`'s own constructor errors.
- `docs/adr/0015-sdk-parity.md` still names this gap as an example. An ADR
  is immutable, so it is left as written.
- `audit-web` was not run (it needs the network and is unaffected).
