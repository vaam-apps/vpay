/**
 * Starts and stops `examples/checkout-browser/serve.mjs` around the Cypress
 * run, for `checkout.cy.ts`.
 *
 * A CHILD PROCESS (`node serve.mjs`) rather than an imported function,
 * deliberately: that script is plain JavaScript with no `.d.ts`, and this
 * package's `tsconfig.json` has no `allowJs` (`tsconfig.base.json` is shared
 * across the whole workspace — widening it here to import one script was not
 * worth doing). Spawning it is also the more honest proof: it is the literal
 * command `examples/checkout-browser/README.md` step 5 tells a human to run,
 * not a second code path that happens to serve the same files.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

// frontends/tests/e2e/cypress/tasks -> repo root is four levels up.
const repoRoot = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
  "..",
  "..",
);
const checkoutBrowserDir = join(repoRoot, "examples", "checkout-browser");

export const CHECKOUT_BROWSER_PORT =
  process.env["CHECKOUT_BROWSER_PORT"] ?? "4180";
export const CHECKOUT_BROWSER_URL = `http://localhost:${CHECKOUT_BROWSER_PORT}`;

let child: ChildProcess | undefined;

async function waitUntilReady(deadlineMs: number): Promise<void> {
  const deadline = Date.now() + deadlineMs;
  let lastError: unknown;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${CHECKOUT_BROWSER_URL}/index.html`);
      if (res.ok) return;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(
    `checkout.cy.ts: examples/checkout-browser's server never answered on ${CHECKOUT_BROWSER_URL} ` +
      `within ${deadlineMs}ms. Last error: ${String(lastError)}`,
  );
}

export async function startCheckoutBrowserServer(): Promise<void> {
  if (child) return; // idempotent — `before:run` should only fire once, but don't double-spawn if it doesn't.
  child = spawn(process.execPath, ["serve.mjs"], {
    cwd: checkoutBrowserDir,
    env: { ...process.env, CHECKOUT_BROWSER_PORT },
    stdio: "inherit",
  });
  child.once("exit", (code) => {
    if (code !== null && code !== 0) {
      // eslint has no say here — this is a dev-time diagnostic, not
      // production logging, exactly like the rest of this Cypress config.
      console.error(`checkout-browser server exited early with code ${code}`);
    }
  });
  await waitUntilReady(15_000);
}

export function stopCheckoutBrowserServer(): void {
  child?.kill();
  child = undefined;
}
