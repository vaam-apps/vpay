/**
 * The ZenStack client, with the access-control plugin installed.
 *
 * The `@@allow` rules in `zenstack/schema.zmodel` are enforced **here and
 * nowhere else**, and they are the rules D13 makes true: the catalogue is
 * read-only to the application, delivery records are append-only, and no
 * order or line is ever deleted. `src/server/store/policies.test.ts` proves
 * the refusals.
 *
 * # What changed at ZenStack 3, and why this file is not a rename
 *
 * v2 was a *wrapper*: `enhance(new PrismaClient())` returned a proxy that
 * evaluated policies and delegated to Prisma. v3 replaced Prisma at runtime
 * with its own ORM over Kysely, so there is no Prisma client here to wrap —
 * the database connection is a `pg` `Pool` handed to `PostgresDialect`, and
 * the policies are a **plugin** the client opts into with `$use`. Prisma has
 * not left the project: `zen migrate` still drives Prisma Migrate to apply
 * the SQL in `zenstack/migrations`. It is no longer in the request path.
 *
 * Two clients are exported for one reason, and it is the reason v3 splits
 * them: `db()` carries the plugin and is what every server module must use;
 * `unenforcedDb()` does not and exists **only** so a test can set a row up
 * that the policies then refuse to change. Nothing under `src/app`,
 * `src/server` or `src/components` calls it — `policies.test.ts` is the only
 * caller in the package, and `no-runtime-imports.test.ts` is the guard that
 * `src/testing` never leaks the other way.
 *
 * The singleton is the standard Next.js shape: a module-scoped instance kept
 * on `globalThis` so `next dev`'s hot reload does not open a new pool per
 * edit.
 */
import { Pool } from "pg";
import { ZenStackClient, type ClientContract } from "@zenstackhq/orm";
import { PostgresDialect } from "@zenstackhq/orm/dialects/postgres";
import { PolicyPlugin } from "@zenstackhq/plugin-policy";
import { schema, type SchemaType } from "../../zenstack/schema";

/**
 * The client every caller holds.
 *
 * Written out rather than inferred from `baseClient()`: a `ReturnType<typeof
 * …>` on a memoised constructor is a type that references itself, and
 * TypeScript answers `any` for it — which would silently take the model
 * methods' types with it. `ClientContract` is the ORM's own name for the
 * shape, and `$use` returns one too, so the enforced and unenforced clients
 * are the same type and `ZenStackShopStore` needs to know nothing about
 * plugins.
 */
export type ShopClient = ClientContract<SchemaType>;

const globalForDb = globalThis as unknown as {
  vpayShopPool?: Pool;
  vpayShopClient?: ShopClient;
};

function pool(): Pool {
  const existing = globalForDb.vpayShopPool;
  if (existing) {
    return existing;
  }
  // `connectionString` from the environment, exactly as the v2 `PrismaClient`
  // read `env("DATABASE_URL")` out of the datasource block. `pg` throws on a
  // missing one at first query rather than at construction, which is why the
  // shop's own `loadShopConfig` does not police it: the failure is a
  // connection error naming the URL, not a config error naming a variable.
  const created = new Pool({ connectionString: process.env["DATABASE_URL"] });
  globalForDb.vpayShopPool = created;
  return created;
}

function baseClient(): ShopClient {
  const existing = globalForDb.vpayShopClient;
  if (existing) {
    return existing;
  }
  const created = new ZenStackClient(schema, {
    dialect: new PostgresDialect({ pool: pool() }),
  });
  globalForDb.vpayShopClient = created;
  return created;
}

/** The policy-enforcing client every server module should use. */
export function db(): ShopClient {
  return baseClient().$use(new PolicyPlugin());
}

/**
 * The same client **without** the policies.
 *
 * Exported so `policies.test.ts` can create the row a refusal is then proven
 * against — a test that could not write a `Product` could only assert that
 * `product.create` fails, which is also what a broken connection does.
 * Nothing in the application calls this.
 */
export function unenforcedDb(): ShopClient {
  return baseClient();
}
