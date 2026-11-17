import { CenteredShell } from "../src/components/centered-shell";
import { NotFoundNotice } from "../src/components/not-found-notice";

/** Every path that is not a page: a plain 404, in the same shell as the rest. */
export default function NotFound() {
  return (
    <CenteredShell>
      <NotFoundNotice />
    </CenteredShell>
  );
}
