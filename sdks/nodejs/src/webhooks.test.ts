import { createHmac } from "node:crypto";
import { describe, expect, it } from "vitest";
import { verifyWebhook } from "./webhooks.js";
import { WebhookSignatureError } from "./errors.js";

const SECRET = "whsec_test_secret";

function sign(timestamp: number, body: string, secret = SECRET): string {
  return createHmac("sha256", secret)
    .update(`${timestamp}.${body}`)
    .digest("hex");
}

const EVENT_BODY = JSON.stringify({
  id: "evt_123",
  object: "event",
  type: "payment_intent.succeeded",
  created: 1_700_000_000,
  livemode: false,
  data: { object: { id: "pi_123" } },
});

describe("verifyWebhook", () => {
  it("accepts a validly signed payload and returns the parsed event", () => {
    const now = 1_700_000_100;
    const header = `t=${now},v1=${sign(now, EVENT_BODY)}`;
    const event = verifyWebhook({
      rawBody: EVENT_BODY,
      signatureHeader: header,
      secret: SECRET,
      now,
    });
    expect(event.id).toBe("evt_123");
    expect(event.type).toBe("payment_intent.succeeded");
  });

  it("rejects a signature computed with the wrong secret", () => {
    const now = 1_700_000_100;
    const header = `t=${now},v1=${sign(now, EVENT_BODY, "whsec_wrong")}`;
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: header,
        secret: SECRET,
        now,
      }),
    ).toThrow(WebhookSignatureError);
  });

  it("rejects a timestamp outside the tolerance window", () => {
    const eventTime = 1_700_000_000;
    const now = eventTime + 301;
    const header = `t=${eventTime},v1=${sign(eventTime, EVENT_BODY)}`;
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: header,
        secret: SECRET,
        now,
      }),
    ).toThrow(WebhookSignatureError);
  });

  it("accepts a timestamp exactly at the tolerance boundary", () => {
    const eventTime = 1_700_000_000;
    const now = eventTime + 300;
    const header = `t=${eventTime},v1=${sign(eventTime, EVENT_BODY)}`;
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: header,
        secret: SECRET,
        now,
      }),
    ).not.toThrow();
  });

  it("respects a custom toleranceSeconds", () => {
    const eventTime = 1_700_000_000;
    const now = eventTime + 60;
    const header = `t=${eventTime},v1=${sign(eventTime, EVENT_BODY)}`;
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: header,
        secret: SECRET,
        now,
        toleranceSeconds: 30,
      }),
    ).toThrow(WebhookSignatureError);
  });

  it("accepts a payload when the second of two v1 signatures matches (secret rotation)", () => {
    const now = 1_700_000_100;
    const wrongSig = sign(now, EVENT_BODY, "whsec_old_wrong");
    const rightSig = sign(now, EVENT_BODY, SECRET);
    const header = `t=${now},v1=${wrongSig},v1=${rightSig}`;
    const event = verifyWebhook({
      rawBody: EVENT_BODY,
      signatureHeader: header,
      secret: SECRET,
      now,
    });
    expect(event.id).toBe("evt_123");
  });

  it("rejects a malformed header with no t=", () => {
    const now = 1_700_000_100;
    const header = `v1=${sign(now, EVENT_BODY)}`;
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: header,
        secret: SECRET,
        now,
      }),
    ).toThrow(WebhookSignatureError);
  });

  it("rejects a malformed header with no v1=", () => {
    const now = 1_700_000_100;
    const header = `t=${now}`;
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: header,
        secret: SECRET,
        now,
      }),
    ).toThrow(WebhookSignatureError);
  });

  it("rejects a completely garbled header", () => {
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: "not-a-valid-header",
        secret: SECRET,
      }),
    ).toThrow(WebhookSignatureError);
  });

  it("rejects when the body changes by even one byte", () => {
    const now = 1_700_000_100;
    const header = `t=${now},v1=${sign(now, EVENT_BODY)}`;
    const tamperedBody = EVENT_BODY.replace("pi_123", "pi_124");
    expect(() =>
      verifyWebhook({
        rawBody: tamperedBody,
        signatureHeader: header,
        secret: SECRET,
        now,
      }),
    ).toThrow(WebhookSignatureError);
  });

  it("verifies a Buffer body identically to its string form", () => {
    const now = 1_700_000_100;
    const header = `t=${now},v1=${sign(now, EVENT_BODY)}`;
    const fromString = verifyWebhook({
      rawBody: EVENT_BODY,
      signatureHeader: header,
      secret: SECRET,
      now,
    });
    const fromBuffer = verifyWebhook({
      rawBody: Buffer.from(EVENT_BODY, "utf8"),
      signatureHeader: header,
      secret: SECRET,
      now,
    });
    expect(fromBuffer).toEqual(fromString);
  });

  // Regression: the HMAC was computed over `${Number(t)}.body`, not over the
  // literal `t` text from the header. docs/flows/merchant-auth.md is explicit
  // that the signed payload is the bytes `"<t>.<raw body>"` — any renderer in
  // between can only diverge from what the sender signed.
  it("signs over the literal t text, not its numeric re-rendering", () => {
    const timestampText = "01700000100";
    const now = 1_700_000_100;
    expect(Number(timestampText)).toBe(now);
    expect(String(Number(timestampText))).not.toBe(timestampText);

    const signature = createHmac("sha256", SECRET)
      .update(`${timestampText}.${EVENT_BODY}`)
      .digest("hex");
    const event = verifyWebhook({
      rawBody: EVENT_BODY,
      signatureHeader: `t=${timestampText},v1=${signature}`,
      secret: SECRET,
      now,
    });
    expect(event.id).toBe("evt_123");
  });

  it("rejects a signature computed over the numeric re-rendering of t", () => {
    const timestampText = "01700000100";
    const now = 1_700_000_100;
    const wrongSignature = createHmac("sha256", SECRET)
      .update(`${Number(timestampText)}.${EVENT_BODY}`)
      .digest("hex");
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: `t=${timestampText},v1=${wrongSignature}`,
        secret: SECRET,
        now,
      }),
    ).toThrow(WebhookSignatureError);
  });

  // A `t` that is not a run of decimal digits is a malformed header, not a
  // tolerance failure and not a signature mismatch. `Number()` reads
  // "1700000000.0" and "0x65566CC0" as perfectly good timestamps, and "" as 0.
  it("rejects a fractional t as malformed, even when signed over that literal", () => {
    const timestampText = "1700000000.0";
    const signature = createHmac("sha256", SECRET)
      .update(`${timestampText}.${EVENT_BODY}`)
      .digest("hex");
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: `t=${timestampText},v1=${signature}`,
        secret: SECRET,
        now: 1_700_000_000,
      }),
    ).toThrow(/malformed/);
  });

  it("rejects an empty t as malformed rather than as a 1970 timestamp", () => {
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: `t=,v1=${sign(1_700_000_100, EVENT_BODY)}`,
        secret: SECRET,
        now: 1_700_000_100,
      }),
    ).toThrow(/malformed/);
  });

  it("tolerates surrounding whitespace in t, which is header formatting and not part of the signed bytes", () => {
    const now = 1_700_000_100;
    const event = verifyWebhook({
      rawBody: EVENT_BODY,
      signatureHeader: `t= ${now} , v1=${sign(now, EVENT_BODY)}`,
      secret: SECRET,
      now,
    });
    expect(event.id).toBe("evt_123");
  });

  it("rejects a hexadecimal t as malformed", () => {
    // Number("0x65566CC0") === 1700162752, so this parsed cleanly before.
    const now = 1_700_162_752;
    expect(Number("0x65566CC0")).toBe(now);
    expect(() =>
      verifyWebhook({
        rawBody: EVENT_BODY,
        signatureHeader: `t=0x65566CC0,v1=${sign(now, EVENT_BODY)}`,
        secret: SECRET,
        now,
      }),
    ).toThrow(/malformed/);
  });

  it("rejects other Number()-friendly t forms as malformed", () => {
    for (const t of ["1e9", "+1700000100", "-1700000100", "Infinity"]) {
      expect(() =>
        verifyWebhook({
          rawBody: EVENT_BODY,
          signatureHeader: `t=${t},v1=${sign(1_700_000_100, EVENT_BODY)}`,
          secret: SECRET,
          now: 1_700_000_100,
        }),
      ).toThrow(/malformed/);
    }
  });
});
