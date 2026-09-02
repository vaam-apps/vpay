import { describe, expect, it } from "vitest";
import { encodeForm, type FormValue } from "./form.js";

describe("encodeForm", () => {
  it("encodes the pinned payment_intents.create example exactly", () => {
    const body = encodeForm({
      amount: 5000,
      currency: "xaf",
      payment_method_types: ["mtn_momo"],
      metadata: { order_id: "1234" },
    });
    expect(body).toBe(
      "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo&metadata[order_id]=1234",
    );
  });

  it("encodes nested objects with bracket notation", () => {
    const body = encodeForm({
      payment_method_data: {
        type: "mtn_momo",
        mtn_momo: { msisdn: "237670000000" },
      },
    });
    expect(body).toBe(
      "payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000",
    );
  });

  it("encodes arrays with numeric indices, in order", () => {
    const body = encodeForm({
      payment_method_types: ["mtn_momo", "orange_money"],
    });
    expect(body).toBe(
      "payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money",
    );
  });

  it("encodes booleans as the literal strings true/false", () => {
    expect(encodeForm({ livemode: true })).toBe("livemode=true");
    expect(encodeForm({ livemode: false })).toBe("livemode=false");
  });

  it("omits undefined fields entirely, including nested ones", () => {
    expect(encodeForm({ amount: 5000, description: undefined })).toBe(
      "amount=5000",
    );
    expect(encodeForm({ metadata: { order_id: "1", note: undefined } })).toBe(
      "metadata[order_id]=1",
    );
  });

  it("omits null fields entirely", () => {
    expect(encodeForm({ amount: 5000, description: null })).toBe("amount=5000");
  });

  it("percent-encodes spaces in values", () => {
    expect(encodeForm({ description: "order number one" })).toBe(
      "description=order%20number%20one",
    );
  });

  it("percent-encodes ampersands and equals signs in values", () => {
    expect(encodeForm({ description: "a&b=c" })).toBe("description=a%26b%3Dc");
  });

  it("percent-encodes unicode in values", () => {
    expect(encodeForm({ description: "café ☕" })).toBe(
      "description=caf%C3%A9%20%E2%98%95",
    );
  });

  it("percent-encodes unicode in nested object keys while keeping brackets literal", () => {
    const body = encodeForm({ metadata: { ключ: "value" } });
    expect(body).toBe("metadata[%D0%BA%D0%BB%D1%8E%D1%87]=value");
  });

  it("produces an empty string for an empty params object", () => {
    expect(encodeForm({})).toBe("");
  });
  it("preserves insertion order across scalars, arrays and objects", () => {
    const body = encodeForm({ b: "2", a: ["x", "y"], c: { z: "3" } });
    expect(body).toBe("b=2&a[0]=x&a[1]=y&c[z]=3");
  });

  // Regression: the key was previously assembled into `metadata[a[b]]` and
  // then re-parsed with a bracket regex, which read the merchant's own
  // brackets as structure and emitted `metadata[b]=x`.
  it("percent-encodes brackets a merchant put inside a key, rather than reading them as structure", () => {
    expect(encodeForm({ metadata: { "a[b]": "x" } })).toBe(
      "metadata[a%5Bb%5D]=x",
    );
    expect(encodeForm({ metadata: { "a]b": "x" } })).toBe("metadata[a%5Db]=x");
    expect(encodeForm({ "top[0]": "x" })).toBe("top%5B0%5D=x");
  });

  it("keeps a merchant bracket key distinct from the structure it imitates", () => {
    const imitation = encodeForm({ metadata: { "a[b]": "x" } });
    const structure = encodeForm({ metadata: { a: { b: "x" } } });
    expect(structure).toBe("metadata[a][b]=x");
    expect(imitation).not.toBe(structure);
  });

  it("throws TypeError on a Date rather than dropping it silently", () => {
    expect(() =>
      encodeForm({ created: new Date(0) as unknown as FormValue }),
    ).toThrow(TypeError);
  });

  it("throws TypeError on a Map or a class instance rather than dropping it silently", () => {
    class Widget {
      readonly kind = "widget";
    }
    expect(() =>
      encodeForm({ m: new Map([["a", "b"]]) as unknown as FormValue }),
    ).toThrow(TypeError);
    expect(() =>
      encodeForm({ w: new Widget() as unknown as FormValue }),
    ).toThrow(TypeError);
    expect(() =>
      encodeForm({ metadata: { when: new Date(0) as unknown as FormValue } }),
    ).toThrow(TypeError);
  });

  it("accepts a null-prototype object as a plain object", () => {
    const metadata = Object.assign(Object.create(null) as object, {
      order_id: "1234",
    }) as Record<string, string>;
    expect(encodeForm({ metadata })).toBe("metadata[order_id]=1234");
  });

  // Regression: `1e21` satisfies Number.isInteger and was emitted as
  // `amount=1e%2B21`, which is not a decimal integer in any parser.
  it("throws TypeError on a number that renders with an exponent", () => {
    expect(() => encodeForm({ amount: 1e21 })).toThrow(TypeError);
    expect(() => encodeForm({ amount: 1e21 })).toThrow(/1e21|1e\+21/);
  });

  it("throws TypeError on an unsafe integer or a non-integer", () => {
    expect(() => encodeForm({ amount: Number.MAX_SAFE_INTEGER + 1 })).toThrow(
      TypeError,
    );
    expect(() => encodeForm({ amount: 50.5 })).toThrow(TypeError);
    expect(() => encodeForm({ amount: NaN })).toThrow(TypeError);
    expect(() => encodeForm({ amount: Infinity })).toThrow(TypeError);
  });

  it("accepts the largest safe integer and a negative safe integer", () => {
    expect(encodeForm({ amount: Number.MAX_SAFE_INTEGER })).toBe(
      "amount=9007199254740991",
    );
    expect(encodeForm({ delta: -5 })).toBe("delta=-5");
  });
});
