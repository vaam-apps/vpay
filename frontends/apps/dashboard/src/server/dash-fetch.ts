/**
 * The one function that puts a request to vpay on the wire, and the one place
 * a credential is attached to it.
 *
 * # One credential per request, and the type is what says so
 *
 * `/dash/v1` takes a request from one of two doors (ADR-0037,
 * `envelope/console.rs`): a COSE_Sign1 message from a paired console key, or a
 * staff access token the authorization-code grant minted. A request that is
 * **both** is refused `400` — "one caller, one credential" — so a client that
 * could attach two would be one bad merge away from a sign-in that fails for a
 * reason no screen explains. {@link Credential} is a union, and
 * {@link credentialHeaders} turns exactly one variant into exactly one header:
 *
 * | variant       | header                 | for                                     |
 * | ------------- | ---------------------- | --------------------------------------- |
 * | `session`     | `X-Vpay-Staff-Session` | the five `/dash/v1/staff/*` routes      |
 * | `bearer`      | `Authorization`        | the reads the token authorises (VPA-5)  |
 * | `console-key` | none can be built      | the signed reader — VP-6b               |
 *
 * `console-key` is **unconstructible on purpose**: its payload is `never`, so
 * no value of the variant exists and the compiler walks every `switch` over
 * the union to its end. It is here so the day VP-6b adds the signed reader,
 * the third door is a new arm in one function rather than a header some call
 * site invented. Until then a value forced past the type system is refused at
 * runtime rather than sent bare.
 *
 * # What this function never does
 *
 * * **It never adds `Authorization` to a session request**, or a session
 *   header to a bearer one. There is no proxy here that "helpfully" attaches a
 *   bearer to every call: a staff route called with one is a `400` waiting to
 *   happen, and the bearer is a live `/dash/v1` credential that has no
 *   business on a sign-in step.
 * * **It never forwards what the browser sent.** It takes no `Headers`, no
 *   `Request` and no header map from a caller; the header set is built here,
 *   from the credential and a fixed list, so a COSE message or an
 *   `Authorization` header on the browser's request to this app cannot reach
 *   vpay by being copied. A server action does not even have the browser's
 *   headers in hand when it calls this.
 * * **It never follows a redirect on its own.** `redirect` is `"error"` unless
 *   the caller says `"manual"`, which only the authorization request does, to
 *   read the `Location` it must not follow.
 */

/** The header vpay takes a staff session token in (`vpay_api::staff::SESSION_HEADER`). */
export const SESSION_HEADER = "x-vpay-staff-session";

/** The one credential a request carries. See the module header. */
export type Credential =
  | { readonly kind: "session"; readonly token: string }
  | { readonly kind: "bearer"; readonly token: string }
  | { readonly kind: "console-key"; readonly token: never };

/** A session credential, for the `/dash/v1/staff/*` routes. */
export function sessionCredential(token: string): Credential {
  return { kind: "session", token };
}

/** A bearer credential, for the reads a `/dash/v1` access token authorises. */
export function bearerCredential(token: string): Credential {
  return { kind: "bearer", token };
}

/**
 * The header a credential travels in — exactly one, never zero for a variant
 * that has a value and never two.
 *
 * Throws for `console-key`, which only a cast can produce: sending the request
 * without its credential would be a request vpay answers `401` for a reason
 * that points nowhere near this call.
 */
export function credentialHeaders(
  credential: Credential,
): Record<string, string> {
  switch (credential.kind) {
    case "session":
      return { [SESSION_HEADER]: credential.token };
    case "bearer":
      return { authorization: `Bearer ${credential.token}` };
    case "console-key":
      throw new Error(
        "A console-key credential cannot be built until the signed reader exists (VP-6b).",
      );
  }
}

/** What one call carries beyond its URL. */
export interface DashRequest {
  readonly method: "GET" | "POST";
  /** The credential, or `undefined` for the two calls that precede one. */
  readonly credential?: Credential;
  /** A form-encoded body (`axum::Form` is what every staff handler takes). */
  readonly form?: Readonly<Record<string, string>>;
  /** `"manual"` only for the authorization request. */
  readonly redirect?: "error" | "manual";
}

/**
 * The request, as `fetch` will see it. Separate from {@link dashFetch} so a
 * test reads the header set without a network.
 */
export function buildInit(request: DashRequest): RequestInit {
  const headers: Record<string, string> = { accept: "application/json" };
  if (request.form !== undefined) {
    headers["content-type"] = "application/x-www-form-urlencoded";
  }
  if (request.credential !== undefined) {
    Object.assign(headers, credentialHeaders(request.credential));
  }
  return {
    method: request.method,
    headers,
    ...(request.form === undefined
      ? {}
      : { body: new URLSearchParams(request.form).toString() }),
    // Every call this app makes is a credential step or a read of live data.
    // A cached sign-in is not a thing that can be allowed to exist, and a
    // dashboard that showed a cached payment would be worse than one that
    // showed none.
    cache: "no-store",
    redirect: request.redirect ?? "error",
  };
}

/** Calls `url` with `request`. May reject (a network failure): callers wrap it. */
export function dashFetch(
  url: string,
  request: DashRequest,
): Promise<Response> {
  return fetch(url, buildInit(request));
}
