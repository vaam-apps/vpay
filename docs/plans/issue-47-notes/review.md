# Issue #47 — sabotage review

Reviewer's record for `git diff 65a5952..4b2c8c7` (the implementation) and
the six commits that follow it. Written 2026-09-06. The implementer's own
account is [impl.md](impl.md); this file records what was **checked** rather
than what was intended, and what changed as a result.

The short version: the implementation is unusually honest about its own
gaps, and the eight-row mutation table in `impl.md` reproduces exactly as
written. Two mutations the implementer did not run found real holes, both in
the adapter, and both are fixed here. The rest of the findings are
documentation claims the tree does not support.

---

## 1. The gate, recipe by recipe

`just ci` on `4b2c8c7`, on this host, Node 22.23.2 (`.nvmrc`), rustc 1.98.0
(`rust-toolchain.toml`), `pnpm install --frozen-lockfile` first. **Exit 0.**

| Recipe | Result |
|---|---|
| `fmt-check` | ok |
| `clippy` (`--workspace --all-targets -D warnings`) | ok |
| `verify` | ok — the ten gates. `verify-status`: 1 unimplemented item (`mtn_momo::refund`), all declared and all still in shipping code — this change adds **no** new token, which is correct, because Orange's absence is a capability and not unbuilt work. `verify-errors`: 16 error types, all classified. `verify-sdk-parity`: 350 named proving tests all exist, 28 dated gaps. `verify-serde`: 51 types, 16 exempted with a reason (`BasicUserInfo` is the new one). `verify-toolchain`, `verify-links` (738 links), `verify-no-mocks`, `verify-npm-scope`, `verify-repositories`, `check-schema`: ok |
| `test-rust` | **1319 passed, 0 skipped, 0 ignored**, 42 binaries, 1134 s |
| `test-doc` | 94 passed, 1 ignored |
| `verify-ignored` | 0 ignored (expected 0), 42 binaries (expected 42), 1319 total (floor 1080) |
| `lint-web`, `test-web` | ok — checkout 302, shop 57, both SDKs |
| `deny` | advisories, bans, licenses, sources ok |

Advisory, not a gate, and worth a reader knowing: `verify-docs` puts
`v1/account_holders.rs:retrieve` in its "production functions of 80 lines or
more" list at **112 lines**, third longest in the workspace. Left alone — the
function is a straight-line validate/ask/render with no branching to hide in,
and splitting it to satisfy an advisory report would spread one request's
story over three places.

## 2. Acceptance criteria

Against the ISSUE's Proposal §§1–3 and the brief's "What to build".

| Criterion | Verdict |
|---|---|
| §2 port method, defaulted to `Unsupported` exactly as `refund` | delivered |
| §2 `Capabilities::supports_account_holder_lookup` | delivered |
| §2 `mtn_momo` `true` + implemented; `orange_money` `false` | delivered |
| §1 `GET /v1/account_holders`, four documented keys | delivered |
| §1 `null`/`false` for no record; a rail that could not be asked is never a 200 | delivered, and it is the property with the most tests behind it |
| §1 keyed on `payment_method_type`, refused on the capability value | delivered — and the unknown/disabled/incapable refusals are byte-identical, which the issue did not ask for and which is right |
| brief: `AccountHolder { name }` and nothing else | delivered, **deviated for the better**: a newtype with a private field and a redacting `Debug` rather than `String`. Judged: the deviation is the reason the projection survives a careless `{:?}` |
| brief: merchant scope, livemode consistent, E.164 validated | scope bound (and unused — see below); msisdn validated server-side for the first time in this repo; **no `livemode` on the object**, deviated, argued in D5, correctly left as a maintainer decision because there is no row to read it off |
| brief: no persistence; masked number in logs; no name in logs | delivered |
| brief: metric per outcome, no label carrying the number | delivered |
| brief: conformance cases parameterised over both adapters | delivered — 5 × 2, from one body, branching on the capability value and not on the rail |
| brief: WireMock mappings in the compose-mounted directory + a demo pair | delivered |
| brief: both SDKs at parity + a `parity.md` row | delivered, 5 rows, all naming tests that exist |
| brief: `docs/flows/account-holder-lookup.md`, `status.md`, `provider-port.md` | delivered |
| §3 **name only** | delivered |
| §3 **rate limited, per merchant** | **NOT built** — reserved |
| §3 **audit-logged** | **NOT built** — reserved |
| §3 **a scope of its own** | **NOT built** — reserved |
| §3 no caching beyond the request | delivered (nothing is stored at all) |

The three reserved items are correctly reserved, not quietly defaulted: each
is argued in the flow doc and in the module header, each names what the
maintainer has to decide, and the audit-log one correctly points out that it
*contradicts* the no-persistence rule the same section asks for. That is a
choice this repository does not let an implementer make quietly, and it was
not made.

`MerchantScope` is extracted and unused. That is deliberate and documented:
there is nothing to scope, and binding it is what keeps the *authentication*
boundary structural. Checked that the extractor fails closed and that
`the_route_is_not_reachable_without_a_token` covers the route directly rather
than only through `V1_ROUTES`' loop.

## 3. Mutations

Every row in `impl.md`'s table was re-run. All eight reproduce. Two new ones
did not, and both are now fixed.

| # | Mutation | Before | Now |
|---|---|---|---|
| M1 | the adapter's `debug!` carries `body = %text` | CAUGHT (conformance PII case) | CAUGHT |
| M1b | **`tracing::debug!(?parsed)` — the wire struct, not the body** | **MISSED** | **CAUGHT** |
| M2 | `#[serde(deny_unknown_fields)]` on `BasicUserInfo` | — | CAUGHT (so the attribute would be wrong here, which is the question the brief asked) |
| M4 | an `msisdn` label on the counter | CAUGHT | CAUGHT |
| M5 | the route logs the unmasked number | CAUGHT | CAUGHT |
| M6 | `404` becomes an `Err` | CAUGHT (unit table) | CAUGHT |
| M7 | Orange declares the capability `true` | CAUGHT (`case_2`, the real assertion, not the helper) | CAUGHT |
| M8 | a transport failure becomes `Ok(None)` | CAUGHT | CAUGHT |
| M9 | the route drops `canonical_msisdn` | CAUGHT | CAUGHT |
| M10 | `verified` is always `true` | CAUGHT | CAUGHT |
| M12 | **delete the `path_segment` call from the adapter** | **MISSED — 113 tests green** | **CAUGHT** |
| M13 | leak *one half* of the holder's name | — | CAUGHT (the widened assertion) |

**M1b, the privacy one.** `wire::BasicUserInfo` derived `Debug`, so a
`tracing::debug!(?parsed)` — the single most likely debugging edit anyone
will ever make on this path — printed `given_name: Some("Amina"),
family_name: Some("Nkeng")`. The conformance case passed, because it greps
for the *joined* `"Amina Nkeng"`: a form the adapter produces and the rail
never sends, so the string it looks for is not the form the leak takes. Fixed
in both directions — a redacting `Debug` on the wire type (structural) and
assertions on the two halves (so a leak that bypasses `Debug` still fails).

**M12, the escaping one.** `a_payer_reference_is_escaped_before_it_becomes_a
_path_segment` asserted on `vpay_provider::http::path_segment` in isolation.
Deleting the call from `account_holder_name` left 113 tests green, because
every stubbed MSISDN is digits-only and escaping them is a no-op. The URL
construction is now `account_holder_url`, a pure function, and the test is
rewritten onto it — pinning the ordinary path byte for byte against the
WireMock mapping, a traversal staying inside its segment, and a `?` not
truncating the path.

Not reproduced independently and taken on the implementer's word: nothing.
Two conformance runs hit a 120 s container-start timeout under load from
other agents on this host and were re-run (the brief's retry rule); the
`404 -> Err` and `Orange true` mutations are recorded from runs that failed
on a real assertion, not on `start()`.

## 4. Documentation claims

Checked rather than read. 393 test names cited across the account-holder docs
and code were extracted and matched against the tree; three were missing, all
three offered as the proof of a privacy or correctness claim. Other findings:
the `capabilities()` comment spelled the path segment upper-case while the
adapter sends lower-case; `just test-doc` moved 90 -> 94 with every entry
above the new one still saying "unchanged at 90"; the metric's own constants
described a narrower vocabulary than the code emits; Orange's item 8 counted
three sibling cases where there are four. All fixed in `e82fb9b`, with the
reasoning in that commit message.

Two claims that turned out **true** and are worth recording as checked rather
than assumed, because they would have been easy to get wrong:

* the new capability table in `provider-port.md` — every cell verified against
  the two adapters, including the `supports_partial_refunds => supports_refunds`
  CHECK in migration `0002`;
* `api/README.md`'s "twelve methods across ten paths" and its
  `502 provider_unavailable` — both match `V1_ROUTES` and
  `Category::Rail`'s code.

## 5. Reserved for the maintainer

Recorded, not decided:

1. **Rate limiting, audit logging, `identity:read`** (issue §3). The flow
   doc's Status section now carries the consequence as a deployment
   condition — *not safe to expose to untrusted merchants without ingress
   rate limiting* — rather than only as a bullet in a list of absences. A
   per-merchant in-process bucket was **not** built and should not be: this
   deployment's rate limiting is an ingress concern, and inventing a limit
   nobody chose inside one handler is the wrong shape.
2. **`livemode` on the object** (D5).
3. **A fifth `account_holder_outcome` value** for a merchant's malformed
   request, so `error` means rail trouble only. Recorded on the constant.
4. **`404 -> Ok(None)`.** Weighed and **kept**. Treating an undocumented 404
   as an `Err` would turn every mistyped number into a 502 the merchant reads
   as an outage, and the integrator fails closed either way — so the mapping
   is the better default. What was missing and is now recorded is the way it
   compounds with the unverified path-segment case: the two wrong *together*
   render every lookup as a silent, plausible `{ name: null }`. Reversal cost
   is one constant or one match arm.

## 6. Closes or Refs

**`Refs #47`, not `Closes #47`.** Unmet, from the issue's own Proposal §3:

- no per-merchant rate limit,
- no audit log,
- no dedicated scope.

Three of the five controls §3 calls the minimum for a route that returns a
third party's name. The route is built, tested and honest about this; it is
not the whole of what the issue asked for, and the tracker should say so.
