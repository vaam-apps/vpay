# 2026-10-21 — the dashboard on Material 3 Expressive, and navigation without reloads

Branch `feat/dashboard-md3e`, from `master` at `e28292d` (#310). Three
changes, one commit each, in `frontends/apps/dashboard` and, for the first
only, `frontends/apps/checkout`: the `@vaam-apps/ui` 0.5.0 upgrade, the
reload fixes, and the redesign. **No route of vpay, no server code, no
migration and no `NotImplemented` token moved.** No read moved to the
browser and no write was added: every screen still reads `/dash/v1` on the
server, with the token that never leaves it.

## How it was measured, and what that cannot show

There is no vpay running behind these numbers. A Docker demo build failed on
this machine (Alpine's package index behind the egress proxy), so the
dashboard's production build (`next build`, `output: standalone`) was run
against a small fake of the `/dash/v1` contract — the staff session read,
`GET /payment_intents` (cursor paging), `GET /payment_intents/{id}`, and the
five `$procs/search*` procedures with their filters and `totalCount` — kept
outside the repository and serving deterministic synthetic rows. It counts
the requests it answers per kind. **It is not a mock reachable from vpay**
and nothing in this repository imports it; it exists so the app's own
behaviour could be measured with a real browser. Cypress was not run: it
needs the full compose stack. What the fake cannot tell you is whether the
real backend answers the shapes the same way — that is `dashboard.cy.ts`'s
job, updated in this change and not run.

## Reloads: what caused them, and the numbers

Headless Chromium, 1440×900, one fixed click-through (open `/payments`, a
row, back, the rail to each of the five resources, the next page). Before is
`master`; after is this branch.

|                                             | before                   | after             |
| ------------------------------------------- | ------------------------ | ----------------- |
| full document loads                         | **6**                    | **1** (the first) |
| `GET /dash/v1/staff/session` per navigation | 2                        | 1                 |
| reads of the next page of payments          | 2 (server + `/api/dash`) | 1                 |
| rail click to full content                  | 1.0–1.2 s                | 0.37–0.39 s       |

The after run continues through the Overview (twelve procedure calls, one
session read, 0.71 s), an Overview tile to `/payments?status=succeeded`, and
the rail back to `/payments`: **1 document load over all 12 steps, 0 BFF
fetches.** A row link is slower on paper (0.26 → 0.38 s to full content):
the new `loading.tsx` shows the screen's skeleton at once, and React holds a
revealed skeleton for about 300 ms before swapping in content. That is
deliberate — before, a click left the previous screen up, unchanged and
clickable, until the next was read in full — and stated rather than hidden.

The four causes are in the reload commit's message and the dashboard
README's "The shell, the screens, and why nothing reloads".

## The redesign, screen by screen

Flow and layout changes, all deliberate: the home route is `/overview`
(was `/payments`); the rail gained the Overview and sections; the status
filter is chips that are links (was a `<select>` and an Apply button); the
date range applies on the picker's Save; refunds, deliveries and checkouts
gained status/state filters (the procedures already accepted them); refunds
and checkouts link to their payment; the payment detail puts the last error
under the at-a-glance panel and the timeline beside the record; the
signed-out pages are one card with the mark and a theme shortcut. Nothing an
operator could do before is impossible now.

## Gates, on this branch's final head

| command                                                   | exit                                                                                          |
| --------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `pnpm install --frozen-lockfile`                          | 0                                                                                             |
| `just fmt-check-web`                                      | 0                                                                                             |
| `just lint-web` (`pnpm -r typecheck`, `pnpm -r lint`)     | 0                                                                                             |
| `pnpm -r test`                                            | 0 — dashboard **455 passed, 1 skipped** (35 files); checkout 556 passed, 1 skipped; tokens 12 |
| `just build-storybook`                                    | 0                                                                                             |
| test-storybook, checkout / dashboard (axe, real Chromium) | 0 / 0 — 26 / **36** stories                                                                   |
| `just audit-web`                                          | 0                                                                                             |
| `just verify-ui`                                          | 0                                                                                             |

`just verify`: fourteen of its fifteen gates and the `verify-docs` report
exit 0 run one by one (`verify-no-mocks`, `verify-status`, `verify-errors`,
`verify-sdk-parity`, `verify-links`, `verify-npm-scope`, `verify-serde`,
`verify-repositories`, `verify-toolchain`, `verify-ui`, `verify-migrations`,
`verify-versions`, `verify-privacy-inventory`, `verify-doc-counts`).
`check-schema` exits 1 on this machine because the pinned `cratestack-cli`
0.15.0 is not installed; this change touches no schema.

`just test-storybook` was run as its two `pnpm --filter … test-storybook`
lines with the browsers already installed, rather than through the recipe,
whose first step downloads Chromium.

## Guards seen failing

- `src/dash/overview.test.ts`: a refused count read as `0` (`overview.ts`'s
  failure branch replaced with `{ ok: true, value: 0 }`) fails "keeps a
  refused count refused — never a zero".
- `PaymentsLoading`/`PaymentsLoadingPhone` stories: the unselected chip
  placeholder without its transparent border fails both, by 2 px a chip.
- Before being trusted, the new geometry stories caught three real defects
  of the first draft: the date field's skeleton 8 px taller than the field,
  its width computed in the wrong font (`ch` of the sans face), and the date
  value itself truncated in the new self-hosted mono face, which needed
  `25ch` where the system monospace fitted in `23ch`.

## Not done

- Cypress (`dashboard.cy.ts`) was updated for the new landing route and the
  chip filter and **not run**.
- No real vpay was behind any screenshot or measurement (above).
- The checkout app was only upgraded, not redesigned; its brand retarget
  sets `--color-primary`, and a component reading an M3 role directly may
  not follow a merchant's colour — not measured.
- At 375 px the phone toolbar shows Overview, Payments, Refunds and
  Checkouts; Deliveries and Customers are behind its "More".
