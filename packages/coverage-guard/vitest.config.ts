import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    // fixtures/ holds a package for the guard to run vitest in, with tests
    // that fail and fall short on purpose. They're its subject, not tests of
    // the guard.
    dir: 'src',
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['src/**/*.ts'],
      // cli.ts only hands run() this process's argv and exit code. Its tests
      // (cli.test.ts) start it as a child process, which V8 coverage in the
      // test process can't see, so counting it would fail the threshold with
      // nothing left untested.
      exclude: ['src/**/*.test.ts', 'src/cli.ts'],
      // Every line, branch, function and statement. The CI gate fails below
      // this.
      thresholds: { 100: true },
    },
  },
});
