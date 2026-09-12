/**
 * The two Refine screens, rendered.
 *
 * They are the half of lanes 3 and 4 that the provider tests cannot reach:
 * a provider that maps a query correctly is no use if the screen it feeds
 * renders an empty state over live rows, or signs somebody out of an outage.
 *
 * Refine is mounted for real here — `<Refine>` with a stub `dataProvider` —
 * rather than mocking `useList`. Mocking the hook would assert that this
 * file calls a function, which is not the thing worth knowing.
 */
import { Refine } from "@refinedev/core";
import { render, screen } from "@testing-library/react";
import type { DataProvider } from "@refinedev/core";
import { describe, expect, it, vi } from "vitest";

import { NO_QUERY } from "../payments-query";
import { DETAIL, INTENT as LIST_INTENT } from "../testing/fixtures";

vi.mock("next/navigation", () => ({
  usePathname: () => "/payments",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: vi.fn() }),
  notFound: () => {
    throw new Error("NEXT_NOT_FOUND");
  },
}));

const { PaymentsScreen } =
  await import("../../app/(dash)/payments/payments-screen");
const { PaymentScreen } =
  await import("../../app/(dash)/payments/[id]/payment-screen");

const INTENT = {
  id: "pi_test_0000000000000001",
  object: "payment_intent",
  amount: 5000,
  currency: "XAF",
  status: "succeeded",
  created: "2026-09-12T09:00:00Z",
  livemode: false,
  description: null,
  customer: null,
  payment_method_types: ["mtn_momo"],
  last_payment_error: null,
  metadata: {},
};

function providerReturning(
  page: unknown,
  error?: { statusCode: number },
): DataProvider {
  return {
    getList: () =>
      error
        ? Promise.reject(Object.assign(new Error("refused"), error))
        : Promise.resolve(page as never),
    getOne: () => Promise.resolve({ data: {} as never }),
    create: () => Promise.resolve({ data: {} as never }),
    update: () => Promise.resolve({ data: {} as never }),
    deleteOne: () => Promise.resolve({ data: {} as never }),
    getApiUrl: () => "/api/dash",
  };
}

function mount(provider: DataProvider) {
  return render(
    <Refine
      dataProvider={provider}
      resources={[{ name: "payment_intents", list: "/payments" }]}
      options={{
        disableTelemetry: true,
        // The same `retry: false` the app sets — see `(dash)/layout.tsx`.
        // Without it a rejected read spins on the skeleton and the error
        // branch is unreachable, here and for an operator.
        reactQuery: {
          clientConfig: { defaultOptions: { queries: { retry: false } } },
        },
      }}
    >
      <PaymentsScreen query={NO_QUERY} />
    </Refine>,
  );
}

describe("the payments screen", () => {
  it("renders the rows it was given", async () => {
    mount(
      providerReturning({
        data: [INTENT],
        total: 0,
        cursor: { next: null, prev: null },
      }),
    );
    expect(await screen.findByText(/5,000|5 000/)).toBeInTheDocument();
  });

  it("says 'no payments' for an empty list, and never over live rows", async () => {
    mount(
      providerReturning({
        data: [],
        total: 0,
        cursor: { next: null, prev: null },
      }),
    );
    expect(await screen.findByText(/No payments/i)).toBeInTheDocument();
  });

  it("renders the failure for an outage rather than an empty list", async () => {
    // The distinction this screen exists to keep: "this merchant has no
    // payments" and "the read was refused" are different answers, and the
    // empty list is the one an operator would believe.
    //
    // `role="alert"` and the message, NOT `role="status"`: the first draft
    // of this case asserted `status`, which `InlineEmptyState` also carries,
    // so deleting the whole error branch left it green. Measured — that
    // mutation is the reason these assertions are this specific.
    mount(providerReturning(null, { statusCode: 503 }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("refused");
    expect(screen.queryByText(/No payments/i)).toBeNull();
  });

  it("treats a 403 as an outage too, not as a sign-out", async () => {
    mount(providerReturning(null, { statusCode: 403 }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("refused");
    expect(screen.queryByText(/No payments/i)).toBeNull();
  });

  it("renders the page heading as an <h2>, under the shell's <h1>", async () => {
    const { container } = mount(
      providerReturning({
        data: [INTENT],
        total: 0,
        cursor: { next: null, prev: null },
      }),
    );
    await screen.findByText(/5,000|5 000/);
    expect(container.querySelector("h2")?.textContent).toBe("Payments");
    expect(container.querySelector("h1")).toBeNull();
  });
});

/**
 * THE READ THE SERVER ALREADY MADE, and the two defects that gate.
 *
 * Both were measured in `dashboard.cy.ts` against the real stack, and
 * neither is a rendering opinion:
 *
 * 1. **The first paint must carry the data.** The masked-payer leg visits
 *    `/payments/{id}` and reads `body` **once**, without retrying. When
 *    `useOne` fetched from the browser, that read landed on a skeleton for
 *    every row in the list, and the leg reported "no payment in this
 *    merchant's list has a charge" about a merchant with four. So the
 *    assertions below are deliberately **synchronous** — `getBy`, never
 *    `findBy` — because `findBy` is exactly the retry the browser leg does
 *    not have.
 * 2. **The screen must ask `/api/dash` for nothing it was handed.** The BFF
 *    legs alias `GET /api/dash/payment_intents` *before* they visit the
 *    page, so a request the page makes for itself is the one `cy.wait`
 *    consumes. That is what made
 *    `refuses the same browser fetch the moment the session cookie is gone`
 *    fail between its `cy.clearCookie` and the `cy.setCookie` that restores
 *    the session — and the six legs after it ran signed out.
 *
 * The stub provider therefore **rejects**: any call at all is the defect,
 * and a resolving stub would only prove the screen preferred one answer to
 * another.
 */
const REFUSES_TO_BE_CALLED = "the browser must not read what the server read";

function spyingProvider(): {
  provider: DataProvider;
  getList: ReturnType<typeof vi.fn>;
  getOne: ReturnType<typeof vi.fn>;
} {
  const getList = vi.fn(() => Promise.reject(new Error(REFUSES_TO_BE_CALLED)));
  const getOne = vi.fn(() => Promise.reject(new Error(REFUSES_TO_BE_CALLED)));
  return {
    provider: {
      getList,
      getOne,
      create: () => Promise.resolve({ data: {} as never }),
      update: () => Promise.resolve({ data: {} as never }),
      deleteOne: () => Promise.resolve({ data: {} as never }),
      getApiUrl: () => "/api/dash",
    },
    getList,
    getOne,
  };
}

function mountWith(provider: DataProvider, children: React.ReactNode) {
  return render(
    <Refine
      dataProvider={provider}
      resources={[
        { name: "payment_intents", list: "/payments", show: "/payments/:id" },
      ]}
      options={{
        disableTelemetry: true,
        reactQuery: {
          clientConfig: { defaultOptions: { queries: { retry: false } } },
        },
      }}
    >
      {children}
    </Refine>,
  );
}

describe("the screens render the read the server already made", () => {
  it("has the server's rows in the FIRST render, and asks the BFF for nothing", () => {
    const { provider, getList } = spyingProvider();
    mountWith(
      provider,
      <PaymentsScreen
        query={NO_QUERY}
        initial={{
          ok: true,
          value: {
            data: [LIST_INTENT],
            total: 0,
            cursor: { next: null, prev: null },
          },
        }}
      />,
    );

    // No `await`. This is the `load` event, which is where the browser leg
    // reads the document.
    expect(screen.getByText(/5,000|5 000/)).toBeInTheDocument();
    expect(screen.queryByText(/No payments/i)).toBeNull();
    expect(getList).not.toHaveBeenCalled();
  });

  it("has the server's payment in the FIRST render, and asks the BFF for nothing", () => {
    const { provider, getOne } = spyingProvider();
    mountWith(
      provider,
      <PaymentScreen
        id="pi_example_1"
        initial={{ ok: true, value: { data: DETAIL } }}
      />,
    );

    expect(screen.getByTestId("detail-id")).toHaveTextContent("pi_example_1");
    // The masked-payer leg's own assertion, at the moment it makes it.
    expect(screen.getByTestId("detail-payer")).toBeInTheDocument();
    expect(screen.getByTestId("detail-rail")).toHaveTextContent("mtn_momo");
    expect(getOne).not.toHaveBeenCalled();
  });

  it("renders a refusal the server met, and does NOT re-read it from the browser", () => {
    // A retry of a refused read is the same refusal a round trip later —
    // except for a `401`, which reaches `authProvider.onError`, whose answer
    // is `{ logout: true }` and therefore the real `signOut()`. The server
    // deliberately kept this session (issue #88 item 2); a browser retry
    // would end it.
    const { provider, getList } = spyingProvider();
    mountWith(
      provider,
      <PaymentsScreen
        query={NO_QUERY}
        initial={{
          ok: false,
          failure: {
            status: 503,
            message: "vpay refused the read.",
            requestId: "req_example_1",
          },
        }}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("refused");
    expect(screen.queryByText(/No payments/i)).toBeNull();
    expect(getList).not.toHaveBeenCalled();
  });
});
