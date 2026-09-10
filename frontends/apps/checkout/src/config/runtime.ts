/**
 * Reading the two mounted YAML files, once, at container start.
 *
 * This is the only file in `src/config/` that touches the filesystem or the
 * environment, and it is deliberately thin: everything that decides what a
 * document *means* is in `settings.ts`, where it is a unit test.
 *
 * **Once, at start.** The result is memoised at module scope, so the read
 * happens on the first import in the server process and never again. A
 * config change is a pod restart, exactly as it is for `vpay-server`
 * (ADR-0003: administration is YAML in git, and a running process does not
 * reload it). Two consequences worth stating: editing a mounted file under a
 * live container changes nothing until it is replaced, and a payer's request
 * never pays for a file read.
 *
 * **A missing file is a log line and defaults, never a blank page.** Every
 * problem — absent, unreadable, malformed, a key of the wrong type — is
 * printed once, at `warn`, naming the file. The page renders either way.
 * That is the trade this repository's `CLAUDE.md` asks for stated the other
 * way round: an operator who mounted nothing must be able to *see* that from
 * the log, rather than from a payment page that looks finished.
 */
import { readFileSync } from "node:fs";

import {
  DEFAULT_RUNTIME_CONFIG,
  assembleRuntimeConfig,
  type RuntimeConfig,
} from "./settings";

/** Where `branding.yaml` is mounted, unless `VPAY_CHECKOUT_BRANDING_FILE` says otherwise. */
export const DEFAULT_BRANDING_PATH = "/etc/vpay/checkout/branding.yaml";
/** Where `config.yaml` is mounted, unless `VPAY_CHECKOUT_CONFIG_FILE` says otherwise. */
export const DEFAULT_CONFIG_PATH = "/etc/vpay/checkout/config.yaml";

/**
 * Bracket notation, for the reason `env.ts` gives at length: Next inlines
 * `process.env.FOO` at build time, and a path baked into the image is not a
 * path an operator can change.
 */
function pathFrom(variable: string, fallback: string): string {
  const value = process.env[variable];
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : fallback;
}

/**
 * The file's text, or `null` with the reason it could not be read.
 *
 * An absent file and an unreadable one are different news — the first is a
 * deployment that configured nothing, the second is a broken mount — so they
 * produce different log lines even though they produce the same defaults.
 */
function readIfPresent(path: string): {
  text: string | null;
  problem: string | null;
} {
  try {
    return { text: readFileSync(path, "utf8"), problem: null };
  } catch (error) {
    const code = (error as NodeJS.ErrnoException | null)?.code;
    if (code === "ENOENT") {
      return { text: null, problem: `no file at ${path} — using defaults` };
    }
    const detail = error instanceof Error ? error.message : String(error);
    return {
      text: null,
      problem: `${path} could not be read (${detail}) — using defaults`,
    };
  }
}

/**
 * Reads both files and returns what the process should serve, plus every
 * problem, without printing anything.
 *
 * Exported so a test can drive the filesystem half against real temporary
 * files without capturing console output, and so `loadRuntimeConfig` below
 * has nothing in it but the printing and the memo.
 */
export function readRuntimeConfig(paths?: {
  brandingFile?: string;
  configFile?: string;
}): { config: RuntimeConfig; problems: readonly string[] } {
  const brandingFile =
    paths?.brandingFile ??
    pathFrom("VPAY_CHECKOUT_BRANDING_FILE", DEFAULT_BRANDING_PATH);
  const configFile =
    paths?.configFile ??
    pathFrom("VPAY_CHECKOUT_CONFIG_FILE", DEFAULT_CONFIG_PATH);

  const branding = readIfPresent(brandingFile);
  const checkout = readIfPresent(configFile);

  const parsed = assembleRuntimeConfig({
    brandingText: branding.text,
    brandingFile,
    configText: checkout.text,
    configFile,
  });

  const problems = [
    ...(branding.problem === null ? [] : [branding.problem]),
    ...(checkout.problem === null ? [] : [checkout.problem]),
    ...parsed.problems,
  ];
  return { config: parsed.value, problems };
}

let memo: RuntimeConfig | null = null;

/**
 * The configuration this container serves, read once.
 *
 * Server-side only — it imports `node:fs`. Every page passes the parts a
 * browser needs down as props; nothing here is fetched from the client.
 */
export function runtimeConfig(): RuntimeConfig {
  if (memo !== null) {
    return memo;
  }
  let result: { config: RuntimeConfig; problems: readonly string[] };
  try {
    result = readRuntimeConfig();
  } catch (error) {
    // Not reachable through `readIfPresent`, which catches its own IO. This
    // is the belt: whatever else goes wrong, a payment page renders.
    const detail = error instanceof Error ? error.message : String(error);
    result = {
      config: DEFAULT_RUNTIME_CONFIG,
      problems: [
        `configuration could not be loaded (${detail}) — using defaults`,
      ],
    };
  }
  for (const problem of result.problems) {
    // eslint-disable-next-line no-console -- the loud log line this feature exists for. Server-side, at start, and it names files and keys only: nothing here has ever touched a session, a payer or a credential.
    console.warn(`[vpay-checkout] configuration: ${problem}`);
  }
  memo = result.config;
  return memo;
}
