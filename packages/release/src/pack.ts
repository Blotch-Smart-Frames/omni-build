import {
  chmodSync,
  copyFileSync,
  cpSync,
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

import { PLATFORMS } from '../../omni-build/src/platforms.ts';
import {
  type Manifest,
  binaryName,
  launcherManifest,
  platformManifest,
  platformReadme,
} from './manifests.ts';
import { type Exec, exec as execCommand, npmCommand } from './process.ts';
import type { Packed } from './registry.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

/** What `pack` writes beside the tarballs, for `check` to serve them by. */
export const INDEX = 'packages.json';

export interface PackOptions {
  /**
   * The release binaries, as CI's build job uploads them: each in a folder
   * named `omni-build-<Rust target>`.
   */
  binaries: string;
  /** Where to write the tarballs. */
  out: string;
  root?: string;
  exec?: Exec;
  log?: (line: string) => void;
}

/**
 * Packs the npm packages a release publishes: one per platform, holding its
 * binary, and the launcher, which lists them all as optional dependencies.
 * Writes their tarballs into `out`, as `npm publish` takes them, with an
 * index of them, and returns it.
 */
export const pack = async ({
  binaries,
  out,
  root = REPO_ROOT,
  exec = execCommand,
  log = console.log,
}: PackOptions): Promise<Packed[]> => {
  const launcherDir = path.join(root, 'packages', 'omni-build');
  const launcher: Manifest = JSON.parse(
    readFileSync(path.join(launcherDir, 'package.json'), 'utf8'),
  );
  if (!existsSync(path.join(launcherDir, 'dist', 'cli.js'))) {
    throw new Error(
      "The launcher isn't built: run `npm run build -w omni-build` first.",
    );
  }
  const license = path.join(root, 'LICENSE');
  const [node, npm] = npmCommand();
  mkdirSync(out, { recursive: true });
  const staging = mkdtempSync(path.join(tmpdir(), 'omni-build-pack-'));

  /** Packs the package in `dir`, and returns its tarball's name. */
  const npmPack = async (dir: string) => {
    // No scripts: the launcher is built already, and its prepack would run
    // TypeScript, which isn't in the staged copy.
    const json = await exec(
      node,
      [
        npm,
        'pack',
        dir,
        '--pack-destination',
        path.resolve(out),
        '--ignore-scripts',
        '--json',
      ],
      { cwd: staging },
    );
    const [{ filename }] = JSON.parse(json) as { filename: string }[];
    return filename;
  };

  try {
    const packed: Packed[] = [];
    for (const [key, target] of Object.entries(PLATFORMS)) {
      const name = binaryName(key);
      const binary = path.join(binaries, `omni-build-${target}`, name);
      if (!existsSync(binary)) {
        throw new Error(`There's no ${target} binary at ${binary}.`);
      }
      const manifest = platformManifest(key, launcher);
      const dir = path.join(staging, key);
      mkdirSync(dir);
      writeFileSync(
        path.join(dir, 'package.json'),
        `${JSON.stringify(manifest, null, 2)}\n`,
      );
      writeFileSync(path.join(dir, 'README.md'), platformReadme(key));
      copyFileSync(license, path.join(dir, 'LICENSE'));
      copyFileSync(binary, path.join(dir, name));
      // Workflow artifacts lose file modes, and npm keeps the one the file
      // has when it packs it.
      chmodSync(path.join(dir, name), 0o755);
      packed.push({ file: await npmPack(dir), manifest });
      log(`Packed ${manifest.name}`);
    }

    const manifest = launcherManifest(launcher, Object.keys(PLATFORMS));
    const dir = path.join(staging, 'omni-build');
    mkdirSync(dir);
    writeFileSync(
      path.join(dir, 'package.json'),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
    cpSync(path.join(launcherDir, 'dist'), path.join(dir, 'dist'), {
      recursive: true,
    });
    copyFileSync(
      path.join(launcherDir, 'README.md'),
      path.join(dir, 'README.md'),
    );
    copyFileSync(license, path.join(dir, 'LICENSE'));
    packed.push({ file: await npmPack(dir), manifest });
    log(`Packed ${manifest.name}`);

    writeFileSync(
      path.join(out, INDEX),
      `${JSON.stringify(packed, null, 2)}\n`,
    );
    return packed;
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
};
