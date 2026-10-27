# `@vpay/dashboard`

The staff dashboard. What it is meant to become is
[`docs/flows/dashboard.md`](../../../docs/flows/dashboard.md)'s job; this file
is the frontend's own, narrower one: what is in this app **today**, and how a
screen in it is built.

## What this app is today: a notice

**It renders one page, and that page says the console signs in with a paired
key and that this is not available until VP-6b.** Every path (`/` and the
`not-found` boundary) renders `ConsoleUnavailable`; there is no form, no
field and no link to a page that would fail.

Since VP-6a ([ADR-0036](../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md))
every `/dash/v1` request is a COSE_Sign1 message under a P-256 key a person's
browser generated and vaam paired through `/svc/v1`. This app used to be an
OAuth client with a password, a TOTP code, an SSO button and a session
cookie, and it called routes that no longer exist in vpay
(`/dash/v1/staff/*`, `/oauth/authorize`, `/oauth/token`, `/staff/sso/*`). All
of that is deleted, with the BFF (`app/api/dash`), the method-policy
middleware, the session and CSRF code, the pages that read through them and
the tests that only exercised them. Nothing here calls vpay at all.

`src/layout.test.tsx` pins it: the only `page.*` under `app/` is the
notice's. The first real page VP-6b adds changes that test in the same commit.

### What VP-6b restores

[`docs/status.md`](../../../docs/status.md), "The dashboard application
cannot sign in until VP-6b", lists the sealed sign-in, the signed reader, the
pages and **each deleted end-to-end case by name**.

## What is kept, and why

The presentational components and the pure logic under them survive, with
their tests, because they depend on nothing that was deleted and VP-6b will
render them again:

| Area                  | Where                                                                                                                                |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Payments table/detail | `src/components/payments-table.tsx`, `payment-detail.tsx`, `payments-filters.tsx`, `payments-pager.tsx`                              |
| Procedure lists       | `src/components/procedure-list-view.tsx`, `procedure-table.tsx`, `procedure-pager.tsx`, over `src/dash/procedure-resources.ts`       |
| Query and paging      | `src/payments-query.ts` (cursors, filters, the URLs), `src/dash/procedure-paging.ts`                                                 |
| Words and colour      | `src/format.ts` (the masked payer is a dash, never derived), `src/payment-status.ts`, `src/status-systems.ts`, from `@vpay/tokens`   |
| Failure shape         | `src/server/api.ts` (`ApiFailure`, the request id, the object types), `src/components/read-failure.tsx`, `src/dash/failure.ts`       |
| Storybook             | `src/components/dashboard-screens.stories.tsx`, run through axe in a real Chromium by `pnpm --filter @vpay/dashboard test-storybook` |

## The rules this app is held to

- **No computed `className`, no raw status colour** (`just verify-ui`, checks
  7a-i, 7a-ii and 7a-iii; `docs/status/gates.md` has the derivation): a class
  string is a literal of 60 characters or fewer, and a status is drawn by its
  status system, never by a local map. A status this build cannot name renders
  as text, not as a coloured pill: a green badge on an unfamiliar value is a
  claim.
- **`data-theme` is `dark`**, and `src/layout.test.tsx` pins it:
  `@vaam-apps/ui` registers `dark` (its default) and an opt-in `light`, and a
  `data-theme` that names neither renders the page unstyled with no error.
- **Absence is the honest shape.** The list has no "Rail" column
  (`payment_method_types` are the rails an intent _may_ use, headed
  **Methods**), and the detail page renders the masked payer from
  `charges.payer_ref_masked`, which nothing writes, as a dash. Deriving a mask
  from the payer's phone number to make a row look full is the trade this
  refuses.
- **Only the generated client's types may be imported** (`eslint.config.js`):
  no module runs `@vpay/dash-client` until VP-6b builds the signed reader.

## Testing this app

```bash
pnpm --filter @vpay/dashboard test
pnpm --filter @vpay/dashboard typecheck
pnpm --filter @vpay/dashboard lint
pnpm --filter @vpay/dashboard build
```

`vitest.config.ts` sets `globals: false`, so Testing Library does not register
its own cleanup; `vitest.setup.ts` calls `afterEach(cleanup)` explicitly.
Without it every render in a file accumulates in one `document.body`.

[`src/a11y.test.tsx`](src/a11y.test.tsx) runs `axe-core`'s structural rules
over the rendered `<body>` of the root layout around the notice, and over each
component that survives, including its error states. Contrast is not checked
there and cannot be, because jsdom computes no paint; the Storybook run does.
