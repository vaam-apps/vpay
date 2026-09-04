/**
 * Liveness for the container. Deliberately does **not** touch Postgres or
 * vpay: a health check that fails when a dependency is down turns one
 * outage into a restart loop, and `prisma migrate deploy` in the entrypoint
 * has already proved the database was reachable at start.
 */
export const dynamic = "force-dynamic";

export function GET(): Response {
  return Response.json({ status: "ok", service: "vpay-shop" });
}
