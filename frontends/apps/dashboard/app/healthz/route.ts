/**
 * `GET /healthz` — "this process is serving", and deliberately nothing more.
 *
 * Added by ADR-0022's Dockerfile-hardening commit, for the same reason
 * `frontends/apps/checkout/app/healthz/route.ts` exists: it is what the
 * chart's probes and the local `docker run --read-only` observation poll,
 * and before this file there was **no** health path in this app at all —
 * the `runner` stage's Dockerfile `HEALTHCHECK` has nothing to point at
 * without it.
 *
 * **It takes no dependency and reports on none.** This app's only upstream
 * is `/dash/v1`, reached server-side through `app/api/dash/*`'s BFF
 * (`docs/adr/0008-dashboard-scope.md`) — and probing that from here would
 * mean a `vpay-server` rollout or a management-tier blip takes this pod's
 * readiness down with it, which is exactly the coupling
 * `checkout`'s own health route argues against. Readiness on this app means
 * "Next is answering"; whether `/dash/v1` is reachable is that surface's own
 * probe's answer.
 *
 * `force-dynamic` for the same reason checkout's route states: the answer
 * must come from the running process, not a statically rendered response
 * the filesystem would keep serving after the server stopped doing anything
 * else.
 */
export const dynamic = "force-dynamic";

export function GET(): Response {
  return new Response("ok\n", {
    status: 200,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}
