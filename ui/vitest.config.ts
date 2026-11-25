import { fileURLToPath } from 'node:url';

import { svelte } from '@sveltejs/vite-plugin-svelte';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

import { appVersion } from './app-version.ts';
import { BudgetReporter } from './budget-reporter.ts';

// Tests run from the repository root so globs and CI `--shard` stay stable.
const repoRoot = fileURLToPath(new URL('..', import.meta.url));
const libDir = fileURLToPath(new URL('./src/lib', import.meta.url));

export default defineConfig({
  root: repoRoot,
  resolve: { alias: { $lib: libDir } },
  define: { __APP_VERSION__: JSON.stringify(appVersion) },
  plugins: [svelte({ configFile: fileURLToPath(new URL('./svelte.config.js', import.meta.url)) }), svelteTesting()],
  test: {
    environment: 'happy-dom',
    // The tests run as a user who prefers reduced motion, so the startup splash stays out of the app's tests;
    // the splash's own tests choose their preference.
    environmentOptions: { happyDOM: { settings: { device: { prefersReducedMotion: 'reduce' } } } },
    setupFiles: [fileURLToPath(new URL('./tests/setup.ts', import.meta.url))],
    include: ['ui/src/**/*.test.ts', 'ui/tests/**/*.test.ts'],
    // The performance budgets (docs/performance-budgets.md) report what they measured next to the usual output.
    reporters: ['default', new BudgetReporter()],
  },
});
