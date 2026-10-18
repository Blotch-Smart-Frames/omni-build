import { defineConfig } from 'vitest/config';

// .mts, an ES module whatever package.json says: this package has no
// "type": "module" (see the README), and Vite warns that it will stop loading
// a .ts config written as a module from such a package.

export default defineConfig({
  test: {
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      // The .ts modules only. The components (.tsx) are presentation, and
      // the build, which renders every page and checks every link, is their
      // test.
      include: ['src/**/*.ts'],
      exclude: ['src/**/*.test.ts'],
      // Every line, branch, function and statement. The CI gate fails below
      // this.
      thresholds: { 100: true },
    },
  },
});
