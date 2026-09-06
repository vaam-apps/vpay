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
import { readFileSync } from "node:fs";
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
