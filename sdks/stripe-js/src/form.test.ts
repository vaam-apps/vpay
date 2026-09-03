/**
 * Exact-bytes tests for the form encoder.
 *
 * Every assertion here is on a literal string, not on a re-parse: a test
 * that decoded the output with `URLSearchParams` and compared objects would
 * pass for an encoder that emitted `+` for spaces or percent-encoded the
 * structural brackets — neither of which vpay's axum form extractor reads
 * the same way.
 */
import { describe, expect, it } from "vitest";
import { encodeForm, FormEncodingError } from "./form.js";

describe("encodeForm", () => {
  it("encodes the browser confirm body exactly as the design specifies", () => {
    expect(
      encodeForm({
        key: "pk_test_abc",
        client_secret: "pi_1_secret_xyz",
        payment_method_data: {
          type: "mtn_momo",
          mtn_momo: { msisdn: "237690000000" },
        },
        return_url: "https://shop.example/thanks",
      }),
    ).toBe(
      "key=pk_test_abc&client_secret=pi_1_secret_xyz&" +
        "payment_method_data[type]=mtn_momo&" +
        "payment_method_data[mtn_momo][msisdn]=237690000000&" +
        "return_url=https%3A%2F%2Fshop.example%2Fthanks",
    );
  });

  it("leaves the structural brackets literal and percent-encodes each segment", () => {
    expect(encodeForm({ "a[b]": { "c d": "e" } })).toBe("a%5Bb%5D[c%20d]=e");
  });

  it("encodes a space as %20, not +", () => {
    expect(encodeForm({ note: "one two" })).toBe("note=one%20two");
  });

  it("indexes arrays", () => {
    expect(
      encodeForm({ payment_method_types: ["mtn_momo", "orange_money"] }),
    ).toBe(
      "payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money",
    );
  });

  it("omits undefined and null entirely, rather than sending an empty value", () => {
    expect(encodeForm({ a: "1", b: undefined, c: null, d: "2" })).toBe(
      "a=1&d=2",
    );
  });

  it("renders booleans as the words the extractor reads", () => {
    expect(encodeForm({ live: true, test: false })).toBe(
      "live=true&test=false",
    );
  });

  it("refuses a non-integer number rather than transmitting it approximately", () => {
    expect(() => encodeForm({ amount: 1.5 })).toThrow(FormEncodingError);
    expect(() => encodeForm({ amount: 1.5 })).toThrow(
      "amount must be a safe integer",
    );
  });

  it("refuses a non-plain object, which Object.entries would silently drop", () => {
    expect(() =>
      encodeForm({ when: new Date(0) as unknown as string }),
    ).toThrow(FormEncodingError);
  });

  it("refuses a value the wire format has no rendering for", () => {
    expect(() =>
      encodeForm({ weird: Symbol("x") as unknown as string }),
    ).toThrow("must be a string, number, boolean, array or plain object");
  });

  it("names the offending path in bracket syntax", () => {
    try {
      encodeForm({ payment_method_data: { mtn_momo: { msisdn: 1.5 } } });
      expect.unreachable("expected a FormEncodingError");
    } catch (err) {
      expect(err).toBeInstanceOf(FormEncodingError);
      expect((err as FormEncodingError).path).toBe(
        "payment_method_data[mtn_momo][msisdn]",
      );
    }
  });
});
