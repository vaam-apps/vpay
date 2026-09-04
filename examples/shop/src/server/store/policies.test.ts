/**
 * The ZenStack access policies in `schema.zmodel`, enforced.
 *
 * These run with **no database**: the Prisma client is pointed at a URL
 * nothing listens on, and every assertion below is a refusal ZenStack makes
 * before a connection is opened. A test that needed Postgres would not run in
 * CI's `web` job, and a policy nothing checks is a policy that decays.
 *
 * The decisive check, run by hand on 2026-09-04: delete
 * `@@allow('read', true)`'s companion — i.e. widen `Product` to
 * `@@allow('all', true)` — regenerate, and the first case here fails.
 */
import { describe, expect, it } from "vitest";
import { PrismaClient } from "@prisma/client";
import { enhance } from "@zenstackhq/runtime";

/** A client that could not reach a database even if a policy let it through. */
function unreachableDb(): PrismaClient {
  return new PrismaClient({
    datasources: { db: { url: "postgresql://nobody:nobody@127.0.0.1:1/nodb" } },
  });
}

const db = enhance(unreachableDb());

describe("the catalogue is read-only to the application", () => {
  it("refuses a product create", async () => {
    await expect(
      db.product.create({
        data: {
          id: "free-stuff",
          name: "Free stuff",
          description: "…",
          priceMinor: 0,
          currency: "xaf",
        },
      }),
    ).rejects.toThrow(/denied by policy/);
  });

  it("refuses a price update", async () => {
    await expect(
      db.product.updateMany({ data: { priceMinor: 1 } }),
    ).rejects.toThrow(/denied by policy/);
  });

  it("refuses a product delete", async () => {
    await expect(db.product.deleteMany({})).rejects.toThrow(/denied by policy/);
  });
});

describe("orders and their lines are never deleted", () => {
  it("refuses an order delete", async () => {
    await expect(db.order.deleteMany({})).rejects.toThrow(/denied by policy/);
  });

  it("refuses an order-item delete", async () => {
    await expect(db.orderItem.deleteMany({})).rejects.toThrow(
      /denied by policy/,
    );
  });
});

describe("the delivery log is append-only", () => {
  it("refuses rewriting a recorded event", async () => {
    await expect(
      db.webhookEvent.updateMany({ data: { type: "something.else" } }),
    ).rejects.toThrow(/denied by policy/);
  });

  it("refuses deleting a recorded event", async () => {
    await expect(db.webhookEvent.deleteMany({})).rejects.toThrow(
      /denied by policy/,
    );
  });
});
