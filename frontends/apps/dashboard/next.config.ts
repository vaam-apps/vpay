import type { NextConfig } from 'next';

const config: NextConfig = {
  reactStrictMode: true,
  // The dashboard talks to /dash/v1 server-side only; no merchant API key ever
  // reaches the browser bundle. See docs/adr/0008-dashboard-scope.md.
  transpilePackages: ['@vpay/ui', '@vpay/tokens', '@vpay/api-client', '@vpay/config'],
  output: 'standalone',
};

export default config;
