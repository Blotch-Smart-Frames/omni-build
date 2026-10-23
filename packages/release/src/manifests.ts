// What each npm package says about itself: the launcher, `@blotch/omni-build`,
// and one package per platform in packages/omni-build/src/platforms.ts, which
// holds that platform's binary.

/** The fields of a package.json the packages are made from. */
export interface Manifest {
  name: string;
  version: string;
  [field: string]: unknown;
}

const OS_NAMES: Record<string, string> = {
  darwin: 'macOS',
  linux: 'Linux',
  win32: 'Windows',
};

/**
 * The scope OmniBuild publishes under, Blotch's. `@omni-build` belongs to
 * someone else, and npm won't take an unscoped `omni-build`, as it refuses
 * names that differ from one it has (`omnibuild`) only in punctuation.
 */
export const SCOPE = '@blotch';

/** The launcher's name on npm. */
export const LAUNCHER = `${SCOPE}/omni-build`;

/**
 * A platform's package, by its key in PLATFORMS (`linux-x64`). Named after
 * the launcher, since the scope holds Blotch's other packages too.
 */
export const packageName = (key: string) => `${LAUNCHER}-${key}`;

/** The binary's file name in a platform's package. */
export const binaryName = (key: string) =>
  key.startsWith('win32-') ? 'omni.exe' : 'omni';

/** A platform, as people name it: "Linux on x64". */
const described = (key: string) => {
  const [os, cpu] = key.split('-');
  return `${OS_NAMES[os] ?? os} on ${cpu}`;
};

/**
 * A platform's package.json, at the launcher's version, with the launcher's
 * links and licence.
 */
export const platformManifest = (key: string, launcher: Manifest) => {
  const [os, cpu] = key.split('-');
  return {
    name: packageName(key),
    version: launcher.version,
    description: `The omni binary for ${described(key)}. Install ${LAUNCHER}, which installs this package on the machines it's for`,
    homepage: launcher.homepage,
    bugs: launcher.bugs,
    license: launcher.license,
    // The binary's source, rather than the launcher's.
    repository: {
      ...(launcher.repository as object),
      directory: 'packages/cli',
    },
    // npm installs an optional dependency only where these match, so each
    // machine gets just its own binary.
    os: [os],
    cpu: [cpu],
    files: [binaryName(key)],
    // Yarn's Plug'n'Play would otherwise keep the binary inside a zip, where
    // nothing can run it.
    preferUnplugged: true,
    // A scoped package is private on its first publish without this.
    publishConfig: { access: 'public' },
  } satisfies Manifest;
};

/**
 * The launcher's package.json as npm gets it: with every platform's package
 * as an optional dependency, at the same version, and without the scripts,
 * which run tools only the repository has.
 */
export const launcherManifest = (launcher: Manifest, keys: string[]) => {
  const published: Manifest = { ...launcher };
  delete published.scripts;
  published.optionalDependencies = Object.fromEntries(
    keys.map((key) => [packageName(key), launcher.version]),
  );
  return published;
};

/** A platform's README, for its page on npm. */
export const platformReadme = (key: string) => `# ${packageName(key)}

The [omni](https://www.npmjs.com/package/${LAUNCHER}) binary for ${described(key)}.

Install \`${LAUNCHER}\` rather than this package. It lists a package like this one for each platform it's built for, and npm installs only the one for your OS and CPU.
`;
