import { ListSkeleton } from "../../../src/components/list-skeleton";

/** The instant a link to /checkouts is followed — see `ListSkeleton`. */
export default function Loading() {
  return <ListSkeleton title="Checkouts" />;
}
