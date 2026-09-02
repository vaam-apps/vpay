/**
 * Verifier for vpay's outbound webhook signature scheme — Stripe's own,
 * copied exactly (docs/flows/webhooks.md, docs/flows/merchant-auth.md's
 * "Webhook verification" section). Productises the hand-rolled version in
 * `examples/webhook-receiver/index.mjs`.
 */
import { createHmac, timingSafeEqual } from "node:crypto";
import { WebhookSignatureError } from "./errors.js";
import type { Event } from "./types.js";

const DEFAULT_TOLERANCE_SECONDS = 300;

export interface VerifyWebhookOptions {
  /** The exact bytes vpay sent. A parsed-and-reserialised body breaks the HMAC. */
  rawBody: string | Buffer;
  /** The `Vpay-Signature` header value: `t=<unix seconds>,v1=<hex>[,v1=<hex>...]`. */
  signatureHeader: string;
  /** The webhook endpoint's signing secret. */
  secret: string;
  /** Default 300 (5 minutes), per docs/flows/webhooks.md. */
  toleranceSeconds?: number | undefined;
  /** Unix seconds. Defaults to `Date.now() / 1000`, injectable for tests. */
  now?: number | undefined;
}

/**
 * The only `t` this verifier accepts: one or more decimal digits, nothing
 * else. `Number()` is far more permissive — it reads `0x65566CC0` as
 * 1700000000, `1700000000.0` as 1700000000 and `""` as 0 — which would turn
 * a malformed header into either a *tolerance* failure or a signature that is
 * computed over bytes the sender never signed.
 */
const TIMESTAMP_PATTERN = /^\d+$/;

function parseSignatureHeader(header: string): {
  /** The literal `t` text from the header — the bytes that were signed. */
  timestampText: string;
  /** The same value as a number, used only for the tolerance comparison. */
  timestamp: number;
  signatures: string[];
} {
  let timestampText: string | undefined;
  const signatures: string[] = [];

  for (const part of header.split(",")) {
    const separatorIndex = part.indexOf("=");
    if (separatorIndex === -1) {
      continue;
    }
    const key = part.slice(0, separatorIndex).trim();
    const value = part.slice(separatorIndex + 1).trim();
    if (key === "t") {
      timestampText = value;
    } else if (key === "v1" && value.length > 0) {
      signatures.push(value);
    }
  }

  if (timestampText === undefined || !TIMESTAMP_PATTERN.test(timestampText)) {
    throw new WebhookSignatureError(
      'malformed Vpay-Signature header: missing or invalid "t"',
    );
  }
  const timestamp = Number(timestampText);
  if (!Number.isFinite(timestamp)) {
    throw new WebhookSignatureError(
      'malformed Vpay-Signature header: missing or invalid "t"',
    );
  }
  if (signatures.length === 0) {
    throw new WebhookSignatureError(
      'malformed Vpay-Signature header: no "v1" signature present',
    );
  }

  return { timestampText, timestamp, signatures };
}

function constantTimeHexEquals(a: string, b: string): boolean {
  const bufferA = Buffer.from(a, "utf8");
  const bufferB = Buffer.from(b, "utf8");
  // Length check first: `timingSafeEqual` throws on a length mismatch
  // rather than returning false.
  return bufferA.length === bufferB.length && timingSafeEqual(bufferA, bufferB);
}

/**
 * Verifies a `Vpay-Signature` header against `rawBody` and returns the
 * parsed {@link Event} only once verification succeeds.
 *
 * @throws {WebhookSignatureError} on a malformed header, a timestamp outside
 *   `toleranceSeconds`, no matching signature, or a body that is not valid
 *   JSON once the signature checks out.
 */
export function verifyWebhook(options: VerifyWebhookOptions): Event {
  const tolerance = options.toleranceSeconds ?? DEFAULT_TOLERANCE_SECONDS;
  const now = options.now ?? Math.floor(Date.now() / 1000);

  const { timestampText, timestamp, signatures } = parseSignatureHeader(
    options.signatureHeader,
  );

  if (Math.abs(now - timestamp) > tolerance) {
    throw new WebhookSignatureError(
      `webhook timestamp ${timestamp} is outside the ${tolerance}s tolerance (now=${now})`,
    );
  }

  const rawBodyBuffer =
    typeof options.rawBody === "string"
      ? Buffer.from(options.rawBody, "utf8")
      : options.rawBody;
  // The signed payload is the literal bytes `"<t>.<raw body>"`
  // (docs/flows/merchant-auth.md). `timestampText`, not `timestamp`: the
  // number is a lossy re-rendering of the header text, and any difference
  // between the two — a leading zero, say — computes an HMAC over bytes the
  // sender never signed, rejecting a genuine delivery.
  const signedPayload = Buffer.concat([
    Buffer.from(`${timestampText}.`, "utf8"),
    rawBodyBuffer,
  ]);
  const expectedSignature = createHmac("sha256", options.secret)
    .update(signedPayload)
    .digest("hex");

  const matched = signatures.some((candidate) =>
    constantTimeHexEquals(candidate, expectedSignature),
  );
  if (!matched) {
    throw new WebhookSignatureError(
      'no "v1" signature in the header matches the computed HMAC',
    );
  }

  try {
    return JSON.parse(rawBodyBuffer.toString("utf8")) as Event;
  } catch (err) {
    throw new WebhookSignatureError(
      "signature verified but the body is not valid JSON",
      { cause: err },
    );
  }
}
