/**
 * Which ways of signing in this deployment serves, asked of vpay itself.
 *
 * `GET /.well-known/vpay-auth` answers `{"methods": ["keypair", ...]}`: the
 * methods the deployment permits **and serves** (ADR-0037 —
 * `AuthMethods::served`, so `human_login` is listed only when both its blocks,
 * `staff_auth` and `dashboard_client`, are configured). It is nothing secret and
 * nothing per-caller, and it is the one switch: this app does not carry a
 * second one that could disagree with the server's.
 *
 * # Three answers, and the third is not the second
 *
 * | answer          | what `/login` renders                                   |
 * | --------------- | ------------------------------------------------------- |
 * | `served`        | the sign-in (or, with no client registration, why not)   |
 * | `paired-key-only` | the notice that this console signs in with a paired key |
 * | `unreadable`    | an outage, with the request id                           |
 *
 * **A discovery that could not be read is never read as "paired key only".**
 * That would tell a staff member whose vpay was restarting that their
 * deployment had no password sign-in, and send them to an operator for a fault
 * that heals itself — the same mistake `gate.ts`'s `refusalFor` records for the
 * session read (issue #88 item 2). Only a body that parsed, whose `methods`
 * array does not name `human_login`, is `paired-key-only`.
 *
 * The body is validated rather than cast: anything that is not an object with
 * an array of strings in `methods` is `unreadable`, so an ingress that
 * answers `200` with an HTML error page cannot be mistaken for a deployment
 * with no password sign-in.
 */
import {
  envelopeMessage,
  requestIdOf,
  unreachable,
  type ApiFailure,
} from "./api";
import { dashFetch } from "./dash-fetch";

/** Where vpay publishes the methods it serves (`vpay_api::AUTH_DISCOVERY_PATH`). */
export const DISCOVERY_PATH = "/.well-known/vpay-auth";

/** The method name whose presence means a staff sign-in is served. */
export const HUMAN_LOGIN = "human_login";

/** What {@link readDiscovery} answers. */
export type Discovery =
  | { readonly kind: "served"; readonly methods: readonly string[] }
  | { readonly kind: "paired-key-only"; readonly methods: readonly string[] }
  | { readonly kind: "unreadable"; readonly failure: ApiFailure };

/** A refusal this app decided on its own, with the response's id if any. */
function unreadable(
  message: string,
  status: number,
  requestId: string | null,
): Discovery {
  return {
    kind: "unreadable",
    failure: { status, message, requestId },
  };
}

/**
 * The deployment's served methods, or why they could not be read.
 *
 * Never throws: a network failure is an `unreadable` answer, like every other
 * refusal in this app.
 */
export async function readDiscovery(apiBaseUrl: string): Promise<Discovery> {
  let response: Response;
  try {
    response = await dashFetch(`${apiBaseUrl}${DISCOVERY_PATH}`, {
      method: "GET",
    });
  } catch (error) {
    return { kind: "unreadable", failure: unreachable(error) };
  }

  const requestId = requestIdOf(response);
  let body: unknown = null;
  try {
    body = await response.json();
  } catch {
    // Handled below: a body that will not parse is not an answer.
  }

  if (!response.ok) {
    return unreadable(
      envelopeMessage(response.status, body),
      response.status,
      requestId,
    );
  }

  const methods = (body as { methods?: unknown } | null)?.methods;
  if (
    !Array.isArray(methods) ||
    !methods.every((method): method is string => typeof method === "string")
  ) {
    return unreadable(
      "vpay's list of sign-in methods was not in the shape this dashboard reads.",
      response.status,
      requestId,
    );
  }

  return methods.includes(HUMAN_LOGIN)
    ? { kind: "served", methods }
    : { kind: "paired-key-only", methods };
}
