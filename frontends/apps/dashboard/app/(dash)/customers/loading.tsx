import { CUSTOMERS_SCREEN } from "../../../src/dash/list-screens";
import { ListSkeleton } from "../../../src/components/list-skeleton";

/** The instant a link to /customers is followed — see `ListSkeleton`. */
export default function Loading() {
  return <ListSkeleton {...CUSTOMERS_SCREEN} />;
}
