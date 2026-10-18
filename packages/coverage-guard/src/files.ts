import { existsSync } from 'node:fs';
import { readdir, readFile } from 'node:fs/promises';
import { join, posix } from 'node:path';

import { type Exec, exec as run } from './exec.ts';

// JavaScript and TypeScript in each of their module flavours, with or
// without JSX: .js, .ts, .tsx, .mjs, .cts and the rest.
const SCRIPT = /\.[cm]?[jt]sx?$/;
const TEST = /\.test\.[cm]?[jt]sx?$/;
const DECLARATIONS = /\.d\.[cm]?ts$/;

export interface Package {
  /** From the repository root, with forward slashes: packages/docs. */
  name: string;
  dir: string;
}

/**
 * The workspace packages, from the root package.json's `workspaces`. Only
 * the two forms a directory of packages takes are understood: a directory,
 * and `<directory>/*` for each package in it. Anything else is an error
 * rather than a pattern that matches nothing, which would leave its
 * packages unchecked.
 */
export const workspacePackages = async (root: string): Promise<Package[]> => {
  const { workspaces = [] } = JSON.parse(
    await readFile(join(root, 'package.json'), 'utf8'),
  ) as { workspaces?: string[] };
  const names: string[] = [];
  for (const pattern of workspaces) {
    const parent = pattern.endsWith('/*') ? pattern.slice(0, -2) : undefined;
    if ((parent ?? pattern).includes('*')) {
      throw new Error(`Unsupported workspace pattern '${pattern}'`);
    }
    if (parent === undefined) {
      names.push(pattern);
    } else {
      const entries = await readdir(join(root, parent), {
        withFileTypes: true,
      });
      names.push(
        ...entries
          .filter((entry) => entry.isDirectory())
          .map((entry) => posix.join(parent, entry.name))
          .sort(),
      );
    }
  }
  return names
    .filter((name) => existsSync(join(root, name, 'package.json')))
    .map((name) => ({ name, dir: join(root, name) }));
};

/**
 * Every file git would commit in a directory, relative to it: the tracked
 * files still there, and the new ones git doesn't ignore. Build output and
 * node_modules are ignored, so they're left out, and a module that isn't
 * committed yet is checked before it is.
 */
export const listFiles = async (
  dir: string,
  exec: Exec = run,
): Promise<string[]> => {
  const { ok, output } = await exec(
    'git',
    ['ls-files', '-z', '--cached', '--others', '--exclude-standard'],
    dir,
  );
  if (!ok) throw new Error(`git ls-files failed in ${dir}: ${output}`);
  return output
    .split('\0')
    .filter((file) => file && existsSync(join(dir, file)))
    .sort();
};

/**
 * A module is code the tests must cover: a script in one of the package's
 * directories. Scripts at the package's root are tool configuration
 * (vitest.config.ts, docusaurus.config.ts), anything under a fixtures
 * directory is test data, and declaration files hold only types.
 */
export const isModule = (file: string) =>
  SCRIPT.test(file) &&
  !TEST.test(file) &&
  !DECLARATIONS.test(file) &&
  file.includes('/') &&
  !file.split('/').includes('fixtures');

/** Rust's source files. Its integration tests are in tests/. */
export const isRustSource = (file: string) =>
  file.startsWith('src/') && file.endsWith('.rs');

/**
 * A module's test is beside it and named after it: run.ts's is run.test.ts,
 * and a .tsx component's can be .test.tsx.
 */
export const testOf = (module: string, files: string[]) => {
  const name = module.replace(SCRIPT, '');
  return files.find(
    (file) => TEST.test(file) && file.replace(TEST, '') === name,
  );
};

/**
 * Whether two absolute paths name the same file. Coverage reports and Node
 * write a Windows path with different separators and drive letter case.
 */
export const samePath = (a: string, b: string) =>
  a.replaceAll('\\', '/').toLowerCase() ===
  b.replaceAll('\\', '/').toLowerCase();
