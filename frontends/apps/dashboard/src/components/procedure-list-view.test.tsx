/**
 * A procedure-backed list's body, mounted inside a real `<Refine>` with the
 * real `@cratestack/refine` provider whose transport is a stub that records
 * whether it was ever reached.
 *
 * What is held here: the server's read is what renders — its rows, or its
 * refusal — and the browser does not read again. A refusal is never drawn as
 * an empty list.
 */
import { createCratestackDataProvider } from "@cratestack/refine";
import { Refine } from "@refinedev/core";
import { render, screen } from "@testing-library/react";
import type { RefundSummary } from "@vpay/dash-client";
import { describe, expect, it, vi } from "vitest";

import type { Initial } from "../dash/initial";
import type { ProcedureListPage } from "../dash/procedure-paging";
import {
  procedureResources,
  type ProcedureLister,
} from "../dash/procedure-resources";
import { REFUNDS } from "../dash/resource-name";
import {
  ProcedureListView,
  type ProcedureListBodyProps,
} from "./procedure-list-view";
import { IdCell, type ProcedureColumn } from "./procedure-table";
import { Amount } from "./values";

vi.mock("next/navigation", () => ({
  usePathname: () => "/refunds",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: vi.fn() }),
}));

/**
 * A list body of the shape each list's client module has: its columns over the
 * generated row type, handed to the view. The pages that carried the real ones
 * are deleted until VP-6b (ADR-0036); the view is what is under test.
 */
const COLUMNS: readonly ProcedureColumn<RefundSummary>[] = [
  { key: "id", header: "Refund", render: (r) => <IdCell value={r.id} /> },
  {
    key: "amount",
    header: "Amount",
    align: "end",
    render: (r) => <Amount minor={r.amount} currency={r.currency_code} />,
  },
];

function RefundsList(props: ProcedureListBodyProps) {
  return (
    <ProcedureListView<RefundSummary>
      {...props}
      columns={COLUMNS}
      idOf={(r) => r.id}
    />
  );
}

const REFUND: RefundSummary = {
  id: "re_test_0000000000000001",
  payment_intent_id: "pi_test_0000000000000001",
  amount: 2500,
  currency_code: "XAF",
  status: "failed",
  failure_code: null,
  reason: null,
  created_at: "2026-10-20T09:00:00Z",
  updated_at: "2026-10-20T09:00:00Z",
};

function mount(initial: Initial<ProcedureListPage<unknown>>) {
  const lister = vi.fn<ProcedureLister>(() =>
    Promise.reject(new Error("the browser read again")),
  );
  render(
    <Refine
      dataProvider={createCratestackDataProvider(procedureResources(lister))}
      resources={[{ name: REFUNDS, list: "/refunds" }]}
      options={{
        disableTelemetry: true,
        reactQuery: {
          clientConfig: {
            defaultOptions: {
              queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
            },
          },
        },
      }}
    >
      <RefundsList
        resource={REFUNDS}
        title="Refunds"
        basePath="/refunds"
        filterField="status"
        chosen=""
        query=""
        offset={0}
        emptyMessage="No refunds. This merchant has no refunds yet."
        initial={initial}
      />
    </Refine>,
  );
  return lister;
}

describe("a procedure-backed list", () => {
  it("renders the server's rows and does not read them again", async () => {
    const lister = mount({
      ok: true,
      value: {
        list: { data: [REFUND], total: 1 },
        totalCount: 1,
        hasNextPage: false,
        offset: 0,
      },
    });
    expect(await screen.findByRole("table")).toBeInTheDocument();
    expect(screen.getAllByText(/2,500|2 500/).length).toBeGreaterThan(0);
    expect(lister).not.toHaveBeenCalled();
  });

  it("renders the server's refusal, never an empty list", async () => {
    // `role="alert"` and the request id, not `role="status"` —
    // `InlineEmptyState` carries `status` too, so asserting that would stay
    // green with the refusal branch deleted (`screens.test.tsx` measured it).
    const lister = mount({
      ok: false,
      failure: {
        status: 503,
        message: "vpay is restarting",
        requestId: "req_refunds",
      },
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "vpay is restarting",
    );
    expect(screen.getByText(/req_refunds/)).toBeInTheDocument();
    expect(screen.queryByText(/No refunds/)).not.toBeInTheDocument();
    expect(lister).not.toHaveBeenCalled();
  });

  it("says 'none' only for a read that answered no rows", async () => {
    mount({
      ok: true,
      value: {
        list: { data: [], total: 0 },
        totalCount: 0,
        hasNextPage: false,
        offset: 0,
      },
    });
    expect(await screen.findByText(/No refunds/)).toBeInTheDocument();
  });
});
