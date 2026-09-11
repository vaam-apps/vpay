/**
 * The method policy in front of `/api/dash/**`, and the one thing it exists
 * to stop Next from answering on its own.
 *
 * # What was measured, and in which Next
 *
 * A route file that exports only `GET` does **not** make every other method a
 * `405`. `next/dist/server/route-modules/app-route/helpers/auto-implement-methods`
 * auto-implements two of them before the userland module is called:
 *
 * - `HEAD` is bound to the `GET` handler itself (`methods.HEAD = handlers.GET`)
 *   with the body discarded. That one is wanted — it costs the same session
 *   read and the same upstream call as a `GET`, and `/dash/v1` admits `HEAD`
 *   too — so it is left alone.
 * - **`OPTIONS` answers `204` with `Allow: GET, HEAD, OPTIONS`, from a closure
 *   built at route-compile time**, so it answers *before* `bff.ts` runs: before
 *   the origin check, before the cookie is read, before anything has decided
 *   whether this caller is anybody. It is the only way to reach an answer from
 *   this surface without passing its gate.
 *
 * The exp55 security review recorded this against **Next 16.3.4**, which is
 * `examples/shop`'s pin and not this app's. This app resolves **Next
 * 15.5.25**, and the helper there is byte-for-byte the same in the part that
 * matters — `AUTOMATIC_ROUTE_METHODS = ['HEAD', 'OPTIONS']`, the same `Allow`
 * assembly, the same `204`. So the finding held, but the version named in it
 * was not the version it was about; `middleware.test.ts` pins the behaviour
 * against whichever Next is installed rather than against either number.
 *
 * # What this middleware does, stated narrowly
 *
 * Every method on `/api/dash/**` that is not `GET` or `HEAD` is answered
 * `405` here, with **no `Allow` header**, before the route module is reached.
 * That is all. It removes the one answer this surface could give without
 * running its own checks, and it removes the method enumeration that came
 * with it.
 *
 * # What it closes, and what it does not — measured against `next start`
 *
 * Measured on a real server, not reasoned about. The middleware runs on the
 * whole matched subtree **before routing**, so an `OPTIONS` to a path under
 * `/api/dash/` that no route file serves answers the same `405` as the two
 * that do:
 *
 * | probe | before | after |
 * | --- | --- | --- |
 * | `OPTIONS /api/dash/payment_intents` | `204` + `Allow` | `405` |
 * | `OPTIONS /api/dash/nope` | `404` | `405` |
 * | `POST /api/dash/payment_intents` | `405` (Next's) | `405` (this one's) |
 * | `GET /api/dash/payment_intents` | the gate | the gate, unchanged |
 * | `GET /api/dash/nope` | `404` | `404` |
 * | `OPTIONS /payments` | `400` | `400`, not matched |
 *
 * So for `OPTIONS` the route-existence question is genuinely closed: every
 * path in the subtree answers alike. **For `GET` it is not, and this file
 * does not claim otherwise** — an unauthenticated `GET` to a route that
 * exists reaches `bff.ts` and gets its `403` (or `503` where the app is
 * unconfigured), where a path nothing serves gets Next's `404`. `OPTIONS` was
 * the loudest discriminator and the only one reachable without passing a
 * check, never the only one; making the `GET` pair uniform would mean
 * answering `404` to an honest signed-out client, which trades a real
 * client's diagnosis for an attacker's inconvenience — and the route names
 * are in this repository anyway.
 *
 * # Why the logic is in this file rather than in `src/server/`
 *
 * `README.md`'s layout rule is that `app/` holds routes and `src/` holds
 * substance, and this file is neither. Next reads a middleware from the
 * project root or from `src/`, whichever holds the app directory — `app/` is
 * at the root here, so this is the root.
 *
 * **And its `config.matcher` has to be a literal in this file**, which is why
 * the obvious tidying (put the rule in `src/server/`, import the matcher) is
 * not available. Read rather than assumed, in
 * `next/dist/build/analysis/extract-const-value.js`: `extractExportedConstValue`
 * walks the SWC AST of the `config` export, and its `Identifier` branch
 * admits exactly one name — `undefined` — and throws
 * `UnsupportedValueError: Unknown identifier "…"` for every other. An
 * imported constant is an `Identifier`. So the matcher is written out here,
 * and `vitest.config.ts` was widened to pick up this file's test, rather than
 * the matcher being hidden behind an indirection Next would have refused.
 */
import { NextResponse } from "next/server";

/**
 * The methods that reach `bff.ts`.
 *
 * `HEAD` is here because Next binds it to the `GET` handler, so it goes
 * through the identical origin check, cookie read and projection; refusing it
 * here would diverge from `/dash/v1`, which serves it.
 */
const READ_METHODS: ReadonlySet<string> = new Set(["GET", "HEAD"]);

/**
 * The refusal, in the same envelope shape the rest of the surface answers in.
 *
 * Written out rather than imported from `bff.ts`: that module pulls
 * `readSession`, `completeAuthorizationCode` and the whole config assembly
 * behind it, and a middleware runs on the edge runtime for every matched
 * request. Four literal lines beat a dependency edge from the edge bundle
 * into the session stack.
 *
 * **No `Allow` header**, which is the entire point — an `Allow` here would
 * re-publish the method list this file exists to withhold. `cache-control`
 * and `x-content-type-options` match `bff.ts`'s `WIRE_HEADERS` so that a
 * refusal from this layer is not distinguishable from one from that layer by
 * its framing.
 */
function methodNotAllowed(): NextResponse {
  return NextResponse.json(
    {
      error: {
        message: "This surface serves reads only.",
        request_id: null,
      },
    },
    {
      status: 405,
      headers: {
        "cache-control": "no-store",
        "x-content-type-options": "nosniff",
      },
    },
  );
}

/**
 * Refuse a non-read on `/api/dash/**`; pass everything else through.
 *
 * @param request the matched request, untouched
 */
export function middleware(request: Request): NextResponse {
  if (READ_METHODS.has(request.method)) {
    return NextResponse.next();
  }
  return methodNotAllowed();
}

/**
 * The paths this runs on — the BFF subtree and nothing else.
 *
 * `:path*` matches zero or more segments, so it covers `/api/dash` itself,
 * the two routes that exist today, and **any route added under it later**.
 * That is the reason this is a middleware rather than an `OPTIONS` export in
 * each `route.ts`: an export has to be remembered once per file, and the
 * third route nobody has written yet would silently get Next's `204` back.
 *
 * Pages, Server Actions and `/signed-out` are not matched and are unaffected:
 * `csrf.ts` is still the only origin check in front of an action, and this
 * file adds nothing to and removes nothing from it.
 */
export const config = {
  matcher: "/api/dash/:path*",
};
