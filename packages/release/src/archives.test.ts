import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
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

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PLATFORMS } from '../../omni-build/src/platforms.ts';
import {
  SUMS,
  archive,
  archiveFolder,
  archiveName,
  checkArchive,
  readSums,
  version,
} from './archives.ts';
import type { Exec } from './process.ts';

const DATE = new Date('2026-10-24T12:34:56Z');

let root: string;
let binaries: string;
let out: string;

beforeEach(() => {
  const dir = mkdtempSync(path.join(tmpdir(), 'omni-build-archives-test-'));
  root = path.join(dir, 'repo');
  binaries = path.join(dir, 'binaries');
  out = path.join(dir, 'out');
  mkdirSync(path.join(root, 'packages', 'omni-build'), { recursive: true });
  writeFileSync(
    path.join(root, 'packages', 'omni-build', 'package.json'),
    JSON.stringify({ name: '@blotch/omni-build', version: '1.2.3' }),
  );
  writeFileSync(path.join(root, 'LICENSE'), 'MIT\n');
  // As the build job's artifacts download, without their mode. Each a
  // script that prints the version, which runs where a script can.
  for (const [key, target] of Object.entries(PLATFORMS)) {
    const folder = path.join(binaries, `omni-build-${target}`);
    mkdirSync(folder, { recursive: true });
    const name = key.startsWith('win32') ? 'omni.exe' : 'omni';
    writeFileSync(path.join(folder, name), '#!/bin/sh\necho omni 1.2.3\n', {
      mode: 0o644,
    });
  }
});

afterEach(() => {
  rmSync(path.dirname(root), { recursive: true, force: true });
  vi.restoreAllMocks();
});

const archiving = () =>
  archive({ binaries, out, root, date: DATE, log: () => {} });

describe('archiveName', () => {
  it('names a zip for Windows and a gzipped tar elsewhere, with no version', () => {
    expect(archiveName('win32-arm64')).toBe(
      'omni-build-aarch64-pc-windows-msvc.zip',
    );
    expect(archiveName('linux-x64')).toBe(
      'omni-build-x86_64-unknown-linux-musl.tar.gz',
    );
    expect(archiveFolder('darwin-arm64')).toBe(
      'omni-build-aarch64-apple-darwin',
    );
  });

  it("matches the template cargo-binstall finds them by, in the crate's manifest", () => {
    const manifest = readFileSync(
      new URL('../../cli/Cargo.toml', import.meta.url),
      'utf8',
    );
    const setting = (name: string) =>
      manifest.match(new RegExp(`^${name} = "(.*)"$`, 'm'))?.[1];
    const fill = (template: string, key: string) =>
      template
        .replaceAll('{ repo }', 'REPO')
        .replaceAll('{ version }', '1.2.3')
        .replaceAll('{ target }', PLATFORMS[key])
        .replaceAll('{ bin }', 'omni')
        .replaceAll('{ binary-ext }', key.startsWith('win32') ? '.exe' : '')
        .replaceAll(
          '{ archive-suffix }',
          key.startsWith('win32') ? '.zip' : '.tar.gz',
        );

    for (const key of Object.keys(PLATFORMS)) {
      expect(fill(setting('pkg-url')!, key)).toBe(
        `REPO/releases/download/v1.2.3/${archiveName(key)}`,
      );
      expect(fill(setting('bin-dir')!, key)).toBe(
        `${archiveFolder(key)}/${key.startsWith('win32') ? 'omni.exe' : 'omni'}`,
      );
    }
  });
});

describe('archive', () => {
  it("puts each platform's binary and the licence in a folder of its own, with their SHA-256s", () => {
    const sums = archiving();

    const names = Object.keys(PLATFORMS).map(archiveName);
    expect([...sums.keys()]).toEqual(names);
    expect(readdirSync(out).sort()).toEqual([...names, SUMS].sort());
    for (const name of names) {
      const sum = createHash('sha256')
        .update(readFileSync(path.join(out, name)))
        .digest('hex');
      expect(sums.get(name)).toBe(sum);
    }
    expect(readFileSync(path.join(out, SUMS), 'utf8')).toBe(
      names.map((name) => `${sums.get(name)}  ${name}\n`).join(''),
    );
    expect(readSums(out)).toEqual(sums);
  });

  it('makes the binary executable, in an archive tar unpacks', () => {
    archiving();
    const dir = path.join(out, 'unpacked');
    mkdirSync(dir);

    execFileSync('tar', [
      '-xzf',
      path.join(out, archiveName('linux-x64')),
      '-C',
      dir,
    ]);

    const folder = path.join(dir, archiveFolder('linux-x64'));
    expect(readdirSync(folder).sort()).toEqual(['LICENSE', 'omni']);
    expect(readFileSync(path.join(folder, 'LICENSE'), 'utf8')).toBe('MIT\n');
    if (process.platform !== 'win32') {
      expect(statSync(path.join(folder, 'omni')).mode & 0o777).toBe(0o755);
    }
  });

  it('says which binary is missing', () => {
    rmSync(path.join(binaries, 'omni-build-x86_64-pc-windows-msvc'), {
      recursive: true,
    });

    expect(archiving).toThrow(
      `There's no x86_64-pc-windows-msvc binary at ${path.join(binaries, 'omni-build-x86_64-pc-windows-msvc', 'omni.exe')}.`,
    );
  });

  it('dates the files now, and says what it wrote on the console', () => {
    const log = vi.spyOn(console, 'log').mockImplementation(() => {});

    archive({ binaries, out, root });

    expect(log).toHaveBeenCalledWith(
      'Archived omni-build-x86_64-unknown-linux-musl.tar.gz',
    );
  });
});

describe('readSums', () => {
  it('says when there are none', () => {
    expect(() => readSums(out)).toThrow(
      `There's no ${SUMS} in ${out}: run archive first.`,
    );
  });

  it('reads binary-mode lines, as sha256sum -b writes them', () => {
    mkdirSync(out);
    writeFileSync(path.join(out, SUMS), 'abc *one.zip\ndef  two.tar.gz\n');

    expect(readSums(out)).toEqual(
      new Map([
        ['one.zip', 'abc'],
        ['two.tar.gz', 'def'],
      ]),
    );
  });
});

describe('version', () => {
  it("is the launcher's", () => {
    expect(version(root)).toBe('1.2.3');
  });
});

describe('checkArchive', () => {
  /** Each command checkArchive ran. */
  let commands: string[][];

  /**
   * The system, unpacking an archive: puts the folder `holds` names in the
   * destination, and omni prints `printed`.
   */
  const system =
    ({
      holds = ['LICENSE', 'omni'],
      printed = 'omni 1.2.3\n',
    }: { holds?: string[]; printed?: string } = {}): Exec =>
    async (command, args) => {
      commands.push([command, ...args]);
      if (args[0] === '--version') {
        return printed;
      }
      const destination =
        command === 'tar'
          ? args[3]
          : /-DestinationPath '(.*)'$/.exec(args[3])![1].replaceAll("''", "'");
      for (const target of Object.values(PLATFORMS)) {
        const folder = path.join(destination, `omni-build-${target}`);
        mkdirSync(folder, { recursive: true });
        for (const file of holds) {
          writeFileSync(path.join(folder, file), '');
        }
      }
      return '';
    };

  const checking = (
    platform: string,
    arch: string,
    exec: Exec = system(),
    log: (line: string) => void = () => {},
  ) => checkArchive({ archives: out, root, exec, platform, arch, log });

  beforeEach(() => {
    commands = [];
  });

  it('checks the SHA-256, unpacks with tar, and runs omni', async () => {
    archiving();
    const log = vi.fn();

    await checking('linux', 'x64', system(), log);

    const [unpack, run] = commands;
    expect(unpack.slice(0, 3)).toEqual([
      'tar',
      '-xzf',
      path.join(out, 'omni-build-x86_64-unknown-linux-musl.tar.gz'),
    ]);
    expect(run).toEqual([
      path.join(unpack[4], 'omni-build-x86_64-unknown-linux-musl', 'omni'),
      '--version',
    ]);
    expect(log.mock.calls).toEqual([
      [
        `omni-build-x86_64-unknown-linux-musl.tar.gz matches its SHA-256 in ${SUMS}`,
      ],
      ['Unpacked, its omni runs: omni 1.2.3'],
    ]);
    // It cleans up after itself.
    expect(existsSync(unpack[4])).toBe(false);
  });

  it('unpacks with Expand-Archive on Windows, quoting the paths', async () => {
    out = path.join(out, "it's here");
    archiving();

    await checking('win32', 'arm64');

    const [unpack, run] = commands;
    const file = path.join(out, 'omni-build-aarch64-pc-windows-msvc.zip');
    const destination = path.dirname(path.dirname(run[0]));
    expect(unpack).toEqual([
      'powershell',
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      `Expand-Archive -LiteralPath '${file.replaceAll("'", "''")}' -DestinationPath '${destination.replaceAll("'", "''")}'`,
    ]);
    expect(path.basename(run[0])).toBe('omni.exe');
  });

  it('runs the real archive with the real system, where a script runs', async () => {
    if (process.platform === 'win32') {
      return;
    }
    archiving();
    const log = vi.fn();

    await checkArchive({
      archives: out,
      root,
      platform: 'linux',
      arch: 'x64',
      log,
    });

    expect(log).toHaveBeenLastCalledWith('Unpacked, its omni runs: omni 1.2.3');
  });

  it("refuses a machine OmniBuild doesn't build for", async () => {
    await expect(checking('freebsd', 'x64')).rejects.toThrow(
      'OmniBuild has no archive for this machine, freebsd-x64.',
    );
  });

  it('says when the archive is missing', async () => {
    mkdirSync(out);

    await expect(checking('darwin', 'arm64')).rejects.toThrow(
      `There's no omni-build-aarch64-apple-darwin.tar.gz in ${out}.`,
    );
  });

  it("refuses an archive that doesn't match its SHA-256, or has none", async () => {
    archiving();
    const name = 'omni-build-aarch64-apple-darwin.tar.gz';
    writeFileSync(path.join(out, name), 'changed');
    const changed = createHash('sha256').update('changed').digest('hex');

    await expect(checking('darwin', 'arm64')).rejects.toThrow(
      `${name}'s SHA-256 is ${changed}, where ${SUMS} says ${readSums(out).get(name)}.`,
    );

    writeFileSync(path.join(out, SUMS), 'abc  other.zip\n');
    await expect(checking('darwin', 'arm64')).rejects.toThrow(
      `${name}'s SHA-256 is ${changed}, where ${SUMS} says nothing.`,
    );
    expect(commands).toEqual([]);
  });

  it('refuses an archive without the licence', async () => {
    archiving();

    await expect(
      checking('linux', 'arm64', system({ holds: ['omni'] })),
    ).rejects.toThrow(
      'omni-build-aarch64-unknown-linux-musl.tar.gz has no LICENSE in omni-build-aarch64-unknown-linux-musl.',
    );
  });

  it("refuses an omni that isn't this version", async () => {
    archiving();

    await expect(
      checking('linux', 'arm64', system({ printed: 'omni 1.2.2\n' })),
    ).rejects.toThrow('omni --version printed "omni 1.2.2", not "omni 1.2.3".');
  });
});
