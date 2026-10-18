import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { type Exec, exec as run } from './exec.ts';

export type RustCoverage =
  | { ok: true; files: string[] }
  | { ok: false; output: string };

/**
 * The files in the report of the last cargo-llvm-cov run, which
 * `npm run test:coverage` has just made. A Rust file that isn't in it was
 * never compiled into a test binary (no `mod` declares it), so no coverage
 * figure counts it, however untested it is.
 */
export const rustCoverage = async (
  dir: string,
  exec: Exec = run,
): Promise<RustCoverage> => {
  const reports = await mkdtemp(join(tmpdir(), 'coverage-guard-'));
  const lcov = join(reports, 'lcov.info');
  try {
    // `report` reads the last run's profiles rather than running the tests
    // again.
    const { ok, output } = await exec(
      'cargo',
      ['llvm-cov', 'report', '--lcov', '--output-path', lcov],
      dir,
    );
    if (!ok) return { ok: false, output };
    const files = (await readFile(lcov, 'utf8'))
      .split(/\r?\n/)
      .filter((line) => line.startsWith('SF:'))
      .map((line) => line.slice('SF:'.length));
    return { ok: true, files };
  } finally {
    await rm(reports, { recursive: true, force: true });
  }
};
