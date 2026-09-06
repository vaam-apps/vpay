/**
 * The ZenStack access policies: the rules `zenstack/schema.zmodel` declares,
 * and the fact that the client every server module holds has the plugin that
 * enforces them.
 *
 * # This file lost something at ZenStack 3, and the loss is the first thing
 * to read
 *
 * Under v2 these cases drove a **live** client at a URL nothing listened on
 * and asserted that `product.create`, `product.deleteMany` and the rest were
 * refused *before a connection was opened* — `enhance()` evaluated the policy
 * first, so the refusal was provable with no database at all.
 *
 * v3's `PolicyPlugin` does not do that, and it does not refuse the same way
 * either. Both measured on 2026-09-06 against `3.9.3`, against a real
 * Postgres:
 *
 * | Call | v2 | v3 |
 * |---|---|---|
 * | `product.create` | threw "denied by policy" | throws "operation is rejected by access policies" |
 * | `product.deleteMany({})` | threw | resolves `{ count: 0 }`, and the five catalogue rows are still there |
 * | `order.deleteMany({})`, `orderItem.deleteMany({})`, `webhookEvent.updateMany`, `webhookEvent.deleteMany` | threw | same: resolve `{ count: 0 }`, nothing written |
 *
 * The **effect** is unchanged — no protected row moves — but a bulk write is
 * now *filtered* rather than refused, so a caller that relied on the throw
 * would silently succeed at doing nothing. Nothing in this shop does; every
 * write it makes is a single-row one on a model that allows it.
 *
 * And every one of those calls reaches the pool first, so with no database
 * they all fail with `connect ECONNREFUSED` rather than with a policy
 * decision. **So the enforcement itself is no longer proven by anything that
 * runs in CI**, and it cannot be without a real Postgres, which CI's `web`
 * job does not have. That is a regression the upgrade cost; it is recorded in
 * `docs/status.md` and in `examples/shop/README.md`, and the table above is
 * a hand-run on 2026-09-06, which is the only evidence there is.
 *
 * What these cases do prove is the two things that can be proven offline, and
 * they are the two mutations that actually happen:
 *
 * 1. **The rules are still the rules.** Widen `Product` to
 *    `@@allow('all', true)` in the zmodel, regenerate, and the first
 *    describe below fails. That is the same decisive check the v2 version of
 *    this file documented.
 * 2. **The client is still enforcing.** Delete `$use(new PolicyPlugin())`
 *    from `src/server/db.ts` and the second describe fails.
 *
 * What is *not* proven is that the plugin, given those rules, refuses. That
 * is ZenStack's own suite's job, and this file now says so rather than
 * implying otherwise.
 */
import { describe, expect, it } from "vitest";
import { PolicyPlugin } from "@zenstackhq/plugin-policy";
import { db, unenforcedDb } from "../db";
import { schema } from "../../../zenstack/schema";

type ModelName = keyof typeof schema.models;

/**
 * The operations a model's `@@allow` rules name, as written.
 *
 * Read off the **generated** schema rather than by grepping the zmodel: the
 * generated file is what the plugin consults at runtime, so a rule that
 * failed to compile through would be invisible to a grep and caught here.
 */
function allowed(model: ModelName): string[] {
  const operations: string[] = [];
  for (const attribute of schema.models[model].attributes ?? []) {
    if (attribute.name !== "@@allow") {
      continue;
    }
    for (const argument of attribute.args ?? []) {
      const value: unknown = argument.value;
      if (
        argument.name === "operation" &&
        typeof value === "object" &&
        value !== null &&
        "value" in value &&
        typeof value.value === "string"
      ) {
        operations.push(...value.value.split(",").map((part) => part.trim()));
      }
    }
  }
  return operations.sort();
}

describe("the policies the schema declares", () => {
  it("makes the catalogue read-only to the application", () => {
    // Seed data (D13). `create`, `update` and `delete` are absent, not
    // denied: ZenStack's default is deny, so the absence *is* the rule.
    expect(allowed("Product")).toEqual(["read"]);
  });

  it("never lets an order or a line be deleted", () => {
    // A cancelled order is a row with `status = cancelled`, not an absence.
    expect(allowed("Order")).toEqual(["create", "read", "update"]);
    expect(allowed("OrderItem")).toEqual(["create", "read"]);
  });

  it("keeps the delivery log append-only", () => {
    // The dedupe guarantee depends on it: a `webhook_events` row that could
    // be rewritten or removed is a replay that could be applied twice.
    expect(allowed("WebhookEvent")).toEqual(["create", "read"]);
  });

  it("declares a rule for every model, so none is silently wide open", () => {
    for (const model of Object.keys(schema.models) as ModelName[]) {
      expect(allowed(model).length, model).toBeGreaterThan(0);
    }
  });
});

describe("the client the shop actually uses", () => {
  it("carries the policy plugin", () => {
    const plugins = db().$options.plugins ?? [];
    expect(plugins.map((plugin) => plugin.id)).toContain(new PolicyPlugin().id);
  });

  it("and the unenforced one, which only a test may hold, does not", () => {
    // `unenforcedDb` exists so a future Postgres-backed suite can set a row
    // up that the policies then refuse to change. Nothing under `src/app`,
    // `src/server` or `src/components` calls it.
    expect(unenforcedDb().$options.plugins ?? []).toEqual([]);
  });
});
