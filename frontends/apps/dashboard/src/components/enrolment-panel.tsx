import { Alert, Code, Heading, Stack, Text } from "@vpay/ui";

export interface EnrolmentPanelProps {
  /** The `otpauth://totp/…` URI vpay minted, as a PNG data URL. */
  qrDataUrl: string;
  /** The same secret in base32, for a phone that cannot scan. */
  secret: string | null;
}

/**
 * First sign-in: the thing an authenticator app has to be given.
 *
 * Enrolment is **mandatory** (ADR-0017 decision 1) — a session never reaches
 * `authenticated` while `staff_members.totp_secret` is `NULL` — and it is
 * completed by the *next* form on this page, not by this panel: nothing is
 * written to `staff_members` until a code generated from this secret
 * verifies. A secret written before that would lock out anybody whose scan
 * failed, permanently, because `Staff::enrol_totp`'s guard is
 * `totp_enrolled_at IS NULL`.
 *
 * The secret is shown as text as well as as a QR code, deliberately: a staff
 * member on a desktop with an authenticator on a phone that cannot scan the
 * screen has no other way in, and "scan this or start again" is a support
 * call. Both render the same value — `secretFrom` reads it out of the URI the
 * QR encodes, so the two cannot disagree about which secret is being
 * enrolled.
 *
 * The QR is a PNG data URL rendered on this server, not markup injected into
 * the page and not a request to a third-party chart service: a TOTP secret
 * must not travel to anybody's URL bar but the person enrolling.
 */
export function EnrolmentPanel({ qrDataUrl, secret }: EnrolmentPanelProps) {
  return (
    <Stack direction="column" gap="md">
      <Heading level={2}>Set up your authenticator</Heading>
      <Text tone="muted" size="sm">
        Scan this with your authenticator app, then enter the six-digit code it
        shows. You will need that app every time you sign in.
      </Text>

      {/*
        eslint-disable-next-line @next/next/no-img-element --
        `next/image` optimises images it can fetch and cache. This is a
        `data:` URI generated for one request, at a fixed size, and it is a
        TOTP secret: routing it through an image optimiser would put a
        credential in an optimiser cache to save nothing.
      */}
      <img
        src={qrDataUrl}
        alt="QR code for this account's one-time password secret"
        width={220}
        height={220}
        data-testid="totp-qr"
      />

      {secret === null ? null : (
        <Stack direction="column" gap="xs">
          <Text size="sm">Cannot scan? Enter this key instead:</Text>
          <Text as="p" size="sm">
            <Code wrap="anywhere" data-testid="totp-secret">
              {secret}
            </Code>
          </Text>
        </Stack>
      )}

      <Alert tone="warning" role="status">
        This secret is shown once. It is not stored against your account until
        you enter a code from it below.
      </Alert>
    </Stack>
  );
}
