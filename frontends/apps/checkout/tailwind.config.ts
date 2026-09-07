import daisyui from 'daisyui';
import type { Config } from 'tailwindcss';

export default {
  content: [
    './app/**/*.{ts,tsx}',
    './src/**/*.{ts,tsx}',
    // Base UI ships its own class-free markup, but its parts are rendered
    // from this app's source, so nothing under `node_modules` needs
    // scanning. Listed as a note rather than a path for exactly that reason.
  ],
  plugins: [daisyui],
  // **One theme, `bumblebee`** (the maintainer's requirement, 2026-09-05).
  // It was `['corporate', 'business']`, the pair `@vpay/ui` verifies contrast
  // against; a payment page with a theme nobody has checked would have a
  // status colour that is green in one view and grey in another, which is
  // what AGENTS.md's `@vpay/tokens` rule exists to stop — so the page keeps
  // taking its status tone from the tokens package and only its *chrome*
  // from the theme.
  //
  // A deployment's own primary colour is a RUNTIME override of this theme's
  // `--p`/`--pc`, emitted into `<head>` by `app/layout.tsx` from
  // `branding.yaml`. It is not a second theme and it is not a build input:
  // one image serves every operator.
  daisyui: { themes: ['bumblebee'], logs: false },
} satisfies Config;
