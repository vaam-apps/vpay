/**
 * A vite ALIAS for the bare specifier `next/navigation`, not a copy of it.
 *
 * # The problem this solves
 *
 * `PaymentsFilters` calls `useRouter()` and `AppShell` calls `usePathname()`
 * (see each component's own doc comment). Outside a real Next app router —
 * which nothing under `@storybook/react-vite` provides, unlike
 * `@storybook/nextjs-vite`, which this app does NOT use — both throw
 * `invariant expected app router to be mounted` the instant the component
 * renders. `src/a11y.test.tsx` and every `*.test.tsx` beside these
 * components handle it with `vi.mock("next/navigation", …)`, which is a
 * vitest-runner feature with no Storybook counterpart: `build-storybook`
 * runs no vitest at all, and even inside `vitest.storybook.config.ts` the
 * module a *story* resolves is decided by `@storybook/addon-vitest`'s own
 * browser-mode module graph, not by this suite's `vi.mock` registry.
 *
 * # Why an alias rather than an `AppRouterContext.Provider` decorator
 *
 * The context that satisfies these hooks
 * (`next/dist/shared/lib/app-router-context.shared-runtime`) is an
 * unexported implementation detail of Next's own package layout — reaching
 * it means importing a `next/dist/...` path with no public contract to hold
 * it stable across a Next upgrade, for a return of exactly two hooks this
 * app calls. Replacing the whole module at the resolver is simpler and is
 * the same mechanism `@vaam-apps/ui/styles/theme.css` already gets aliased
 * by in this same `main.ts`.
 *
 * # What is deliberately fixed rather than reactive
 *
 * `usePathname` always answers `/payments`, the one route
 * `NAV_ENTRIES[0]` names (`src/dash/resources.ts` — there is exactly one
 * resource; see that file's own doc for why). `useRouter().push` is a
 * no-op. No story here exercises "click Apply and see the URL change" or
 * "the rail highlights a different route" — those are `payments-filters.test.tsx`
 * and `app-shell.test.tsx`'s job, in jsdom, where `vi.mock` supplies a real
 * spy. This stub exists only to let `AppShell` and `PaymentsFilters` render
 * far enough for axe to see their markup; it is not a routing simulator, and
 * pretending otherwise would be the "mock adapter to make local development
 * easier" failure mode `CLAUDE.md` names second.
 */
export function useRouter() {
  return { push: () => undefined };
}

export function usePathname() {
  return "/payments";
}
