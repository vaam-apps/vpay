import {
  CHECKOUT_STATUS,
  checkoutStatusLabel,
  DELIVERY_STATE,
  deliveryStateLabel,
  REFUND_STATUS,
  refundStatusLabel,
} from "@vpay/tokens";

import type { ProcedureFilter } from "../components/procedure-list-screen";

/**
 * The words and filters of the four procedure-backed lists, in one place.
 *
 * Shared by each list's `page.tsx` and its `loading.tsx`, so the skeleton is
 * sized by the very strings the screen will render — a description that
 * wraps on a phone wraps in its placeholder too — and a `page.tsx` keeps
 * only the exports Next allows a page to have.
 */
export interface ListScreenCopy {
  readonly title: string;
  readonly description: string;
  readonly filter?: ProcedureFilter;
}

export const REFUNDS_SCREEN: ListScreenCopy = {
  title: "Refunds",
  description:
    "Money returned to payers, newest first. Each refund links to the payment it reverses.",
  filter: {
    param: "status",
    field: "status",
    label: "Filter refunds by status",
    options: REFUND_STATUS.map((value) => ({
      value,
      label: refundStatusLabel[value],
    })),
  },
};

export const DELIVERIES_SCREEN: ListScreenCopy = {
  title: "Deliveries",
  description:
    "Every webhook vpay owes this merchant's endpoints, newest first, and whether it arrived.",
  filter: {
    param: "state",
    field: "state",
    label: "Filter deliveries by state",
    options: DELIVERY_STATE.map((value) => ({
      value,
      label: deliveryStateLabel[value],
    })),
  },
};

export const CHECKOUTS_SCREEN: ListScreenCopy = {
  title: "Checkouts",
  description:
    "Hosted and embedded checkout sessions, newest first, and the payment each one opened.",
  filter: {
    param: "status",
    field: "status",
    label: "Filter checkouts by status",
    options: CHECKOUT_STATUS.map((value) => ({
      value,
      label: checkoutStatusLabel[value],
    })),
  },
};

export const CUSTOMERS_SCREEN: ListScreenCopy = {
  title: "Customers",
  description: "The payers this merchant has saved, newest first.",
};
