"use client";

import { PAYMENT_STATUS } from "@vpay/tokens";
import {
  Button,
  DateRangePicker,
  DatePickerClear,
  DatePickerContent,
  DatePickerTrigger,
  DatePickerValue,
  FormField,
  type IsoDateRange,
} from "@vaam-apps/ui";
import { useRouter } from "next/navigation";
import { useState } from "react";

/** What the URL currently asks for. */
export interface PaymentsFilterValues {
  /** One `IntentStatus` wire value, or `''` for every status. */
  status: string;
  /** `YYYY-MM-DD`, inclusive lower bound on `created`, or `''`. */
  createdFrom: string;
  /** `YYYY-MM-DD`, inclusive upper bound, or `''`. */
  createdTo: string;
}

export interface PaymentsFiltersProps {
  values: PaymentsFilterValues;
}

/** The "every status" choice. An empty value, so it drops out of the query. */
const ANY_STATUS = "";

/**
 * `PaymentsFilterValues`' two dates, in `DateRangePicker`'s own shape.
 *
 * Two "unset" spellings meet here: this app's URL uses `''` (what
 * `payments-query.ts`'s `one()` returns for a missing parameter), the picker
 * uses `undefined`. An all-`''` pair becomes `undefined` outright rather than
 * `{ from: undefined, to: undefined }` — the picker's own `hasDate` check
 * (`date-picker.js` in 0.3.0; a `hasValue` flag in 0.2.x) treats those the
 * same for display, but only the `undefined` form is what a freshly-mounted,
 * filter-free screen should hand it.
 */
function rangeFromValues(
  values: PaymentsFilterValues,
): IsoDateRange | undefined {
  if (values.createdFrom === "" && values.createdTo === "") {
    return undefined;
  }
  return {
    from: values.createdFrom === "" ? undefined : values.createdFrom,
    to: values.createdTo === "" ? undefined : values.createdTo,
  };
}

/**
 * The status and date-range filters above the table: one row wherever it
 * fits, wrapping onto more lines where it does not.
 *
 * **Was a plain `method="get"` `<form>`** — a screenshot from the e2e run
 * showed it four rows tall, one control per line, and the redesign asked for
 * one row instead. That form shape is also the reason this is no longer it:
 * `@vaam-apps/ui`'s `DateRangePicker` is a controlled
 * component (`value`/`onValueChange`, `date-picker.d.ts`), not a form field
 * with a `name`, so it cannot sit inside a `<form method="get">` the way the
 * two `<input type="date">`s it replaces did. The dashboard already renders
 * this screen through Refine as a client component reading
 * `useSearchParams()` (`payments-screen.tsx`), so the fix is the one that
 * screen already takes: hold the picks in state, and push a built URL on
 * Apply. The URL stays the source of truth either way — a filtered list is
 * still bookmarkable and pasteable into a ticket, and `PaymentsScreen` still
 * reads only `searchParams`, never a prop this component hands it directly.
 *
 * **The status control is still a native `<select id="payments-filter-status">`,
 * not `@vaam-apps/ui`'s `Select`.** That `Select` wraps a Headless UI
 * `Listbox` and forwards no `name` and no ref a plain `onChange` can read
 * without one — the native element is simply the one shape this file already
 * knew how to read reliably, and nothing about moving to client state changed
 * that. The status list is `PAYMENT_STATUS` from `@vpay/tokens` — the same
 * five values `../payment-status.ts` renders as a pill — so a status can
 * never be filterable but unrenderable, or the other way round. vpay answers
 * `400` naming `status` for a value outside its own vocabulary rather than an
 * empty list, which is the right answer and one this form cannot provoke.
 * Its `h-10` is for the date field beside it: see "The two labelled fields
 * line up" below.
 *
 * **Both fields carry a visible label (2026-09-24).** The date range sits in
 * a `FormField` labelled "Created between", as Status does beside it: the
 * label's `htmlFor` is `DatePickerTrigger`'s `id` (`payments-filter-created`),
 * and the trigger has no `aria-label` — the `<label for>` names it. The dates
 * stay its description: the trigger points `aria-describedby` at
 * `DatePickerValue`'s span once it finds a label pointing at it, so a screen
 * reader hears "Created between, button, 2026-09-01 → 2026-09-07". Read off
 * Chromium's own accessibility tree in the built Storybook: the name
 * "Created between", from `labelfor`; the description the dates, or
 * "Any time" while empty.
 *
 * The placeholder is "Any time", this field's "Any status": what an unset
 * filter means. It was "Created between", the label's own words, and the
 * shown text is the description: under the `aria-label` the empty trigger
 * was already "Created between, button, Created between" (measured on
 * `c562564b` in jsdom), and under a visible label the words would also have
 * been drawn twice.
 *
 * Clicking the label focuses the trigger and opens nothing, which is what
 * clicking "Status" does to the select. That is `FormField`'s `Label`, not
 * the browser: it is Headless UI's, which cancels the native label click (on
 * a `<button>` that would be a click, and would open the picker) and focuses
 * the control instead; Enter then opens it. Measured in the built
 * Storybook, and held in jsdom by `payments-filters.test.tsx`.
 *
 * What holds each part, measured by mutation: dropping the trigger's `id`,
 * or pointing `htmlFor` anywhere else, fails the four
 * `payments-filters.test.tsx` cases that find the field by name, and both
 * `FiltersWithRange` stories; putting `aria-label="Created between"` back in
 * the label's place fails only the case asserting that a `<label>` names it,
 * because the two names read the same to `getByRole`; changing or dropping
 * the placeholder fails the "Any time" case and the `FiltersWithRange`
 * story, which clears the range and reads the description; and dropping
 * the `id` and the placeholder together also makes axe's `button-name`
 * fire, in `a11y.test.tsx` and in the `Filters` and `FiltersLight` stories.
 *
 * _History, kept because each step was a decision._ Until 0.3.0 the picker
 * took no `id`, so a `<label htmlFor>` would have pointed at nothing, and it
 * named itself by repeating `placeholder` in a `sr-only` span:
 * `"Created between Created between"` empty and `"2026-09-01 → 2026-09-07
 * Created between"` with a range, read off a real render
 * (`docs/status/verification/2026-09-12-dashboard-refine.md`). The 0.3.0
 * migration kept the row without a visible label and named the trigger with
 * `aria-label="Created between"`, the package's rule for a picker outside a
 * `FormField`, because adopting a label changed what the row shows — a
 * decision above this file. Its cost was written here: once a range was
 * picked, an operator returning to a bookmarked filtered URL saw two bare
 * dates beside a labelled Status field. The maintainer took that decision on
 * vaam-apps/vpay#258's review; the label above is it.
 *
 * **The range is committed on Save, not per click (0.3.0).** Tapping days
 * only stages a pick; `onValueChange` — `setRange` here — fires once, when
 * the operator presses Save, and Cancel, Escape or a press outside throw the
 * pick away. The range rule is M3's: the first tap sets a start with an open
 * end, a tap on or after it sets the end, and a tap on a whole range starts
 * a new one. Two consequences an operator will notice against 0.2.x, which
 * committed every click and used `react-day-picker`'s rule: a single tap
 * then Save is `{ from, to: undefined }`, which `apply()` sends as
 * `created_from` alone — "from that day on", where 0.2.x's first click was a
 * one-day range; and a new start no longer needs Clear first, because a tap
 * on a whole range starts a new one rather than extending it. While the
 * picker is open the rest of the page is inert, so Apply beside an open
 * picker takes two presses: the first only closes it.
 *
 * **The two labelled fields line up.** The row is `items-end`, so the
 * controls' bottom edges meet, and the labels meet only if the controls are
 * the same height. They were not: the native `<select>` renders 24 px tall
 * with nothing on it and `DatePickerTrigger`'s `.input` 40 px, so "Status"
 * sat 16 px below "Created between" (label tops at y=32 and y=16, measured).
 * `h-10` gives the select the trigger's 40 px: on one line, both labels at
 * y=16, both controls and Apply at y=42–82, and a 40 px target for the
 * select where it had 24 px. That ties it to the field height at the
 * package's default, compact density, which this app never changes.
 *
 * **The date field is as wide as its longest value, and no wider** (unless
 * the column itself is narrower: see the 314 px paragraph below). Inside
 * a `FormField` it no longer fills the row (see the history below): the
 * `FormField` is a flex item sized to its content, so with no width of its
 * own the field hugs whatever it shows — 104.6 px empty, 187.6 px from an
 * open start, 261.8 px with a full range — and Apply jumps up to 157 px
 * sideways each time a range is picked or cleared.
 * `w-[calc(23ch+16px+3.5rem)]` fixes it at the longest of those: the 23
 * characters of `YYYY-MM-DD → YYYY-MM-DD` in the value's mono face, plus the
 * trigger's chrome in the units the package draws it in — 16 px for the two
 * 1 px borders and the 14 px calendar icon, and 3.5 rem for the 0.75 rem
 * left padding, the icon's 0.5 rem gap and the 2.25 rem the Clear button's
 * tap target claims on the right (`DatePickerTrigger`'s own
 * `pr-[calc(2.25rem+…)]`, at compact density). At the browser's default
 * 16 px root that is 72 px of chrome.
 *
 * **Pixels and rems apart, not one `72px`, because they scale differently
 * (found on review, 2026-09-24).** A browser's default font size (Chrome's
 * Settings → Appearance → Font size: "Large" is 20 px, "Very large" 24 px)
 * scales every rem in that chrome and nothing else — the value's
 * `text-prose` is a fixed 14 px, so `ch` does not move. Written as `72px`,
 * the field cut the range short at every window width once the root was
 * 20 px (a 179.9 px slot for 188.8 px of text) or 24 px (165.9 px for
 * 187.8 px). Split, the slot stays 193.9 px at both and the range fits,
 * and at a 16 px root nothing changes. Measured in the built Storybook,
 * headless Chromium, with the default font size set through CDP's
 * `Page.setFontSizes`. Page zoom needs none of this: it scales pixels and
 * rems alike, so a 200 % zoom of a 1280 px window lays out as the 640 px
 * window below.
 *
 * `ch` and not pixels, because this app ships no mono face: `--font-mono`
 * is a stack (JetBrains Mono, `ui-monospace`, SF Mono, Cascadia Mono, Menlo,
 * `monospace`) with no `@font-face` behind it, so the face is whatever the
 * operator's machine resolves, and `ch` is measured in that face. The
 * `font-mono text-prose` beside the width is there only so that `ch` is the
 * value's own face at the value's own 14 px: `className` lands on the
 * trigger's wrapper `div`, the button sets its own `font-sans text-prose`,
 * and Clear, which does inherit the two, draws only a 14 px icon — every
 * box inside the wrapper measured the same with and without them.
 *
 * Measured in the built Storybook, headless Chromium, 2026-09-24, where the
 * face resolves to DejaVu Sans Mono (`CSS.getPlatformFontsForNode`): `ch`
 * is 8.42 px, so the field is 265.9 px; a full range needs 261.8 px of it
 * (189.8 px of text), and at 261 px it truncates. The 4 px over is the
 * root's `-0.011em` tracking (`theme.css`'s `html` rule, -0.176 px a glyph),
 * which `ch` does not see. It is kept rather than subtracted, because it is
 * also what absorbs a face without its own `→`: Ubuntu Mono draws that one
 * glyph from DejaVu and still fits (a 252.3 px field, 248.9 px needed).
 * Liberation Mono, Noto Mono and FreeMono: a 265.2 px field, ~261.2 px
 * needed. On 0.2.x the trigger was ~262 px, so this is the width the row
 * had before 0.3.0.
 *
 * The `FiltersWithRange` story checks all of this in a real browser, and
 * each of these mutations fails it on its own: the width class removed
 * (cleared, the field shrinks to 104.6 px), the width at `20ch` (190 px of
 * text in a 169 px slot), `font-mono text-prose` removed (`ch` then comes
 * from the sans face: 20.7 px unused), and the select's `h-10` removed (the
 * labels 16 px apart). The `20ch` mutation fails `FiltersWithRangePhone`
 * too. The story also sets the root to 20 px and checks the range is still
 * whole, which the single `72px` fails. The wrap's and the cap's own
 * mutations are below.
 *
 * **One line where it fits, wrapped where it does not (2026-09-24).** The
 * row is `flex-wrap` at every width and never scrolls: Status, the date
 * field and Apply keep their own widths, share a line while there is room,
 * and move to the next line when there is not. So Apply can never be hidden
 * behind a sideways scroll, and no breakpoint has to track the app shell's
 * geometry. The redesign's "one row, not four" still holds wherever the row
 * fits. The maintainer decided this on vaam-apps/vpay#258's review, after a
 * wrap below `sm` alone had left the row scrolling, with Apply hidden, in
 * 640–725 px windows (see the history below).
 *
 * Measured in the built Storybook, headless Chromium, 2026-09-24, with the
 * payments screen composed inside `AppShell`, whose content column is the
 * window less 48 px below 640 px and less 144 px from 640 px up: one line at
 * 726, 1024 and 1280 px; two lines at 700 and 725 px (Apply wraps) and at
 * 640 px (the date field and Apply wrap together, in a 496 px column); three
 * at 320 and 375 px. At every one of those widths nothing ends past the
 * row's edge and neither the row nor the document scrolls sideways, and a
 * range picked and saved from the wrapped row reaches the field.
 *
 * Which control goes down first at 640 px depends on the sans face, which
 * this app does not ship either (`--font-sans` resolves through
 * `system-ui`): the select is as wide as `requires_payment_method` in it —
 * 219 px in Noto Sans, the face above, and the date field misses the first
 * line by under a pixel; 207 px in Liberation Sans or 211 px in Ubuntu, and
 * the date field stays up beside Status with only Apply wrapping. So the
 * stories assert that Apply wraps in that 496 px column, which holds in
 * every face measured (with the narrowest, Cantarell, one line still needs
 * ~550 px), and not which line the date field lands on.
 *
 * **Below a 314 px window the date field is wider than the column, and it
 * shrinks to fit and truncates rather than push the page sideways** — one
 * field cut short is better than a whole screen that scrolls (the
 * maintainer's call, same review). Without a cap, at 280 px its 265.9 px
 * overflowed the 232 px column and the document scrolled 10 px, a spill the
 * old scrolling row had contained. `max-w-full` caps the trigger at its
 * `FormField`, and the `FormField` needs its own `max-w-full` to be capped
 * at the row: measured, the trigger's alone changes nothing, because the
 * `FormField` is a flex item and will not shrink below its content's
 * 265.9 px. With both, at 280 px the field is 232 px, its value ends in an
 * ellipsis, the dates stay whole in its description, and the document stays
 * at 280 px; at 320 px and up the field is its full 265.9 px, unchanged.
 * `FiltersWithRangePhone` checks the 232 px column, and dropping either
 * `max-w-full` fails it.
 *
 * **Status is capped the same way, for the larger default font sizes above
 * (found on review, 2026-09-24).** At a 16 px root its widest option makes
 * the select 219 px, narrower than the column of any window down to 280 px,
 * so nothing changes there. But its text is sized from the root, and at a
 * 20 px root the select is 268 px: in a 280 px window (a 220 px column, the
 * shell's gutter being rem too) that overflowed the column and scrolled the
 * document 18 px sideways (298 px of `scrollWidth`); at a 24 px root,
 * 318 px, it overflowed even a 375 px window's 303 px column.
 * `max-w-full` on its `FormField` keeps it inside the column. The select
 * needs none of its own, unlike the date trigger: it has no width of its
 * own, so as the `FormField`'s flex-column child it stretches to the
 * `FormField`'s width, capped or not, where the trigger's fixed width does
 * not stretch (measured: the select's own `max-w-full` changed nothing). A
 * native select clips its own shown option, and "Any status" fits either
 * way. `FiltersWithRangePhone` repeats the 232 px check at a 20 px root,
 * and dropping this `max-w-full` fails it.
 *
 * Mutations, each run on its own: the row back to the scrolling line
 * (`flex-nowrap` with `overflow-x-auto`) fails `FiltersWithRange`, which
 * squeezed to that 496 px column finds Apply still on the first line, and
 * `FiltersWithRangePhone` (582 px of row in a 375 px box); wrapping below
 * `sm` only fails `FiltersWithRange` the same way; and always stacked
 * (`flex-col`) fails `FiltersWithRange` at 1280 px, where the date field
 * sits 78 px below Status instead of beside it.
 *
 * _History: the row scrolled sideways until 2026-09-24._ `flex` with no
 * `flex-wrap` guaranteed the controls never went to a second row — but on
 * 0.2.x nothing here shrank, and that was measured rather than reasoned:
 * rendered against this app's own compiled `globals.css` in a real browser,
 * the three controls were 211 px, 262 px and 71 px at *every* container
 * width from 1104 px down to 320 px. The `<select>`'s intrinsic minimum is
 * its widest option (`requires_payment_method`), and the picker's
 * `truncate` never engaged because its trigger sat in a block wrapper the
 * row could not squeeze. So below ~568 px of content width the row simply
 * spilled, and — being a direct child of `ScreenStack`, whose own `min-w-0`
 * protects the column and not its children — it took the whole document
 * into horizontal scroll with it. That is the failure `ScreenStack`'s doc
 * names in as many words, and `PaymentsTable` is already wrapped against it
 * the same way. `min-w-0` let the row be narrower than its contents, and
 * `overflow-x-auto` kept the spill inside the row's own box: at 375 px the
 * document's `scrollWidth` equalled its `clientWidth`. Measured again with
 * both fields labelled and the date field at its fixed width: at 375 px the
 * row's content was 582 px in a 343 px box (327 px inside the app shell),
 * the date field starting 231 px in with its label cut off. For one commit
 * the row then wrapped below `sm` only (`max-sm:flex-wrap`, with
 * `overflow-x-auto` shortened to the equivalent `overflow-auto` to fit
 * `verify-ui`'s 60-character class budget), which fixed phones and left the
 * one line scrolling in the shell's 640–725 px windows, where the column is
 * narrower than the row's 582 px — at 640 px the date field's end and Apply
 * were off-screen.
 *
 * _History: 0.3.0's width, before the label._ 0.3.0 removed the block
 * wrapper 0.2.x put around the trigger, so `DatePickerTrigger`'s own
 * `inline-flex w-full` box became the row's flex item and took whatever the
 * row had left: 932 px in a 1248 px row, 352 px in a 668 px row, and at
 * 375 px ~158 px empty before the row spilled anyway (the built Storybook
 * `Filters` story, 2026-09-24). The `FormField` around it ended that, and
 * the width above replaced it.
 *
 * Safe with the calendar, checked and not assumed. The row is no longer a
 * scroll container, but the reasoning that made the picker safe inside one
 * holds for any ancestor. 0.2.x portalled its panel (`PopoverPanel`'s
 * `anchor` forced `portal`), so it was not a descendant of the row at all.
 * 0.3.0 renders every picker surface inline, as a descendant, but
 * `position: fixed` (the docked panel placed under the trigger by Floating
 * UI from 640 px up, the full-screen range picker below), and a fixed box is
 * not clipped by an `overflow` ancestor unless something between them has a
 * `transform`, `filter`, `backdrop-filter`, `contain` or `will-change` —
 * nothing here or in `ScreenStack` does. Measured: at 1280 px the docked
 * panel is 714×398 px under a 50 px-tall row, and its Save button, ~370 px
 * below the row's bottom edge, took a real click. Re-measured with the
 * labels (2026-09-24, the payments screen composed inside `AppShell` at
 * 1280×800): the row is 66 px tall, the panel is still 714×398 px, 4 px
 * under the trigger, and its Save button, ~357 px below the row's bottom
 * edge, took a real click; at 375×812 the full-screen picker covers the
 * viewport (375×812) and its Save took one too.
 *
 * Applying always starts from the first page: the built URL never carries
 * `after`/`before`, because `starting_after`/`ending_before` name a row in
 * the *previous* result set, and carrying one across a filter change would
 * ask for "the page after a payment that is no longer in this list".
 */
export function PaymentsFilters({ values }: PaymentsFiltersProps) {
  const router = useRouter();
  // Local state, seeded once from `values` — the same "reads the URL once,
  // on mount" contract the `<input defaultValue>`s this replaces already
  // had. A filter change elsewhere (the pager, a pasted URL) remounts this
  // screen's data, not this component, so re-seeding on every render is not
  // a gap this change introduces.
  const [status, setStatus] = useState(values.status);
  const [range, setRange] = useState<IsoDateRange | undefined>(
    rangeFromValues(values),
  );

  function apply() {
    const search = new URLSearchParams();
    if (status.length > 0) {
      search.set("status", status);
    }
    if (range?.from !== undefined) {
      search.set("created_from", range.from);
    }
    if (range?.to !== undefined) {
      search.set("created_to", range.to);
    }
    const query = search.toString();
    router.push(query.length > 0 ? `/payments?${query}` : "/payments");
  }

  return (
    <div className="flex min-w-0 flex-wrap items-end gap-3">
      <FormField
        label="Status"
        htmlFor="payments-filter-status"
        className="max-w-full"
      >
        <select
          id="payments-filter-status"
          name="status"
          className="h-10"
          value={status}
          onChange={(event) => setStatus(event.target.value)}
        >
          <option value={ANY_STATUS}>Any status</option>
          {PAYMENT_STATUS.map((paymentStatus) => (
            <option key={paymentStatus} value={paymentStatus}>
              {paymentStatus}
            </option>
          ))}
        </select>
      </FormField>

      <FormField
        label="Created between"
        htmlFor="payments-filter-created"
        className="max-w-full"
      >
        <DateRangePicker value={range} onValueChange={setRange}>
          <DatePickerTrigger
            id="payments-filter-created"
            className="w-[calc(23ch+16px+3.5rem)] max-w-full font-mono text-prose"
          >
            <DatePickerValue placeholder="Any time" />
            <DatePickerClear />
          </DatePickerTrigger>
          <DatePickerContent />
        </DateRangePicker>
      </FormField>

      <Button type="button" variant="secondary" onClick={apply}>
        Apply
      </Button>
    </div>
  );
}
