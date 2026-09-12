"use client";

import { Refine } from "@refinedev/core";
import routerProvider from "@refinedev/nextjs-router";
import { Suspense } from "react";

import { dashAuthProvider } from "../../src/dash/auth-provider";
import { dashDataProvider } from "../../src/dash/data-provider";
import { REFINE_RESOURCES } from "../../src/dash/resources";
import { checkSession, signOut } from "../../src/server/actions";

/**
 * The signed-in shell — Lane 3 of
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
 * # This is a client component and its children are still server components
 *
 * Next passes `children` as a slot, so every page inside this group keeps
 * running `requireStaff()` on the server before anything renders. That
 * matters: the 80 %-of-TTL re-mint and the `401`-vs-outage distinction stay
 * on the server, where the token is, and this file has never seen one.
 */
export default function DashLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <Suspense fallback={null}>
      <Refine
        routerProvider={routerProvider}
        dataProvider={dashDataProvider("/api/dash")}
        authProvider={dashAuthProvider({ signOut, check: checkSession })}
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
