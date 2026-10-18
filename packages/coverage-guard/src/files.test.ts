import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { exec } from './exec.ts';
import {
  isModule,
  isRustSource,
  listFiles,
  samePath,
  testOf,
  workspacePackages,
} from './files.ts';

let temp: string | undefined;

// A directory of its own for each test, with these files in it.
const directory = async (files: Record<string, string>) => {
  temp = await mkdtemp(join(tmpdir(), 'coverage-guard-test-'));
  for (const [path, content] of Object.entries(files)) {
    await mkdir(join(temp, dirname(path)), { recursive: true });
    await writeFile(join(temp, path), content);
  }
  return temp;
};

afterEach(async () => {
  if (temp) await rm(temp, { recursive: true, force: true });
  temp = undefined;
});

describe('workspacePackages', () => {
  it('finds each package in a directory of them, and named ones', async () => {
    const root = await directory({
      'package.json': JSON.stringify({ workspaces: ['packages/*', 'tools'] }),
      'packages/web/package.json': '{}',
      'packages/cli/package.json': '{}',
      // Not packages: no package.json, and not a directory.
      'packages/notes/README.md': '',
      'packages/README.md': '',
      'tools/package.json': '{}',
    });

    expect(await workspacePackages(root)).toEqual([
      { name: 'packages/cli', dir: join(root, 'packages/cli') },
      { name: 'packages/web', dir: join(root, 'packages/web') },
      { name: 'tools', dir: join(root, 'tools') },
    ]);
  });

  it('finds none in a repository without workspaces', async () => {
    const root = await directory({ 'package.json': '{}' });

    expect(await workspacePackages(root)).toEqual([]);
  });

  it("refuses a pattern it can't follow rather than check nothing", async () => {
    const root = await directory({
      'package.json': JSON.stringify({ workspaces: ['packages/**'] }),
    });

    await expect(workspacePackages(root)).rejects.toThrow(
      "Unsupported workspace pattern 'packages/**'",
    );
  });
});

describe('listFiles', () => {
  it('lists the files git would commit: tracked, and new but not ignored', async () => {
    const dir = await directory({
      '.gitignore': 'dist/\n',
      'src/tracked.ts': '',
      'src/deleted.ts': '',
      'src/new.ts': '',
      'dist/built.js': '',
    });
    await exec('git', ['init', '--quiet'], dir);
    await exec('git', ['add', 'src/tracked.ts', 'src/deleted.ts'], dir);
    await rm(join(dir, 'src/deleted.ts'));

    expect(await listFiles(dir)).toEqual([
      '.gitignore',
      'src/new.ts',
      'src/tracked.ts',
    ]);
  });

  it('fails outside a git repository', async () => {
    const dir = await directory({});

    await expect(listFiles(dir)).rejects.toThrow(
      `git ls-files failed in ${dir}`,
    );
  });
});

describe('isModule', () => {
  it.each([
    'src/run.ts',
    'src/components/hero.tsx',
    'lib/index.js',
    'src/config.mts',
    'src/legacy.cjs',
  ])('counts %s', (file) => {
    expect(isModule(file)).toBe(true);
  });

  it.each([
    // Tests, of each kind.
    'src/run.test.ts',
    'src/hero.test.tsx',
    // Tool configuration, at the package's root.
    'vitest.config.ts',
    'docusaurus.config.ts',
    // Test data.
    'fixtures/app/src/index.ts',
    'tests/fixtures/sample.js',
    // Types only.
    'src/env.d.ts',
    // Not scripts.
    'src/hero.module.css',
    'src/main.rs',
  ])('leaves out %s', (file) => {
    expect(isModule(file)).toBe(false);
  });
});

describe('isRustSource', () => {
  it("counts the crate's sources, not its integration tests or other files", () => {
    expect(isRustSource('src/main.rs')).toBe(true);
    expect(isRustSource('src/android/mod.rs')).toBe(true);
    expect(isRustSource('tests/cli.rs')).toBe(false);
    expect(isRustSource('src/README.md')).toBe(false);
  });
});

describe('testOf', () => {
  const files = [
    'src/run.ts',
    'src/run.test.ts',
    'src/hero.tsx',
    'src/hero.test.tsx',
    'src/rerun.ts',
    'src/lonely.ts',
    'src/other/lonely.test.ts',
  ];

  it('finds the test beside a module, named after it', () => {
    expect(testOf('src/run.ts', files)).toBe('src/run.test.ts');
    expect(testOf('src/hero.tsx', files)).toBe('src/hero.test.tsx');
  });

  it('finds none for a module whose name is only part of a test’s, or whose test is elsewhere', () => {
    expect(testOf('src/rerun.ts', files)).toBeUndefined();
    expect(testOf('src/lonely.ts', files)).toBeUndefined();
  });
});

describe('samePath', () => {
  it('matches a path written with either separator, in any case', () => {
    expect(samePath('C:\\Repo\\src\\run.ts', 'c:/repo/src/run.ts')).toBe(true);
    expect(samePath('/repo/src/run.ts', '/repo/src/run.ts')).toBe(true);
    expect(samePath('/repo/src/run.ts', '/repo/src/rerun.ts')).toBe(false);
  });
});
