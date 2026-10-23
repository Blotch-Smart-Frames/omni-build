import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { LAUNCHER, SCOPE, packageName } from './manifests.ts';
import { INDEX } from './pack.ts';
import { type Exec, exec as execCommand, npmCommand } from './process.ts';
import { type Packed, serve } from './registry.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

export interface CheckOptions {
  /** Where `pack` wrote the tarballs. */
  packages: string;
  root?: string;
  exec?: Exec;
  /** A `process.platform` value. */
  platform?: string;
  /** A `process.arch` value. */
  arch?: string;
  log?: (line: string) => void;
}

/**
 * Installs the packed packages as a user would, with `npm install
 * @blotch/omni-build` in an empty project, from a registry on this computer that has
 * them and nothing else. Then checks that npm installed this machine's
 * binary and no other, that `omni` runs it, and that the API a build
 * tool imports does too. Throws at the first thing that isn't so.
 */
export const check = async ({
  packages,
  root = REPO_ROOT,
  exec = execCommand,
  platform = process.platform,
  arch = process.arch,
  log = console.log,
}: CheckOptions) => {
  const packed: Packed[] = JSON.parse(
    readFileSync(path.join(packages, INDEX), 'utf8'),
  );
  const launcher = packed.find(({ manifest }) => manifest.name === LAUNCHER);
  if (!launcher) {
    throw new Error(`There's no ${LAUNCHER} package in ${packages}.`);
  }
  const { version } = launcher.manifest;
  const own = packageName(`${platform}-${arch}`);
  if (!packed.some(({ manifest }) => manifest.name === own)) {
    throw new Error(
      `There's no ${own} in ${packages}: OmniBuild has no binary for this machine.`,
    );
  }

  const registry = await serve(packages, packed);
  const dir = mkdtempSync(path.join(tmpdir(), 'omni-build-check-'));
  try {
    const project = path.join(dir, 'project');
    const home = path.join(dir, 'home');
    mkdirSync(project);
    mkdirSync(home);
    writeFileSync(
      path.join(project, 'package.json'),
      `${JSON.stringify({ name: 'omni-build-check', private: true, type: 'module' })}\n`,
    );
    writeFileSync(path.join(dir, 'npmrc'), '');
    // Nothing of the npm that may be running this (`npm run` passes its
    // settings on to scripts in npm_ variables), and no OMNI_BUILD_BINARY,
    // which would run another binary than the one npm installs. A home
    // folder of its own, for the debug key omni creates.
    const env = {
      ...Object.fromEntries(
        Object.entries(process.env).filter(
          ([name]) => !/^npm_/i.test(name) && name !== 'OMNI_BUILD_BINARY',
        ),
      ),
      HOME: home,
      USERPROFILE: home,
    };
    const [node, npm] = npmCommand();
    // A cache and settings of its own, so nothing comes from the user's.
    const settings = [
      '--registry',
      registry.url,
      '--cache',
      path.join(dir, 'cache'),
      '--userconfig',
      path.join(dir, 'npmrc'),
      '--no-audit',
      '--no-fund',
      '--no-update-notifier',
    ];
    const inProject = { cwd: project, env };

    await exec(
      node,
      [npm, 'install', `${LAUNCHER}@${version}`, ...settings],
      inProject,
    );
    // The scope's folder holds the launcher too.
    const scope = path.join(project, 'node_modules', SCOPE);
    const installed = existsSync(scope)
      ? readdirSync(scope)
          .map((folder) => `${SCOPE}/${folder}`)
          .filter((name) => name !== LAUNCHER)
          .sort()
      : [];
    if (installed.join() !== own) {
      throw new Error(
        `npm installed ${installed.join(', ') || 'no platform package'}, where it should have installed ${own} alone.`,
      );
    }
    log(
      `npm installed ${LAUNCHER} ${version} with ${own}, and no other platform's package`,
    );

    const printed = (
      await exec(
        node,
        [npm, 'exec', '--offline', ...settings, '--', 'omni', '--version'],
        inProject,
      )
    ).trim();
    if (printed !== `omni ${version}`) {
      throw new Error(
        `omni --version printed "${printed}", not "omni ${version}".`,
      );
    }
    log(`omni --version runs it: ${printed}`);

    const app = path.join(dir, 'app.apk');
    const args = [
      'repackage',
      path.join(root, 'packages', 'cli', 'tests', 'fixtures', 'shell.apk'),
      '--out',
      app,
    ];
    const script = `import { omniBuild } from '${LAUNCHER}';\nconsole.log(JSON.stringify(await omniBuild(${JSON.stringify(args)})));`;
    const result = JSON.parse(
      await exec(node, ['--input-type=module', '-e', script], inProject),
    );
    if (result.event !== 'repackaged' || !existsSync(app)) {
      throw new Error(
        `omniBuild() ended with ${JSON.stringify(result)}, not having repackaged the shell.`,
      );
    }
    log('omniBuild() runs it, as a build tool would: it repackaged a shell');
  } finally {
    await registry.close();
    // Windows can take a moment to let go of what npm just ran.
    rmSync(dir, { recursive: true, force: true, maxRetries: 5 });
  }
};
