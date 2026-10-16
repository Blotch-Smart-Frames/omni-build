import { createRequire } from 'node:module';
import path from 'node:path';

import { PLATFORMS } from './platforms.js';

const require = createRequire(import.meta.url);

/**
 * Finds the omni-build binary to run: the one `OMNI_BUILD_BINARY` names, or
 * otherwise the one in the platform package npm installed for this machine.
 *
 * The options default to the current process and exist for tests.
 *
 * @param {object} [options]
 * @param {string} [options.platform] A `process.platform` value.
 * @param {string} [options.arch] A `process.arch` value.
 * @param {Record<string, string | undefined>} [options.env]
 * @param {(name: string) => string} [options.resolvePackageJson] Returns the
 *   path of an installed package's package.json, or throws.
 * @returns {string}
 */
export const resolveBinary = ({
  platform = process.platform,
  arch = process.arch,
  env = process.env,
  resolvePackageJson = (name) => require.resolve(`${name}/package.json`),
} = {}) => {
  // For a platform we publish no binary for (build one with `cargo install
  // omni-build`), and for running a binary built from this repository.
  if (env.OMNI_BUILD_BINARY) {
    return env.OMNI_BUILD_BINARY;
  }

  const key = `${platform}-${arch}`;
  if (!Object.hasOwn(PLATFORMS, key)) {
    throw new Error(
      `There's no prebuilt omni-build binary for ${key}. ` +
        'Build one with `cargo install omni-build` and set OMNI_BUILD_BINARY to its path.',
    );
  }

  const packageName = `@omni-build/${key}`;
  let packageJson;
  try {
    packageJson = resolvePackageJson(packageName);
  } catch {
    throw new Error(
      `${packageName}, which holds the omni-build binary for this machine, isn't installed. ` +
        'npm leaves it out when optional dependencies are omitted (--omit=optional) ' +
        'or when node_modules was installed on another OS or CPU. Reinstall without those.',
    );
  }
  return path.join(
    path.dirname(packageJson),
    platform === 'win32' ? 'omni-build.exe' : 'omni-build',
  );
};
