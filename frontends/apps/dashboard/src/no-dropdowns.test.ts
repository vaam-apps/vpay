/**
 * No dropdown menus, anywhere in this app — a choice opens a bottom sheet.
 *
 * The owner's ruling (2026-09-13, on the product this dashboard's sign-in
 * follows): *"avoid drop down menus. We're a mobile-first app. Even on the
 * web. Use only bottom sheets."* A dropdown is a desktop control: it hides its
 * own options until opened, wants precise aiming, and renders differently on
 * every platform. A choice here is a sheet (`@vaam-apps/ui`'s `Drawer`, used
 * by `ManualKeySheet`), or a row of filter chips where the options are few
 * enough to show.
 *
 * This is a gate and not a checklist item for the reason its sibling is: the
 * rule was prose only until somebody found a menu on his own phone, written
 * after the ruling was recorded. It scans every `.ts` and `.tsx` file under
 * `app/` and `src/` for the control, not the word:
 *
 * * a native `<select>`, `<datalist>` or `<optgroup>`;
 * * `aria-haspopup`, and the `menu`, `menubar`, `listbox`, `combobox` and
 *   `menuitem*` roles, which are how a popup says what it is;
 * * an identifier from `@vaam-apps/ui` that opens a popup over its trigger —
 *   `Select*`, `DropdownMenu*`, `Popover*`, `CommandMenu*`, `DatePicker*`,
 *   `DateRangePicker*` — and a `Dropdown`/`PopupMenu` identifier of any origin;
 * * an import of a headless popup library by name (`Listbox`, `Combobox`,
 *   `Menu`, `Popover`, Radix's `select`/`dropdown-menu`/`popover`, Base UI's
 *   `select`/`menu`).
 *
 * Comments are stripped first, so a sentence explaining why a `<select>` was
 * removed does not fail the gate that keeps it removed.
 *
 * # The allow-list
 *
 * Keyed by file and by the one pattern it may name, each entry with its
 * reason, and an entry that matches nothing fails the scan — an allowance outliving what it allowed is how a gate rots.
 * The one entry is the payments filter's date range: `DatePickerContent` is a
 * popover at `sm` and above (a full-screen dialog below), it is not mounted by
 * any page (the read screens are a later ticket), and it is replaced by a
 * sheet when those screens return.
 */
import { describe, expect, it } from "vitest";

import { appSources, stripComments } from "./testing/sources";

/** What a popup control looks like in source. Each is a regex over stripped text. */
const PATTERNS: readonly { readonly name: string; readonly re: RegExp }[] = [
  { name: "a native <select>", re: /<select[\s>/]/i },
  { name: "a <datalist>", re: /<datalist[\s>/]/i },
  { name: "an <optgroup>", re: /<optgroup[\s>/]/i },
  { name: "aria-haspopup", re: /aria-haspopup/i },
  {
    name: "a menu, listbox or combobox role",
    re: /role=["'{`\s]*(menu|menubar|menuitem\w*|listbox|combobox)\b/i,
  },
  {
    name: "a popup component",
    re: /\b(Select\w*|DropdownMenu\w*|Popover\w*|CommandMenu\w*|DatePicker\w*|DateRangePicker\w*|Dropdown\w*|PopupMenu\w*)\b(?=[\s,}<(]|$)/,
  },
  {
    name: "a headless popup import",
    re: /\b(?:Listbox|Combobox|Menu|Popover)\b[^;]*from\s*["']@headlessui\/react["']|["']@radix-ui\/react-(?:select|dropdown-menu|popover)["']|["']@base-ui(?:-components)?\/react\/(?:select|menu|popover)["']/,
  },
];

/**
 * File -> the one kind of control it may name, and why.
 *
 * The allowance is for a **pattern**, not for the file: a `<select>` added to
 * an allow-listed file is still a `<select>`.
 */
const ALLOWED: Readonly<
  Record<string, { readonly pattern: string; readonly reason: string }>
> = {
  "src/components/payments-filters.tsx": {
    pattern: "a popup component",
    reason:
      "the date range is a popover at sm and above; no page mounts the payments filters until the read screens return, and a sheet replaces it then",
  },
};

/** Every pattern a text trips. */
export function violations(text: string): string[] {
  const stripped = stripComments(text);
  return PATTERNS.filter((pattern) => pattern.re.test(stripped)).map(
    (pattern) => pattern.name,
  );
}

/** The first pattern a text trips, or `null`. */
export function violation(text: string): string | null {
  return violations(text)[0] ?? null;
}

describe("this app has no dropdown", () => {
  const sources = appSources(/\.(ts|tsx)$/).filter(
    // This file and its sibling quote the patterns they scan for.
    (source) => !/src\/no-(dropdowns|shadows)\.test\.ts$/.test(source.path),
  );

  it("scans the whole source tree — not an empty glob", () => {
    expect(sources.length).toBeGreaterThan(40);
    expect(sources.some((source) => source.path === "app/page.tsx")).toBe(true);
    expect(
      sources.some(
        (source) => source.path === "src/components/one-question-steps.tsx",
      ),
    ).toBe(true);
  });

  it("finds no popup control outside the allow-list", () => {
    const found = sources.flatMap((source) =>
      violations(source.text)
        .filter((name) => ALLOWED[source.path]?.pattern !== name)
        .map((name) => `${source.path}: ${name}`),
    );
    expect(found).toEqual([]);
  });

  it("holds every allowance to a file that still needs it", () => {
    for (const [path, allowance] of Object.entries(ALLOWED)) {
      const source = sources.find((candidate) => candidate.path === path);
      expect(
        source,
        `${path} is allow-listed but does not exist`,
      ).toBeDefined();
      expect(
        violations(source?.text ?? ""),
        `${path} is allow-listed for "${allowance.pattern}" and no longer names it`,
      ).toContain(allowance.pattern);
    }
  });
});

describe("the scan itself", () => {
  it.each<[string, string]>([
    ["a native select", "<select name=x><option>a</option></select>"],
    ["a datalist", "<datalist id=x />"],
    ["aria-haspopup", '<button aria-haspopup="menu">x</button>'],
    ["a listbox role", '<ul role="listbox" />'],
    ["a combobox role", "<div role='combobox' />"],
    ["a menu role in braces", '<div role={"menu"} />'],
    ["a ui Select", 'import { Select, SelectTrigger } from "@vaam-apps/ui";'],
    ["a DropdownMenu", "<DropdownMenuContent />"],
    ["a Popover", "<PopoverTrigger />"],
    ["a DatePicker", "<DatePickerContent />"],
    ["a DateRangePicker", "<DateRangePicker value={v} />"],
    ["a Dropdown identifier", "const Dropdown = () => null;"],
    ["a Radix select import", 'import * as S from "@radix-ui/react-select";'],
    [
      "a headless Listbox import",
      'import { Listbox } from "@headlessui/react";',
    ],
  ])("trips on %s", (_name, text) => {
    expect(violation(text)).not.toBeNull();
  });

  it.each<[string, string]>([
    ["a sheet", "<MoreDetailDrawer open onOpenChange={set} title={t} />"],
    ["a button", "<Button type='button'>Choose</Button>"],
    ["a dialog role", '<div role="dialog" />'],
    [
      "a word in a line comment",
      "// the native <select> was removed\nconst x = 1;",
    ],
    ["a word in a block comment", "/* aria-haspopup is banned */ const x = 1;"],
    ["a word in a JSX comment", "<p>{/* <select> is banned */}</p>"],
    ["a plain word", "const selected = Selection.of(options);"],
    ["a URL", 'const url = "https://example.test/select";'],
  ])("does not trip on %s", (_name, text) => {
    expect(violation(text)).toBeNull();
  });
});
