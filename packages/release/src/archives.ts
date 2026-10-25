import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

import { PLATFORMS } from '../../omni-build/src/platforms.ts';
import { type Manifest, binaryName } from './manifests.ts';
import { type Exec, exec as execCommand } from './process.ts';
import { tar } from './tar.ts';
import { zip } from './zip.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

/**
 * The archives' checksums, beside them on the release, in the format
 * sha256sum writes, so `sha256sum --check --ignore-missing SHA256SUMS`
 * checks a download.
 */
export const SUMS = 'SHA256SUMS';

/**
 * The folder an archive holds, `omni-build-<Rust target>`, and the archive's
 * name without its extension. The names carry no version, so
 * `releases/latest/download/<name>` always gets the newest, and cargo-binstall
 * finds them by the template in packages/cli/Cargo.toml.
 */
export const archiveFolder = (key: string) => `omni-build-${PLATFORMS[key]}`;

/**
 * A platform's archive, by its key in PLATFORMS: a zip for Windows, which
 * opens them itself, and a gzipped tar elsewhere, which keeps the binary's
 * mode.
 */
export const archiveName = (key: string) =>
  `${archiveFolder(key)}.${key.startsWith('win32-') ? 'zip' : 'tar.gz'}`;

/** The version every package has, from the launcher's package.json. */
export const version = (root: string) =>
  (
    JSON.parse(
      readFileSync(
        path.join(root, 'packages', 'omni-build', 'package.json'),
        'utf8',
      ),
    ) as Manifest
  ).version;

/** A file's SHA-256, in hex. */
const sha256 = (data: Buffer) =>
  createHash('sha256').update(data).digest('hex');

/** The checksums in `dir`'s SHA256SUMS, by file name. */
export const readSums = (dir: string) => {
  const file = path.join(dir, SUMS);
  if (!existsSync(file)) {
    throw new Error(`There's no ${SUMS} in ${dir}: run archive first.`);
  }
  return new Map(
    readFileSync(file, 'utf8')
      .trim()
      .split('\n')
      .map((line) => {
        const [sum, name] = line.split(/\s+\*?/);
        return [name, sum] as const;
      }),
  );
};

export interface ArchiveOptions {
  /**
   * The release binaries, as CI's build job uploads them: each in a folder
   * named `omni-build-<Rust target>`.
   */
  binaries: string;
  /** Where to write the archives. */
  out: string;
  root?: string;
  /** When the archives' files are dated. */
  date?: Date;
  log?: (line: string) => void;
}

/**
 * Puts each platform's binary in an archive of its own for the GitHub
 * release, with the licence, inside a folder named as the archive is, so
 * unpacking one leaves a single folder. Writes their checksums beside them
 * in SHA256SUMS, and returns them by file name.
 */
export const archive = ({
  binaries,
  out,
  root = REPO_ROOT,
  date = new Date(),
  log = console.log,
}: ArchiveOptions) => {
  const license = readFileSync(path.join(root, 'LICENSE'));
  mkdirSync(out, { recursive: true });
  const sums = new Map<string, string>();
  for (const [key, target] of Object.entries(PLATFORMS)) {
    const binary = path.join(binaries, `omni-build-${target}`, binaryName(key));
    if (!existsSync(binary)) {
      throw new Error(`There's no ${target} binary at ${binary}.`);
    }
    const folder = archiveFolder(key);
    const entries = [
      { name: folder, mode: 0o755 },
      // Executable, whatever mode the workflow's artifact left it with.
      {
        name: `${folder}/${binaryName(key)}`,
        data: readFileSync(binary),
        mode: 0o755,
      },
      { name: `${folder}/LICENSE`, data: license, mode: 0o644 },
    ];
    const name = archiveName(key);
    const data = key.startsWith('win32-')
      ? zip(entries, date)
      : gzipSync(tar(entries, date), { level: 9 });
    writeFileSync(path.join(out, name), data);
    sums.set(name, sha256(data));
    log(`Archived ${name}`);
  }
  writeFileSync(
    path.join(out, SUMS),
    [...sums].map(([name, sum]) => `${sum}  ${name}\n`).join(''),
  );
  return sums;
};

export interface CheckArchiveOptions {
  /** Where `archive` wrote them. */
  archives: string;
  root?: string;
  exec?: Exec;
  /** A `process.platform` value. */
  platform?: string;
  /** A `process.arch` value. */
  arch?: string;
  log?: (line: string) => void;
}

/**
 * Does with this machine's archive what a user who downloads it would:
 * checks it against SHA256SUMS, unpacks it, with PowerShell's
 * Expand-Archive on Windows and tar elsewhere, and runs the omni inside.
 * Throws at the first thing that isn't so.
 */
export const checkArchive = async ({
  archives,
  root = REPO_ROOT,
  exec = execCommand,
  platform = process.platform,
  arch = process.arch,
  log = console.log,
}: CheckArchiveOptions) => {
  const key = `${platform}-${arch}`;
  if (!(key in PLATFORMS)) {
    throw new Error(`OmniBuild has no archive for this machine, ${key}.`);
  }
  const name = archiveName(key);
  const file = path.join(archives, name);
  if (!existsSync(file)) {
    throw new Error(`There's no ${name} in ${archives}.`);
  }
  const expected = readSums(archives).get(name);
  const actual = sha256(readFileSync(file));
  if (actual !== expected) {
    throw new Error(
      `${name}'s SHA-256 is ${actual}, where ${SUMS} says ${expected ?? 'nothing'}.`,
    );
  }
  log(`${name} matches its SHA-256 in ${SUMS}`);

  const dir = mkdtempSync(path.join(tmpdir(), 'omni-build-archive-'));
  try {
    if (platform === 'win32') {
      // Single quotes are PowerShell's literal strings, in which only a
      // single quote needs escaping, by doubling it.
      const quoted = (text: string) => `'${text.replaceAll("'", "''")}'`;
      await exec('powershell', [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        `Expand-Archive -LiteralPath ${quoted(file)} -DestinationPath ${quoted(dir)}`,
      ]);
    } else {
      await exec('tar', ['-xzf', file, '-C', dir]);
    }
    const folder = path.join(dir, archiveFolder(key));
    if (!existsSync(path.join(folder, 'LICENSE'))) {
      throw new Error(`${name} has no LICENSE in ${archiveFolder(key)}.`);
    }
    const printed = (
      await exec(path.join(folder, binaryName(key)), ['--version'])
    ).trim();
    const expectedVersion = `omni ${version(root)}`;
    if (printed !== expectedVersion) {
      throw new Error(
        `omni --version printed "${printed}", not "${expectedVersion}".`,
      );
    }
    log(`Unpacked, its omni runs: ${printed}`);
  } finally {
    rmSync(dir, { recursive: true, force: true, maxRetries: 5 });
  }
};
