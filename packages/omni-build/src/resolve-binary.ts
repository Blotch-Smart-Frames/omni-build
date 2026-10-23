import { createRequire } from 'node:module';
import path from 'node:path';

import { PLATFORMS } from './platforms.ts';

const require = createRequire(import.meta.url);

export interface ResolveBinaryOptions {
  /** A `process.platform` value. */
  platform?: string;
  /** A `process.arch` value. */
  arch?: string;
  env?: Record<string, string | undefined>;
  /** Returns the path of an installed package's package.json, or throws. */
  resolvePackageJson?: (name: string) => string;
}

/**
 * Finds the omni binary to run: the one `OMNI_BUILD_BINARY` names, or
 * otherwise the one in the platform package npm installed for this machine.
 *
 * The options default to the current process and exist for tests.
 */
export const resolveBinary = ({
  platform = process.platform,
  arch = process.arch,
  env = process.env,
  resolvePackageJson = (name) => require.resolve(`${name}/package.json`),
}: ResolveBinaryOptions = {}): string => {
  // For a platform we publish no binary for (build one with `cargo install
  // omni-build`), and for running a binary built from this repository.
  if (env.OMNI_BUILD_BINARY) {
    return env.OMNI_BUILD_BINARY;
  }

  const key = `${platform}-${arch}`;
  if (!Object.hasOwn(PLATFORMS, key)) {
    throw new Error(
      `There's no prebuilt omni binary for ${key}. ` +
        'Build one with `cargo install omni-build` and set OMNI_BUILD_BINARY to its path.',
    );
  }

  const packageName = `@blotch/omni-build-${key}`;
  let packageJson;
  try {
    packageJson = resolvePackageJson(packageName);
  } catch {
    throw new Error(
      `${packageName}, which holds the omni binary for this machine, isn't installed. ` +
        'npm leaves it out when optional dependencies are omitted (--omit=optional) ' +
        'or when node_modules was installed on another OS or CPU. Reinstall without those.',
    );
  }
  return path.join(
    path.dirname(packageJson),
    platform === 'win32' ? 'omni.exe' : 'omni',
  );
};
