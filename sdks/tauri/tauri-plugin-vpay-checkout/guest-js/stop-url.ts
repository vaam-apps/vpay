/**
 * D2's stop rules: the port of `StopUrlSpec` from
 * `sdks/flutter/vpay_checkout_flutter/lib/src/checkout_controller.dart`.
 *
 * A stop URL is **derived** from the session the pre-flight read, never
 * configured on this side. That is the whole point: the merchant already
 * told vpay its `success_url` and `cancel_url` when it created the session,
 * and a second copy here is a second thing that can be wrong.
 */
import type { CheckoutStopUrl } from "./types.js";

/** D2's substitution token, as `vpay_core` renders it into `success_url`. */
const SESSION_ID_TOKEN = "{CHECKOUT_SESSION_ID}";

/**
 * The port a URL of this scheme uses when it names none.
 *
 * `0` for anything that is not `http`/`https` — an app scheme
 * (`myshop://return`) has no port concept at all, and giving every such
 * scheme the same sentinel is what makes the comparison in
 * {@link StopUrlSpec.matches} total rather than special-cased.
 */
function defaultPortFor(scheme: string): number {
  if (scheme === "https") {
    return 443;
  }
  if (scheme === "http") {
    return 80;
  }
  return 0;
}

/** `new URL(raw)`, or `undefined` when `raw` is not an absolute URL. */
function tryParse(raw: string): URL | undefined {
  try {
    return new URL(raw);
  } catch {
    // Deliberately swallowed and deliberately not logged: `raw` is a URL
    // and this package does not put URLs in messages (D6). A `success_url`
    // that will not parse simply yields no stop rule, which is the same
    // answer a `null` one yields.
    return undefined;
  }
}

/**
 * One stop URL, normalised: scheme, host, port and path only.
 *
 * A class rather than a bare object because {@link matches} belongs with
 * the four fields it compares — the Android and iOS hosts each own their
 * own copy of this comparison, and the three must agree, so having exactly
 * one place on this side that spells it is worth a constructor.
 */
export class StopUrlSpec implements CheckoutStopUrl {
  readonly scheme: string;
  readonly host: string;
  readonly port: number;
  readonly path: string;

  private constructor(
    scheme: string,
    host: string,
    port: number,
    path: string,
  ) {
    this.scheme = scheme;
    this.host = host;
    this.port = port;
    this.path = path;
  }

  /**
   * Substitutes `{CHECKOUT_SESSION_ID}` (D2) and parses `raw` into a spec.
   *
   * Answers `null` when `raw` is absent, is not an absolute URL, or names
   * no host — a `null` `success_url`/`cancel_url` on an embedded session is
   * expected, not an error, and this function is not the place that refuses
   * an embedded session (the pre-flight is).
   */
  static fromConfigured(
    raw: string | null | undefined,
    sessionId: string,
  ): StopUrlSpec | null {
    if (raw === null || raw === undefined) {
      return null;
    }
    // `split`/`join` rather than a regex: `raw` is server-supplied and a
    // global replace is the whole requirement — the token can legitimately
    // appear more than once (a path segment and a query parameter, say).
    const substituted = raw.split(SESSION_ID_TOKEN).join(sessionId);
    const parsed = tryParse(substituted);
    if (parsed === undefined || parsed.hostname.length === 0) {
      return null;
    }
    return new StopUrlSpec(
      parsed.protocol.replace(":", ""),
      parsed.hostname,
      parsed.port === ""
        ? defaultPortFor(parsed.protocol.replace(":", ""))
        : Number(parsed.port),
      parsed.pathname,
    );
  }

  /**
   * `true` when `url`'s scheme, host, port and path match this spec.
   *
   * Query and fragment are ignored (D2), so a merchant's own tracking
   * parameters on `success_url` cannot break the match — and, far more
   * importantly, so that nothing about the *outcome* can be smuggled in
   * through one. A URL that does not parse does not match.
   */
  matches(url: string | URL): boolean {
    const parsed = typeof url === "string" ? tryParse(url) : url;
    if (parsed === undefined) {
      return false;
    }
    const scheme = parsed.protocol.replace(":", "");
    const port =
      parsed.port === "" ? defaultPortFor(scheme) : Number(parsed.port);
    return (
      scheme === this.scheme &&
      parsed.hostname === this.host &&
      port === this.port &&
      parsed.pathname === this.path
    );
  }

  /** The four keys the `show` wire carries. */
  toJSON(): CheckoutStopUrl {
    return {
      scheme: this.scheme,
      host: this.host,
      port: this.port,
      path: this.path,
    };
  }
}
