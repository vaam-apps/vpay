/**
 * The test-number table, checked against `README.md` in **both** directions.
 *
 * Two copies of a table of fake numbers is exactly the kind of thing that is
 * true on the day it is written: the panel a buyer reads on `/checkout` comes
 * from `test-numbers.ts`, and a developer reads the README. If a mapping
 * changes and only one of them is updated, whichever the reader happened to
 * open is a lie.
 *
 * So this parses the README's own tables — the ones under the two rail
 * headings — and asserts that the number, the resulting order status, the
 * vpay failure code and the rail's own word match the module row for row and
 * in order, with no row on either side that the other does not have.
 *
 * The decisive check is the last one in each direction: delete a row from
 * either file and this fails naming it.
 */
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { TEST_NUMBERS, testNumbersFor, type RailCode } from "./test-numbers";
import { FAILURE_COPY } from "./failures";

const README = readFileSync(
  fileURLToPath(new URL("../../README.md", import.meta.url)),
  "utf8",
);

/** One README row, reduced to the four cells this table is about. */
interface DocRow {
  msisdn: string;
  orderStatus: string;
  failureCode: string;
  railReason: string;
}

/** Strips the backticks a code span is written with. */
function bare(cell: string): string {
  return cell.trim().replace(/^`|`$/gu, "");
}

/**
 * The rows of the markdown table that follows `#### … (\`{rail}\`)`.
 *
 * Anchored on the heading rather than on a comment marker, because the
 * heading is a thing a reader sees: a table moved out from under it is a
 * table a reader would no longer associate with the rail, and this should
 * fail then too.
 */
function docRows(rail: RailCode): DocRow[] {
  const heading = README.indexOf(`(\`${rail}\`)`);
  expect(heading, `README.md has no heading for ${rail}`).toBeGreaterThan(-1);
  const rows: DocRow[] = [];
  let started = false;
  for (const line of README.slice(heading).split("\n").slice(1)) {
    if (!line.startsWith("|")) {
      if (started) {
        break;
      }
      continue;
    }
    const cells = line.split("|").slice(1, -1);
    const first = bare(cells[0] ?? "");
    if (first.startsWith("---") || first === "Number") {
      started = true;
      continue;
    }
    started = true;
    rows.push({
      msisdn: first,
      orderStatus: bare(cells[2] ?? ""),
      failureCode: bare(cells[3] ?? ""),
      railReason: bare(cells[4] ?? ""),
    });
  }
  return rows;
}

describe("the README and the panel show the same numbers", () => {
  for (const entry of TEST_NUMBERS) {
    it(`agrees with the README for ${entry.rail}, row for row`, () => {
      const expected: DocRow[] = entry.numbers.map((number) => ({
        msisdn: number.msisdn,
        orderStatus: number.orderStatus,
        failureCode: number.failureCode ?? "—",
        railReason: number.railReason,
      }));
      // Whole arrays, in order: a comparison per row would pass a README
      // that had grown a sixth row nothing in the module knows about.
      expect(docRows(entry.rail)).toEqual(expected);
    });
  }
});

/**
 * Every MSISDN a WireMock mapping for `rail` actually keys on.
 *
 * Read out of the stubs' own JSON rather than grepped, and only from each
 * mapping's `request` — a number named in a `metadata.why` comment is
 * documentation, not steering, and a table that matched against prose would
 * pass for a mapping somebody had deleted. Leaves are filtered to the
 * `23760…` block so that a catch-all pattern (`.*`) cannot make the
 * assertion below vacuous.
 */
function steeringPatterns(rail: "mtn" | "orange"): string[] {
  const dir = fileURLToPath(
    new URL(
      `../../../../backends/tests/conformance/wiremock/${rail}/mappings/`,
      import.meta.url,
    ),
  );
  const found: string[] = [];
  const walk = (node: unknown): void => {
    if (typeof node === "string") {
      if (node.includes("23760")) {
        found.push(node);
      }
      return;
    }
    if (Array.isArray(node)) {
      node.forEach(walk);
      return;
    }
    if (typeof node === "object" && node !== null) {
      Object.values(node).forEach(walk);
    }
  };
  for (const file of readdirSync(dir).filter((name) =>
    name.endsWith(".json"),
  )) {
    const parsed: unknown = JSON.parse(readFileSync(dir + file, "utf8"));
    const mappings = (parsed as { mappings?: unknown[] }).mappings ?? [];
    for (const mapping of mappings) {
      walk((mapping as { request?: unknown }).request);
    }
  }
  return found;
}

describe("the stubs that honour the numbers", () => {
  // The README-vs-module check above proves two copies of a table agree. It
  // says nothing about the thing that actually produces the outcome, so a
  // number whose mapping had been deleted or renumbered would leave both
  // copies happily agreeing about a payment nobody can make.
  for (const [rail, dir] of [
    ["mtn_momo", "mtn"],
    ["orange_money", "orange"],
  ] as const) {
    it(`keys a ${rail} mapping on every number that is meant to fail`, () => {
      const patterns = steeringPatterns(dir);
      expect(
        patterns.length,
        `${dir} mappings name no MSISDN at all`,
      ).toBeGreaterThan(0);
      const entry = TEST_NUMBERS.find((candidate) => candidate.rail === rail);
      expect(entry, rail).toBeDefined();
      for (const number of entry?.numbers ?? []) {
        if (number.failureCode === null) {
          // The success row is deliberately NOT steered — "any number not
          // listed below does the same" is the stubs' catch-all.
          continue;
        }
        const steered = patterns.some(
          (pattern) =>
            pattern === number.msisdn ||
            pattern.includes(number.msisdn) ||
            new RegExp(`^(?:${pattern})$`, "u").test(number.msisdn),
        );
        expect(
          steered,
          `${rail} ${number.msisdn} is in no mapping's request`,
        ).toBe(true);
      }
    });
  }
});

/**
 * The `PRODUCED_FAILURE_CODES` an adapter declares, read out of its Rust
 * source.
 *
 * The same technique `failures.test.ts` uses on `vpay-core/src/failure.rs`,
 * and for the same reason: a hand-maintained expected list here would agree
 * with itself forever. This one is pointed at the *adapters*, because the
 * question this file asks is not "what codes exist" but "what codes can this
 * rail actually reach" — and those are different questions with different
 * answers per rail, which is the whole subject of issue #59.
 */
function producedCodes(crate: string): Set<string> {
  const source = readFileSync(
    fileURLToPath(
      new URL(
        `../../../../backends/crates/${crate}/src/mapping.rs`,
        import.meta.url,
      ),
    ),
    "utf8",
  );
  const start = source.indexOf("pub const PRODUCED_FAILURE_CODES");
  expect(start, `${crate} has no PRODUCED_FAILURE_CODES`).toBeGreaterThan(-1);
  const end = source.indexOf("];", start);
  expect(
    end,
    `${crate}'s PRODUCED_FAILURE_CODES is unterminated`,
  ).toBeGreaterThan(start);
  return new Set(
    [...source.slice(start, end).matchAll(/FailureCode::(\w+)/gu)].map(
      (match) =>
        // `InsufficientFunds` → `insufficient_funds`, the spelling `/v1` uses.
        (match[1] ?? "").replace(/(?<!^)([A-Z])/gu, "_$1").toLowerCase(),
    ),
  );
}

/** The crate behind each rail this table describes. */
const ADAPTER_CRATE: Readonly<Record<RailCode, string>> = {
  mtn_momo: "vpay-adapter-mtn-momo",
  orange_money: "vpay-adapter-orange-money",
};

describe("the panel promises only outcomes the rail can actually reach", () => {
  // The check that did not exist, and whose absence *is* issue #59. Two
  // copies of a table agreeing (above) proves they were copied from each
  // other; a table agreeing with the stubs proves a number is steered.
  // Neither says the outcome the number is steered to is one the rail's
  // adapter can produce — so this file was able to promise buyers an
  // outcome, and to explain at length why a *different* outcome was
  // impossible, while the impossible one was the one with copy written for
  // it.
  for (const entry of TEST_NUMBERS) {
    const crate = ADAPTER_CRATE[entry.rail];

    it(`only promises ${entry.rail} outcomes its adapter declares`, () => {
      const produced = producedCodes(crate);
      // Sanity on the parse: a regex that matched nothing would make every
      // assertion below vacuous, and pass.
      expect(produced.size, `${crate} declares no codes`).toBeGreaterThan(0);

      for (const number of entry.numbers) {
        if (number.failureCode === null) {
          continue;
        }
        expect(
          produced.has(number.failureCode),
          `${entry.rail} ${number.msisdn} promises ${number.failureCode}, which ` +
            `${crate}'s PRODUCED_FAILURE_CODES does not list`,
        ).toBe(true);
      }
    });

    it(`calls nothing unreachable on ${entry.rail} that its adapter can produce`, () => {
      const produced = producedCodes(crate);
      for (const gap of entry.cannotExpress) {
        // The direction that was wrong. `payer_declined` sat here for MTN
        // with a paragraph of justification, and MTN publishes
        // `PAYMENT_NOT_APPROVED`; the paragraph was about the adapter's
        // table, which is a thing this repository controls.
        expect(
          produced.has(gap.outcome),
          `${entry.rail} claims it cannot express ${gap.outcome}, but ` +
            `${crate}'s PRODUCED_FAILURE_CODES lists it`,
        ).toBe(false);
      }
    });
  }

  it("measures the two rails against each other, and finds them different", () => {
    // The single fact the panel exists to teach a merchant, pinned as a
    // number so that neither side can quietly drift to the other. MTN
    // reaches the whole taxonomy; Orange reaches three of it.
    expect(producedCodes(ADAPTER_CRATE.mtn_momo).size).toBe(11);
    expect(producedCodes(ADAPTER_CRATE.orange_money).size).toBe(3);
    expect([...producedCodes(ADAPTER_CRATE.orange_money)].sort()).toEqual([
      "payer_timeout",
      "provider_account_blocked",
      "provider_error",
    ]);
  });

  it("gives every rail a number for payer_declined, or says it cannot", () => {
    // The specific regression issue #59 is: a code with buyer copy, SDK
    // types and a place in the core, reachable from nothing. Either a rail
    // has a number that reaches it, or that rail says in `cannotExpress`
    // that it cannot — silence is what this repository had, and silence is
    // what reads as a promise.
    for (const entry of TEST_NUMBERS) {
      const promised = entry.numbers.some(
        (number) => number.failureCode === "payer_declined",
      );
      const disclaimed = entry.cannotExpress.some(
        (gap) => gap.outcome === "payer_declined",
      );
      expect(
        promised !== disclaimed,
        `${entry.rail}: payer_declined is ${promised ? "both promised and disclaimed" : "neither promised nor disclaimed"}`,
      ).toBe(true);
    }
  });
});

describe("the table itself", () => {
  it("names only failure codes the shop has copy for", () => {
    for (const entry of TEST_NUMBERS) {
      for (const number of entry.numbers) {
        if (number.failureCode !== null) {
          expect(
            Object.hasOwn(FAILURE_COPY, number.failureCode),
            `${entry.rail} ${number.msisdn} → ${number.failureCode}`,
          ).toBe(true);
        }
      }
      for (const gap of entry.cannotExpress) {
        // A gap must name a real code too: a row saying a rail cannot
        // produce `payer_confused` would be describing nothing.
        expect(Object.hasOwn(FAILURE_COPY, gap.outcome)).toBe(true);
      }
    }
  });

  it("gives every rail exactly one number per outcome, and one that succeeds", () => {
    for (const entry of TEST_NUMBERS) {
      const codes = entry.numbers.map((number) => number.failureCode);
      expect(new Set(codes).size, entry.rail).toBe(codes.length);
      expect(codes, entry.rail).toContain(null);
    }
  });

  it("makes every row that does not settle say why, in the panel and here", () => {
    for (const entry of TEST_NUMBERS) {
      for (const number of entry.numbers) {
        const settles =
          number.orderStatus === "paid" || number.orderStatus === "failed";
        // A row promising anything other than a settled order is a row about
        // a gap in vpay, and a gap with no sentence beside it is exactly the
        // "looks more finished than it is" this example exists against. The
        // converse is asserted too: a note on a row that settles normally
        // would be an explanation of nothing.
        expect(
          number.note !== undefined,
          `${entry.rail} ${number.msisdn} (${number.orderStatus})`,
        ).toBe(!settles);
      }
    }
  });

  it("uses documentation MSISDNs only — 237600000xxx, digits, twelve long", () => {
    for (const entry of TEST_NUMBERS) {
      for (const number of entry.numbers) {
        // Digits only, because vpay's checkout page refuses letters: the
        // hex-suffixed steering numbers (`237600000f01`) are unusable from a
        // form (`frontends/apps/checkout/src/lib/msisdn.ts`).
        expect(number.msisdn).toMatch(/^237600000\d{3}$/u);
        expect(number.msisdn).toHaveLength(12);
      }
    }
  });

  it("says nothing about a rail this deployment does not offer", () => {
    expect(testNumbersFor(["orange_money"]).map((e) => e.rail)).toEqual([
      "orange_money",
    ]);
    expect(testNumbersFor([])).toEqual([]);
  });
});
