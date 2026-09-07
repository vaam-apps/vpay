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
the label/error association, `Input` for the control, `Button` for submit.
Deliberately agnostic about the second factor (`docs/flows/dashboard.md`
records that choice as not yet taken): swap the `code` field's `type`/
`autoComplete` for whatever exp24 lands and the composition is unchanged.

```tsx
import { Alert, Button, Field, FieldError, FieldLabel, Input } from '@vpay/ui';

export function SignInForm({ error = null, onSubmit }: SignInFormProps) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
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
        <Input id="dashboard-signin-email" name="email" type="email" required autoComplete="username" />
      </Field>
      <Field invalid={error !== null}>
        <FieldLabel htmlFor="dashboard-signin-code">One-time code</FieldLabel>
        <Input
          id="dashboard-signin-code"
          name="code"
          type="text"
          inputMode="numeric"
          autoComplete="one-time-code"
          required
        />
        <FieldError match={error !== null}>{error}</FieldError>
      </Field>
      {error !== null ? <Alert tone="error">{error}</Alert> : null}
      <Button type="submit" block>
        Sign in
      </Button>
    </form>
  );
}
```

Proven by [`sign-in-form.test.tsx`](src/recipes/sign-in-form.test.tsx): a
submit reads the two fields out of a plain `FormData` (the same pattern
`frontends/apps/checkout`'s `MsisdnForm` uses); an `error` prop renders as an
`alert` and marks both fields `aria-invalid` through `Field`'s own validation
state, not a hand-rolled `aria-*` prop.

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
had zero test files and no jsdom environment configured. Both mirror
`frontends/packages/ui`'s own setup exactly (same jsdom polyfills for the
Base UI pointer-capture APIs jsdom doesn't implement), because the recipes
render the same Base UI primitives (`Input`, `Button`, `Field`) that
package's component tests do, and hit the same gap.

```bash
pnpm --filter @vpay/dashboard test        # 4 files, 6 tests
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
