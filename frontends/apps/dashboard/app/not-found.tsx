import { ConsoleUnavailable } from "../src/components/console-unavailable";

/**
 * Every path that is not `/`: the same answer. There are no other pages until
 * VP-6b, and a `404` that said nothing would read as a broken link rather than
 * as a console that cannot sign anybody in yet.
 */
export default function NotFound() {
  return <ConsoleUnavailable />;
}
