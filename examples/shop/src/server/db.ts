/**
 * The Prisma client, wrapped by ZenStack's `enhance`.
 *
 * `enhance` is not decoration: the `@@allow` rules in `schema.zmodel` are
 * enforced here and nowhere else, and they are the rules D13 makes true —
 * the catalogue is read-only to the application, delivery records are
 * append-only, and no order or line is ever deleted. `src/server/store/
 * policies.test.ts` proves the refusals (and needs no database to do it:
 * ZenStack evaluates a `create`/`delete` policy before it opens a
 * connection).
 *
 * The singleton is the standard Next.js shape: a module-scoped instance kept
 * on `globalThis` so `next dev`'s hot reload does not open a new pool per
 * edit.
 */
import { PrismaClient } from "@prisma/client";
import { enhance } from "@zenstackhq/runtime";

const globalForPrisma = globalThis as unknown as {
  vpayShopPrisma?: PrismaClient;
};

function basePrisma(): PrismaClient {
  const existing = globalForPrisma.vpayShopPrisma;
  if (existing) {
    return existing;
  }
  const client = new PrismaClient();
  globalForPrisma.vpayShopPrisma = client;
  return client;
}

/** The policy-enforcing client every server module should use. */
export function db(): PrismaClient {
  return enhance(basePrisma());
}
