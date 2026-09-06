import { describe, expect, it } from "vitest";
import {
  allSelectedRails,
  loadShopConfig,
  railsForCurrency,
  type EnvRecord,
} from "./config";

const COMPLETE: EnvRecord = {
  VPAY_API_URL: "http://vpay-server:8080",
  VPAY_CLIENT_ID: "shop-merchant",
  VPAY_PRIVATE_KEY_FILE: "/secrets/shop-merchant.pem",
  VPAY_PUBLISHABLE_KEY: "pk_test_shopmerchantsandbox1",
  VPAY_CHECKOUT_URL: "http://localhost:4200",
  VPAY_WEBHOOK_SECRET: "whsec_shop",
  SHOP_PUBLIC_URL: "http://localhost:3000/",
};

describe("loadShopConfig", () => {
  it("strips a trailing slash so a return URL never doubles it", () => {
    expect(loadShopConfig(COMPLETE).shopPublicUrl).toBe(
      "http://localhost:3000",
    );
  });

  it("defaults the browser API URL to the server one, and lets it differ", () => {
    expect(loadShopConfig(COMPLETE).vpayBrowserApiUrl).toBe(
      "http://vpay-server:8080",
    );
    expect(
      loadShopConfig({
        ...COMPLETE,
        VPAY_BROWSER_API_URL: "http://localhost:8080",
      }).vpayBrowserApiUrl,
    ).toBe("http://localhost:8080");
  });

  it("offers both rails by default and takes a narrower list", () => {
    expect(loadShopConfig(COMPLETE).rails).toEqual({
      kind: "all",
      rails: ["mtn_momo", "orange_money"],
    });
    expect(
      loadShopConfig({ ...COMPLETE, SHOP_PAYMENT_METHOD_TYPES: "orange_money" })
        .rails,
    ).toEqual({ kind: "all", rails: ["orange_money"] });
  });

  it("refuses a rail it does not know rather than passing it to vpay", () => {
    expect(() =>
      loadShopConfig({ ...COMPLETE, SHOP_PAYMENT_METHOD_TYPES: "bitcoin" }),
    ).toThrow(/rails this shop does not know: bitcoin/);
  });

  it("names the variable that is missing", () => {
    for (const key of Object.keys(COMPLETE)) {
      const partial = { ...COMPLETE };
      delete partial[key];
      expect(() => loadShopConfig(partial)).toThrow(new RegExp(key));
    }
  });

  it("refuses a URL that is not absolute http(s)", () => {
    expect(() =>
      loadShopConfig({ ...COMPLETE, SHOP_PUBLIC_URL: "not a url" }),
    ).toThrow(/must be an absolute URL/);
    // `new URL("localhost:3000")` parses — with `localhost:` as its
    // protocol — so the scheme check, not the parse, is what catches the
    // commonest mistake in this file.
    expect(() =>
      loadShopConfig({ ...COMPLETE, SHOP_PUBLIC_URL: "localhost:3000" }),
    ).toThrow(/must be http\(s\)/);
    expect(() =>
      loadShopConfig({ ...COMPLETE, VPAY_CHECKOUT_URL: "ftp://checkout" }),
    ).toThrow(/must be http\(s\)/);
  });

  it("defaults the checkout surface to the redirect, and takes all three", () => {
    expect(loadShopConfig(COMPLETE).checkoutMode).toBe("hosted");
    for (const mode of ["hosted", "embedded", "popup"]) {
      expect(
        loadShopConfig({ ...COMPLETE, SHOP_CHECKOUT_MODE: mode }).checkoutMode,
      ).toBe(mode);
    }
  });

  it("refuses a checkout surface it does not have", () => {
    expect(() =>
      loadShopConfig({ ...COMPLETE, SHOP_CHECKOUT_MODE: "lightbox" }),
    ).toThrow(/SHOP_CHECKOUT_MODE must be one of/);
  });

  it("never puts a secret in the message when a value is missing", () => {
    const partial = { ...COMPLETE };
    delete partial["VPAY_API_URL"];
    let message = "";
    try {
      loadShopConfig(partial);
    } catch (err) {
      message = err instanceof Error ? err.message : String(err);
    }
    expect(message).toContain("VPAY_API_URL");
    expect(message).not.toContain("whsec_shop");
    expect(message).not.toContain("/secrets/shop-merchant.pem");
  });
});

describe("SHOP_PAYMENT_METHOD_TYPES, per currency", () => {
  it("reads a per-currency map and answers per currency", () => {
    const config = loadShopConfig({
      ...COMPLETE,
      SHOP_PAYMENT_METHOD_TYPES: "xaf:orange_money;eur:mtn_momo",
    });
    expect(config.rails).toEqual({
      kind: "by_currency",
      byCurrency: { xaf: ["orange_money"], eur: ["mtn_momo"] },
    });
    expect(railsForCurrency(config.rails, "xaf")).toEqual(["orange_money"]);
    expect(railsForCurrency(config.rails, "EUR")).toEqual(["mtn_momo"]);
    // A currency the map does not name gets nothing, which `placeOrder`
    // turns into a refusal naming the currency — never a silent fallback to
    // "all rails", which is how a payer ends up on a rail that refuses them.
    expect(railsForCurrency(config.rails, "ngn")).toEqual([]);
  });

  it("applies a plain list to every currency", () => {
    const config = loadShopConfig({
      ...COMPLETE,
      SHOP_PAYMENT_METHOD_TYPES: "orange_money",
    });
    expect(railsForCurrency(config.rails, "xaf")).toEqual(["orange_money"]);
    expect(railsForCurrency(config.rails, "eur")).toEqual(["orange_money"]);
  });

  it("does not read an inherited property for an exotic currency code", () => {
    const config = loadShopConfig({
      ...COMPLETE,
      SHOP_PAYMENT_METHOD_TYPES: "xaf:orange_money",
    });
    expect(railsForCurrency(config.rails, "constructor")).toEqual([]);
    expect(railsForCurrency(config.rails, "toString")).toEqual([]);
  });

  it("refuses a group that names a currency twice", () => {
    expect(() =>
      loadShopConfig({
        ...COMPLETE,
        SHOP_PAYMENT_METHOD_TYPES: "xaf:orange_money;xaf:mtn_momo",
      }),
    ).toThrow(/names xaf twice/);
  });

  it("refuses an unknown rail inside a per-currency group", () => {
    expect(() =>
      loadShopConfig({
        ...COMPLETE,
        SHOP_PAYMENT_METHOD_TYPES: "xaf:bitcoin",
      }),
    ).toThrow(/does not know for xaf: bitcoin/);
  });

  it("enumerates every rail named anywhere, for the test-numbers panel", () => {
    const config = loadShopConfig({
      ...COMPLETE,
      SHOP_PAYMENT_METHOD_TYPES: "xaf:orange_money;eur:mtn_momo",
    });
    expect(allSelectedRails(config.rails).sort()).toEqual([
      "mtn_momo",
      "orange_money",
    ]);
  });
});
