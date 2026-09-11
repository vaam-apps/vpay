/**
 * The backend-for-frontend: `/dash/v1`'s two reads, on this app's own origin,
 * authenticated by the httpOnly session cookie.
 *
 * # What this changes about the app, stated first because it is the point
 *
 * Until this file existed, **nothing in this app was reachable from a browser
 * except a page and four Server Actions.** Every `/dash/v1` read happened
 * inside a render. This adds two `GET` endpoints a script on this origin can
 * call, which is a new authenticated surface and is the reason Lane 2 is its
 * own change with its own review rather than a detail of the lane that will
 * consume it.
 *
 * **Whether this app should have such a surface at all is a decision this
 * file does not take.** It is RD5 in
 * `docs/plans/exp55-refine-seam-bff-notes/refine-plan.md` §8: it reverses a
 * stated property of the app's security model, and it is reserved to the
 * maintainer. What is here is that surface built as carefully as it can be,
 * with its checks written as mutations, so that the decision is taken against
 * something real rather than against a description of it. **No page and no
 * component calls it today**; `docs/status.md` says so in as many words, and
 * nothing in the app stops working if these two route files are deleted.
 *
 * # The three things it must never do
 *
 * 1. **Reach vpay for a caller it has not authenticated.** The cookie is read
 *    before anything is sent, and a request without one costs no upstream
 *    call at all. `bff.test.ts` asserts the stub `fetch` was never invoked,
 *    not merely that the status was `401` — a `401` produced *after* an
 *    upstream round trip is a different program with the same status code.
 * 2. **Accept a request this dashboard did not issue.** {@link apiIsSameOrigin}
 *    applies `csrf.ts`'s `originIsAllowed` — the same function, not a second
 *    copy of the rule — and adds the check `originIsAllowed` cannot make on a
 *    `GET`; see that function.
 * 3. **Let the bearer token out.** The body is re-serialised from named fields
 *    rather than streamed, and no upstream header is copied — which here is
 *    structural rather than disciplined: `api.ts`'s `getJson` answers an
 *    `ApiResult`, so **this file never holds the upstream `Response` at all**
 *    and there is no object in scope for a header to be copied off. The token
 *    is an argument to `readDash` and never reaches a response. The
 *    projection is what the test can still break — **on the detail read, and
 *    only there.** `serve`'s `project` was described here as the decisive
 *    thing for both handlers and it is not: `dashProvider.getList` already
 *    answers `{ data, hasMore, cursor }`, a shape it built field by field, so
 *    the list handler's own projection renames those three keys and drops
 *    nothing. Measured, exp55 review: replace `served(project(result.value))`
 *    with `served(result.value)` and `puts it in no part of the detail read
 *    either` goes red while `puts the bearer token in no header and no body,
 *    ever` — the list case — stays green. `getOne` answers the parsed
 *    upstream document whole, which is why the detail read is where a field
 *    vpay grew would otherwise reach the browser.
 *
 * # And the one it must never be talked into doing
 *
 * **No caller-supplied merchant id, audience or scope is forwarded, and there
 * is no parameter for one to arrive through.** The tenant is
 * `MerchantScope::for_dashboard`, inserted server-side from the YAML
 * dashboard binding and never read from the token, and the audience is the
 * registered `dashboard_client.client_id` — both of them vpay's, neither of
 * them this app's to name. The upstream query string is built by
 * `apiQueryString`, which emits exactly `limit`, `status`, `created_gte`,
 * `created_lte` and one of `starting_after`/`ending_before`; anything else in
 * the caller's URL is not dropped so much as never looked at.
 *
 * # Why it is not `requireStaff`
 *
 * `requireStaff` *redirects*, because its callers are pages: a browser that
 * has lost its session should end up at a form. A `307` to `/login` answered
 * to a `fetch` is HTML with a `200` by the time the caller sees it, which is
 * indistinguishable from success — and `refusalFor`'s whole rule (`401` ends
 * a session, everything else including `403` is an outage) is carried by the
 * status code. So the decisions are reused and the *acting* is different,
 * which is the split `gate.ts` and `session.ts` already make.
 *
 * That leaves two transcriptions of one branch table, and the honest thing is
 * to name it rather than to claim the duplication away: {@link bearerFor}
 * below mirrors `requireStaff`'s token branches, its comments say which
 * branch of `requireStaff` each one is, and `bff.test.ts` pins every one. If
 * `requireStaff` changes, this has to change with it.
 */
import { NextResponse } from "next/server";

import type { DashboardConfig } from "../config/settings";
import {
  dashProvider,
  PAYMENT_INTENTS,
  type DashSession,
} from "../dash/provider";
import { queryFrom } from "../payments-query";
import type { ApiFailure, ApiResult } from "./api";
import { SESSION_COOKIE } from "./cookies";
import { originIsAllowed, HOST_HEADER, ORIGIN_HEADER } from "./csrf";
import { gateFor, refusalFor } from "./gate";
import { completeAuthorizationCode } from "./oauth";
import { readSession } from "./session";

/**
 * The header that says what kind of context issued a request.
 *
 * Browser-set and `Sec-`prefixed, which means a page cannot override it:
 * the Fetch spec forbids script from setting any header whose name begins
 * `Sec-`.
 */
const FETCH_SITE_HEADER = "sec-fetch-site";

/** The sentence a refused request answers with, in vpay's own envelope shape. */
interface ErrorBody {
  readonly error: {
    readonly message: string;
    readonly request_id: string | null;
  };
}

/**
 * Whether this request came from this dashboard's own pages.
 *
 * # Two headers, because `Origin` alone cannot answer it on a `GET`
 *
 * `csrf.ts` compares `Origin` against the configured public origin, and that
 * is the right comparison — it is the one value in the exchange no caller can
 * influence. But **a browser sends no `Origin` at all on a same-origin `GET`**
 * (Fetch, "origin header": it is set for CORS-mode requests and for requests
 * whose method is neither `GET` nor `HEAD`), and script cannot add one:
 * `Origin` is a forbidden header name. So a rule that required `Origin` would
 * refuse every honest call this surface exists to serve, and a rule that
 * treated its absence as "fine" would be no rule at all.
 *
 * `Sec-Fetch-Site` is what a browser sends instead, on every request,
 * unforgeably. `same-origin` is the only value this surface accepts:
 *
 * | Value | What it is | Here |
 * | --- | --- | --- |
 * | `same-origin` | this app's own page, fetching | allowed |
 * | `same-site` | another host under the same registrable domain | refused |
 * | `cross-site` | anybody else | refused |
 * | `none` | a URL typed or bookmarked | refused — this is not a page |
 * | absent | no fetch metadata at all | refused |
 *
 * **"No fetch metadata" is a larger set than it sounds, and this refusal is
 * the availability cost of the rule rather than a free win.** Chromium has
 * sent `Sec-Fetch-Site` since 2019 and Firefox since 2020, but **Safari only
 * since 16.4 (March 2023)** — so every WebKit browser older than that,
 * including the system web view on an older iOS, is refused by this surface
 * with a `403` that says the request did not come from this dashboard. The
 * direction is still the right one (the alternative is a rule an attacker
 * removes by removing a header), and it costs nothing today because nothing
 * calls these handlers — but a client that starts to must decide what it
 * shows such a browser, and this is not something the unit tests below can
 * see.
 *
 * **Absent is refused, which is the opposite of what `signed-out.ts` does**,
 * and the divergence is deliberate rather than an oversight. That route
 * forgets a cookie for a person arriving on a page, so refusing a client that
 * sends no metadata would break it for the honest caller while the attack
 * needs a browser that does send it. This one is read by this app's own
 * script and by nothing else, so the direction that keeps it working is to
 * insist on the metadata.
 *
 * **And `Origin` is still checked whenever it is there**, with
 * `csrf.ts`'s own function and no second copy of its rule — including its
 * refusal of an `Origin: null` and its `Host` fallback when the deployment
 * has not set `VPAY_DASHBOARD_PUBLIC_ORIGIN`. Deleting that call is the
 * decisive mutation for this lane, and `refuses a request whose Origin is
 * not this deployment's` is the case that goes red.
 *
 * @param headers the request's headers, verbatim
 * @param publicOrigin `VPAY_DASHBOARD_PUBLIC_ORIGIN`, or `null` if unset
 */
export function apiIsSameOrigin(
  headers: Headers,
  publicOrigin: string | null,
): boolean {
  const origin = headers.get(ORIGIN_HEADER);
  if (
    origin !== null &&
    !originIsAllowed(origin, publicOrigin, headers.get(HOST_HEADER))
  ) {
    return false;
  }
  return headers.get(FETCH_SITE_HEADER) === "same-origin";
}

/**
 * The staff session token this request presented, or `null`.
 *
 * Parsed off the request rather than read through `next/headers`' `cookies()`
 * for the reason `signed-out.ts` gives about writing one: a handler that can
 * only be exercised inside a request scope is a handler nothing checks, and
 * the case that matters here — "no cookie, and therefore nothing sent
 * upstream" — is precisely the one a browser test cannot observe.
 *
 * The value is taken up to the first `;` and after the first `=`, so a token
 * containing `=` survives and a cookie named `x_vpay_dash_session` does not
 * match.
 *
 * # And it is percent-decoded, because the writer percent-encodes
 *
 * `setSessionCookie` writes through `next/headers`' `cookies().set`, which
 * serialises with `cookie.serialize` and therefore **percent-encodes** the
 * value; `cookies().get()` decodes it again, so `session.ts` and this file
 * are two readers of one cookie and only one of them was undoing the
 * encoding. Nothing vpay mints today is affected — the session token is
 * base64url, whose alphabet `serialize` leaves alone — so this is a
 * divergence that has never fired rather than a bug that has. It would fire
 * closed if it ever did (this surface would present a differently-spelled
 * token and get vpay's `401`), which is why it is worth a line and not a
 * redesign: a second transcription that disagrees with the first only under
 * a change nobody is thinking about is exactly the shape this file's own doc
 * comment warns about for `bearerFor`.
 *
 * A value that is not valid percent-encoding is returned verbatim rather than
 * refused: `decodeURIComponent` throws on a lone `%`, and a cookie this app
 * did not write is vpay's to reject, not this function's.
 */
export function sessionCookieOf(headers: Headers): string | null {
  const header = headers.get("cookie");
  if (header === null) {
    return null;
  }
  for (const pair of header.split(";")) {
    const eq = pair.indexOf("=");
    if (eq < 0) {
      continue;
    }
    if (pair.slice(0, eq).trim() !== SESSION_COOKIE) {
      continue;
    }
    const value = pair.slice(eq + 1).trim();
    return value.length > 0 ? decodeCookieValue(value) : null;
  }
  return null;
}

/** `decodeURIComponent`, with a malformed escape left as it was written. */
function decodeCookieValue(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/**
 * The headers every answer of this surface carries, refusal and success alike.
 *
 * `cache-control: no-store` — a dashboard that showed a cached payment would
 * be worse than one that showed none, and a *shared* cache holding one
 * merchant's page would be worse than either. `api.ts` says the first half of
 * that for the upstream call; this says the second, for the browser and for
 * anything in front of it.
 *
 * `x-content-type-options: nosniff` — these two URLs are reachable by
 * same-origin navigation, not only by `fetch`, so a browser can be made to
 * treat the answer as a document rather than as data. The body is vpay's own
 * JSON, merchant-authored `description` and `metadata` included; declaring
 * that its content type is not a suggestion costs nothing and removes the
 * question.
 */
const WIRE_HEADERS: Readonly<Record<string, string>> = {
  "cache-control": "no-store",
  "x-content-type-options": "nosniff",
};

/** A refusal, as JSON, with no upstream header on it and nothing cached. */
function refusal(
  status: number,
  message: string,
  requestId: string | null = null,
): NextResponse<ErrorBody> {
  return NextResponse.json(
    { error: { message, request_id: requestId } },
    {
      status,
      headers: WIRE_HEADERS,
    },
  );
}

/**
 * What a `status: 0` failure says on the wire, instead of what it says here.
 *
 * `api.ts`'s `unreachable` builds its message out of the thrown error, which
 * on this path is Node's: `connect ECONNREFUSED 10.42.3.17:8080
 * (vpay-server.vpay-prod.svc.cluster.local)` names the payments API's internal
 * address, and a `JSON.parse` failure names the first bytes of whatever
 * answered instead — `Unexpected token '<', "<html><hea"... is not valid
 * JSON`, which is a fragment of an upstream proxy's error page. `failureOf`
 * already refuses to render an unparseable body raw for exactly that reason
 * (`api.ts`, "A body this app cannot parse is **not** rendered raw"); this is
 * the same rule applied to the other half of the same file, because `status:
 * 0` is the one failure whose message was written from an exception rather
 * than from vpay's envelope.
 *
 * It matters more here than on a page: a page renders this to a staff member
 * who is already signed in, and this is a scriptable endpoint that answers a
 * caller holding **any** cookie value — the session read is what fails, so the
 * refusal is reached before anybody has been authenticated.
 */
const UNREACHABLE = "vpay could not be reached.";

/**
 * An upstream refusal, as this surface's own.
 *
 * The status is carried through unchanged, because the status **is** the
 * meaning: `refusalFor`'s rule is that `401` ends a session and everything
 * else — `403` included — is an outage (issue #88 item 2), and a client
 * cannot apply that rule to a status this layer flattened. `status: 0` is
 * `api.ts`'s "no response at all", which is a bad gateway and not a `0` — and
 * is the one case whose *message* is replaced rather than carried: see
 * {@link UNREACHABLE}.
 */
function upstreamRefusal(failure: ApiFailure): NextResponse<ErrorBody> {
  return failure.status === 0
    ? refusal(502, UNREACHABLE, failure.requestId)
    : refusal(failure.status, failure.message, failure.requestId);
}

/** What {@link bearerFor} answered: a token to read with, or the refusal. */
type Bearer =
  | { readonly ok: true; readonly accessToken: string }
  | { readonly ok: false; readonly response: NextResponse<ErrorBody> };

/**
 * The `/dash/v1` bearer for this session, minting one where a render would.
 *
 * Every branch below is `requireStaff`'s, with a status where it has a
 * redirect. They are named so a reader can check the two against each other:
 *
 * | `requireStaff` | here |
 * | --- | --- |
 * | session read fails, `outage` | the upstream status (`0` → `502`) |
 * | session read fails, `401` | `401` |
 * | `must-change-password` → `/login/password` | `403`, and see below |
 * | `ready` | the token on the row |
 * | `stale-token`, mint `ok` | the minted token |
 * | `stale-token`, mint outage → render anyway | the token on the row |
 * | `stale-token`, mint `401` → `/signed-out` | `401` |
 * | `needs-token`, mint `ok` | the minted token |
 * | `needs-token`, mint outage | `502` |
 * | `needs-token`, mint `401` → `/signed-out` | `401` |
 *
 * **`must-change-password` is a `403` and this surface cannot do better.** A
 * page sends such a session to `/login/password`; an endpoint has nowhere to
 * send anybody, and answering `401` would sign out a session that is not over
 * — it is a step away from being usable. It is a state a caller reaching this
 * from a rendered page cannot be in, because `requireStaff` redirected them
 * before the page existed; a client that meets it has to route the person
 * itself, and none exists yet. `docs/status.md` carries that as a gap rather
 * than this file inventing a mapping for it.
 */
async function bearerFor(
  config: DashboardConfig,
  sessionToken: string,
): Promise<Bearer> {
  const { session, failure } = await readSession(config, sessionToken);
  if (session === null) {
    if (failure === null) {
      return { ok: false, response: refusal(502, UNREACHABLE) };
    }
    return { ok: false, response: upstreamRefusal(failure) };
  }

  const gate = gateFor(session, Date.now());
  if (gate.kind === "must-change-password") {
    return {
      ok: false,
      response: refusal(
        403,
        "This session must replace its one-time password before it can read anything.",
      ),
    };
  }
  if (gate.kind === "ready") {
    return { ok: true, accessToken: gate.accessToken };
  }

  const minted = await completeAuthorizationCode(config, sessionToken);
  if (minted.ok) {
    return { ok: true, accessToken: minted.value.access_token };
  }
  if (refusalFor(minted.failure) === "outage") {
    // A stale token is not a missing one. The one in hand is still inside its
    // TTL — that is what `REMINT_AFTER_FRACTION`'s margin bought — so a vpay
    // that could not be reached for the re-mint costs nothing here, and if it
    // does expire mid-request `readDash` answers the `401` with its own
    // attempt. `requireStaff` takes the identical fallback.
    if (gate.kind === "stale-token") {
      return { ok: true, accessToken: gate.accessToken };
    }
    return { ok: false, response: upstreamRefusal(minted.failure) };
  }
  // A `401` is NOT fallen back on, stale token or not: `/oauth/authorize`
  // refused this session on this request — signed out elsewhere, the account
  // disabled, the staff member moved to another merchant — and reading on
  // with a credential that refusal has just invalidated is the hole the exp24
  // review closed one layer down.
  return { ok: false, response: upstreamRefusal(minted.failure) };
}

/**
 * Everything both handlers do before they differ, in the order it must happen.
 *
 * Origin first, so a request this dashboard did not issue is refused without
 * its cookie ever being looked at; then the cookie, so a caller without one
 * costs no upstream call; then the session, which is the first thing that
 * reaches vpay at all.
 */
async function readerFor(
  request: Request,
  config: DashboardConfig | null,
): Promise<
  | { readonly ok: true; readonly session: DashSession }
  | { readonly ok: false; readonly response: NextResponse<ErrorBody> }
> {
  if (config === null) {
    // `/login` is where an operator reads which variables are missing. Here
    // there is nothing to read with and nothing to say about it.
    return {
      ok: false,
      response: refusal(503, "This dashboard is not configured."),
    };
  }
  if (!apiIsSameOrigin(request.headers, config.publicOrigin)) {
    return {
      ok: false,
      response: refusal(
        403,
        "This request did not come from this dashboard. Reload the page and try again.",
      ),
    };
  }
  const token = sessionCookieOf(request.headers);
  if (token === null) {
    return {
      ok: false,
      response: refusal(401, "This request carried no staff session."),
    };
  }
  const bearer = await bearerFor(config, token);
  if (!bearer.ok) {
    return bearer;
  }
  return {
    ok: true,
    session: {
      config,
      accessToken: bearer.accessToken,
      sessionToken: token,
    },
  };
}

/**
 * A request's query string in the shape `queryFrom` expects, repeats and all.
 *
 * `Object.fromEntries(searchParams.entries())` is the obvious spelling and it
 * is **wrong here**: it keeps the *last* value of a repeated parameter, and
 * `payments-query.ts`'s `one()` is explicit that the **first** wins — so the
 * page and this surface would disagree about what `?status=succeeded&status=canceled`
 * asks for, which is exactly the kind of second opinion the seam exists to
 * prevent. `getAll` keeps the array `one()` was written for, and Next hands a
 * Server Component the same shape.
 */
function searchRecord(url: string): Record<string, string[]> {
  const params = new URL(url).searchParams;
  // `Object.create(null)` and not `{}`: the keys here are the caller's, and on
  // a plain object `record['__proto__'] = […]` does not add a key — it invokes
  // `Object.prototype`'s `__proto__` setter and *replaces this object's
  // prototype* with the array. `queryFrom` reads five fixed names and was
  // measured not to be steerable that way (`?__proto__=…` leaves the upstream
  // query string as `limit=25` and `Object.prototype` untouched), so this is
  // defence in depth rather than a hole being closed — but a map of
  // attacker-chosen keys should not be one whose assignment can run a setter
  // at all.
  const record = Object.create(null) as Record<string, string[]>;
  for (const key of new Set(params.keys())) {
    record[key] = params.getAll(key);
  }
  return record;
}

/** A success, as JSON, with nothing on it but a content type and {@link WIRE_HEADERS}. */
function served(body: unknown): NextResponse {
  return NextResponse.json(body, {
    status: 200,
    headers: WIRE_HEADERS,
  });
}

/**
 * `GET /api/dash/payment_intents` — one page of this merchant's intents.
 *
 * The filters are read out of **this** request's query string by `queryFrom`,
 * which names five parameters and reads no others, and are turned back into
 * vpay's vocabulary by `apiQueryString`. A caller who adds `merchant_id`,
 * `audience` or `scope` to the URL is not refused; the parameter is simply
 * never read, which is a stronger property than a denylist because it holds
 * for the next parameter somebody thinks of too.
 *
 * The answer is built field by field from the parsed upstream document. It is
 * not the upstream `Response` and nothing of it is streamed: `url`, which
 * `ListObject` carries and nothing here needs, does not survive, and neither
 * would anything else vpay grew.
 */
export async function paymentIntentsListResponse(
  request: Request,
  config: DashboardConfig | null,
): Promise<NextResponse> {
  const reader = await readerFor(request, config);
  if (!reader.ok) {
    return reader.response;
  }

  const query = queryFrom(searchRecord(request.url));
  const result = await dashProvider(reader.session).getList({
    resource: PAYMENT_INTENTS,
    query,
  });

  return serve(result, (page) => ({
    object: "list",
    data: page.data,
    has_more: page.hasMore,
    cursor: page.cursor,
  }));
}

/**
 * `GET /api/dash/payment_intents/{id}` — everything `/dash/v1` knows about one
 * payment.
 *
 * A `404` is carried through as a `404`, and carrying it through is the whole
 * of the care this handler needs: `/dash/v1` answers the **identical** body
 * for another merchant's id and for one that never existed
 * (`dashboard_read_surface.rs:531` compares the two byte for byte), and a
 * layer that added the id, the resource name or a different sentence to one
 * branch would turn this app into an oracle for which ids exist in other
 * tenants. `upstreamRefusal` writes `failure.message`, which is vpay's own
 * one sentence, and knows nothing about the id.
 */
export async function paymentIntentResponse(
  request: Request,
  id: string,
  config: DashboardConfig | null,
): Promise<NextResponse> {
  const reader = await readerFor(request, config);
  if (!reader.ok) {
    return reader.response;
  }

  const result = await dashProvider(reader.session).getOne({
    resource: PAYMENT_INTENTS,
    id,
  });

  return serve(result, (detail) => ({
    object: detail.object,
    payment_intent: detail.payment_intent,
    charge: detail.charge,
    refunds: detail.refunds,
    events: detail.events,
  }));
}

/**
 * A read's outcome as a response: the projection on success, the status and
 * the sentence on a refusal.
 *
 * `project` is where "re-serialise rather than forward" is enforced — it names
 * the keys, so a key vpay adds tomorrow reaches nobody until somebody writes
 * it down here. The projection is one level deep: what sits under
 * `payment_intent` or `charge` is vpay's own object shape, published to
 * merchants through two SDKs, and re-listing its fields here would be a third
 * copy of a schema to drift.
 */
function serve<T>(
  result: ApiResult<T>,
  project: (value: T) => unknown,
): NextResponse {
  return result.ok
    ? served(project(result.value))
    : upstreamRefusal(result.failure);
}
