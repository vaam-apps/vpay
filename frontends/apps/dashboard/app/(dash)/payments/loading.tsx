import { PaymentsSkeleton } from "../../../src/components/payments-skeleton";

/** The instant a link to /payments is followed — the screen's own skeleton. */
export default function Loading() {
  return <PaymentsSkeleton />;
}
