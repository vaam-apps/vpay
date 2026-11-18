/**
 * `railChoices` over the server's rail spec, and `railLabel`'s fallbacks.
 *
 * The flows end to end, against the browser stub, are in `controller.test.ts`
 * ("the choices are the server's rail spec…"); these pin the pure rules: what
 * an entry must ask for before this page can drive it, how the operator's
 * allow-list narrows, and what a payer reads when the page has no message.
 */
import { describe, expect, it } from "vitest";

import { translator } from "../i18n/index";
import { makeIntent } from "../testing/fixtures";
import { railChoices, railLabel, type SupportedRail } from "./rails";
import type { RailSpec } from "./types";

const MSISDN = {
  name: "msisdn",
  type: "phone",
  region: "CM",
  phone_type: "mobile",
  required: true,
};

function push(code: string, extra: Partial<RailSpec> = {}): RailSpec {
  return {
    code,
    flow: "push",
    label_key: `rail.${code}`,
    fields: [MSISDN],
    ...extra,
  };
}

const intent = makeIntent();

describe("railChoices over the server's spec", () => {
  it("drives a push that asks for a Cameroon number, and a redirect that asks for nothing", () => {
    const choices = railChoices(intent, null, [
      push("mtn_wallet", {
        display_name: { en: "MTN Mobile Money", fr: "MTN Mobile Money" },
      }),
      {
        code: "orange_money",
        flow: "redirect",
        label_key: "rail.orange_money",
        fields: [],
      },
    ]);
    expect(choices).toEqual({
      supported: [
        {
          code: "mtn_wallet",
          flow: "mobile_money_push",
          label: null,
          name: { en: "MTN Mobile Money", fr: "MTN Mobile Money" },
        },
        {
          code: "orange_money",
          flow: "redirect",
          label: "rail.orange_money",
          name: null,
        },
      ],
      unsupported: [],
    });
  });

  it("lists as unsupported what the page has no screen for, rather than dropping it (D9)", () => {
    const choices = railChoices(intent, null, [
      push("card", { fields: [{ name: "pan", type: "card_number" }] }),
      push("abroad", { fields: [{ ...MSISDN, region: "SN" }] }),
      push("two_fields", { fields: [MSISDN, { name: "pin", type: "text" }] }),
      { code: "qr", flow: "qr_code", label_key: "rail.qr", fields: [] },
      {
        code: "odd_redirect",
        flow: "redirect",
        label_key: "rail.x",
        fields: [MSISDN],
      },
    ]);
    expect(choices.supported).toEqual([]);
    expect(choices.unsupported).toEqual([
      "card",
      "abroad",
      "two_fields",
      "qr",
      "odd_redirect",
    ]);
  });

  it("is narrowed by the operator's allow-list, and only narrowed", () => {
    const choices = railChoices(
      intent,
      ["orange_wallet", "notchpay"],
      [push("mtn_wallet"), push("orange_wallet")],
    );
    expect(choices.supported.map((rail) => rail.code)).toEqual([
      "orange_wallet",
    ]);
    expect(choices.unsupported).toEqual(["mtn_wallet"]);
  });

  it("falls back to the page's own two rails only when the read had no spec", () => {
    const own = railChoices(
      makeIntent({ payment_method_types: ["mtn_momo", "notchpay"] }),
      null,
      null,
    );
    expect(own.supported.map((rail) => rail.code)).toEqual(["mtn_momo"]);
    expect(own.unsupported).toEqual(["notchpay"]);
    // An empty spec is the server saying "nothing", not "I am old".
    expect(railChoices(intent, null, [])).toEqual({
      supported: [],
      unsupported: [],
    });
  });
});

describe("railLabel", () => {
  const base: SupportedRail = {
    code: "notchpay",
    flow: "mobile_money_push",
    label: null,
    name: { en: "Mobile money (Notch Pay)", fr: "Mobile money via Notch Pay" },
  };

  it("uses the page's own message when it has one", () => {
    expect(
      railLabel({ ...base, label: "rail.mtn_momo" }, translator("fr")),
    ).toBe("MTN Mobile Money");
  });

  it("uses the server's name in the page's language, then the other one, then the code", () => {
    expect(railLabel(base, translator("en"))).toBe("Mobile money (Notch Pay)");
    expect(railLabel(base, translator("fr"))).toBe(
      "Mobile money via Notch Pay",
    );
    expect(
      railLabel({ ...base, name: { en: "Only English" } }, translator("fr")),
    ).toBe("Only English");
    expect(railLabel({ ...base, name: null }, translator("fr"))).toBe(
      "notchpay",
    );
  });
});
