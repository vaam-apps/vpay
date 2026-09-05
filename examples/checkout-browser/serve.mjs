#!/usr/bin/env node
/**
 * A zero-dependency static file server for this directory.
 *
 * Why not the dashboard container, and why not a package like `serve`:
 * `frontends/Dockerfile` deliberately does NOT copy `examples/` into its
 * build context (its own comment says so — the pnpm workspace glob is meant
 * to resolve to zero matches there), so wiring this page into the Next.js
 * dashboard would mean fighting that boundary rather than using it. Adding a
 * static-file-server package for two files and a vendored dependency
 * directory is more dependency than the job needs. `node:http` + `node:fs`
 * already do the whole job, with no dependency to audit or pin.
 *
 * This same module is what `frontends/tests/e2e/cypress.config.ts` imports
 * to serve the page for `checkout.cy.ts` — the Cypress run exercises exactly
 * the server a human runs by hand (`node serve.mjs`, or `pnpm --filter
 * @vpay-examples/checkout-browser serve`), not a second implementation.
 *
 * Serves this directory only, including `dist/stripe-js/` — the vendored
 * copy of `@vaam-apps/vpay-stripe-js`'s build output that `just build-checkout-browser`
 * produces (see the justfile). A missing `dist/stripe-js/index.js` is not an
 * error this server detects; the browser's own module-loading 404 is the
 * honest signal that the build step was skipped.
 */
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL(".", import.meta.url));

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
};

/**
 * Starts the server and resolves once it is actually listening. Returns the
 * `http.Server` so a caller (Cypress's `before:run`/`after:run`, or a signal
 * handler below) can close it.
 */
export function startCheckoutBrowserServer(port) {
  const server = createServer((req, res) => {
    void handle(req, res);
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, () => resolve(server));
  });
}

async function handle(req, res) {
  try {
    const requestUrl = new URL(req.url ?? "/", "http://localhost");
    const pathname =
      requestUrl.pathname === "/" ? "/index.html" : requestUrl.pathname;

    // Collapse `..` before joining, then re-check the join still lands
    // inside ROOT — belt and braces against a path-traversal request. This
    // page has no secrets of its own, but the habit belongs in every static
    // file server this repository ships, demo or not.
    const safe = normalize(pathname).replace(/^(\.\.(\/|\\|$))+/, "");
    const filePath = join(ROOT, safe);
    // `ROOT` (from `new URL(".", import.meta.url)`) already ends in `sep`.
    // `join(ROOT, "")` re-normalises it rather than trusting that, and
    // appending a SECOND `sep` (an earlier version of this line did
    // `ROOT + sep`) made every `startsWith` check fail — a 403-everything
    // server, found by hand running it for real (`GET /index.html` came back
    // 403 rather than the page).
    const root = join(ROOT, "");
    if (!filePath.startsWith(root)) {
      res.writeHead(403, { "content-type": "text/plain; charset=utf-8" });
      res.end("forbidden");
      return;
    }

    const body = await readFile(filePath);
    res.writeHead(200, {
      "content-type": MIME[extname(filePath)] ?? "application/octet-stream",
      // This page is served on a different origin than vpay-server
      // (`docs/flows/browser-checkout.md`), which is exactly what proves
      // `/v1/browser`'s CORS layer is doing real work rather than hiding a
      // same-origin call. Nothing here needs its own CORS header — the
      // browser only ever fetches vpay-server cross-origin, never this
      // server.
    });
    res.end(body);
  } catch (error) {
    if (
      error &&
      typeof error === "object" &&
      "code" in error &&
      error.code === "ENOENT"
    ) {
      res.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
      res.end("not found");
      return;
    }
    res.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
    res.end("internal error");
  }
}

// Run directly: `node serve.mjs` (also `pnpm --filter
// @vpay-examples/checkout-browser serve`). Guarded so importing
// `startCheckoutBrowserServer` from cypress.config.ts does not also start a
// second, unwanted listener.
if (import.meta.url === `file://${process.argv[1]}`) {
  const port = Number(process.env.CHECKOUT_BROWSER_PORT ?? 4180);
  const server = await startCheckoutBrowserServer(port);
  console.log(
    `checkout-browser: serving http://localhost:${port}/  (Ctrl+C to stop)`,
  );
  console.log(
    `checkout-browser: needs \`just build-checkout-browser\` run at least once — it vendors ` +
      `@vaam-apps/vpay-stripe-js's dist/ into dist/stripe-js/, which is gitignored and not built by this script.`,
  );
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.on(signal, () => server.close(() => process.exit(0)));
  }
}
