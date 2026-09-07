# `@vpay/dashboard`

The staff dashboard. What it is, and what it is not, is
[`docs/flows/dashboard.md`](../../../docs/flows/dashboard.md)'s job — this
file is the frontend's own, narrower one: how a screen in this app is built.

## State, as of this pass (exp26 Lane D, `docs/plans/2026-09-07-ui-revamp.md`)

Still the scaffold `docs/flows/dashboard.md` describes: one route (`/`), no
data source, no session. What changed here is the styling substrate, not the
product surface — `@base-ui/react` 1.8.0 + Tailwind 4 + daisyUI 5 through
`@vpay/ui`, replacing the raw Tailwind/daisyUI-4 classes the scaffold used to
write inline. `app/layout.tsx` and `app/page.tsx` are now pure composition —
zero `className` strings, zero raw utility classes — because every visual
decision they used to make (`min-h-screen bg-base-200`, `mx-auto max-w-3xl
p-8`, the `alert alert-warning` markup, …) now lives behind `PageShell`,
`Heading`, `Alert`, `Stack`, `Text` and `StatusBadge` from `@vpay/ui`.

`data-theme` moved from `corporate` to `bumblebee` (the maintainer's
2026-09-05 preference, applied everywhere by the revamp).

## The nav rule

> "The navigation is only ever allowed to link to slices that exist. A menu
> entry for a page nobody wrote is the same lie as an empty table."
> — `docs/flows/dashboard.md`

`app/layout.tsx` renders the brand (`<h1>vpay dashboard</h1>`) as the app's
persistent chrome and, today, **no other links** — there is exactly one page.
The day a second page lands (sign-in, the payments list), add its link to
that `<nav>` in the same commit that adds the page, never before.

That is a **gate**, not an honour system:
[`src/layout.test.tsx`](src/layout.test.tsx) resolves every internal `href`
the layout renders against `app/**/page.tsx` on disk and fails on a link to a
page nobody wrote. It was added by the review, which measured that the rule
had been stated in three places and enforced in none — a dangling
`<a href="/payments">` left the whole suite, `lint` and `dashboard.cy.ts`
green.

## Recipes for the next pages

Four patterns the plan (§3, "the component set") and `docs/flows/dashboard.md`
(slice 1: sign-in, a payments list, a payment detail) both point at, built
here so whoever lands those pages composes `@vpay/ui` rather than reaching
for raw Tailwind. Each one lives in
[`src/recipes/`](src/recipes/) as a real, exported, type-checked component —
not a snippet transcribed into this file by hand — and each has a test in the
same directory that renders it and asserts on what a user or Cypress would
actually see. `pnpm --filter @vpay/dashboard test` runs all of them; none is
wired into `app/page.tsx`, because none has a data source yet and a dashboard
screen showing invented rows is the failure mode `AGENTS.md` names first.

None of the four needed a component `@vpay/ui` doesn't already export
(`Tabs`, `Skeleton` and `Toast` stay unbuilt — plan §3 makes them conditional
on this lane actually needing them, and it doesn't).

### 1. Sign-in form

[`src/recipes/sign-in-form.tsx`](src/recipes/sign-in-form.tsx) — `Field` for
the label/error association, `Input` for the control, `Button` for submit,
`Alert` for the one place an error is shown.

**It is one leg, and that is the first thing to know about it.** The staff
login `claude/exp24-staff-auth` is building (its ADR-0017) is two:
`POST /dash/v1/staff/login` with an argon2id password, then
`POST /dash/v1/staff/totp` with the code. This recipe has **no password
control**, because adding one would import a decision this branch's own
[`docs/flows/dashboard.md`](../../../docs/flows/dashboard.md) still records as
open ("how does a human staff member prove who they are?"). Building the
second leg is another `Field` + `FieldLabel` + `Input type="password"` in
exactly this shape — see
[`lane-d-review.md`](../../../docs/plans/exp26-notes/lane-d-review.md)
finding 4, which measured what the recipe could and could not express.

What the review **did** add are the three controls that leg needs whichever
factor wins, each of them missing from the first draft and each now proven by
a test:

| prop | why it exists |
|---|---|
| `pending` | Disables every control and makes the submit handler a no-op. The one-time code is verified behind a compare-and-swap replay guard, so the *second* submission of one code is refused **on purpose** — a double-click would show "invalid code" to someone who typed a good one. Without this prop a double-click called `onSubmit` twice (measured). |
| `requestId` | vpay emits `request-id`/`x-request-id` with one value on every response, and `vpay-api`'s error envelope deliberately carries no `request_id` field because the header already does. A staff member who cannot sign in has nothing else to quote to an operator. |
| `codeLength` | `maxLength` + `pattern`, six by default (RFC 6238). The first draft constrained the field to nothing at all. |

```tsx
import { Alert, Button, Field, FieldLabel, Input, Text } from '@vpay/ui';

export function SignInForm({
  error = null,
  requestId = null,
  pending = false,
  codeLength = DEFAULT_CODE_LENGTH,
  onSubmit,
}: SignInFormProps) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (pending) {
          return;
        }
        const data = new FormData(event.currentTarget);
        const email = data.get('email');
        const code = data.get('code');
        onSubmit({
          email: typeof email === 'string' ? email : '',
          code: typeof code === 'string' ? code : '',
        });
      }}
    >
      <Field invalid={error !== null}>
        <FieldLabel htmlFor="dashboard-signin-email">Work email</FieldLabel>
        <Input
          id="dashboard-signin-email"
          name="email"
          type="email"
          required
          disabled={pending}
          autoComplete="username"
        />
      </Field>
      <Field invalid={error !== null}>
        <FieldLabel htmlFor="dashboard-signin-code">One-time code</FieldLabel>
        <Input
          id="dashboard-signin-code"
          name="code"
          type="text"
          inputMode="numeric"
          autoComplete="one-time-code"
          maxLength={codeLength}
          pattern={`[0-9]{${codeLength}}`}
          required
          disabled={pending}
        />
      </Field>
      {error === null ? null : (
        <Alert tone="error">
          {error}
          {requestId === null ? null : (
            <Text as="span" size="xs" tone="muted">
              Request <code>{requestId}</code>
            </Text>
          )}
        </Alert>
      )}
      <Button type="submit" block disabled={pending}>
        {pending ? 'Signing in…' : 'Sign in'}
      </Button>
    </form>
  );
}
```

The error is rendered in **one** place. The first draft put it through
`FieldError` *and* through `Alert`, so every failed sign-in printed the same
sentence twice; `sign-in-form.test.tsx` now counts the occurrences rather
than trusting the reading. A genuinely *field-level* error (this one is
form-level — the server rejected the pair) is what `FieldError match={…}` is
for, and `@vpay/ui` still exports it.

Proven by [`sign-in-form.test.tsx`](src/recipes/sign-in-form.test.tsx), 8
cases: a submit reads the two fields out of a plain `FormData` (the same
pattern `frontends/apps/checkout`'s `MsisdnForm` uses); an `error` prop
renders as an `alert` and marks both fields `aria-invalid` through `Field`'s
own validation state, not a hand-rolled `aria-*` prop; the error appears
exactly once; the request id reaches the alert; the code field carries
`maxlength`/`pattern`/`inputmode`; and `pending` both disables every control
and refuses a `submit` raised any other way.

### 2. Data table with status pills

[`src/recipes/payments-table.tsx`](src/recipes/payments-table.tsx) — `Table`
for structure, `StatusBadge` in the status column. `StatusBadge` is the same
component `/`'s reference legend renders, so a status can never read a
different colour in a list than it does anywhere else — both take tone from
`@vpay/tokens`, never from a local map.

```tsx
import type { PaymentStatus } from '@vpay/tokens';
import { StatusBadge, Table } from '@vpay/ui';

export function PaymentsTable({ rows }: PaymentsTableProps) {
  return (
    <Table zebra>
      <thead>
        <tr>
          <th>Payment</th>
          <th>Amount</th>
          <th>Status</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr key={row.id}>
            <td>{row.id}</td>
            <td>{row.amount}</td>
            <td>
              <StatusBadge status={row.status} />
            </td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
```

`rows` is a prop, not a fetch and not a fixture rendered on any real page —
this file takes no position on `/dash/v1`. Proven by
[`payments-table.test.tsx`](src/recipes/payments-table.test.tsx) with two
deliberately obvious placeholder ids (`pi_example_1`, `pi_example_2`),
asserting the rendered pill carries both the right tone class
(`badge-success`, `badge-info`) and the right copy (`Succeeded`) for its
status.

### 3. Detail timeline

[`src/recipes/detail-timeline.tsx`](src/recipes/detail-timeline.tsx) — the
event history `GET /dash/v1/payment_intents/{id}` will return
(`docs/flows/dashboard.md`), one line per event. `List` for vertical rhythm,
`Stack` to line up the label against the timestamp — no raw `flex` written
in the app.

```tsx
import { List, Stack, Text } from '@vpay/ui';

export function DetailTimeline({ events }: DetailTimelineProps) {
  if (events.length === 0) {
    return <Text tone="muted">No events yet.</Text>;
  }
  return (
    <List>
      {events.map((event) => (
        <li key={event.id}>
          <Stack justify="between" gap="md">
            <Text as="span" weight="medium">
              {event.label}
            </Text>
            <Text as="span" tone="muted" size="xs">
              {event.at}
            </Text>
          </Stack>
        </li>
      ))}
    </List>
  );
}
```

No icon set or connector graphic — none exists in `@vpay/ui` and this recipe
has no consumer that has asked for one. An empty list states plainly that
there are no events rather than rendering a blank section, the same rule the
scaffold notice on `/` follows. Proven by
[`detail-timeline.test.tsx`](src/recipes/detail-timeline.test.tsx): both the
populated and the empty case.

### 4. Empty state

[`src/recipes/empty-state.tsx`](src/recipes/empty-state.tsx) — the "nothing
matched" pattern for a filtered list (as opposed to `/`'s "no data source at
all" notice, which stays the `Alert` it already is). Centred `Stack`
composition only.

```tsx
import { Heading, Stack, Text } from '@vpay/ui';

export function EmptyState({ title, description }: EmptyStateProps) {
  return (
    <Stack direction="column" align="center" justify="center" gap="sm">
      <Heading level={3}>{title}</Heading>
      <Text tone="muted" size="sm">
        {description}
      </Text>
    </Stack>
  );
}
```

Proven by [`empty-state.test.tsx`](src/recipes/empty-state.test.tsx).

## Testing this app

`vitest.config.ts` + `vitest.setup.ts` are new in this pass — the scaffold
had zero test files and no jsdom environment configured.
[`src/a11y.test.tsx`](src/a11y.test.tsx) runs `axe-core@4.13.0`'s structural
rules (plan §7 row 5's exact list) over the **real rendered `<body>`** of the
layout wrapped around the page, and over every recipe including its error and
pending states. It exists because the first draft of this pass dropped the
`<main>` landmark the scaffold had and every other gate stayed green:
`region` went 0 → 1 violation and nothing said so. Contrast is **not**
checked here and cannot be — jsdom computes no paint (plan §7 row 6). Both mirror
`frontends/packages/ui`'s own setup exactly (same jsdom polyfills for the
Base UI pointer-capture APIs jsdom doesn't implement), because the recipes
render the same Base UI primitives (`Input`, `Button`, `Field`) that
package's component tests do, and hit the same gap.

```bash
pnpm --filter @vpay/dashboard test        # 6 files, 22 tests
pnpm --filter @vpay/dashboard typecheck
pnpm --filter @vpay/dashboard lint
pnpm --filter @vpay/dashboard build       # also proves the compiled CSS
                                           # actually carries these classes —
                                           # see plan §6.4
```

## What this does not cover

- **No page uses these recipes yet.** They exist so the next page composes
  `@vpay/ui` instead of raw Tailwind, not because slice 1 landed. See
  `docs/flows/dashboard.md` for what is actually built.
- **No auth mechanism is chosen.** `SignInForm` is a layout, not a decision
  about password vs. TOTP vs. WebAuthn — that decision is recorded as open in
  `docs/flows/dashboard.md`.
- **No `Tabs`, `Skeleton` or `Toast`.** Plan §3 makes them conditional on
  this lane's screens needing them; none of the four recipes did.
