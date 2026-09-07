import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import QRCode from 'qrcode';

import { Heading, Stack, Text } from '@vpay/ui';

import { EnrolmentPanel } from '../../../src/components/enrolment-panel';
import { TotpForm } from '../../../src/components/totp-form';
import { submitTotp } from '../../../src/server/actions';
import { ENROLMENT_COOKIE } from '../../../src/server/cookies';
import { decodePendingEnrolment, secretFrom } from '../../../src/server/enrolment';
import { LOGIN_PATH, sessionToken } from '../../../src/server/session';

/**
 * `/login/totp` — leg two, and on a first sign-in the enrolment it completes.
 *
 * Which of the two this is is decided by the **cookie**, not by a query
 * parameter: `?enrol=1` in an address bar is something anybody can type, and
 * a page that showed an enrolment panel on the strength of it would be
 * offering to enrol a secret it does not have. The sealed blob either is in
 * the cookie `signIn` set or it is not.
 *
 * A visitor with no session at all goes back to `/login`. That is the only
 * branch here: whether the session is at the right *stage* is vpay's
 * decision, taken when the code is presented, and duplicating it in this app
 * would be a second copy of the rule that could disagree with the first.
 */
export const dynamic = 'force-dynamic';

export default async function TotpPage() {
  const token = await sessionToken();
  if (token === null) {
    redirect(LOGIN_PATH);
  }

  const store = await cookies();
  const pending = decodePendingEnrolment(store.get(ENROLMENT_COOKIE)?.value);

  if (pending === null) {
    return (
      <Stack direction="column" gap="md">
        <Heading level={2}>Enter your code</Heading>
        <Text tone="muted" size="sm">
          Open your authenticator app and enter the six-digit code for this
          account.
        </Text>
        <TotpForm action={submitTotp} />
      </Stack>
    );
  }

  // Rendered on this server, into a data URL. The secret never travels to a
  // third-party QR service, and no markup is injected into the page.
  const qrDataUrl = await QRCode.toDataURL(pending.otpauth, { margin: 1, width: 220 });

  return (
    <Stack direction="column" gap="lg">
      <EnrolmentPanel qrDataUrl={qrDataUrl} secret={secretFrom(pending.otpauth)} />
      <TotpForm action={submitTotp} enrolling />
    </Stack>
  );
}
