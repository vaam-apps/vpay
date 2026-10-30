# `@vpay/dashboard`

The staff dashboard. What it is meant to become is
[`docs/flows/dashboard.md`](../../../docs/flows/dashboard.md)'s job; this file
is the frontend's own, narrower one: what is in this app **today**, and how a
screen in it is built.

## What this app is today: a sign-in and an account page

**It signs a staff member in with a password and an authenticator code, and
lands them on a page that says who they are.** Nothing in it reads `/dash/v1`
yet: the payments, refunds, deliveries and customers screens are a later
ticket, so the account page links to none of them.

It does this **only where vpay serves `human_login`**
([ADR-0037](../../../docs/adr/0037-authentication-methods-are-deployment-configuration.md)).
`/login` reads `GET /.well-known/vpay-auth` on the server, on every render, and
decides which page it is:

| vpay says                             | `/login` renders                                                     |
| ------------------------------------- | -------------------------------------------------------------------- |
| it could not be asked                 | "Sign-in is not reachable", with the request id. Never the next row. |
| `human_login` is not served           | "This console signs in with a paired key" (VP-6b), and no field      |
| served, the client values are missing | the variable names to set, and no form                               |
| served, configured                    | the first question                                                   |

Since VP-6a ([ADR-0036](../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md))
a deployment that does not serve `human_login` is signed with a paired P-256
key and nothing else; this app cannot do that until VP-6b.

### The pages

| Route             | What it asks or shows                                                                           |
| ----------------- | ----------------------------------------------------------------------------------------------- |
| `/login`          | the work email, then the password                                                               |
| `/login/totp`     | the six-digit code; a first sign-in also shows the QR code (the typed key is in a bottom sheet) |
| `/login/password` | while the one-time password is in force: the current, the new, the new again                    |
| `/`               | the account page: who, which merchant, Sign out                                                 |
| `/signed-out`     | a route handler: forgets a dead session cookie, then `/login`                                   |
| `/healthz`        | "Next is answering"; probes nothing                                                             |

**One screen collects one piece of information.** The email and the password are
two screens (`src/components/one-question-steps.tsx`), the password change is
three. Continue is disabled until the answer is valid, Back keeps what was
typed, and nothing is posted until the last answer. It needs JavaScript, and
that is the cost of the rule.

**Every choice is a bottom sheet** (`@vaam-apps/ui`'s `MoreDetailDrawer`), and
there is no dropdown anywhere: `src/no-dropdowns.test.ts` fails on a `<select>`,
`aria-haspopup`, a menu/listbox/combobox role or a popup component. **This app
asks for no shadow** (`src/no-shadows.test.ts`): never `variant="elevated"`,
never `shadow-*`.

### How it signs in

The app is the OAuth client and runs the code leg in its own process
(ADR-0017 decision 4). The staff session token lives in an httpOnly, Secure,
`SameSite=Lax` cookie on this app's origin (`__Host-vpay_dash_session`, no
`Domain`, 12 hours; see [the cookie needs a secure
origin](../../../docs/flows/dashboard-auth/human-login.md#the-cookie-needs-a-secure-origin)). The `/dash/v1` access token is minted by running
`GET /oauth/authorize` and `POST /oauth/token` from the session, with PKCE
S256, an exact `redirect_uri` and a checked `state`, and is read back off the
session row on every render. **There is no refresh grant** (the grant serves
`authorization_code` alone): a token at 80% of its life is replaced by running
the request again. `dashboard_client` is a public client; no secret exists.

`src/server/dash-fetch.ts` is the one function that builds a request to vpay.
It attaches **one** credential (a session header, a bearer, and, from VP-6b, a
signed console key, which cannot be constructed today) and copies nothing from
the browser's request. A request that carries both a signature and an
`Authorization` header is `400` at vpay.

Which credential wins when a browser holds both a session and a key is VP-6b's
to decide, not this app's.

### Configuration

Read once at start, through bracket notation (`src/config/runtime.ts`).

| Variable                       | Needed for                                                                        |
| ------------------------------ | --------------------------------------------------------------------------------- |
| `VPAY_DASH_API`                | everything: vpay's base URL as this server reaches it                             |
| `VPAY_DASHBOARD_CLIENT_ID`     | signing in: `dashboard_client.client_id`                                          |
| `VPAY_DASHBOARD_REDIRECT_URI`  | signing in: one of `dashboard_client.redirect_uris`, byte for byte, never visited |
| `VPAY_DASHBOARD_SCOPE`         | signing in: `dashboard_client.scope`                                              |
| `VPAY_DASHBOARD_PUBLIC_ORIGIN` | optional: the origin a browser reaches this app on (the origin check)             |

The three `VPAY_DASHBOARD_` client values matter only where vpay serves
`human_login`. When it does and one is missing, `/login` names them and renders
no form; when it does not, `/login` says "paired key" whatever they are.

### Not here, on purpose

The BFF (`app/api/dash`), `middleware.ts`, `src/dash/*`'s providers and Refine
auth are deleted and stay so. `@vpay/dash-client` may be imported for its types
only (`eslint.config.js`). Single sign-on is VPA-5b, the end-to-end cases are
VPA-5c, and the pages that read `/dash/v1` come with VP-6b.

## What is kept, and why

The presentational components and the pure logic under them survive, with
their tests, because they depend on nothing that was deleted and VP-6b will
render them again:

| Area                  | Where                                                                                                                                                |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Payments table/detail | `src/components/payments-table.tsx`, `payment-detail.tsx`, `payments-filters.tsx`, `payments-pager.tsx`                                              |
| Procedure lists       | `src/components/procedure-list-view.tsx`, `procedure-table.tsx`, `procedure-pager.tsx`, over `src/dash/procedure-resources.ts`                       |
| Query and paging      | `src/payments-query.ts` (cursors, filters, the URLs), `src/dash/procedure-paging.ts`                                                                 |
| Words and colour      | `src/format.ts` (the masked payer is a dash, never derived), `src/payment-status.ts`, `src/status-systems.ts`, from `@vpay/tokens`                   |
| Failure shape         | `src/server/api.ts` (`ApiFailure`, the request id, the object types and the sign-in calls), `src/components/read-failure.tsx`, `src/dash/failure.ts` |
| Storybook             | `src/components/dashboard-screens.stories.tsx`, run through axe in a real Chromium by `pnpm --filter @vpay/dashboard test-storybook`                 |

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
- **One credential per request, forwarded from nowhere**
  (`src/server/dash-fetch.ts`).
- **No dropdown and no shadow** (`src/no-dropdowns.test.ts`,
  `src/no-shadows.test.ts`).

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
over the rendered `<body>` of the root layout around a page, over every screen
of the sign-in (the key sheet open included) and over each component that
survives, including its error states. Contrast is not checked
there and cannot be, because jsdom computes no paint; the Storybook run does.
