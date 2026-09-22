import { describe, expect, it } from "vitest";

import { StopUrlSpec } from "./stop-url.js";

/** `fromConfigured` answers `null` for a URL it will not take; these all resolve. */
function spec(raw: string, sessionId = "cs_123"): StopUrlSpec {
  const parsed = StopUrlSpec.fromConfigured(raw, sessionId);
  if (parsed === null) {
    throw new Error("fixture did not parse");
  }
  return parsed;
}

describe("StopUrlSpec — substitution and matching (D2)", () => {
  it("{CHECKOUT_SESSION_ID} is substituted before a URL becomes a stop rule", () => {
    const rule = spec("https://shop.example/return/{CHECKOUT_SESSION_ID}");

    expect(rule.path).toBe("/return/cs_123");
    expect(rule.matches("https://shop.example/return/cs_123")).toBe(true);
    expect(
      rule.matches("https://shop.example/return/%7BCHECKOUT_SESSION_ID%7D"),
    ).toBe(false);
  });

  it("matching is scheme+host+port+path — query and fragment are ignored", () => {
    const rule = spec("https://shop.example/thanks");

    expect(
      rule.matches("https://shop.example/thanks?utm_source=vpay&x=1#top"),
    ).toBe(true);
  });

  it("a different path does not match", () => {
    const rule = spec("https://shop.example/thanks");

    expect(rule.matches("https://shop.example/thanks/")).toBe(false);
    expect(rule.matches("https://shop.example/other")).toBe(false);
  });

  it("a different host or scheme does not match", () => {
    const rule = spec("https://shop.example/thanks");

    expect(rule.matches("https://evil.example/thanks")).toBe(false);
    expect(rule.matches("http://shop.example/thanks")).toBe(false);
  });

  it("an implicit default port matches an explicit one and vice versa", () => {
    expect(
      spec("https://shop.example/thanks").matches(
        "https://shop.example:443/thanks",
      ),
    ).toBe(true);
    expect(
      spec("https://shop.example:443/thanks").matches(
        "https://shop.example/thanks",
      ),
    ).toBe(true);
    expect(spec("https://shop.example/thanks").port).toBe(443);
    expect(spec("http://shop.example/thanks").port).toBe(80);
  });

  it("a scheme with no port concept is normalised to 0 rather than guessed at", () => {
    // An app scheme is what a merchant's deep link actually looks like on
    // Android and iOS, and 443 would be a fabricated value.
    expect(spec("myshop://checkout/return").port).toBe(0);
  });

  it("a null configured URL yields no stop rule at all", () => {
    // A `null` `success_url`/`cancel_url` on an embedded session is
    // expected, not an error — refusing an embedded session is the
    // pre-flight's job, not this function's.
    expect(StopUrlSpec.fromConfigured(null, "cs_123")).toBeNull();
    expect(StopUrlSpec.fromConfigured(undefined, "cs_123")).toBeNull();
  });

  it("a relative or unparseable configured URL yields no stop rule at all", () => {
    expect(StopUrlSpec.fromConfigured("/return", "cs_123")).toBeNull();
    expect(StopUrlSpec.fromConfigured("not a url", "cs_123")).toBeNull();
  });

  it("a URL that will not parse never matches", () => {
    expect(spec("https://shop.example/thanks").matches("not a url")).toBe(
      false,
    );
  });

  it("toJSON carries the four keys the show wire spells, and nothing else", () => {
    expect(spec("https://shop.example/thanks?keep=out").toJSON()).toEqual({
      scheme: "https",
      host: "shop.example",
      port: 443,
      path: "/thanks",
    });
  });
});
