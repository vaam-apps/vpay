import { resolve } from 'node:path';

import type { NextConfig } from 'next';

/**
 * The hosted checkout page.
 *
 * `output: 'standalone'` because lane 4 ships this as its own image and a
 * `next start` over a full `node_modules` is not what a payment page should
 * be deployed as. No server actions and no cookies: every credential this
 * app handles arrives in the URL (fragment for secrets, query for the
 * publishable key) and lives only in the payer's tab.
 *
 * Security headers — `Referrer-Policy`, `Cache-Control`, `X-Content-Type-Options`
 * and the `frame-ancestors` CSP — are set in `middleware.ts`, not here: the
 * embedded route's `frame-ancestors` value depends on a per-request lookup
 * against vpay's own origins route, which a static `headers()` table cannot
 * express, and having two places set security headers is how one of them
 * ends up stale.
 */
const config: NextConfig = {
  reactStrictMode: true,
  transpilePackages: ['@vpay/tokens'],
  output: 'standalone',
  /**
   * The monorepo root, so the standalone bundle is laid out relative to it.
   *
   * Without this Next *infers* a root from the nearest lockfiles, and in a
   * checkout that has more than one (a git worktree inside another clone,
   * which is how this lane was developed) it picks the outer one and emits
   * `.next/standalone/<the whole path back down to here>/server.js`. Lane 4
   * copies this output into an image; a path that depends on where the repo
   * happened to be cloned is not something a Dockerfile can spell.
   *
   * `process.cwd()` is this app's directory during `next build` — pnpm runs
   * a package script there — and three levels up is the repository root.
   */
  outputFileTracingRoot: resolve(process.cwd(), '../../..'),
};

export default config;
