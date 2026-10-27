import { ConsoleUnavailable } from "../src/components/console-unavailable";

/**
 * `/` — the whole app until VP-6b (ADR-0036).
 *
 * See {@link ConsoleUnavailable}. The pages that read `/dash/v1` are deleted,
 * not hidden: their reads ran through a staff session and a bearer token that
 * no longer exist, and a page nothing can feed would only be a place for the
 * next change to forget that. `docs/status.md` lists what VP-6b restores.
 */
export const dynamic = "force-dynamic";

export default function Home() {
  return <ConsoleUnavailable />;
}
