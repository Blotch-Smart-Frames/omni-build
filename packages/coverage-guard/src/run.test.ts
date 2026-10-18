import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, relative } from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Isolation } from './isolate.ts';
import { HELP, type RunOptions, run } from './run.ts';

let root: string | undefined;

// A repository of its own for each test, with these files in it.
const repository = async (files: Record<string, string>) => {
  root = await mkdtemp(join(tmpdir(), 'coverage-guard-test-'));
  const all = {
    'package.json': JSON.stringify({ workspaces: ['packages/*'] }),
    ...files,
  };
  for (const [path, content] of Object.entries(all)) {
    await mkdir(join(root, dirname(path)), { recursive: true });
    await writeFile(join(root, path), content);
  }
  return root;
};

afterEach(async () => {
  if (root) await rm(root, { recursive: true, force: true });
  root = undefined;
});

// git ls-files stands in for itself: every file under the package, as it
// would list a repository with nothing ignored.
const listFiles = async (dir: string) =>
  (await readdir(dir, { recursive: true, withFileTypes: true }))
    .filter((entry) => entry.isFile())
    .map((entry) =>
      relative(dir, join(entry.parentPath, entry.name)).replaceAll('\\', '/'),
    )
    .sort();

const COVERED: Isolation = { kind: 'covered' };

const check = (args: string[], options: RunOptions = {}) => {
  const log = vi.fn();
  const isolate = vi.fn(async () => COVERED);
  const rustCoverage = vi.fn(async (dir: string) => ({
    ok: true as const,
    files: [join(dir, 'src/main.rs')],
  }));
  return {
    log,
    isolate,
    rustCoverage,
    result: run(args, {
      root,
      log,
      listFiles,
      isolate,
      rustCoverage,
      ...options,
    }),
  };
};

// A package with a command, as the others are: a module, the cli.ts that
// wires it to the process, their tests, and its config at the root.
const WEB = {
  'packages/web/package.json': '{}',
  'packages/web/vitest.config.ts': 'export default {};',
  'packages/web/src/run.ts': 'export const run = () => 0;',
  'packages/web/src/run.test.ts': '',
  'packages/web/src/cli.ts': 'run();',
  'packages/web/src/cli.test.ts': '',
  'packages/web/fixtures/app/index.ts': 'untested();',
};
const RUST = {
  'packages/rs/package.json': '{}',
  'packages/rs/Cargo.toml': '',
  'packages/rs/src/main.rs': 'fn main() {}',
  'packages/rs/tests/cli.rs': '',
};

describe('coverage-guard', () => {
  it('prints its usage', async () => {
    const log = vi.fn();

    expect(await run(['--help'], { log })).toEqual({ status: 0 });
    expect(log).toHaveBeenCalledWith(HELP);
  });

  it('rejects an option it doesn’t know', async () => {
    expect(await run(['--bogus'])).toEqual({
      status: 2,
      message: "coverage-guard: Unknown option '--bogus'",
    });
  });

  it('passes a repository whose every module its own test covers, and whose Rust is all measured', async () => {
    await repository({ ...WEB, ...RUST });
    const { log, isolate, rustCoverage, result } = check([]);

    expect(await result).toEqual({ status: 0 });
    // Only the module: cli.ts runs only as a child process, and the root's
    // config and the fixtures aren't modules.
    expect(isolate.mock.calls).toEqual([
      [join(root!, 'packages/web'), 'src/run.ts', 'src/run.test.ts'],
    ]);
    expect(rustCoverage).toHaveBeenCalledWith(join(root!, 'packages/rs'));
    expect(log.mock.calls).toEqual([
      ["coverage-guard: running each module's test on its own (1)…"],
      [
        'coverage-guard: every module is covered by its own test (2), every Rust file is measured (1), and nothing is hidden from coverage.',
      ],
    ]);
  });

  it('reports each way 100% could pass over untested code, sorted by file', async () => {
    await repository({
      ...WEB,
      'packages/web/src/hidden.ts':
        'export const a = 1;\n/* v8 ignore next */\nexport const b = 2;',
      'packages/web/src/hidden.test.ts': '',
      'packages/web/src/lonely.tsx': 'export default () => null;',
      'packages/web/src/short.ts': '',
      'packages/web/src/short.test.ts': '',
      'packages/web/src/excluded.ts': '',
      'packages/web/src/excluded.test.ts': '',
      'packages/web/src/broken.ts': '',
      'packages/web/src/broken.test.ts': '',
      ...RUST,
      'packages/rs/src/stray.rs': '#[coverage(off)]\nfn f() {}',
    });
    const results: Record<string, Isolation> = {
      'src/short.ts': {
        kind: 'short',
        shortfalls: ['87.5% of branches', '90% of lines'],
      },
      'src/excluded.ts': { kind: 'unmeasured' },
      'src/broken.ts': { kind: 'failed', output: 'FAIL broken\nexpected 1\n' },
    };
    const { result } = check([], {
      isolate: async (_dir, module) => results[module] ?? COVERED,
    });

    expect(await result).toEqual({
      status: 1,
      message: `${[
        "packages/rs/src/stray.rs isn't in the Rust coverage report: nothing compiles it into the tests. Is a mod declaration missing?",
        'packages/rs/src/stray.rs:1 hides code from coverage: #[coverage(off)]. Test the code instead.',
        'packages/web/src/broken.ts: src/broken.test.ts fails when it runs on its own:\n    FAIL broken\n    expected 1',
        "packages/web/src/excluded.ts isn't measured: its package's coverage settings exclude it.",
        'packages/web/src/hidden.ts:2 hides code from coverage: /* v8 ignore next */. Test the code instead.',
        'packages/web/src/lonely.tsx has no test beside it, named after it (src/lonely.test.tsx).',
        "packages/web/src/short.ts: its own test, src/short.test.ts, covers only 87.5% of branches, 90% of lines. A module's test must cover all of it; coverage from other modules' tests doesn't count.",
      ].join('\n\n')}\n\ncoverage-guard: 7 problems.`,
    });
  });

  it('keeps a cli.ts, which nothing measures, to a few lines of wiring', async () => {
    // Comments and blank lines don't count; the sixteenth line of code does.
    const wiring = [
      '#!/usr/bin/env node',
      '// Wires run() to the process.',
      '/*',
      ' * More about why.',
      ' */',
      '',
      ...Array.from({ length: 14 }, (_, i) => `step${i}();`),
    ].join('\n');
    await repository({
      'packages/web/package.json': '{}',
      'packages/web/src/cli.ts': wiring,
      'packages/web/src/cli.test.ts': '',
      'packages/big/package.json': '{}',
      'packages/big/src/cli.ts': `${wiring}\nstep14();`,
      'packages/big/src/cli.test.ts': '',
    });

    expect(await check([]).result).toEqual({
      status: 1,
      message:
        "packages/big/src/cli.ts has 16 lines of code. Only its test's child process runs it and nothing measures it, so it may only wire run.ts to the process, in at most 15: move the rest into run.ts, where tests can reach it.\n\ncoverage-guard: 1 problem.",
    });
  });

  it('counts a single problem as one', async () => {
    await repository({ ...WEB, 'packages/web/src/lonely.ts': '' });

    const { message } = await check([]).result;

    expect(message).toMatch(/\n\ncoverage-guard: 1 problem\.$/);
  });

  it("fails when it can't read the Rust coverage report, saying what Cargo said", async () => {
    await repository(RUST);
    const { result } = check([], {
      rustCoverage: async () => ({
        ok: false,
        output: 'error: no profraw files found\n',
      }),
    });

    expect(await result).toEqual({
      status: 1,
      message:
        "packages/rs: can't read cargo-llvm-cov's report of the last run. Run npm run test:coverage, which makes it, first.\n    error: no profraw files found\n\ncoverage-guard: 1 problem.",
    });
  });

  it("stops when it can't list a package's files", async () => {
    await repository(WEB);
    const { result } = check([], {
      listFiles: async () => {
        throw new Error(
          'git ls-files failed in packages/web: not a repository',
        );
      },
    });

    expect(await result).toEqual({
      status: 2,
      message:
        'coverage-guard: git ls-files failed in packages/web: not a repository',
    });
  });

  it('runs a few modules’ tests at once, and at least one', async () => {
    await repository({
      'packages/web/package.json': '{}',
      ...Object.fromEntries(
        ['a', 'b', 'c', 'd', 'e'].flatMap((name) => [
          [`packages/web/src/${name}.ts`, ''],
          [`packages/web/src/${name}.test.ts`, ''],
        ]),
      ),
    });
    const peak = async (concurrency: number) => {
      let running = 0;
      let most = 0;
      const { result } = check([], {
        concurrency,
        isolate: async () => {
          running += 1;
          most = Math.max(most, running);
          await new Promise((resolve) => setTimeout(resolve, 5));
          running -= 1;
          return COVERED;
        },
      });
      expect(await result).toEqual({ status: 0 });
      return most;
    };

    expect(await peak(2)).toBe(2);
    expect(await peak(0)).toBe(1);
  });
});
