#!/usr/bin/env node

import { run } from './run.ts';

// Only wires run() to this process: its arguments and exit code. The logic
// is in run.ts, where tests can reach it in-process; cli.test.ts runs this
// file as a child process.

// Not process.exit(), which can cut off output still being written to a
// pipe, as it is in CI.
process.exitCode = await run(process.argv.slice(2));
