import type { NextConfig } from "next";

/**
 * The `Referrer-Policy` every page of this shop is served with.
 *
 * Load-bearing for the **popup** integration, not hygiene (issue #65). vpay's
 * checkout page, running in the popup, decides which window it may report
 * completion to by matching `document.referrer` against the origins this
 * merchant registered (`checkout_origins`). It refuses rather than guesses
 * when the referrer is empty — so a shop served with `no-referrer` (or
 * `same-origin`, which strips a cross-origin referrer just as completely)
 * silently loses vpay's own `vpay:complete`, and the popup completes only
 * through the slower `success_url` fallback, after vpay's page has navigated
 * the popup back here.
 *
 * `strict-origin-when-cross-origin` sends vpay **only this shop's origin** —
 * never a path, so no order id or cart route leaves — which is exactly what
 * the match needs. It is the browsers' own default; it is stated here anyway
 * because a reverse proxy, a CDN rule or a security-header middleware added
 * in front of a shop is the usual way a stricter one arrives, and this file
 * is where the reason it must not is written down. `origin` would work too.
 * Note the `strict-` half: an `https` shop opening an `http` checkout sends
 * no referrer at all, so a deployment must not mix the two.
 */
export const REFERRER_POLICY = "strict-origin-when-cross-origin";

const config: NextConfig = {
  reactStrictMode: true,
  // A standalone server, so `examples/shop/Dockerfile`'s runtime stage copies
  // a self-contained `server.js` plus the node_modules it actually needs,
  // rather than the whole workspace.
  output: "standalone",
  // The workspace root, not `examples/shop`: without it Next infers the root
  // from the nearest lockfile and warns about the monorepo on every build.
  outputFileTracingRoot: new URL("../../", import.meta.url).pathname,
  // Every route, not only `/checkout` where the popup is opened today: a
  // policy scoped to one path would silently break the first other page
  // that opens a popup. See `REFERRER_POLICY`.
  headers: () =>
    Promise.resolve([
      {
        source: "/:path*",
        headers: [{ key: "Referrer-Policy", value: REFERRER_POLICY }],
      },
    ]),
};

export default config;
