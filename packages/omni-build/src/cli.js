#!/usr/bin/env node

import { run } from './run.js';

// Only wires run() to this process: its arguments, stderr and exit. The
// launcher adds no behaviour of its own, so `omni-build` works the same from
// npm as from `cargo install`. The logic is in run.js, where tests can reach
// it in-process; cli.test.js runs this file as a child process.

const { status, signal, message } = run(process.argv.slice(2));

if (message) {
  console.error(message);
}
if (signal) {
  // Die of the same signal, so whatever started us (a shell, CI, the Angular
  // builder) sees why the binary stopped instead of an invented exit code.
  process.kill(process.pid, signal);
}
process.exit(status);
