import { defineConfig } from 'vitest/config';

// .mts, an ES module whatever package.json says: this package has no
// "type": "module" (see the README), and Vite warns that it will stop loading
// a .ts config written as a module from such a package.

export default defineConfig({
  // tsconfig.json keeps the JSX for Docusaurus to compile ("preserve"), and
  // Vite follows it unless told otherwise. Docusaurus compiles with React's
  // automatic runtime, so the tests do too.
  oxc: { jsx: { runtime: 'automatic' } },
  resolve: {
    // The modules Docusaurus provides only at build time, as aliases of its
    // own. Pointing them at the files it aliases them to lets vitest resolve
    // them, so the tests can vi.mock them: each needs a running Docusaurus
    // site (its router, config and theme) to render for real.
    alias: [
      {
        find: /^@docusaurus\/(BrowserOnly|Link)$/,
        replacement: '@docusaurus/core/lib/client/exports/$1.js',
      },
      {
        find: /^@theme\/(.+)$/,
        replacement: '@docusaurus/theme-classic/lib/theme/$1/index.js',
      },
    ],
  },
  test: {
    // Every module here runs in a browser, the components and the client
    // modules alike.
    environment: 'jsdom',
    setupFiles: ['./vitest.setup.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      // Every module, whatever its extension. A file this leaves out is
      // missing from the report rather than counted as untested, so the
      // coverage gate can't see it: `src/**/*.ts` once let every component
      // through untested.
      include: ['src/**/*.{js,jsx,ts,tsx,mjs,mts,cjs,cts}'],
      exclude: ['src/**/*.test.{ts,tsx}'],
      // Every line, branch, function and statement. The CI gate fails below
      // this.
      thresholds: { 100: true },
    },
  },
});
