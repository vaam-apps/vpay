/**
 * The dashboard's information architecture, as data — Lane 4 of
 * `docs/plans/exp55-refine-seam-bff-notes/refine-plan.md`.
 *
 * **One declaration, not two.** This replaces `src/nav.tsx`'s `NAV_LINKS`,
 * which was a second list that could disagree with what the app actually
 * routes. Refine's `resources` array is the single source: the nav renders
 * from it, Refine routes from it, and `layout.test.tsx`'s gate reads it —
 * so a resource nobody wrote a page for, or a page missing from the nav,
 * fails rather than shipping a dead link.
 *
 * # Reads only, and it says so structurally
 *
 * No entry declares a `create` or `edit` route, and `meta.canDelete` is
 * `false`.
 * `/dash/v1` answers `403` to every non-`GET` at its boundary, and
 * ADR-0008 gates writes behind an `audit_log` that does not exist. With
 * these flags Refine renders no create button and builds no edit route, so
 * the refusal is expressed in the routing rather than discovered at submit
 * time.
 *
 * # There is exactly one resource, and that is the surface's fault
 *
 * `/dash/v1` serves `payment_intents` and nothing else — one list route and
 * one get-by-id. An admin console with fifteen sections is what the backend
 * exposes, not what the framework provides, and adding entries here for
 * routes nobody serves would be a nav full of dead links.
 */
import type { ResourceProps } from "@refinedev/core";
import {
  CreditCard,
  ShoppingCart,
  RotateCcw,
  Users,
  Webhook,
  type LucideIcon,
} from "lucide-react";

import {
  CHECKOUT_SESSIONS,
  CUSTOMERS,
  PAYMENT_INTENTS,
  REFUNDS,
  WEBHOOK_DELIVERIES,
} from "./resource-name";

/** A resource, plus the two things the nav needs that Refine does not model. */
export interface DashResourceEntry {
  readonly resource: ResourceProps;
  readonly label: string;
  readonly icon: LucideIcon;
}

export const DASH_RESOURCES: readonly DashResourceEntry[] = [
  {
    resource: {
      name: PAYMENT_INTENTS,
      list: "/payments",
      show: "/payments/:id",
      // `create` and `edit` are ROUTE PATHS in Refine, so the way to say
      // "there is no such route" is to omit them entirely, not to pass
      // `false`. Absent here means Refine builds no create/edit route and
      // renders no button that would lead to one.
      meta: { label: "Payments", canDelete: false },
    },
    label: "Payments",
    icon: CreditCard,
  },
  // The three Lane D slices. Each is a CrateStack procedure behind
  // `/api/dash/<name>`; `create`/`edit` are omitted for the reason the
  // comment above gives, and `canDelete` stays false — `/dash/v1` answers
  // `403` to every non-`GET` at its boundary and ADR-0008 gates writes
  // behind an `audit_log` that does not exist.
  {
    resource: {
      name: REFUNDS,
      list: "/refunds",
      meta: { label: "Refunds", canDelete: false },
    },
    label: "Refunds",
    icon: RotateCcw,
  },
  {
    resource: {
      name: WEBHOOK_DELIVERIES,
      list: "/deliveries",
      meta: { label: "Deliveries", canDelete: false },
    },
    label: "Deliveries",
    icon: Webhook,
  },
  {
    resource: {
      name: CHECKOUT_SESSIONS,
      list: "/checkouts",
      meta: { label: "Checkouts", canDelete: false },
    },
    label: "Checkouts",
    icon: ShoppingCart,
  },
  {
    resource: {
      name: CUSTOMERS,
      list: "/customers",
      meta: { label: "Customers", canDelete: false },
    },
    label: "Customers",
    icon: Users,
  },
];

/** What `<Refine resources=…>` takes. */
export const REFINE_RESOURCES: readonly ResourceProps[] = DASH_RESOURCES.map(
  (entry) => entry.resource,
);

/**
 * The routes the nav offers, derived from the same array Refine routes on.
 * `layout.test.tsx` asserts every one of these resolves to a page.
 */
export const NAV_ENTRIES: readonly {
  readonly href: string;
  readonly label: string;
  readonly icon: LucideIcon;
}[] = DASH_RESOURCES.filter(
  (entry) => typeof entry.resource.list === "string",
).map((entry) => ({
  href: entry.resource.list as string,
  label: entry.label,
  icon: entry.icon,
}));

/**
 * Is `href` the destination the reader is currently inside?
 *
 * **One rule, in one place, because there are two renderers.** `SideNav`
 * draws the rail and `MoreMenu` draws the drawer's copy of the same tree,
 * and each has to decide `aria-current` for the same entry at the same
 * moment. Two expressions is two answers: `more-menu.tsx` shipped with
 * `entry.href === currentPath` and the rail disagreed with it on every
 * detail route — measured on a real render at `/payments/pi_3Nk`, the rail
 * marked `/payments` current on all eight of its links and the drawer
 * marked nothing.
 *
 * The rule is `SideNav`'s, deliberately: a prefix match ending at a
 * SEGMENT boundary. `@vaam-apps/ui` keeps its own `isActive` private
 * (`side-nav.js`, not exported), so this cannot import it and is a
 * transcription of it instead — which is why `more-menu.test.tsx` does not
 * assert this expression's shape but asserts the drawer and the rail AGREE
 * on the same render. If the package ever changes its rule, that test is
 * what notices; re-reading this comment is not.
 *
 * The `/` special case is `SideNav`'s too, and it is load-bearing rather
 * than decorative: without it `startsWith("/")` is true of every path in
 * the app, so a root entry would be permanently current.
 */
export function isNavActive(href: string, currentPath: string): boolean {
  if (href === "/") return currentPath === "/";
  return currentPath === href || currentPath.startsWith(`${href}/`);
}
