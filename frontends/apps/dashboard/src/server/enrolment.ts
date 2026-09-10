/**
 * What has to survive between the password and the first TOTP code, and the
 * shape it survives in.
 *
 * A first sign-in answers `POST /dash/v1/staff/login` with two things the
 * next request needs and vpay will not hand out again:
 *
 * * `enrolment` — the TOTP secret **sealed** with AES-256-GCM under a
 *   deployment key this app does not have. It goes back to `/staff/totp` with
 *   the first code, which is what commits the enrolment. It cannot be
 *   committed at login: a secret written before the person proved they can
 *   generate a code from it locks them out permanently, because
 *   `Staff::enrol_totp`'s guard is `totp_enrolled_at IS NULL`.
 * * `otpauth_uri` — the same secret in **plaintext**, in Key Uri Format, for
 *   the QR code and the typed-secret fallback. There is no way to enrol an
 *   authenticator without showing it.
 *
 * # Why both live in one httpOnly cookie
 *
 * The sealed blob teaches its holder nothing (they have no key) and replaying
 * it buys nothing (`enrol_totp` is a compare-and-swap, so only the first
 * completion wins). The URI does carry the plaintext secret — and the screen
 * this cookie feeds renders that secret on purpose, so the cookie is not a
 * disclosure the enrolment does not already make. It is `httpOnly` anyway, so
 * no script on this origin can read it, and it lives ten minutes.
 *
 * One cookie rather than two because they are one fact: an enrolment in
 * progress. Two could go out of step, and a screen holding a QR code whose
 * sealed twin had expired would ask for a code nothing could accept.
 */

/** The pair, as it travels. */
export interface PendingEnrolment {
  /** The sealed secret, opaque to this app. */
  readonly sealed: string;
  /** `otpauth://totp/…` — the QR code's content. */
  readonly otpauth: string;
}

/**
 * The pair as one cookie value.
 *
 * JSON, then URL-encoded: an `otpauth://` URI carries `?`, `&`, `=` and `:`,
 * and a cookie value containing them is legal but is exactly the kind of
 * string a proxy or a framework decides to be helpful about.
 */
export function encodePendingEnrolment(pending: PendingEnrolment): string {
  return encodeURIComponent(JSON.stringify(pending));
}

/**
 * The pair back, or `null` for anything that is not one.
 *
 * `null` rather than a throw for every malformed case — a truncated cookie, a
 * value from an older shape, a JSON object missing a field. The page's answer
 * to `null` is to send the staff member back to `/login`, which is right for
 * all of them and is one branch instead of a `try` at every call site.
 */
export function decodePendingEnrolment(
  value: string | undefined,
): PendingEnrolment | null {
  if (typeof value !== "string" || value.length === 0) {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(decodeURIComponent(value));
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return null;
  }
  const { sealed, otpauth } = parsed as { sealed?: unknown; otpauth?: unknown };
  if (typeof sealed !== "string" || sealed.length === 0) {
    return null;
  }
  if (typeof otpauth !== "string" || !otpauth.startsWith("otpauth://")) {
    return null;
  }
  return { sealed, otpauth };
}

/**
 * The base32 secret out of an `otpauth://` URI, for the "type it in" fallback.
 *
 * Every authenticator app takes a typed secret, and a staff member on a
 * desktop with the app on a phone that cannot scan needs it. Read out of the
 * URI rather than passed alongside it, so the two can never disagree about
 * which secret is being enrolled.
 *
 * `null` if the URI carries no `secret` parameter, which would mean vpay
 * answered something `otpauth_uri`'s own test says it cannot.
 */
export function secretFrom(otpauth: string): string | null {
  const query = otpauth.indexOf("?");
  if (query < 0) {
    return null;
  }
  const secret = new URLSearchParams(otpauth.slice(query + 1)).get("secret");
  return secret === null || secret.length === 0 ? null : secret;
}
