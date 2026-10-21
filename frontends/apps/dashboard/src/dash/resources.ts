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
  LayoutDashboard,
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
  /**
   * The rail's section header for this destination. A field on the entry
   * rather than a second list of groups, so a resource cannot be routed by
   * Refine and missing from the rail, or the other way round.
   */
  readonly group: NavGroupName;
}

/** The rail's sections, in the order they render. */
export const NAV_GROUP_NAMES = ["Money", "Customers", "Integrations"] as const;
export type NavGroupName = (typeof NAV_GROUP_NAMES)[number];

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
    group: "Money",
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
    group: "Money",
    icon: RotateCcw,
  },
  {
    resource: {
      name: WEBHOOK_DELIVERIES,
      list: "/deliveries",
      meta: { label: "Deliveries", canDelete: false },
    },
    label: "Deliveries",
    group: "Integrations",
    icon: Webhook,
  },
  {
    resource: {
      name: CHECKOUT_SESSIONS,
      list: "/checkouts",
      meta: { label: "Checkouts", canDelete: false },
    },
    label: "Checkouts",
    group: "Money",
    icon: ShoppingCart,
  },
  {
    resource: {
      name: CUSTOMERS,
      list: "/customers",
      meta: { label: "Customers", canDelete: false },
    },
    label: "Customers",
    group: "Customers",
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
 * The Overview: the rail's top item, and the one destination that is not a
 * Refine resource — it lists nothing, it counts. `layout.test.tsx` holds it
 * to the same rule as every resource: a page must exist at its href.
 */
export const OVERVIEW_NAV = {
  href: "/overview",
  label: "Overview",
  icon: LayoutDashboard,
} as const;

/**
 * `NAV_ENTRIES`, sectioned by each entry's `group`, in `NAV_GROUP_NAMES`
 * order. Derived, never declared, so it holds exactly the entries the rail
 * would otherwise have listed flat — `layout.test.tsx` checks that too.
 */
export const NAV_GROUPS: readonly {
  readonly label: NavGroupName;
  readonly items: readonly (typeof NAV_ENTRIES)[number][];
}[] = NAV_GROUP_NAMES.map((label) => ({
  label,
  items: DASH_RESOURCES.filter(
    (entry) => entry.group === label && typeof entry.resource.list === "string",
  ).map((entry) => ({
    href: entry.resource.list as string,
    label: entry.label,
    icon: entry.icon,
  })),
})).filter((group) => group.items.length > 0);
