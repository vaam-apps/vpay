import { vpayEslintConfig } from "@vpay/config/eslint";

export default vpayEslintConfig({
  tsconfigRootDir: import.meta.dirname,
  // `mint.mjs` prints a ready-to-open URL and `serve.mjs` prints its port;
  // both are run by hand. `checkout.js` is the payer page and is not exempt.
  scripts: ["mint.mjs", "serve.mjs"],
  browser: ["checkout.js"],
});
