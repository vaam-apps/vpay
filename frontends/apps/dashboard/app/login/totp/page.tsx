import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import QRCode from "qrcode";

import { ScreenStack } from "@vaam-apps/ui";

import { dashboardConfig } from "../../../src/config/runtime";
import { EnrolmentPanel } from "../../../src/components/enrolment-panel";
import { ReadFailure } from "../../../src/components/read-failure";
import { TotpForm } from "../../../src/components/totp-form";
import { submitTotp } from "../../../src/server/actions";
import { ENROLMENT_COOKIE } from "../../../src/server/cookies";
import {
  decodePendingEnrolment,
  secretFrom,
} from "../../../src/server/enrolment";
import { totpGateFor } from "../../../src/server/gate";
import {
  HOME_PATH,
  LOGIN_PATH,
  readSessionStage,
  sessionToken,
} from "../../../src/server/session";

/**
 * `/login/totp` — leg two, and on a first sign-in the enrolment it completes.
 *
 * Which of the two this is is decided by the **cookie**, not by a query
 * parameter: `?enrol=1` in an address bar is something anybody can type, and
 * a page that showed an enrolment panel on the strength of it would be
 * offering to enrol a secret it does not have. The sealed blob either is in
 * the cookie `signIn` set or it is not.
 *
 * # This page reads the session, and until 2026-09-10 it did not
 *
 * It rendered the code form for whatever cookie was present. That is what
 * forced `submitTotp` to infer the session's fate from the `401` that
 * `POST /staff/totp` answers for a **wrong code** as well as for a session it
 * refuses — so a typo cleared the cookie and sent a staff member back to the
 * email-and-password form (the exp36 review's F6; F1 was the same shape one
 * route over, on `/login/password`).
 *
 * `GET /dash/v1/staff/session/stage` is the read, and it exists because
 * `/staff/session` is refused at this stage. Its answer, and nothing the
 * action returns, is what ends a session here:
 *
 * * a `401` — absent, expired, idle, forged, disabled — is the only thing
 *   that sends somebody to {@link LOGIN_PATH};
 * * a session already carrying both factors goes on to the payments list,
 *   which is where a back button lands somebody who has finished;
 * * a vpay that could not be reached renders the failure and keeps the
 *   cookie, exactly as `/login/password` does (issue #88 item 2);
 * * anything else is a live session still owed a code, which is this page.
 *
 * Whether the session is at the right stage to *accept* a code remains vpay's
 * decision, taken when the code is presented. This page reads a stage to know
 * whether the session is alive, not to duplicate that rule.
 */
export const dynamic = "force-dynamic";

export default async function TotpPage() {
  const { config } = dashboardConfig();
  if (config === null) {
    redirect(LOGIN_PATH);
  }

  const token = await sessionToken();
  if (token === null) {
    redirect(LOGIN_PATH);
  }

  const { stage, failure } = await readSessionStage(config, token);
  const gate = totpGateFor(stage, failure);
  if (gate.kind === "dead") {
    redirect(LOGIN_PATH);
  }
  if (gate.kind === "signed-in") {
    redirect(HOME_PATH);
  }
  if (gate.kind === "outage") {
    return (
      <ScreenStack>
        <h2 className="text-title-sm font-medium text-foreground">
          Enter your code
        </h2>
        <ReadFailure failure={gate.failure} />
      </ScreenStack>
    );
  }

  const store = await cookies();
  const pending = decodePendingEnrolment(store.get(ENROLMENT_COOKIE)?.value);

  if (pending === null) {
    return (
      <ScreenStack>
        <h2 className="text-title-sm font-medium text-foreground">
          Enter your code
        </h2>
        <p className="text-body text-muted-foreground">
          Open your authenticator app and enter the six-digit code for this
          account.
        </p>
        <TotpForm action={submitTotp} />
      </ScreenStack>
    );
  }

  // Rendered on this server, into a data URL. The secret never travels to a
  // third-party QR service, and no markup is injected into the page.
  const qrDataUrl = await QRCode.toDataURL(pending.otpauth, {
    margin: 1,
    width: 220,
  });

  return (
    // `ScreenStack` has one fixed gap (2026-09-12, `@vaam-apps/ui` cutover —
    // no `gap="lg"` counterpart), so the enrolment branch loses the extra
    // spacing the old `Stack gap="lg"` gave it. A small, stated change.
    <ScreenStack>
      <EnrolmentPanel
        qrDataUrl={qrDataUrl}
        secret={secretFrom(pending.otpauth)}
      />
      <TotpForm action={submitTotp} enrolling />
    </ScreenStack>
  );
}
