import { DELIVERIES_SCREEN } from "../../../src/dash/list-screens";
import { ListSkeleton } from "../../../src/components/list-skeleton";

/** The instant a link to /deliveries is followed — see `ListSkeleton`. */
export default function Loading() {
  return <ListSkeleton {...DELIVERIES_SCREEN} />;
}
