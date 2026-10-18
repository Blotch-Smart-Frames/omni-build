import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { run } from './process.ts';
import { SHELL } from './shell.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));
/**
 * The app the tests make from each shell. Each of its values differs from
 * the shell's, so an app that reports them got them from omni-build.
 */
const APP_CONFIG = fileURLToPath(new URL('../app/app.json', import.meta.url));
const APP: {
  id: string;
  name: string;
  version: string;
  build: number;
  /** The app's payload folder, relative to app.json. */
  payload: string;
} = JSON.parse(readFileSync(APP_CONFIG, 'utf8'));
/** The colour of every pixel of the app's icon, `app/icon.png`. */
const APP_ICON = '#E8590C';

/** The OS a repackaged app's file name says it was repackaged on. */
const OS_NAMES: Partial<Record<NodeJS.Platform, string>> = {
  darwin: 'macos',
  linux: 'linux',
  win32: 'windows',
};

/**
 * The only environment variables omni-build gets: where the user's home and
 * temporary folders are, and Windows's own folder, which some of its system
 * calls read. No PATH, so it can't start a program by name, and none of the
 * variables that lead tools to a JDK, the Android SDK or Xcode (JAVA_HOME,
 * ANDROID_HOME, DEVELOPER_DIR, ...). GitHub's Linux and Windows runners come
 * with a JDK and the Android SDK, and its macOS runners with Xcode, so a
 * step that quietly relied on one would otherwise pass in CI and fail on a
 * user's machine without it.
 */
const KEPT_VARIABLES = [
  'HOME',
  'USERPROFILE',
  'TMPDIR',
  'TEMP',
  'TMP',
  'SystemRoot',
];

const toolchainFree = () =>
  Object.fromEntries(
    KEPT_VARIABLES.filter((name) => process.env[name] !== undefined).map(
      (name) => [name, process.env[name]],
    ),
  );

/**
 * The binary under test: the one OMNI_BUILD_BINARY names (as for the npm
 * launcher), or the release build in this repository's target/.
 */
const omniBuildBinary = () => {
  const binary =
    process.env.OMNI_BUILD_BINARY ??
    path.join(
      REPO_ROOT,
      'target/release',
      process.platform === 'win32' ? 'omni-build.exe' : 'omni-build',
    );
  if (!existsSync(binary)) {
    throw new Error(
      `There's no omni-build binary at ${binary}. Build one with \`npm run build -w @omni-build/cli\`, or set OMNI_BUILD_BINARY to one.`,
    );
  }
  return binary;
};

/**
 * What each platform's apps must report after the repackage stage, and
 * what signed them: the app config's values where omni-build rewrites
 * them, and the shell's own elsewhere.
 */
export const expected = (
  platform: 'ios' | 'android',
): typeof SHELL & { signer?: string } => {
  const app = {
    ...SHELL,
    id: APP.id,
    name: APP.name,
    version: APP.version,
    build: String(APP.build),
    payload: readFileSync(
      path.join(path.dirname(APP_CONFIG), APP.payload, 'payload.txt'),
      'utf8',
    ),
  };
  return platform === 'android'
    ? {
        ...app,
        icon: APP_ICON,
        // The debug key omni-build creates and signs with when it's given
        // no key: only it could have signed the app with this.
        signer: 'CN=OmniBuild Debug, O=OmniBuild',
      }
    : // omni-build can't write Assets.car or sign iOS apps yet, so they
      // keep the shell's icon, and the Simulator runs them with the
      // signature Xcode gave the shell, which no longer matches.
      app;
};

/**
 * The repackage stage: what OmniBuild users do, on any OS, with nothing but
 * the omni-build binary. Writes the result into `outDir`, named for the OS
 * it ran on, and returns its path.
 */
export const repackage = async ({
  shell,
  outDir,
  log,
}: {
  shell: string;
  outDir: string;
  log: string;
}) => {
  mkdirSync(outDir, { recursive: true });
  const extension = path.extname(shell);
  // CI repackages on three OSes and tests every result, so the name says
  // which one each came from.
  const output = path.join(
    outDir,
    `${path.basename(shell, extension)}-${OS_NAMES[process.platform] ?? process.platform}${extension}`,
  );
  await run(
    omniBuildBinary(),
    ['repackage', shell, '--config', APP_CONFIG, '--out', output],
    { log, env: toolchainFree() },
  );
  return output;
};
