import type { Config } from 'tailwindcss';

export default {
  content: ['./src/**/*.{ts,tsx,mdx}', '../../apps/**/*.{ts,tsx,mdx}'],
  theme: { extend: {} },
  plugins: [require('daisyui')],
  daisyui: {
    // One light and one dark theme. More would make the status colours in
    // @vpay/tokens unverifiable for contrast.
    themes: ['corporate', 'business'],
    logs: false,
  },
} satisfies Config;
