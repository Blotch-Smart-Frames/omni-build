import { spawnSync } from 'node:child_process';

import { resolveBinary } from './resolve-binary.js';

/**
 * Runs the omni-build binary with `args` on this process's terminal, and
 * returns how the launcher should end so that it ends as the binary did.
 *
 * The options default to the real binary and process, and exist for tests.
 *
 * @param {string[]} args
 * @param {object} [options]
 * @param {() => string} [options.resolve] Returns the binary's path, or throws.
 * @param {typeof spawnSync} [options.spawn]
 * @returns {{ status: number, signal?: NodeJS.Signals | null, message?: string }}
 *   `message` is for stderr. `signal` is set when the binary was killed by one.
 */
export const run = (
  args,
  { resolve = resolveBinary, spawn = spawnSync } = {},
) => {
  let binary;
  try {
    binary = resolve();
  } catch (error) {
    return { status: 1, message: `omni-build: ${error.message}` };
  }

  const result = spawn(binary, args, { stdio: 'inherit' });
  if (result.error) {
    return {
      status: 1,
      message: `omni-build: couldn't run ${binary}: ${result.error.message}`,
    };
  }
  // A binary killed by a signal has no exit status. The launcher then dies of
  // the same signal, and exits with 1 only if that doesn't stop it.
  return { status: result.status ?? 1, signal: result.signal };
};
