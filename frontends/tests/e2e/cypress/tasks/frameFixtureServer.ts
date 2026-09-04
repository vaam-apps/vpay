/**
 * A page on an origin the merchant never registered, which frames vpay's
 * embedded checkout — the negative half of `shop-embedded.cy.ts`.
 *
 * It exists as its own tiny `node:http` server rather than as a route on the
 * shop, because the thing under test IS the origin: `shop-merchant`'s
 * `checkout_origins` names `http://localhost:{demo_shop_port}` and nothing
 * else, so a fixture served by the shop would be *allowed* to frame and
 * would prove the opposite of what it is for. A different port is a
 * different origin to a browser and to
 * `vpay_config::validate_checkout_origins` alike.
 *
 * The page takes the frame's `src` as a query parameter rather than building
 * one, so the spec can hand it the byte-identical URL `initEmbeddedCheckout`
 * would have used. Nothing here knows what a checkout session is.
 *
 * It also records every `message` event it receives, in `window.__vpayFrameMessages`,
 * so a spec can assert that the refused frame sends its would-be parent
 * nothing at all — the mirror image of the positive test, where the same
 * page sends `vpay:resize` and `vpay:complete` within seconds.
 */
import { createServer, type Server } from "node:http";

export const FRAME_FIXTURE_PORT =
  process.env["VPAY_E2E_FRAME_FIXTURE_PORT"] ?? "4181";
export const FRAME_FIXTURE_URL = `http://localhost:${FRAME_FIXTURE_PORT}`;

/**
 * The fixture page.
 *
 * No `Referrer-Policy` header and no `referrerpolicy` attribute: the
 * browser default (`strict-origin-when-cross-origin`) sends this page's
 * ORIGIN as the `Referer` of the frame's request, which is exactly what
 * `resolveParentOrigin` (`frontends/apps/checkout/src/lib/origins.ts`) reads
 * to decide whether it is allowed to be here. Stripping it would make the
 * page refuse for a second reason and the test would stop distinguishing
 * "this origin is not registered" from "this frame has no referrer at all".
 */
function page(src: string): string {
  const escaped = src
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
  return `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Not a registered checkout origin</title></head>
<body>
<h1 id="fixture-heading">A shop vpay has never heard of</h1>
<p>This page is served on an origin that is not in any merchant's
<code>checkout_origins</code>. The frame below asks vpay to render an embedded
checkout session it holds a real credential for.</p>
<iframe id="framed" title="Checkout" src="${escaped}"
        sandbox="allow-scripts allow-same-origin allow-forms"
        style="border:0;width:100%;height:600px;display:block"></iframe>
<script>
  window.__vpayFrameMessages = [];
  window.addEventListener('message', function (event) {
    window.__vpayFrameMessages.push({ origin: event.origin, data: event.data });
  });
</script>
</body>
</html>
`;
}

let server: Server | undefined;

export async function startFrameFixtureServer(): Promise<void> {
  if (server) return; // idempotent, like `checkoutBrowserServer`'s spawn guard.
  const created = createServer((request, response) => {
    const url = new URL(request.url ?? "/", FRAME_FIXTURE_URL);
    if (url.pathname === "/healthz") {
      response.writeHead(200, {
        "Content-Type": "text/plain; charset=utf-8",
        "Cache-Control": "no-store",
      });
      response.end("ok");
      return;
    }
    if (url.pathname !== "/frame") {
      response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
      response.end("not found");
      return;
    }
    const src = url.searchParams.get("src");
    if (src === null || src.length === 0) {
      response.writeHead(400, { "Content-Type": "text/plain; charset=utf-8" });
      response.end("frame fixture: ?src= is required");
      return;
    }
    response.writeHead(200, {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
    });
    response.end(page(src));
  });
  await new Promise<void>((resolve, reject) => {
    created.once("error", reject);
    created.listen(Number(FRAME_FIXTURE_PORT), "127.0.0.1", () => {
      created.removeListener("error", reject);
      resolve();
    });
  });
  server = created;
}

export function stopFrameFixtureServer(): void {
  server?.close();
  server = undefined;
}
