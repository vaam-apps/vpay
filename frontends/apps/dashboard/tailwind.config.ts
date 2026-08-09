import type { Config } from 'tailwindcss';

export default {
  content: [
    './app/**/*.{ts,tsx}',
    './components/**/*.{ts,tsx}',
    '../../packages/ui/src/**/*.{ts,tsx}',
  ],
  plugins: [require('daisyui')],
  daisyui: { themes: ['corporate', 'business'], logs: false },
} satisfies Config;
