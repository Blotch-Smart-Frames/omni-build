import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { PLATFORMS } from '../../omni-build/src/platforms.ts';
import { INDEX, pack } from './pack.ts';
import type { Exec } from './process.ts';

let root: string;
let binaries: string;
let out: string;
/** What each `npm pack` found in the folder it packed. */
let staged: Record<string, { files: string[]; mode: number; args: string[] }>;

/**
 * npm, packing a staged package: notes what it holds, writes a tarball
 * named as npm names it, and prints what `npm pack --json` does.
 */
const npm: Exec = async (_node, args) => {
  const [, command, dir, , destination] = args;
  expect(command).toBe('pack');
  const manifest = JSON.parse(
    readFileSync(path.join(dir, 'package.json'), 'utf8'),
  );
  const files = readdirSync(dir, { recursive: true }).map(String).sort();
  const binary = files.find((file) => file.startsWith('omni-build'));
  staged[manifest.name] = {
    files,
    mode: binary ? statSync(path.join(dir, binary)).mode & 0o777 : 0,
    args: args.slice(1),
  };
  const filename = `${manifest.name.replace('@', '').replace('/', '-')}-${manifest.version}.tgz`;
  writeFileSync(path.join(destination, filename), 'tarball');
  return JSON.stringify([{ filename }]);
};

beforeEach(() => {
  const dir = mkdtempSync(path.join(tmpdir(), 'omni-build-pack-test-'));
  root = path.join(dir, 'repo');
  binaries = path.join(dir, 'binaries');
  out = path.join(dir, 'out');
  staged = {};
  const launcher = path.join(root, 'packages', 'omni-build');
  mkdirSync(path.join(launcher, 'dist'), { recursive: true });
  writeFileSync(
    path.join(launcher, 'package.json'),
    JSON.stringify({
      name: 'omni-build',
      version: '1.2.3',
      scripts: { build: 'tsc' },
    }),
  );
  writeFileSync(path.join(launcher, 'dist', 'cli.js'), '');
  writeFileSync(path.join(launcher, 'README.md'), '# omni-build\n');
  writeFileSync(path.join(root, 'LICENSE'), 'MIT\n');
  // As the build job's artifacts download, without their mode.
  for (const [key, target] of Object.entries(PLATFORMS)) {
    mkdirSync(path.join(binaries, `omni-build-${target}`), { recursive: true });
    const name = key.startsWith('win32') ? 'omni-build.exe' : 'omni-build';
    writeFileSync(path.join(binaries, `omni-build-${target}`, name), target, {
      mode: 0o644,
    });
  }
});

afterEach(() => {
  rmSync(path.dirname(root), { recursive: true, force: true });
});

describe('pack', () => {
  it('packs a package per platform, and the launcher listing them', async () => {
    const logged: string[] = [];

    const packed = await pack({
      binaries,
      out,
      root,
      exec: npm,
      log: (line) => logged.push(line),
    });

    const names = packed.map(({ manifest }) => manifest.name);
    expect(names).toEqual([
      ...Object.keys(PLATFORMS).map((key) => `@omni-build/${key}`),
      'omni-build',
    ]);
    expect(logged).toEqual(names.map((name) => `Packed ${name}`));
    expect(staged['@omni-build/linux-x64']).toEqual({
      files: ['LICENSE', 'README.md', 'omni-build', 'package.json'],
      // Runnable, whatever mode the artifact came with. Windows has no
      // such modes, and CI packs on Linux.
      mode: process.platform === 'win32' ? expect.any(Number) : 0o755,
      args: [
        'pack',
        expect.any(String),
        '--pack-destination',
        path.resolve(out),
        '--ignore-scripts',
        '--json',
      ],
    });
    expect(staged['@omni-build/win32-x64'].files).toContain('omni-build.exe');
    expect(staged['omni-build'].files).toEqual([
      'LICENSE',
      'README.md',
      'dist',
      path.join('dist', 'cli.js'),
      'package.json',
    ]);
    const launcher = packed.at(-1)!;
    expect(launcher.file).toBe('omni-build-1.2.3.tgz');
    expect(launcher.manifest.optionalDependencies).toMatchObject({
      '@omni-build/linux-x64': '1.2.3',
    });
    expect(launcher.manifest.scripts).toBeUndefined();
    // The index check serves the tarballs by, beside them.
    expect(JSON.parse(readFileSync(path.join(out, INDEX), 'utf8'))).toEqual(
      packed,
    );
    expect(readdirSync(out)).toHaveLength(packed.length + 1);
  });

  it('needs the launcher built', async () => {
    rmSync(path.join(root, 'packages', 'omni-build', 'dist'), {
      recursive: true,
    });

    await expect(pack({ binaries, out, root, exec: npm })).rejects.toThrow(
      "The launcher isn't built: run `npm run build -w omni-build` first.",
    );
  });

  it("needs every platform's binary", async () => {
    const missing = path.join(
      binaries,
      'omni-build-aarch64-pc-windows-msvc',
      'omni-build.exe',
    );
    rmSync(missing);

    await expect(
      pack({ binaries, out, root, exec: npm, log: () => {} }),
    ).rejects.toThrow(
      `There's no aarch64-pc-windows-msvc binary at ${missing}.`,
    );
    // Nothing staged is left behind.
    expect(existsSync(path.join(out, INDEX))).toBe(false);
  });
});
