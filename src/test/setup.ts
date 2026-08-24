import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/svelte'
import { afterEach } from 'vitest'

// Testing Library does not auto-cleanup outside of a supported test framework's
// global afterEach hook (we run with `globals: false`), so do it explicitly.
afterEach(() => {
  cleanup()
})
