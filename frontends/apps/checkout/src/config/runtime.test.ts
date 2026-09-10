/**
 * The filesystem half: real files, real permissions, real absences.
 *
 * `readRuntimeConfig` is exercised rather than `runtimeConfig`, deliberately.
 * The memo in `runtimeConfig` is the behaviour a container wants (read once,
 * at start) and the behaviour a test cannot re-enter, and a test-only reset
 * export would be a hole in shipping code to serve a test. What the memo
 * adds over what is covered here is one `if`.
 */
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { DEFAULT_BRANDING, DEFAULT_CHECKOUT_SETTINGS } from "./settings";
import {
  DEFAULT_BRANDING_PATH,
  DEFAULT_CONFIG_PATH,
  readRuntimeConfig,
} from "./runtime";

const made: string[] = [];

function mount(files: Record<string, string>): {
  brandingFile: string;
  configFile: string;
} {
  const dir = mkdtempSync(join(tmpdir(), "vpay-checkout-config-"));
  made.push(dir);
  for (const [name, body] of Object.entries(files)) {
    writeFileSync(join(dir, name), body, "utf8");
  }
  return {
    brandingFile: join(dir, "branding.yaml"),
    configFile: join(dir, "config.yaml"),
  };
}

afterEach(() => {
  for (const dir of made.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

describe("readRuntimeConfig", () => {
  it("reads both mounted files", () => {
    const paths = mount({
      "branding.yaml":
        'display_name: Vaam Payments\nprimary_color: "#f3c623"\n',
      "config.yaml": "checkout:\n  allowed_methods: [mtn_momo]\n",
    });
    const { config, problems } = readRuntimeConfig(paths);
    expect(problems).toEqual([]);
    expect(config.branding.displayName).toBe("Vaam Payments");
    expect(config.checkout.allowedMethods).toEqual(["mtn_momo"]);
  });

  it("serves defaults and names each absent file — the loud line, not a blank page", () => {
    const paths = mount({});
    const { config, problems } = readRuntimeConfig(paths);
    expect(config.branding).toEqual(DEFAULT_BRANDING);
    expect(config.checkout).toEqual(DEFAULT_CHECKOUT_SETTINGS);
    expect(problems).toHaveLength(2);
    expect(problems[0]).toContain(paths.brandingFile);
    expect(problems[0]).toContain("no file at");
    expect(problems[1]).toContain(paths.configFile);
  });

  it("tells a broken mount apart from an empty one", () => {
    const paths = mount({
      "branding.yaml": "display_name: X\n",
      "config.yaml": "checkout: {}\n",
    });
    // A file that exists and cannot be read is a different fault from a file
    // that is not there, and an operator chasing one must not be shown the
    // message for the other.
    chmodSync(paths.brandingFile, 0o000);
    const { config, problems } = readRuntimeConfig(paths);
    // Running as root defeats the mode bit; the assertion is then vacuous, so
    // skip rather than pass. `process.getuid` is absent on Windows.
    if (process.getuid?.() === 0) {
      expect(problems).toEqual([]);
      return;
    }
    expect(config.branding).toEqual(DEFAULT_BRANDING);
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("could not be read");
    expect(problems[0]).not.toContain("no file at");
  });

  it("reads one file when only one is mounted", () => {
    const paths = mount({
      "config.yaml": "checkout:\n  features:\n    page_memory: false\n",
    });
    const { config, problems } = readRuntimeConfig(paths);
    expect(config.checkout.features.pageMemory).toBe(false);
    expect(config.branding).toEqual(DEFAULT_BRANDING);
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain("branding.yaml");
  });

  it("reports a malformed file’s problem beside the absent one’s", () => {
    const paths = mount({ "config.yaml": "checkout: [not, a, mapping]\n" });
    const { config, problems } = readRuntimeConfig(paths);
    expect(config.checkout).toEqual(DEFAULT_CHECKOUT_SETTINGS);
    expect(problems).toHaveLength(2);
    expect(problems.join(" ")).toContain("no file at");
    expect(problems.join(" ")).toContain("checkout: is not a mapping");
  });

  it("defaults to the paths the image and the chart mount into", () => {
    // A change to either of these is a change to a Dockerfile, a compose file
    // and the chart, and this is the line that makes that a failing test.
    expect(DEFAULT_BRANDING_PATH).toBe("/etc/vpay/checkout/branding.yaml");
    expect(DEFAULT_CONFIG_PATH).toBe("/etc/vpay/checkout/config.yaml");
  });

  it("takes its paths from the environment at RUNTIME, not from a build-time literal", () => {
    const paths = mount({ "branding.yaml": "display_name: From the env\n" });
    process.env["VPAY_CHECKOUT_BRANDING_FILE"] = paths.brandingFile;
    process.env["VPAY_CHECKOUT_CONFIG_FILE"] = paths.configFile;
    try {
      // No argument: the environment is the only source. Set AFTER this
      // module was imported, which is the property `env.ts` explains at
      // length — Next inlines `process.env.FOO` and would have baked one
      // deployment's path into the image.
      expect(readRuntimeConfig().config.branding.displayName).toBe(
        "From the env",
      );
    } finally {
      delete process.env["VPAY_CHECKOUT_BRANDING_FILE"];
      delete process.env["VPAY_CHECKOUT_CONFIG_FILE"];
    }
  });

  it("ignores a blank environment variable rather than reading the empty path", () => {
    process.env["VPAY_CHECKOUT_BRANDING_FILE"] = "   ";
    try {
      const { problems } = readRuntimeConfig();
      expect(problems.join(" ")).toContain(DEFAULT_BRANDING_PATH);
    } finally {
      delete process.env["VPAY_CHECKOUT_BRANDING_FILE"];
    }
  });
});
