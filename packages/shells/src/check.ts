import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { type Manifest, writeManifest } from './manifest.ts';
import { run } from './process.ts';
import { PLATFORMS, type Variant, variants } from './variants.ts';

/**
 * The app the check makes of each variant, with an ID, name, version, icon
 * and payload of its own, so each of omni's rewrites runs on it.
 */
const CHECK = (file: string) =>
  fileURLToPath(new URL(`../check/${file}`, import.meta.url));
/**
 * A key and a provisioning profile made for the check, as the e2e tests'
 * are (packages/shells/README.md says how): self-signed, for a team that
 * doesn't exist, so no device would install what they sign. The profile's
 * App ID is a wildcard, which covers whatever IDs a shell's extensions,
 * App Clip and watch app get under the app's, and it allows what an App
 * Clip's profile does, which omni only gives an App Clip.
 */
const SIGNING_KEY = CHECK('signing-key.p12');
const PROFILE = CHECK('OmniBuildCheck.mobileprovision');
const KEY_PASSWORD = 'omnibuild-check';

/** An event omni reports with --json, as much as the check reads of one. */
interface OmniEvent {
  event: string;
  platform?: string;
  format?: string;
  message?: string;
}

/** The last line of JSON omni wrote: its result or its error. */
const lastEvent = (stdout: string) => {
  const lines = stdout.trimEnd().split('\n');
  const line = lines[lines.length - 1];
  try {
    return JSON.parse(line) as OmniEvent;
  } catch {
    return undefined;
  }
};

const list = (shells: Variant[]) => shells.map(({ file }) => file).join(', ');

/**
 * The variants of the shell `name` in `dir`: every one of a platform's, for
 * each platform it has any of. Throws if a platform has only some, or if
 * there's none.
 */
const found = (dir: string, name: string) => {
  const shells = PLATFORMS.flatMap((platform) => {
    const all = variants(name, platform);
    const there = all.filter(({ file }) => existsSync(path.join(dir, file)));
    const missing = all.filter((variant) => !there.includes(variant));
    if (there.length > 0 && missing.length > 0) {
      throw new Error(
        `${dir} has ${list(there)}, but not ${list(missing)}: build every variant.`,
      );
    }
    return there;
  });
  if (shells.length === 0) {
    throw new Error(
      `There's no ${name} shell in ${dir}: build it first (${list(variants(name, 'android'))}, ${list(variants(name, 'ios'))}).`,
    );
  }
  return shells;
};

export interface CheckOptions {
  /** The folder build wrote the variants to, for one platform or both. */
  shells: string;
  name: string;
  /** The omni binary, or the npm package's command. */
  omni: string;
  /** See RunOptions.log. */
  log?: string;
}

/**
 * Checks that omni repackages each variant of the shell `name` in
 * `shells`, as the platform and format its file's name says, and writes
 * their manifest beside them (manifest.ts), which it returns.
 */
export const check = async ({
  shells,
  name,
  omni,
  log,
}: CheckOptions): Promise<Manifest> => {
  const checking = found(shells, name);
  const { stdout } = await run(omni, ['--version'], { log });
  const version = /^omni (\S+)$/m.exec(stdout)?.[1];
  if (!version) {
    throw new Error(`${omni} isn't omni: \`--version\` printed ${stdout}`);
  }

  const out = mkdtempSync(path.join(os.tmpdir(), 'omni-build-shells-'));
  try {
    for (const variant of checking) {
      const { file, platform, format } = variant;
      const result = await run(
        omni,
        [
          'repackage',
          '--json',
          path.join(shells, file),
          '--config',
          CHECK('app.json'),
          '--out',
          path.join(out, file),
          ...(format === 'ipa'
            ? ['--signing-key', SIGNING_KEY, '--profile', PROFILE]
            : []),
        ],
        {
          log,
          env: { ...process.env, OMNI_BUILD_KEY_PASSWORD: KEY_PASSWORD },
          allowFailure: true,
        },
      );
      const event = lastEvent(result.stdout);
      if (result.code !== 0 || event?.event !== 'repackaged') {
        throw new Error(
          `omni couldn't repackage ${file}: ${event?.message ?? result.output.trimEnd()}`,
        );
      }
      if (event.platform !== platform || event.format !== format) {
        throw new Error(
          `omni repackaged ${file} as ${event.platform} ${event.format}, where its name says ${platform} ${format}.`,
        );
      }
      console.log(`✓ omni ${version} repackaged ${file}`);
    }
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
  return writeManifest({
    dir: shells,
    name,
    omni: version,
    variants: checking,
  });
};
