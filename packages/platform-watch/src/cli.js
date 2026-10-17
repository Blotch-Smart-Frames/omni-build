#!/usr/bin/env node

import { run } from './run.js';

// Only wires run() to this process: its arguments, stderr and exit code. The
// logic is in run.js, where tests can reach it in-process; cli.test.js runs
// this file as a child process.

const { status, message } = await run(process.argv.slice(2));

if (message) {
  console.error(message);
}
// Not process.exit(), which can cut off output still being written to a
// pipe, as it is in CI.
process.exitCode = status;
