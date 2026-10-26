import type { NextConfig } from "next";

const config: NextConfig = {
  reactStrictMode: true,
  // The dashboard talks to /dash/v1 server-side only; no merchant API key ever
  // reaches the browser bundle. See docs/adr/0008-dashboard-scope.md.
  transpilePackages: [
    "@vpay/tokens",
    "@vpay/api-client",
    "@vpay/config",
    "@vpay/dash-client",
  ],
  output: "standalone",
  // `@vpay/dash-client` is generated TypeScript whose relative imports carry
  // a `.js` suffix (`./runtime.js` names `runtime.ts`), the ESM spelling the
  // CrateStack generator emits. Vite/esbuild resolve that and webpack does
  // not, so without this `next build` fails with `Module not found`.
  //
  // exp26 removed an identical entry by deleting the suffixes from
  // `@vpay/ui`'s own sources instead. That route is closed here: the package
  // is generator output, held byte for byte by `just check-schema`'s
  // `--check`, so this is the one place the mapping can live.
  webpack(webpackConfig: { resolve: { extensionAlias?: object } }) {
    webpackConfig.resolve.extensionAlias = {
      ...webpackConfig.resolve.extensionAlias,
      ".js": [".ts", ".tsx", ".js"],
    };
    return webpackConfig;
  },
};

export default config;
