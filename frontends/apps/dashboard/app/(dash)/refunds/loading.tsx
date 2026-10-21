import { REFUNDS_SCREEN } from "../../../src/dash/list-screens";
import { ListSkeleton } from "../../../src/components/list-skeleton";

/** The instant a link to /refunds is followed — see `ListSkeleton`. */
export default function Loading() {
  return <ListSkeleton {...REFUNDS_SCREEN} />;
}
