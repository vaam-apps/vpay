import { describe, expect, it } from "vitest";
import { loadShopConfig, type EnvRecord } from "./config";

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
    expect(loadShopConfig(COMPLETE).paymentMethodTypes).toEqual([
      "mtn_momo",
      "orange_money",
    ]);
    expect(
      loadShopConfig({ ...COMPLETE, SHOP_PAYMENT_METHOD_TYPES: "orange_money" })
        .paymentMethodTypes,
    ).toEqual(["orange_money"]);
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
