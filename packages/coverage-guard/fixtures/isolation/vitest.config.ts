import { defineConfig } from 'vitest/config';

// A package as the guard finds them: a coverage threshold, and a module its
// coverage settings leave out.
export default defineConfig({
  test: {
    coverage: {
      provider: 'v8',
      exclude: ['src/excluded.ts'],
      thresholds: { 100: true },
    },
  },
});
