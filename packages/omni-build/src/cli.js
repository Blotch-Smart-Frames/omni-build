#!/usr/bin/env node

import { spawnSync } from 'node:child_process';

import { resolveBinary } from './resolve-binary.js';

// Runs the native binary with this process's arguments and terminal, then
// exits the way it did. The launcher adds no behaviour of its own, so
// `omni-build` works the same from npm as from `cargo install`.

let binary;
try {
  binary = resolveBinary();
} catch (error) {
  console.error(`omni-build: ${error.message}`);
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  console.error(`omni-build: couldn't run ${binary}: ${result.error.message}`);
  process.exit(1);
}

if (result.signal) {
  // Die of the same signal, so whatever started us (a shell, CI, the Angular
  // builder) sees why the binary stopped instead of an invented exit code.
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
