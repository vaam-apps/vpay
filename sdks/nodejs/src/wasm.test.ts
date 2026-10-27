import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { loadWasm } from "./wasm.js";

const wasmDir = fileURLToPath(new URL("../wasm/", import.meta.url));

describe("the sealing module", () => {
  it("loads", () => {
    expect(typeof loadWasm().Sealer.fromSeed).toBe("function");
  });

  // A bundler that inlines the glue (Next's Turbopack does) rewrites
  // `__dirname` to a build-time path, so a module that reads its `.wasm` from
  // beside itself fails at run time with ENOENT. The shop's standalone image
  // did exactly that and never redirected a payer to hosted checkout.
  it("carries its own bytes: no file read, no __dirname", () => {
    const glue = readFileSync(join(wasmDir, "vpay_sdk_wasm.js"), "utf8");
    expect(glue).not.toContain("__dirname");
    expect(glue).not.toContain("readFileSync");
  });

  it("loads from a directory that holds no .wasm file", () => {
    const dir = mkdtempSync(join(tmpdir(), "vpay-wasm-"));
    try {
      mkdirSync(dir, { recursive: true });
      for (const name of ["vpay_sdk_wasm.js", "package.json"]) {
        copyFileSync(join(wasmDir, name), join(dir, name));
      }
      const alone = createRequire(import.meta.url)(
        join(dir, "vpay_sdk_wasm.js"),
      ) as { Sealer: { fromSeed: unknown } };
      expect(typeof alone.Sealer.fromSeed).toBe("function");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
