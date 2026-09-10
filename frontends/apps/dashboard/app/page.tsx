import { redirect } from "next/navigation";

import { alreadySignedIn, HOME_PATH, LOGIN_PATH } from "../src/server/session";

/**
 * `/` — the door, and nothing else.
 *
 * It used to be the whole app: a scaffold notice plus a legend of every
 * status badge, because there was no data source and rendering invented rows
 * would have been the failure mode `CLAUDE.md` names third. There is a data
 * source now (`/dash/v1`, ADR-0017's sign-in in front of it), so this route
 * has nothing of its own to say and sends a visitor to whichever of the two
 * real pages applies.
 *
 * The status legend went with it. It was a reference for a payments list
 * nobody had written; the list exists, and every badge on it is the same
 * `StatusBadge` taking its tone from `@vpay/tokens`.
 */
export const dynamic = "force-dynamic";

export default async function Home() {
  redirect((await alreadySignedIn()) ? HOME_PATH : LOGIN_PATH);
}
