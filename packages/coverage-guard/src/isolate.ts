import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { type Exec, exec as run } from './exec.ts';
import { samePath } from './files.ts';

const METRICS = ['lines', 'statements', 'functions', 'branches'] as const;

type Summary = Record<
  string,
  Record<(typeof METRICS)[number], { pct: number }>
>;

export type Isolation =
  | { kind: 'covered' }
  /** Each as "87.5% of branches". */
  | { kind: 'short'; shortfalls: string[] }
  | { kind: 'unmeasured' }
  | { kind: 'failed'; output: string };

/**
 * Vitest reads --coverage.include as a glob, so a file name with a glob
 * character in it, such as [id].tsx, has it escaped to match only itself.
 */
export const escapeGlob = (path: string) =>
  path.replace(/[*?[\]{}()!+@]/g, '\\$&');

// The package's own vitest, run by this Node rather than through npx, which
// would add a process and a shell for each module.
const vitestFrom = (dir: string) => {
  const require = createRequire(join(dir, 'package.json'));
  const manifest = require.resolve('vitest/package.json');
  const { bin } = require(manifest) as { bin: { vitest: string } };
  return join(manifest, '..', bin.vitest);
};

// Vitest writes no report when a test fails.
const readSummary = async (path: string) => {
  try {
    return JSON.parse(await readFile(path, 'utf8')) as Summary;
  } catch {
    return undefined;
  }
};

/**
 * Runs a module's test on its own, measuring only that module, and says how
 * much of the module it covers.
 *
 * The whole suite's coverage counts every test that runs a line, so a
 * component a page renders is covered by the page's test whether or not
 * its own test checks it. Run alone, a module's test gets credit only for
 * what it runs itself. The package's vitest config still applies, so its
 * environment, setup and aliases do, and so does its coverage `exclude`:
 * a module that leaves out is unmeasured.
 */
export const isolate = async (
  dir: string,
  module: string,
  test: string,
  exec: Exec = run,
): Promise<Isolation> => {
  let vitest;
  try {
    vitest = vitestFrom(dir);
  } catch {
    return { kind: 'failed', output: `Can't find vitest from ${dir}.` };
  }
  const reports = await mkdtemp(join(tmpdir(), 'coverage-guard-'));
  try {
    const { ok, output } = await exec(
      process.execPath,
      [
        vitest,
        'run',
        resolve(dir, test),
        '--coverage.enabled',
        `--coverage.include=${escapeGlob(module)}`,
        '--coverage.reporter=json-summary',
        `--coverage.reportsDirectory=${reports}`,
      ],
      dir,
    );
    const summary = await readSummary(join(reports, 'coverage-summary.json'));
    const path = resolve(dir, module);
    const entry =
      summary &&
      Object.entries(summary).find(([file]) => samePath(file, path))?.[1];
    // Before the exit code: a coverage threshold in the package's config
    // fails the run when the module falls short.
    const shortfalls = entry
      ? METRICS.filter((metric) => entry[metric].pct < 100).map(
          (metric) => `${entry[metric].pct}% of ${metric}`,
        )
      : [];
    if (shortfalls.length) return { kind: 'short', shortfalls };
    if (!ok) return { kind: 'failed', output };
    if (!entry) return { kind: 'unmeasured' };
    return { kind: 'covered' };
  } finally {
    await rm(reports, { recursive: true, force: true });
  }
};
