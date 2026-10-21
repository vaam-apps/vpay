"use client";

import { Refine } from "@refinedev/core";
import routerProvider from "@refinedev/nextjs-router";
import { Suspense } from "react";

import { dashAuthProvider } from "./auth-provider";
import { dashDataProvider } from "./data-provider";
import { REFINE_RESOURCES } from "./resources";
import { signOut } from "../server/actions";

/**
 * The signed-in data layer — Lane 3 of
 * `docs/plans/exp55-refine-seam-bff-notes/refine-plan.md`.
 *
 * # The `<Suspense>` is load-bearing, and not where you would guess
 *
 * `<Refine>` **itself** calls `useSearchParams()`, through its internal
 * `Telemetry` component — not only the data hooks do. Without a boundary
 * here, `next build` fails at prerender with
 * *"useSearchParams() should be wrapped in a suspense boundary"* for every
 * route in this group.
 *
 * Wrapping the individual screens is **not** enough: measured, the trace
 * simply moves from `useList` to `Telemetry`. The boundary belongs around
 * `<Refine>`, which is why it is in the layout and not in each page.
 *
 * # Moved out of `app/(dash)/layout.tsx` on 2026-10-21
 *
 * The layout became a Server Component so it can gate once and render the
 * shell (`AppShell`) that Next then keeps mounted across navigations; this
 * client half is what it wraps the shell in. `children` is still a slot, so
 * every page inside the group keeps running `requireStaff()` on the server
 * before anything renders. That
 * matters: the 80 %-of-TTL re-mint and the `401`-vs-outage distinction stay
 * on the server, where the token is, and this file has never seen one.
 */
export function DashProviders({ children }: { children: React.ReactNode }) {
  return (
    <Suspense fallback={null}>
      <Refine
        routerProvider={routerProvider}
        dataProvider={dashDataProvider("/api/dash")}
        authProvider={dashAuthProvider({ signOut })}
        resources={[...REFINE_RESOURCES]}
        options={{
          // The banner is a network call and an opinion about this product
          // that this product did not ask for.
          disableTelemetry: true,
          syncWithLocation: true,
          warnWhenUnsavedChanges: false,
          reactQuery: {
            clientConfig: {
              defaultOptions: {
                queries: {
                  /**
                   * **One attempt, and then say so.**
                   *
                   * TanStack Query retries a failed read three times with
                   * backoff by default, and Refine does not change that. On
                   * this surface that is the wrong trade twice over: a
                   * refusal from `/dash/v1` is a decision, not a blip — a
                   * `401` is the session being over and a `403` is a
                   * deployment problem, and retrying either just delays the
                   * answer — while an outage leaves an operator watching a
                   * skeleton for several seconds before `ReadFailure` ever
                   * appears.
                   *
                   * Measured while writing `screens.test.tsx`: with the
                   * default, a rejected read still rendered the loading
                   * skeleton a second later and the error branch was
                   * unreachable in a test. That is what an operator would
                   * have seen too.
                   */
                  retry: false,
                  /**
                   * **The server's read is the answer, and the client keeps
                   * no copy of it beyond the screen.** Every list and detail
                   * here is read by its Server Component and handed in as
                   * `initialData` (`src/dash/initial.ts`). With TanStack's
                   * default `staleTime: 0`, moving to the next page changed
                   * the query key on a mounted observer and the browser read
                   * the same page AGAIN through `/api/dash` — measured
                   * 2026-10-21: "next page" cost two upstream list reads
                   * where one was needed. `Infinity` stops that refetch;
                   * `gcTime: 0` drops a key's cache the moment no screen
                   * shows it, so returning to a page shows the server's
                   * fresh read rather than a cached one that would shadow
                   * `initialData`.
                   */
                  staleTime: Number.POSITIVE_INFINITY,
                  gcTime: 0,
                },
              },
            },
          },
        }}
      >
        {children}
      </Refine>
    </Suspense>
  );
}
