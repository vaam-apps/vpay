import { defineConfig } from 'cypress';

export default defineConfig({
  e2e: {
    baseUrl: process.env.VPAY_DASHBOARD_URL ?? 'http://localhost:3000',
    supportFile: 'cypress/support/e2e.ts',
    specPattern: 'cypress/e2e/**/*.cy.ts',
    video: false,
    retries: { runMode: 2, openMode: 0 },
  },
});
