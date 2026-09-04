import type { NextConfig } from "next";

const config: NextConfig = {
  reactStrictMode: true,
  // A standalone server, so `examples/shop/Dockerfile`'s runtime stage copies
  // a self-contained `server.js` plus the node_modules it actually needs,
  // rather than the whole workspace.
  output: "standalone",
  // The workspace root, not `examples/shop`: without it Next infers the root
  // from the nearest lockfile and warns about the monorepo on every build.
  outputFileTracingRoot: new URL("../../", import.meta.url).pathname,
};

export default config;
