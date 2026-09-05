/**
 * Stripe's bracket-nested `application/x-www-form-urlencoded` encoding —
 * the same one `@vaam-apps/vpay-sdk`'s `src/form.ts` implements for `/v1`, because
 * `/v1/browser` is served by the same axum form extractor.
 *
 * | Shape | Wire form |
 * |---|---|
 * | scalar | `key=pk_test_x` |
 * | nested object | `payment_method_data[mtn_momo][msisdn]=237690000000` |
 * | array | `a[0]=x&a[1]=y` (indexed) |
 * | boolean | `true` / `false` |
 * | `undefined` / `null` | omitted entirely |
 *
 * A key is carried through the flattener as an **array of path segments**
 * and only assembled into wire syntax at the end, so the brackets a nested
 * key is written with are structural while every segment — including one a
 * merchant supplied containing `[` or `]` — is percent-encoded. Assembling
 * first and re-parsing with a bracket regex is what silently corrupted
 * `{ "a[b]": "x" }` into `a[b]=x`-meaning-something-else in the Node SDK
 * before that bug was fixed; this is the fixed version.
 *
 * The browser package differs from the Node one in a single respect: it is
 * fed `payment_method_data` typed `Record<string, unknown>`, i.e. whatever a
 * merchant's page passed. Anything the wire format cannot carry exactly
 * ({@link FormEncodingError}) is refused here so that
 * {@link import('./client.js').VpayStripe.confirmPayment} can answer with a
 * typed `invalid_request_error` naming the parameter, rather than sending an
 * approximation of what the caller wrote to a payment API.
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

/**
 * A value that cannot be encoded exactly. Thrown only by
 * {@link encodeForm}, and only ever caught — never surfaced to a merchant as
 * a rejection.
 */
export class FormEncodingError extends Error {
  /** The offending path in bracket syntax, e.g. `payment_method_data[mtn_momo]`. */
  readonly path: string;

  constructor(path: string, detail: string) {
    super(`${path} ${detail}`);
    this.name = "FormEncodingError";
    this.path = path;
  }
}

/** Renders a path back into bracket syntax, for error messages only. */
function describePath(path: readonly string[]): string {
  const [head, ...rest] = path;
  return `${head ?? ""}${rest.map((segment) => `[${segment}]`).join("")}`;
}

/**
 * True only for `{}`-shaped values: an object literal, or one created with a
 * null prototype. A `Date`, `Map`, `URL` or class instance is not one — and
 * must not be, because `Object.entries` on it yields no own enumerable keys,
 * so it would vanish from the request body entirely rather than fail.
 */
function isPlainObject(value: object): boolean {
  const prototype: unknown = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

/**
 * Renders a number, refusing anything that is not an exact decimal integer.
 *
 * vpay's wire format is decimal integers with no separators
 * (`docs/flows/money.md` bans floating point in the money path outright), so
 * a float, a `NaN`, or a value past `Number.MAX_SAFE_INTEGER` is a bug in the
 * caller rather than a value to transmit approximately.
 */
function encodeNumber(value: number, path: readonly string[]): string {
  if (!Number.isSafeInteger(value)) {
    throw new FormEncodingError(
      describePath(path),
      `must be a safe integer, got ${value}`,
    );
  }
  return String(value);
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
    (value as readonly FormValue[]).forEach((item, index) =>
      flattenValue([...path, String(index)], item, out),
    );
    return;
  }
  if (typeof value === "object") {
    if (!isPlainObject(value)) {
      throw new FormEncodingError(
        describePath(path),
        "must be a plain object, array or scalar",
      );
    }
    for (const [childKey, childValue] of Object.entries(
      value as { [key: string]: FormValue },
    )) {
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
  if (typeof value !== "string") {
    // Reachable only from an `unknown` a merchant passed through
    // `payment_method_data` — a `symbol`, a `bigint`, a function.
    throw new FormEncodingError(
      describePath(path),
      `must be a string, number, boolean, array or plain object, got ${typeof value}`,
    );
  }
  out.push([path, value]);
}

/** Flattens a params object into ordered `[path segments, value]` pairs. */
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
 * Assembles one flattened path into wire syntax: every segment
 * percent-encoded, the brackets between them literal.
 */
function encodePath(path: readonly string[]): string {
  const [head, ...rest] = path;
  const encodedRest = rest
    .map((segment) => `[${encodeURIComponent(segment)}]`)
    .join("");
  return `${encodeURIComponent(head ?? "")}${encodedRest}`;
}

/**
 * Encodes `params` as an `application/x-www-form-urlencoded` body — or,
 * identically, as a query string.
 *
 * `encodeURIComponent` and not `URLSearchParams`: the latter encodes a space
 * as `+` and leaves `!'()*~` alone, so two encoders in one codebase would
 * put different bytes on the wire for the same MSISDN-bearing body. This is
 * byte-for-byte what `@vaam-apps/vpay-sdk` sends.
 */
export function encodeForm(params: Record<string, FormValue>): string {
  return flattenForm(params)
    .map(([path, value]) => `${encodePath(path)}=${encodeURIComponent(value)}`)
    .join("&");
}
