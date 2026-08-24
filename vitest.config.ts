import { defineConfig } from 'vitest/config'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// W30: component-level Vitest + Testing Library config (ui.md §16). Kept separate from
// vite.config.ts (Tauri's dev/build config) so `npm run test` never pulls in Tauri's
// dev-server settings, and so Tauri's `beforeBuildCommand`/`beforeDevCommand` never try to
// spawn the test runner.
export default defineConfig({
  plugins: [svelte({ hot: false })],
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    globals: false,
    css: true,
  },
  resolve: {
    conditions: ['browser'],
  },
})
