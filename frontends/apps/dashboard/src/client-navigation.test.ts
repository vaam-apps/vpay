import { afterEach, describe, expect, it } from "vitest";

import { inAppNavigationTarget, type ClickLike } from "./client-navigation";

const ORIGIN = "http://localhost:3000";

function anchor(attrs: Record<string, string>): HTMLAnchorElement {
  const a = document.createElement("a");
  for (const [k, v] of Object.entries(attrs)) a.setAttribute(k, v);
  const icon = document.createElement("span");
  a.appendChild(icon);
  document.body.appendChild(a);
  return a;
}

function click(
  target: EventTarget | null,
  over: Partial<ClickLike> = {},
): ClickLike {
  return {
    button: 0,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    defaultPrevented: false,
    target,
    ...over,
  };
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("which rail clicks become client-side navigations", () => {
  it("takes an ordinary click on an in-app link, including one on its icon", () => {
    const a = anchor({ href: "/refunds" });
    expect(inAppNavigationTarget(click(a), ORIGIN)).toBe("/refunds");
    // The rail's links wrap an icon; the click lands on the child.
    expect(inAppNavigationTarget(click(a.firstChild), ORIGIN)).toBe("/refunds");
  });

  it("keeps the query and the hash", () => {
    const a = anchor({ href: "/payments?status=succeeded#top" });
    expect(inAppNavigationTarget(click(a), ORIGIN)).toBe(
      "/payments?status=succeeded#top",
    );
  });

  it("leaves every click a browser treats specially to the browser", () => {
    const a = anchor({ href: "/refunds" });
    for (const over of [
      { metaKey: true },
      { ctrlKey: true },
      { shiftKey: true },
      { altKey: true },
      { button: 1 },
      { defaultPrevented: true },
    ]) {
      expect(
        inAppNavigationTarget(click(a, over), ORIGIN),
        JSON.stringify(over),
      ).toBeNull();
    }
    expect(
      inAppNavigationTarget(
        click(anchor({ href: "/x", target: "_blank" })),
        ORIGIN,
      ),
    ).toBeNull();
    expect(
      inAppNavigationTarget(
        click(anchor({ href: "/x", download: "" })),
        ORIGIN,
      ),
    ).toBeNull();
  });

  it("never routes another origin through the app router", () => {
    const a = anchor({ href: "https://vpay.example/payments" });
    expect(inAppNavigationTarget(click(a), ORIGIN)).toBeNull();
  });

  it("ignores a click that is not on a link at all", () => {
    const div = document.createElement("div");
    document.body.appendChild(div);
    expect(inAppNavigationTarget(click(div), ORIGIN)).toBeNull();
    expect(inAppNavigationTarget(click(null), ORIGIN)).toBeNull();
  });
});
