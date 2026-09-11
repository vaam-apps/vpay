/**
 * The method policy, checked against **Next's own implementation** of the
 * thing it exists to suppress rather than against a description of it.
 *
 * The first case below imports
 * `next/dist/server/route-modules/app-route/helpers/auto-implement-methods` —
 * the function Next actually calls to build a route module's method table —
 * and runs the real `app/api/dash/payment_intents/route.ts` through it. It
 * therefore measures, on whichever Next this workspace resolved, that an
 * unauthenticated `OPTIONS` would be answered `204` with
 * `Allow: GET, HEAD, OPTIONS` **by code that runs before `bff.ts` does**. If
 * Next ever stops doing that, this case fails and somebody re-reads whether
 * `middleware.ts` still has a job.
 *
 * A deep import into `next/dist` is a private path and is deliberate: the
 * alternative is a test that asserts what this file believes Next does, which
 * would keep passing on the day Next changed and would have been the reason
 * the finding was recorded as "assumed" in the first place (exp55 review, on
 * the claim that "only `GET` is exported, so Next answers `405` to every
 * other method").
 *
 * **THE DECISIVE CASE for this change is
 * `answers OPTIONS 405 with no Allow header, before the route module can`.**
 * Delete the `READ_METHODS` branch in `middleware.ts` — or return
 * `NextResponse.next()` unconditionally, which is the shape of "no method
 * policy at all" — and it goes red on both assertions: the status becomes the
 * pass-through and the `Allow` header comes back from the auto-implemented
 * handler. It is written as a pair with the case above it so that the two
 * read as one sentence: this is what Next would answer, and this is what the
 * caller gets instead.
 */
import { autoImplementMethods } from "next/dist/server/route-modules/app-route/helpers/auto-implement-methods";
import type {
  AppRouteHandlerFn,
  AppRouteHandlers,
} from "next/dist/server/route-modules/app-route/module";
import { describe, expect, it } from "vitest";

import * as detailRoute from "./app/api/dash/payment_intents/[id]/route";
import * as listRoute from "./app/api/dash/payment_intents/route";
import { config, middleware } from "./middleware";

/** A request to the BFF, method-only — nothing here authenticates anything. */
function probe(method: string, path = "/api/dash/payment_intents"): Request {
  return new Request(`https://dash.example${path}`, { method });
}

/**
 * A route module as Next's own loader hands it to `autoImplementMethods`.
 *
 * The parameter is `object` rather than a route module's own type, and the
 * widening is the point rather than laziness. `AppRouteHandlerFn`'s context is
 * `params?: Promise<Record<string, string | string[] | undefined>>`, while the
 * `[id]` route declares `params: Promise<{ id: string }>` — the narrower type
 * Next's own generated `.next/types` gives that file — so passing that module
 * straight in is a `strictFunctionTypes` error on the context parameter. Next
 * erases the difference at runtime by calling the handler with whatever it
 * parsed out of the path. Doing the widening once, here, is more honest than
 * loosening the route's own signature to please a test.
 */
function asHandlers(module: object): AppRouteHandlers {
  return module;
}

/** The method table's functions take a context Next builds; these take none. */
const NO_CONTEXT = {} as Parameters<AppRouteHandlerFn>[1];

describe("what Next would answer on its own", () => {
  it("auto-implements OPTIONS as a 204 that enumerates the methods", async () => {
    // The route module, exactly as Next loads it. `dynamic` is an extra
    // export and `autoImplementMethods` reads only the HTTP-method names.
    const methods = autoImplementMethods(asHandlers(listRoute));

    const response = (await methods.OPTIONS(
      probe("OPTIONS") as Parameters<AppRouteHandlerFn>[0],
      NO_CONTEXT,
    )) as Response;

    // 204, with the method list, and — the part that makes it a finding
    // rather than a curiosity — from a closure that never calls the route
    // file. `paymentIntentsListResponse` is not on this path at all, so the
    // origin check and the cookie read are not either.
    expect(response.status).toBe(204);
    expect(response.headers.get("allow")).toBe("GET, HEAD, OPTIONS");
  });

  it("auto-implements HEAD as the GET handler itself, on both routes", () => {
    const list = autoImplementMethods(asHandlers(listRoute));
    const detail = autoImplementMethods(asHandlers(detailRoute));

    // Not a refusal and not a copy: the same function object. A `HEAD`
    // therefore costs the same session read, the same possible token mint
    // and the same upstream call as a `GET`, which is why `middleware.ts`
    // lets it through rather than refusing it.
    expect(list.HEAD).toBe(listRoute.GET);
    expect(detail.HEAD).toBe(detailRoute.GET);
  });
});

describe("the method policy", () => {
  it("answers OPTIONS 405 with no Allow header, before the route module can", async () => {
    const response = middleware(probe("OPTIONS"));

    expect(response.status).toBe(405);
    // The two assertions are different claims and both are load-bearing.
    // The status says the auto-implemented `204` is not what the caller
    // gets; the absent `Allow` says the method enumeration did not survive
    // in another form. A `405` carrying `Allow: GET, HEAD` would have
    // published most of the same thing.
    expect(response.headers.get("allow")).toBeNull();
    expect(await response.text()).not.toContain("GET");
  });

  it("refuses every write method the same way, named or not", () => {
    // `POST` and `DELETE` were already Next's `405`; `PROPFIND` is here
    // because the policy is a whitelist of two and not a list of the methods
    // somebody thought of.
    for (const method of ["POST", "PUT", "PATCH", "DELETE", "PROPFIND"]) {
      const response = middleware(probe(method));
      expect(response.status, method).toBe(405);
      expect(response.headers.get("allow"), method).toBeNull();
    }
  });

  it("lets the two reads through untouched", () => {
    for (const method of ["GET", "HEAD"]) {
      const response = middleware(probe(method));
      // `NextResponse.next()` is the pass-through: the route handler runs and
      // `bff.ts` makes every decision. This middleware authenticates nothing
      // and must not look as though it does.
      expect(response.status, method).toBe(200);
      expect(response.headers.get("x-middleware-next"), method).toBe("1");
    }
  });

  it("is refused before the route module, not by it", () => {
    // The refusal carries no `request_id`, because nothing upstream was
    // asked. `bff.ts`'s refusals can carry one; this layer never can, and
    // that is the honest difference between them.
    const response = middleware(probe("OPTIONS"));
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("x-content-type-options")).toBe("nosniff");
  });

  it("answers alike for a path in the subtree that no route file serves", () => {
    // Which is why the OPTIONS half of the route-existence question really is
    // closed rather than merely quieter: the middleware runs before routing,
    // so it cannot tell these two apart and neither can the caller. Measured
    // against `next start` as well as here — `OPTIONS /api/dash/nope`
    // answered `404` before this file existed and answers `405` now, the
    // same as `OPTIONS /api/dash/payment_intents`.
    //
    // The GET half is NOT closed and nothing here should be read as saying
    // it is: `GET /api/dash/nope` is still Next's `404` while the real route
    // reaches `bff.ts`'s gate.
    const real = middleware(probe("OPTIONS", "/api/dash/payment_intents"));
    const absent = middleware(probe("OPTIONS", "/api/dash/nope"));

    expect(absent.status).toBe(real.status);
    expect([...absent.headers].sort()).toEqual([...real.headers].sort());
  });

  it("matches the BFF subtree, including routes nobody has written", () => {
    // The matcher is Next's to apply and this cannot run it — what it can do
    // is pin the literal, so that narrowing it to the two paths that exist
    // today is a visible change rather than a quiet one. `:path*` is zero or
    // more segments.
    expect(config.matcher).toBe("/api/dash/:path*");
  });
});
