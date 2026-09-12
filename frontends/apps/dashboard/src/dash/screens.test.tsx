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
      <PaymentsScreen />
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
