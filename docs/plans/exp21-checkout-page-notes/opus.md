# exp21 — the checkout page restyled and made runtime-configurable

Working notes for the change described in
[`../../flows/hosted-checkout.md`](../../flows/hosted-checkout.md)'s four new
sections. Branch `claude/exp21-checkout-page`, base `06e27f9`.

The maintainer's four requirements (2026-09-05) and what happened to each:

| | Asked for | Delivered |
|---|---|---|
| 1 | daisyUI **bumblebee** + Base UI defaults, minimal custom CSS, accessible, same screens in both modes | done |
| 2 | Replace the auto-forward countdown with a **"Back to {merchant}"** button; failure shows the rail's reason | done |
| 3 | Runtime `branding.yaml` + `config.yaml`, missing file → defaults + a loud log | done |
| 4 | Page memory: prefill number and method; **PIN vault** in IndexedDB | number and method done; **the PIN vault was refused** — see below |

A fifth item arrived mid-task from the `examples/shop` track: **the popup
peer** — a popup is not a frame, so `window.parent === window` inside one and
vpay said nothing to a merchant who opened the hosted page with
`window.open`. Done, with three deliberate departures, all recorded in the
flow doc's "The popup, and why it is a third peer".

A sixth followed it: the maintainer took the return-trip decision that item 4
of §2 had surfaced, and it is built — see that entry.

---

## 1. What was refused, and why

**There is no PIN vault, and no PIN field.** The requirement asked the page to
remember a PIN "purely for recall into the form; it NEVER authorises anything
and is never sent anywhere except into the same form field the payer would
type into". *There is no such field, anywhere in this system.*

- `frontends/apps/checkout` collects one thing on the MTN path: a Cameroon
  MSISDN. Orange collects nothing here at all.
- `POST /v1/browser/payment_intents/{id}/confirm` accepts
  `payment_method_data[type]` and `payment_method_data[mtn_momo][msisdn]`.
  That is the whole payload.
- `grep -rni '\bpin\b' backends/ --include='*.rs'` returns `Box::pin`, "pin
  this", and nothing else. No `ProviderAdapter` takes a PIN.

Building the vault would have meant **adding a PIN input to vpay's checkout
page that goes nowhere**: a control that looks like it does something and does
not. That is the first item in this repository's own list of ways to damage
it, and on a payment page a field asking for a mobile-money PIN that nothing
consumes is indistinguishable from phishing.

So the store, the opt-in, the ninety-day horizon and the clear control are all
built and tested — for the number and the method, which are real fields — and
a `pin` member is the one change if a rail is ever integrated that takes one
through the API. **This is a maintainer decision and is surfaced rather than
taken.**

## 2. Maintainer decisions surfaced, not taken

1. **`@base-ui-components/react` is `1.0.0-rc.0`.** That package has no stable
   release; `latest` is the release candidate. It is pinned exactly. A
   pre-1.0 dependency on a payment page is a call for the maintainer.
2. **Base UI's checkbox is not a native control.** It renders
   `<span role="checkbox" tabindex="0">` with a visually hidden input beside
   it. The page's stated a11y invariant used to be "every control is a native
   `button`, `input` or `input[type=radio]`"; that sentence is now false and
   has been corrected in `screens.tsx` rather than left standing. The
   *property* it defended is still held and is now tested explicitly (role,
   tab index, accessible name, `aria-checked`, label click, space key — the
   last two measured, not assumed).
3. **The failed outcome renders a NEUTRAL alert and the canceled one renders
   RED.** `statusTone['requires_payment_method']` is neutral and
   `statusTone['canceled']` is error, in `@vpay/tokens`. That is **unchanged
   by this branch** and visible in `outcomes-hosted.png`; AGENTS.md forbids
   inlining a status colour in a component, so fixing it means changing the
   tokens package, which is a different change with a different blast radius.
4. ~~**The popup's return trip is not wired.**~~ **Taken by the maintainer,
   same day, and built**: the return page pins its opener with `soleOrigin`
   — the merchant's single registered `checkout_origins` entry where there is
   exactly one, and no channel with none or with several. The rationale
   recorded with the decision: with one registered origin the `postMessage`
   target *is* the merchant's own origin and is the only party the message
   could ever have been for, so the worst case is a message delivered to its
   intended reader. `soleOrigin` counts after normalising, so one malformed
   registered origin is none rather than one to pin to. Six unit cases in
   `entry.test.ts`, three in `origins.test.ts`, five in `return.test.ts`.
5. **The chart templates no ConfigMap for either YAML file**, so a Kubernetes
   deployment has no supported way to supply them. `compose.demo.yml` mounts
   them; the chart was left alone because `just helm-check` needs the network
   and is not part of `just ci`, so a template added here would be one nothing
   in this pass could gate.

## 3. What was NOT done

- **No Cypress run.** `pnpm exec cypress install` needs the CDN and the
  binary was not fetched in this environment. The two specs are **unchanged**
  and every selector they use was preserved deliberately — `[data-screen]`,
  `[data-outcome] button`, `button[data-rail=…]`, `#vpay-msisdn`,
  `button[type="submit"]`, `button.btn-primary` — with a vitest case
  (`leaves the MSISDN form's only submit button the submit button`) pinning
  the one the memory controls could have broken. **Nothing here proves those
  specs still pass.** Two of their comments still describe "the five-second
  countdown"; they were left untouched because editing a spec nothing ran
  would be worse than a stale comment.
- **No container was run with the mounted files, and no pod ever.** The
  filesystem layer is tested against real temporary files
  (`src/config/runtime.test.ts`, including a `chmod 000` unreadable mount);
  `compose.demo.yml` mounts the examples; nothing started that stack.
- **No real browser touched IndexedDB.** The adapter is driven by a
  hand-written `IDBFactory` stub (`src/testing/idb-stub.ts`) that models the
  object graph the adapter walks and nothing else — no transaction lifetime,
  no quota, no version above 1.
- **The Rust API was not touched.** It did not need to be: the session read
  has carried `merchant: { name }` since Step 9 lane 1b, which is what the
  "Back to {merchant}" button uses.
- **The dashboard was not touched.**

## 4. The screenshots

Rendered from the **shipping components** (`CheckoutView`, via
`renderToStaticMarkup`) with the **shipping stylesheet** (`next build`'s own
`.next/static/css/*.css`), in headless Chrome at 420 px. They are a render of
the design, **not** a container serving a live session — no API, no session,
no rail.

| File | What it shows |
|---|---|
| [`outcomes-hosted.png`](outcomes-hosted.png) | succeeded / failed / canceled, `ui_mode: hosted` |
| [`outcomes-embedded.png`](outcomes-embedded.png) | the same three, `ui_mode: embedded` |
| [`entry-screens.png`](entry-screens.png) | rail selector, MSISDN form, redirect prompt, waiting — in French |
| [`branded.png`](branded.png) | `branding.yaml` applied: operator name, support line, and `primary_color: "#1d4ed8"` |

**The two outcome images are pixel-identical, and that is the point rather
than an oversight**: both modes render the same screens from the same state
machine, which the flow doc has said since Step 9. What differs between them
is behaviour, not pixels — where the button goes, and whether a
`vpay:complete` is posted — and that lives in `controller.test.ts`, not in a
screenshot.

Looking at them caught one real defect: the test-mode banner had been changed
from a paragraph to a daisyUI `badge`, and `.badge` is `whitespace-nowrap` by
design, so *"Test mode — no money moves on this deployment."* spilled straight
out of the card. Reverted to a paragraph, with the reason written next to it.
`branded.png` is the evidence the colour conversion works end to end: the
button is `#1d4ed8` in OKLCh with a **white** foreground, derived by daisyUI's
own rule rather than configured.

Regenerating them (the harness is deliberately not checked in — it is a
one-off, and a test that writes PNGs on every `just test-web` would not be):

```bash
cd frontends/apps/checkout
NEXT_PUBLIC_VPAY_API_URL=http://localhost:8080 pnpm run build   # for .next/static/css
# render CheckoutView at each state with renderToStaticMarkup, inlining that
# stylesheet, into <out>/*.html  (see git history of this note for the exact
# 90-line vitest file used), then:
python3 -m http.server 8791 --bind 127.0.0.1 --directory <out> &
for f in outcomes-hosted outcomes-embedded entry-screens branded; do
  google-chrome-stable --headless=new --disable-gpu --hide-scrollbars \
    --window-size=420,2200 --screenshot="docs/plans/exp21-checkout-page-notes/$f.png" \
    "http://127.0.0.1:8791/$f.html"
done
```

## 5. Decisive mutations

Each was applied to the shipping source, the whole suite was run, and the
source was restored. A mutation that did not fail anything is a test that was
not testing.

| Mutation | Result |
|---|---|
| `middleware.ts`: `contentSecurityPolicy(embedded ? origins : [])` → `(origins)` | **2 failed** — the hosted page became framable by every registered origin |
| `rails.ts`: drop the `allowed_methods` narrowing | **4 failed** |
| `memory.ts`: `isStoredMsisdn` → `value.length > 0` | **4 failed** |
| `memory.ts`: drop the ninety-day horizon | **3 failed** |
| `memory.ts`: `pageMemoryFor` ignores the `page_memory` flag | **1 failed** |
| `theme.ts`: `themeStyleSheet` emits an unvalidated colour | **1 failed** |
| `controller.ts`: a popup asks its opener to navigate | **1 failed** |
| `controller.ts`: `vpay:complete` is not deduped | **1 failed** |
| `entry.ts`: never resolve an opener | **1 failed** |
| `screens.tsx`: outcome button loses the merchant's name | **17 failed** |
| `settings.ts`: swallow a wrong-typed key instead of reporting it | **1 failed** |
| `origins.ts`: `soleOrigin` pins the first of several instead of refusing | **1 failed** |
| `entry.ts`: the return page never pins an opener | **1 failed** |
| `middleware.ts`: the return page's CSP takes the resolved list | **3 failed** |

**One mutation survived on the first pass and produced a code change**:
`checkout-client.tsx` with `offered: true` hard-coded passed all 426 tests.
The view's own cases covered `offered: false` and nothing covered how that
value was *arrived at*. The policy moved out of the component into
`pageMemoryFor` in `memory.ts`, where it is three cases; the mutation now
fails.

Two more findings came out of the tests themselves rather than from a
mutation:

- `memory.ts` briefly carried its own `/^237[0-9]{9}$/`, which accepts
  `237571234567` — a number with no mobile prefix that the confirm would have
  refused and this page would have prefilled. Replaced with
  `normalizeCameroonMsisdn(value) === value`, so the rule lives in one file.
- `theme.ts`'s achromatic guard was `chroma === 0`, and `#ffffff` comes out of
  the OKLab matrices with a chroma around 1e-8 — so white got a hue of
  89.87° where daisyUI emits `0`. The guard is on the *rounded* chroma now.

## 6. The gate

`just ci` on the final tree, **exit 0**, read from a file rather than from a
banner. Recipe by recipe:

| Recipe | Result |
|---|---|
| `fmt-check` | ok (Rust only — `just fmt` also runs prettier over ~222 unrelated files and was deliberately NOT run) |
| `clippy` | ok, `-D warnings` |
| `verify` (ten gates) | all ok — `verify-links` over **806 links in 146 tracked files**, `verify-status` 1 declared unimplemented item, `verify-toolchain` 1.98.0 |
| `test-rust` | **1382 run, 1382 passed, 0 skipped**, 1084 s, 43 binaries, real Postgres + real WireMock rails |
| `test-doc` | **91 passed, 1 ignored** |
| `verify-ignored` | **0 ignored (expected 0), 43 binaries (expected 43), 1382 total (floor 1080)** |
| `lint-web` | ok — `pnpm -r typecheck` and `pnpm -r lint --max-warnings 0` over 15 packages |
| `test-web` | ok — `@vpay/checkout` **442 cases in 22 files, 0 skipped** (was 302 in 17) |
| `deny` | advisories ok, bans ok, licenses ok, sources ok |

**Nothing under `backends/` was touched**, so every Rust number is `master`'s.

One honest caveat about *what* was gated: the run above was started on the
tree at `19274df` and finished on a tree that differed by four
comment-and-prose edits made while it compiled — three source comments
corrected (one of which had overstated what `secrets.test.ts` covers) and two
documentation paragraphs. All four were re-checked with `tsc --noEmit` and
`eslint --max-warnings 0` before the web steps ran, so `lint-web` and
`test-web` saw the final text. The `Last verified` block added to
`docs/status.md` afterwards is prose only; `verify-status` lexes for
`NotImplemented` tokens and `verify-links` for relative links, and it adds
neither.
