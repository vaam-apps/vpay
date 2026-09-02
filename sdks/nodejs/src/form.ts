/**
 * Stripe's bracket-nested `application/x-www-form-urlencoded` encoding
 * (docs/flows/merchant-auth.md, "Encoding" table), used for every request
 * body and — identically — every `GET` query string.
 *
 * | Shape | Wire form |
 * |---|---|
 * | scalar | `amount=5000` |
 * | nested object | `metadata[order_id]=1234` |
 * | array | `payment_method_types[0]=mtn_momo&payment_method_types[1]=…` (indexed) |
 * | boolean | `true` / `false` |
 * | `undefined` | omitted entirely |
 *
 * A key is carried through the flattener as an **array of path segments**, and
 * only assembled into wire syntax at the very end. The brackets a nested key
 * is written with are therefore structural — produced by this encoder — while
 * every segment, including one a merchant supplied containing `[` or `]`, is
 * percent-encoded via `encodeURIComponent`. (An earlier version re-parsed the
 * assembled key with a bracket regex, which silently corrupted a merchant key
 * such as `metadata: { "a[b]": "x" }` into `metadata[b]=x`.)
 *
 * Two things are refused rather than silently mangled, because both would
 * otherwise reach a payment API as something other than what the caller wrote:
 *
 * - a value that is an object but not a plain one (a `Date`, a `Map`, a class
 *   instance) — `Object.entries` on it yields no own enumerable keys, so it
 *   would vanish from the request entirely;
 * - a number that is not a non-negative-or-negative *safe* integer, or whose
 *   `String()` form carries an exponent (`1e21` → `1e+21`). vpay's wire format
 *   is decimal integers with no separators (docs/flows/money.md bans floating
 *   point in the money path outright), so a float or a `1e21` is a bug in the
 *   caller, not a value to transmit approximately.
 */

/** A value the form encoder can flatten. `null` is treated like `undefined`: omitted. */
export type FormValue =
  | string
  | number
  | boolean
  | undefined
  | null
  | readonly FormValue[]
  | { [key: string]: FormValue };

/** Renders a path back into the bracket syntax, for error messages only. */
function describePath(path: readonly string[]): string {
  const [head, ...rest] = path;
  return `${head ?? ""}${rest.map((segment) => `[${segment}]`).join("")}`;
}

/**
 * True only for `{}`-shaped values: an object literal, or one created with a
 * null prototype. A `Date`, `Map`, `URL` or class instance is not one. Nor,
 * deliberately, is a plain object from another realm (`node:vm`,
 * `worker_threads`) — its prototype is that realm's `Object.prototype`, not
 * this one's — so structured data crossing a realm boundary must be
 * re-created (e.g. via `JSON.parse(JSON.stringify(..))`) before it is
 * encoded, rather than silently accepted on one side and refused on the
 * other.
 */
function isPlainObject(value: object): boolean {
  const prototype: unknown = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

/** Renders a number for the wire, refusing anything that is not an exact decimal integer. */
function encodeNumber(value: number, path: readonly string[]): string {
  if (!Number.isSafeInteger(value)) {
    throw new TypeError(
      `${describePath(path)} must be a safe integer, got ${value}`,
    );
  }
  const text = String(value);
  // Subsumed by the check above as the language stands today — JavaScript
  // renders every integer below 1e21 without an exponent, and 1e21 is far
  // past `Number.MAX_SAFE_INTEGER`, so no safe integer reaches here with an
  // `e` in it. Kept as an independent guard on the one property the wire
  // format actually requires: no test can kill this branch on its own, and
  // the comment says so rather than implying it is covered.
  if (text.includes("e") || text.includes("E")) {
    throw new TypeError(
      `${describePath(path)} must encode as a plain decimal integer, got ${text}`,
    );
  }
  return text;
}

function flattenValue(
  path: readonly string[],
  value: FormValue,
  out: Array<[readonly string[], string]>,
): void {
  if (value === undefined || value === null) {
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item: FormValue, index: number) =>
      flattenValue([...path, String(index)], item, out),
    );
    return;
  }
  if (typeof value === "object") {
    if (!isPlainObject(value)) {
      throw new TypeError(
        `${describePath(path)} must be a plain object, array or scalar, got ${
          (value as object).constructor?.name ?? "an object"
        }`,
      );
    }
    for (const [childKey, childValue] of Object.entries(value)) {
      flattenValue([...path, childKey], childValue, out);
    }
    return;
  }
  if (typeof value === "boolean") {
    out.push([path, value ? "true" : "false"]);
    return;
  }
  if (typeof value === "number") {
    out.push([path, encodeNumber(value, path)]);
    return;
  }
  out.push([path, value]);
}

/**
 * Flattens a params object into ordered `[path segments, value]` pairs.
 *
 * The key is still segments at this point, deliberately: assembling it into
 * `a[b][c]` before encoding is what made merchant-supplied brackets
 * ambiguous.
 */
export function flattenForm(
  params: Record<string, FormValue>,
): Array<[readonly string[], string]> {
  const out: Array<[readonly string[], string]> = [];
  for (const [key, value] of Object.entries(params)) {
    flattenValue([key], value, out);
  }
  return out;
}

/**
 * Assembles one flattened path into wire syntax: every segment percent-encoded,
 * the brackets between them literal.
 */
function encodePath(path: readonly string[]): string {
  const [head, ...rest] = path;
  const encodedRest = rest
    .map((segment) => `[${encodeURIComponent(segment)}]`)
    .join("");
  return `${encodeURIComponent(head ?? "")}${encodedRest}`;
}

/** Encodes `params` as an `application/x-www-form-urlencoded` body (or query string). */
export function encodeForm(params: Record<string, FormValue>): string {
  return flattenForm(params)
    .map(([path, value]) => `${encodePath(path)}=${encodeURIComponent(value)}`)
    .join("&");
}
