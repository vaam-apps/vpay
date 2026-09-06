/**
 * Page memory: what this browser, on this device, may remember between
 * payments.
 *
 * **Two values and no more** — the mobile-money number a payer last paid
 * with, and the rail they last chose. Both are conveniences for re-typing;
 * neither authorises anything. The record never leaves the device: it is not
 * sent to vpay, not put in a URL, not `postMessage`d, and not read by
 * anything on the server. `secrets.test.ts`' credential trace covers the
 * page it lives on.
 *
 * **Opt-in, per device, and only ever written on a deliberate act.** There
 * is exactly one moment a record is written: a payer ticks the box and
 * submits an entry screen. Nothing is stored by merely visiting the page,
 * choosing a rail, or reaching an outcome. Unticking the box on a later
 * payment clears the record, and so does the explicit "forget" control.
 *
 * **The honest cost.** A phone that is shared — a household handset, a
 * borrowed device, a phone shop's demo unit — hands the number to whoever
 * uses it next, and the copy survives closing the tab. The checkbox says so
 * in the payer's own language rather than in this comment
 * (`memory.warning`).
 *
 * **There is no PIN here, and that is deliberate**: this page has no PIN
 * field to recall one into. MTN's push flow is approved on the handset, and
 * Orange's is approved on Orange's own page; neither vpay's browser API nor
 * `ProviderAdapter` accepts a PIN. See `docs/flows/hosted-checkout.md`,
 * "What is not built".
 *
 * This module is pure. {@link PageMemory} is the port; `memory-idb.ts` is
 * the IndexedDB adapter behind it.
 */
import { normalizeCameroonMsisdn } from './msisdn';


/** What the page may remember. Every member is optional in meaning, `null` in shape. */
export interface PageMemoryRecord {
  /** Canonical `2376XXXXXXXX`, as `normalizeCameroonMsisdn` produced it — never raw input. */
  msisdn: string | null;
  /** A rail code the intent offered, e.g. `mtn_momo`. */
  rail: string | null;
  /** Unix milliseconds. What {@link MEMORY_MAX_AGE_MS} is measured against. */
  savedAt: number;
}

/**
 * Ninety days.
 *
 * A convenience nobody has used in three months is a phone number sitting on
 * a device for no reason. The horizon is enforced on **read** rather than by
 * a sweep — there is no process here to run one — so an expired record is
 * never returned and is cleared the next time anything writes.
 */
export const MEMORY_MAX_AGE_MS = 90 * 24 * 60 * 60 * 1000;

/**
 * The store, as the page uses it.
 *
 * Async because IndexedDB is. `read` answering `null` covers every way a
 * store can be unavailable — a private window, a browser with storage
 * disabled, a quota refusal — and none of them is an error a payer should
 * see: the form simply starts empty.
 */
export interface PageMemory {
  read(): Promise<PageMemoryRecord | null>;
  write(record: PageMemoryRecord): Promise<void>;
  clear(): Promise<void>;
}

/**
 * Whether a stored value is a number this page would itself have produced.
 *
 * `normalizeCameroonMsisdn` re-run on the stored string, rather than a
 * second regular expression: the rule for what a Cameroon MSISDN is lives in
 * one file, and a copy of it here would be a copy that could drift. It was
 * one, briefly — `/^237[0-9]{9}$/`, which accepted `237571234567`, a number
 * with no mobile prefix that the confirm would have refused and this page
 * would have prefilled.
 */
function isStoredMsisdn(value: string): boolean {
  return normalizeCameroonMsisdn(value) === value;
}

/** A rail code as the wire spells one. */
const STORED_RAIL = /^[a-z0-9_]{1,64}$/;

/**
 * Whatever came back out of the store → a record this page will act on, or
 * `null`.
 *
 * Defensive on purpose, and not as a formality: the value was written by
 * *some* version of this page, possibly an older one, possibly by a
 * different site on a shared origin in a browser with a bug, and possibly
 * corrupted. A number that is not a Cameroon MSISDN must never reach the
 * form, because a payer who does not look would send a push to it.
 *
 * `now` is a parameter rather than a `Date.now()` call so the horizon is a
 * test rather than a wait.
 */
export function parseMemoryRecord(value: unknown, now: number): PageMemoryRecord | null {
  if (typeof value !== 'object' || value === null) {
    return null;
  }
  const raw = value as Record<string, unknown>;
  const savedAt = raw['savedAt'];
  if (typeof savedAt !== 'number' || !Number.isFinite(savedAt)) {
    return null;
  }
  // A record from the future is a clock that moved, not a record to trust.
  if (savedAt > now || now - savedAt > MEMORY_MAX_AGE_MS) {
    return null;
  }
  const msisdn = raw['msisdn'];
  const rail = raw['rail'];
  const record: PageMemoryRecord = {
    msisdn: typeof msisdn === 'string' && isStoredMsisdn(msisdn) ? msisdn : null,
    rail: typeof rail === 'string' && STORED_RAIL.test(rail) ? rail : null,
    savedAt,
  };
  // A record that remembers nothing is the same as no record, and answering
  // `null` keeps the "is there anything to forget?" question one comparison.
  return record.msisdn === null && record.rail === null ? null : record;
}

/**
 * The record a submitted entry screen writes, or `null` when there is
 * nothing worth writing.
 *
 * `msisdn` is `null` for a redirect rail: Orange collects the number on its
 * own page, so this page never has one to remember.
 */
export function memoryRecordFor(
  input: { msisdn: string | null; rail: string | null },
  now: number,
): PageMemoryRecord | null {
  const msisdn =
    typeof input.msisdn === 'string' && isStoredMsisdn(input.msisdn) ? input.msisdn : null;
  const rail = typeof input.rail === 'string' && STORED_RAIL.test(input.rail) ? input.rail : null;
  if (msisdn === null && rail === null) {
    return null;
  }
  return { msisdn, rail, savedAt: now };
}

/**
 * A {@link PageMemory} that remembers nothing.
 *
 * What the page holds when `config.yaml` says `page_memory: false`, and when
 * the browser has no IndexedDB. **Not a test double**: it is the shipping
 * behaviour of a deployment that turned the feature off, and it is reachable
 * only because the component that would show the checkbox is not rendered
 * at all in that case. `verify-no-mocks` governs Rust; the reason this is
 * not one in spirit either is that it implements "off", not "pretend".
 */
export const NO_PAGE_MEMORY: PageMemory = Object.freeze({
  read: (): Promise<PageMemoryRecord | null> => Promise.resolve(null),
  write: (): Promise<void> => Promise.resolve(),
  clear: (): Promise<void> => Promise.resolve(),
});

/**
 * Whether this page offers to remember anything, and what it remembers with.
 *
 * Two independent reasons to answer "no", and both must produce the *same*
 * pair — a store that forgets **and** an offer that is not made:
 *
 * - `config.yaml` says `page_memory: false`. The operator turned it off, and
 *   a checkbox that still stored would make that a lie.
 * - the browser has no IndexedDB (`store === null`) — a private window with
 *   storage walled off, or a build with it disabled. Offering a checkbox
 *   that silently forgets is worse than not offering one.
 *
 * A pure function rather than two expressions inside the component, because
 * it is a policy and the component is wiring. It became one after a mutation
 * — `offered: true`, hard-coded in `checkout-client.tsx` — passed the whole
 * suite: the view's own tests covered `offered: false` and nothing at all
 * covered how that value was arrived at.
 */
export function pageMemoryFor(
  enabled: boolean,
  store: PageMemory | null,
): { memory: PageMemory; offered: boolean } {
  if (!enabled || store === null) {
    return { memory: NO_PAGE_MEMORY, offered: false };
  }
  return { memory: store, offered: true };
}
