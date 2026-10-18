import { readFile } from 'node:fs/promises';
import { availableParallelism } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

import {
  isModule,
  isRustSource,
  listFiles as listGitFiles,
  samePath,
  testOf,
  workspacePackages,
} from './files.ts';
import { hints } from './hints.ts';
import { type Isolation, isolate as isolateModule } from './isolate.ts';
import { type RustCoverage, rustCoverage as readRustCoverage } from './rust.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

export const HELP = `Usage: npm run check -w @omni-build/coverage-guard

Checks that the 100% coverage gate can't pass over untested code. For every
workspace package:

  - each module (a script in one of its directories, tests, fixtures and
    root config files aside) has a test beside it, and that test on its own
    covers all of the module: coverage the module only gets from other
    modules' tests doesn't count
  - no module hides code from coverage with an ignore comment, and a
    cli.ts, which only its test's child process runs, stays a few lines
  - every Rust file in src/ is in cargo-llvm-cov's report, and none hides
    code from it with a coverage attribute or cfg

npm run test:coverage runs it after the tests, as CI does: the Rust check
reads the report the tests just made.

Options:
  --help  Show this.`;

export interface RunOptions {
  /** The repository root. */
  root?: string;
  log?: (line: string) => void;
  listFiles?: (dir: string) => Promise<string[]>;
  isolate?: (dir: string, module: string, test: string) => Promise<Isolation>;
  rustCoverage?: (dir: string) => Promise<RustCoverage>;
  /** How many modules' tests run at once. */
  concurrency?: number;
}

export interface RunResult {
  status: number;
  /** For stderr. */
  message?: string;
}

// In a package with a command, cli.ts only wires run.ts to the process, and
// its test runs it as a child process, which V8 coverage in the test
// process can't see. Its package leaves it out of coverage, so there's
// nothing to measure it alone; it still needs the test. Kept to a few lines,
// so the exemption can't become somewhere to put code no test measures: the
// largest wiring there is takes 10.
const CHILD_PROCESS_ONLY = 'src/cli.ts';
const CHILD_PROCESS_LINES = 15;

// Lines of code, not counting blank lines and lines that are only a comment.
const codeLines = (source: string) =>
  source.split('\n').filter((line) => !/^\s*(?:$|\/\/|\/\*|\*)/.test(line))
    .length;

const indent = (output: string) =>
  output
    .trimEnd()
    .split('\n')
    .slice(-40)
    .map((line) => `    ${line}`)
    .join('\n');

const explain = (where: string, test: string, result: Isolation) => {
  switch (result.kind) {
    case 'covered':
      return undefined;
    case 'short':
      return `${where}: its own test, ${test}, covers only ${result.shortfalls.join(', ')}. A module's test must cover all of it; coverage from other modules' tests doesn't count.`;
    case 'unmeasured':
      return `${where} isn't measured: its package's coverage settings exclude it.`;
    case 'failed':
      return `${where}: ${test} fails when it runs on its own:\n${indent(result.output)}`;
  }
};

// Runs the tasks at most `limit` at a time, keeping their results in order.
const pool = async <T>(tasks: (() => Promise<T>)[], limit: number) => {
  const results: T[] = [];
  let next = 0;
  const worker = async () => {
    while (next < tasks.length) {
      const index = next;
      next += 1;
      results[index] = await tasks[index]();
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, limit) }, worker));
  return results;
};

/**
 * Checks every package, and reports each problem found.
 *
 * The options default to the real repository, git, vitest and Cargo, and
 * exist for tests.
 */
export const run = async (
  args: string[],
  {
    root = REPO_ROOT,
    log = console.log,
    listFiles = listGitFiles,
    isolate = isolateModule,
    rustCoverage = readRustCoverage,
    concurrency = availableParallelism(),
  }: RunOptions = {},
): Promise<RunResult> => {
  let values;
  try {
    ({ values } = parseArgs({
      args,
      options: { help: { type: 'boolean', default: false } },
    }));
  } catch (error) {
    return {
      status: 2,
      message: `coverage-guard: ${(error as Error).message}`,
    };
  }
  if (values.help) {
    log(HELP);
    return { status: 0 };
  }

  const problems: string[] = [];
  const isolations: (() => Promise<string | undefined>)[] = [];
  let modules = 0;
  let rustFiles = 0;
  try {
    for (const { name, dir } of await workspacePackages(root)) {
      const files = await listFiles(dir);
      for (const module of files.filter(isModule)) {
        modules += 1;
        const where = `${name}/${module}`;
        const source = await readFile(join(dir, module), 'utf8');
        for (const { line, text } of hints(source, 'script')) {
          problems.push(
            `${where}:${line} hides code from coverage: ${text}. Test the code instead.`,
          );
        }
        const test = testOf(module, files);
        if (!test) {
          problems.push(
            `${where} has no test beside it, named after it (${module.replace(/(\.[^.]+)$/, '.test$1')}).`,
          );
          continue;
        }
        if (module === CHILD_PROCESS_ONLY) {
          const lines = codeLines(source);
          if (lines > CHILD_PROCESS_LINES) {
            problems.push(
              `${where} has ${lines} lines of code. Only its test's child process runs it and nothing measures it, so it may only wire run.ts to the process, in at most ${CHILD_PROCESS_LINES}: move the rest into run.ts, where tests can reach it.`,
            );
          }
          continue;
        }
        isolations.push(async () =>
          explain(where, test, await isolate(dir, module, test)),
        );
      }

      if (!files.includes('Cargo.toml')) continue;
      const sources = files.filter(isRustSource);
      rustFiles += sources.length;
      for (const file of sources) {
        const source = await readFile(join(dir, file), 'utf8');
        for (const { line, text } of hints(source, 'rust')) {
          problems.push(
            `${name}/${file}:${line} hides code from coverage: ${text}. Test the code instead.`,
          );
        }
      }
      const report = await rustCoverage(dir);
      if (!report.ok) {
        problems.push(
          `${name}: can't read cargo-llvm-cov's report of the last run. Run npm run test:coverage, which makes it, first.\n${indent(report.output)}`,
        );
        continue;
      }
      for (const file of sources) {
        if (!report.files.some((path) => samePath(path, join(dir, file)))) {
          problems.push(
            `${name}/${file} isn't in the Rust coverage report: nothing compiles it into the tests. Is a mod declaration missing?`,
          );
        }
      }
    }
  } catch (error) {
    return {
      status: 2,
      message: `coverage-guard: ${(error as Error).message}`,
    };
  }

  log(
    `coverage-guard: running each module's test on its own (${isolations.length})…`,
  );
  for (const problem of await pool(isolations, concurrency)) {
    if (problem) problems.push(problem);
  }

  if (problems.length) {
    const count =
      problems.length === 1 ? '1 problem' : `${problems.length} problems`;
    return {
      status: 1,
      message: `${problems.sort().join('\n\n')}\n\ncoverage-guard: ${count}.`,
    };
  }
  log(
    `coverage-guard: every module is covered by its own test (${modules}), every Rust file is measured (${rustFiles}), and nothing is hidden from coverage.`,
  );
  return { status: 0 };
};
