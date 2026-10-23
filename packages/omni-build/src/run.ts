import { type SpawnSyncReturns, spawnSync } from 'node:child_process';

import { resolveBinary } from './resolve-binary.ts';

/** What run() reads of spawnSync()'s result. */
export type SpawnResult = Pick<
  SpawnSyncReturns<Buffer>,
  'status' | 'signal' | 'error'
>;

export interface RunOptions {
  /** Returns the binary's path, or throws. */
  resolve?: () => string;
  /** spawnSync(), narrowed to the call run() makes so tests can fake it. */
  spawn?: (
    command: string,
    args: string[],
    options: { stdio: 'inherit' },
  ) => SpawnResult;
}

export interface RunResult {
  status: number;
  /** Set when the binary was killed by one. */
  signal?: NodeJS.Signals | null;
  /** For stderr. */
  message?: string;
}

/**
 * Runs the omni binary with `args` on this process's terminal, and
 * returns how the launcher should end so that it ends as the binary did.
 *
 * The options default to the real binary and process, and exist for tests.
 */
export const run = (
  args: string[],
  { resolve = resolveBinary, spawn = spawnSync }: RunOptions = {},
): RunResult => {
  let binary;
  try {
    binary = resolve();
  } catch (error) {
    return { status: 1, message: `omni: ${(error as Error).message}` };
  }

  const result = spawn(binary, args, { stdio: 'inherit' });
  if (result.error) {
    return {
      status: 1,
      message: `omni: couldn't run ${binary}: ${result.error.message}`,
    };
  }
  // A binary killed by a signal has no exit status. The launcher then dies of
  // the same signal, and exits with 1 only if that doesn't stop it.
  return { status: result.status ?? 1, signal: result.signal };
};
