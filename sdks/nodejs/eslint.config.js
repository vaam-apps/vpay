import { vpayEslintConfig } from "@vpay/config/eslint";

export default [
  // Build outputs of `scripts/build-wasm.sh` (wasm-bindgen's generated glue),
  // not source.
  { ignores: ["wasm/**", "wasm-test/**"] },
  ...vpayEslintConfig({ tsconfigRootDir: import.meta.dirname }),
];
