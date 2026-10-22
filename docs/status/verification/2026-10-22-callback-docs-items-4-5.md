# 2026-10-22 — RFC-0013 § 11 items 4 and 5: `scan:live` is not hourly, and MTN does not require an IP-allowlisted callback host

Branch `claude/happy-hopper-r3h6rz`, from `master` at `90fe1b9` (#324,
release v0.10.2). This completes RFC-0013's delivery step 0. vaam-apps/vpay#319
landed items 1–3 on 2026-10-21
([2026-10-21-rail-callback-retries.md](2026-10-21-rail-callback-retries.md)).
These are the last two.

**No behaviour moved.** The change is documentation, two Rust comments, one
WireMock mapping's description strings, two Helm template messages and two
`values.yaml` comments. No route, migration, capability value, gate or
`NotImplemented` token changed. In particular MTN's `requires_ip_allowlist`
is still `true` (see item 5).

## Item 4: `scan:live` runs every ten minutes, not hourly

`vpay_worker::handlers::SCAN_INTERVAL` is `Duration::from_secs(10 * 60)`
(`handlers.rs:151`). `scan_live_charges` asks for live charges stale by
`SCAN_INTERVAL` and ends with `Outcome::RescheduleAfter(SCAN_INTERVAL)`. The
job has run every ten minutes since it was written. The hourly job is a
different one, `sweep:expired` (`SWEEP_INTERVAL`, `handlers.rs:103`).

Three documents called `scan:live` hourly. Each now says every ten minutes,
with a dated correction:

| File                            | Was                                                       |
| ------------------------------- | --------------------------------------------------------- |
| `docs/flows/reconciler.md`      | "The hourly `scan:live` job is a **backstop only**"       |
| `docs/flows/crash-safety.md`    | "The hourly `scan:live` backstop covers only what…"       |
| `docs/runbooks/worker-queue.md` | "invisible until the hourly backstop scan (`scan:live`)…" |

RFC-0013 named only `reconciler.md`. The other two were found by searching
every tracked file except `docs/plans/` and `docs/status/verification/` (dated
history) for "hourly", "every hour" and "once an hour" near `scan:live`,
"backstop" or "stale". Every other hourly claim that search found is about
`sweep:expired`, which is hourly. That covers `checkout_sessions.rs`'s
comment and `docs/status/backend.md`'s expiry-sweep row. The places that
already said ten minutes needed nothing: `vpay-db`'s `jobs.rs`,
`docs/reference/vpay-api/provider-callback.md`, and
`docs/flows/webhooks/events-api-and-recovery.md`.

## Item 5: the `callback_url` override and the IP allowlist

`vpay-config`'s `ProviderHost::callback_url` doc comment justified the
override this way: "a rail whose callbacks must arrive on a separate ingress
(an IP-allowlisted host — MTN requires one,
`Capabilities::requires_ip_allowlist`)". No MTN page that RFC-0013's research
read says that:

- MTN's **Best Practices** page
  (<https://momoapi.mtn.com/content/html_widgets/j2w89.html>, read
  2026-10-20) lists IP whitelisting for "Disbursements / Transfers",
  "Remittances / Transfers", "Collections / Pre-Approval" and "Dynamic
  Journey Services". RequestToPay is not in the list, and the page does not
  say whose IPs are meant. The same page says callbacks "originate from
  approved IP ranges" and recommends validating the source, but publishes no
  ranges.
- MTN's **Callback** page
  (<https://momodeveloper.mtn.com/api-documentation/callback>, read
  2026-10-20) names no allowlist. What it does require is that the
  `X-Callback-Url` "must be hosted on the same domain specified when creating
  the API user", the `providerCallbackHost`.

That second rule is a real, documented reason for a rail to need a callback
host other than `public_base_url`, so the comment now gives it. The sentence
the old comment seeded, "MTN additionally allows an IP-allowlisted callback
host of its own", is corrected wherever it was repeated:

| File                                                                 | Now says                                                                                                                                      |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `backends/crates/vpay-config/src/config.rs`                          | the override exists for a separate callback host; MTN's is the `providerCallbackHost` rule, quoted and sourced; dated note on the old wording |
| `docs/reference/rails.md`                                            | the host varies because a rail may have a callback host of its own, MTN's being the registered `providerCallbackHost`; dated note             |
| `deploy/helm/vpay/values.yaml` (`ingress.provider.sourceRange`)      | MTN says callbacks come from approved, unpublished IP ranges, so an operator who has a rail's ranges can pin them here; dated note            |
| `deploy/helm/vpay/values.yaml` (`servedElsewhere`)                   | "a callback host of its own", not "an IP-allowlisted host of its own"                                                                         |
| `deploy/helm/vpay/templates/ingress-provider.yaml`                   | the same as `sourceRange`, in the comment rendered beside the annotation                                                                      |
| `deploy/helm/vpay/templates/_validate.tpl`                           | the `provider-callback-routable` guard's example says "a callback host of the rail's own (`providers[].callback_url`)"                        |
| `deploy/helm/vpay/README.md`                                         | turning the callback Ingress off is defensible because a rail can have its own callback host, MTN's being the registered one; dated note      |
| `backends/tests/conformance/wiremock/mtn/mappings/requesttopay.json` | the description strings say the same as `rails.md`                                                                                            |

**`requires_ip_allowlist: true` on `mtn_momo` is unchanged.** It now carries a
comment: the value is not a callback requirement, and it rests on the Best
Practices page's Disbursements entry, unconfirmed. `Adapter::refund` is MTN's
Disbursements `transfer`. RFC-0013 § 11 says the flag "may be right for that
other reason", and nothing here measured it either way. Changing a capability
value on documentation alone is what step 0 rules out ("No code").

## Measured

Searches over every tracked file except `docs/plans/` and
`docs/status/verification/`, after the change:

- **"hourly", "every hour" or "once an hour" near `scan:live`, "backstop
  scan" or `scan_live`:** two hits, both in RFC-0013 itself. One is § 11 item
  4's own text and one is the dated note this change adds.
- **"allowlisted host", "allow-listed host", "requires one," or "MTN
  requires":** the remaining hits fall into three groups:
  - the dated corrections this change adds, in `config.rs`, the chart's
    `README.md` and `values.yaml`, and RFC-0013;
  - § 11 item 5's own text;
  - one unrelated sentence, `docs/runbooks/rotate-rail-credentials.md`'s
    "MTN requires `target_environment` + `api_user`…".

Gates on this branch:

- **`cargo fmt --all -- --check`:** exit 0.
- **`cargo clippy -p vpay-config -p vpay-adapter-mtn-momo --all-targets -- -D warnings`:** exit 0.
- **`cargo test -p vpay-adapter-mtn-momo`:** 89 passed and 1 doctest passed,
  0 ignored. The integration and conformance tests that load the MTN WireMock
  mappings need Docker, which this machine does not have. CI's `rust` job
  runs them. The mapping still parses as JSON (`python3 -c 'json.load(…)'`),
  and only its description strings changed.
- **`cargo doc` with `-D warnings`** fails on `vpay-config` identically on
  `master`: four private-item links, in `cli.rs` and at `config.rs:652`. None
  is in a line this change touches. CI does not run `cargo doc`.
- **`helm-check`'s recipe body** (Helm v3.16.2, kubeconform v0.6.7): `ok`.
  That covers lint, 4 renders, 24 guards, both rate-limit paths, both
  rail-callback paths, management coherence and the wrapper chart, with
  kubeconform reporting 68 of 68 valid.
- **The two edited template messages, rendered directly.** No CI fixture sets
  `ingress.provider.sourceRange`, so:
  - `values-full.yaml` with `--set ingress.provider.sourceRange=203.0.113.0/24`
    renders the new comment above
    `whitelist-source-range: "203.0.113.0/24"`, and kubeconform passes that
    output (24 valid, 4 skipped CRDs).
  - The `provider-callback-routable` guard fixture still fails by its own
    guard's name, with the new wording.

`cargo xtask`, with this page staged, exit 0 for each of the thirteen:

```text
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 2 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-errors: ok — 20 error type(s), all classified; …
verify-sdk-parity: ok — 790 proving test(s) named in docs/sdks/parity.md all exist, 44 dated gap(s), …
verify-links: ok — 2569 repository link(s) in 479 tracked markdown file(s) resolve to a tracked path …
verify-npm-scope, verify-serde, verify-repositories, verify-toolchain, verify-migrations: ok
verify-versions: ok — 26 version references all say 0.10.2
verify-privacy-inventory: ok — 314 database column(s) classified …
verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s) agree with what this tree measures
```

## Not done

- **Whether MTN's `requires_ip_allowlist: true` is right.** It needs MTN to
  say whether Disbursements whitelisting applies, and to whose IPs. It is
  recorded on the flag, not decided.
- **Whether MTN's callbacks arrive from published ranges.** None were found.
  `sourceRange` stays empty by default.
- **No MTN or Orange callback has reached vpay.** Nothing here changes that.
