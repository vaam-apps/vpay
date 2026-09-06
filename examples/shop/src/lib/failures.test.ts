/**
 * The buyer-facing copy for vpay's failure vocabulary.
 *
 * The two things worth asserting are the two that decay: that the eleven
 * codes here are still the eleven `vpay_core::failure::FailureCode` has —
 * read out of the Rust source, not copied into a second list — and that
 * `retryable`, which is the only thing deciding whether this shop offers a
 * buyer a way to try again, still stands in the documented relation to
 * `payer_actionable` and `merchant_actionable`.
 *
 * Reading the Rust is the point. A hand-maintained expected list here would
 * agree with itself forever; this fails the day someone adds a twelfth code
 * to the core and does not write a sentence for it.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { FAILURE_COPY, UNKNOWN_FAILURE, failureCopy } from "./failures";

const RUST = readFileSync(
  fileURLToPath(
    new URL(
      "../../../../backends/crates/vpay-core/src/failure.rs",
      import.meta.url,
    ),
  ),
  "utf8",
);

/** `InsufficientFunds` → `insufficient_funds`, the spelling `/v1` puts on the wire. */
function snake(variant: string): string {
  return variant.replace(/(?<!^)([A-Z])/gu, "_$1").toLowerCase();
}

/** The variants named in `FailureCode::ALL`, in the order the core lists them. */
function coreCodes(): string[] {
  const start = RUST.indexOf("pub const ALL:");
  expect(start, "FailureCode::ALL moved").toBeGreaterThan(-1);
  const end = RUST.indexOf("];", start);
  const block = RUST.slice(start, end);
  return [...block.matchAll(/Self::(\w+)/gu)].map((match) =>
    snake(match[1] ?? ""),
  );
}

/** The variants inside one of the core's `matches!` predicates. */
function predicate(name: string): Set<string> {
  const start = RUST.indexOf(`pub const fn ${name}`);
  expect(start, `${name} moved`).toBeGreaterThan(-1);
  const end = RUST.indexOf("\n    }", start);
  const block = RUST.slice(start, end);
  return new Set(
    [...block.matchAll(/Self::(\w+)/gu)].map((match) => snake(match[1] ?? "")),
  );
}

describe("the copy table and vpay-core", () => {
  it("carries a sentence for every code the core defines, and no other", () => {
    const codes = coreCodes();
    expect(codes).toHaveLength(11);
    expect(Object.keys(FAILURE_COPY).sort()).toEqual([...codes].sort());
  });

  it("offers a retry everywhere FailureCode::payer_actionable does", () => {
    const actionable = predicate("payer_actionable");
    // Sanity on the parse itself: a regex that matched nothing would make
    // the loop below assert nothing at all, and pass.
    expect(actionable.size).toBe(4);
    for (const code of actionable) {
      expect(
        FAILURE_COPY[code as keyof typeof FAILURE_COPY]?.retryable,
        code,
      ).toBe(true);
    }
  });

  it("never offers a retry for a failure that is the merchant's to fix", () => {
    const merchant = predicate("merchant_actionable");
    expect(merchant.size).toBe(2);
    for (const code of merchant) {
      expect(
        FAILURE_COPY[code as keyof typeof FAILURE_COPY]?.retryable,
        code,
      ).toBe(false);
    }
  });

  it("adds exactly two rail-side codes to the payer-actionable set, and no more", () => {
    // The judgement this shop makes and the core does not, pinned so a third
    // code cannot join it quietly. `provider_unavailable` and
    // `provider_error` are not things the *payer* can do differently — but a
    // fresh order on a rail that has come back up succeeds, and the buyer is
    // the only person present to press the button.
    const actionable = predicate("payer_actionable");
    const extra = Object.entries(FAILURE_COPY)
      .filter(([code, copy]) => copy.retryable && !actionable.has(code))
      .map(([code]) => code)
      .sort();
    expect(extra).toEqual(["provider_error", "provider_unavailable"]);
  });

  it("writes every sentence for a buyer, with no rail vocabulary in it", () => {
    for (const [code, copy] of Object.entries(FAILURE_COPY)) {
      expect(copy.title.length, code).toBeGreaterThan(8);
      expect(copy.detail.length, code).toBeGreaterThan(20);
      // The rails' own strings belong under "for the runbook", never in the
      // sentence a buyer reads.
      expect(copy.detail, code).not.toMatch(/[A-Z]{4,}_[A-Z]/u);
    }
  });
});

describe("failureCopy", () => {
  it("answers the code's own copy", () => {
    expect(failureCopy("insufficient_funds")).toBe(
      FAILURE_COPY.insufficient_funds,
    );
  });

  it("answers the unknown copy for a null, an unknown code, or an inherited one", () => {
    for (const code of [null, "payer_confused", "constructor", "toString"]) {
      expect(failureCopy(code)).toBe(UNKNOWN_FAILURE);
    }
  });

  it("leaves an unreadable outcome retryable rather than declaring it final", () => {
    // A code this build predates is not something to call terminal: a fresh
    // order is cheap, and refusing one on an outcome nobody here can read
    // would be the shop guessing.
    expect(UNKNOWN_FAILURE.retryable).toBe(true);
  });
});
