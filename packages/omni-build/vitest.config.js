import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['src/**/*.js'],
      // cli.js only hands run() this process's argv and exit. Its tests
      // (cli.test.js) start it as a child process, which V8 coverage in the
      // test process can't see, so counting it would fail the threshold with
      // nothing left untested.
      exclude: ['src/**/*.test.js', 'src/cli.js'],
      // Every line, branch, function and statement. The CI gate fails below
      // this.
      thresholds: { 100: true },
    },
  },
});
