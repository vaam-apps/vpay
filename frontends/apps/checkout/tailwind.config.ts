import daisyui from 'daisyui';
import type { Config } from 'tailwindcss';

export default {
  content: ['./app/**/*.{ts,tsx}', './src/**/*.{ts,tsx}'],
  plugins: [daisyui],
  // The same two themes @vpay/ui verifies contrast against. A payment page
  // with a third theme would have a status colour nobody has checked.
  daisyui: { themes: ['corporate', 'business'], logs: false },
} satisfies Config;
