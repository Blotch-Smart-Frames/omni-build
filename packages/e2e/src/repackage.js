import { copyFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { run } from './process.js';
import { SHELL } from './shell.js';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));
/**
 * The app the tests make from each shell. Each of its values differs from
 * the shell's, so an app that reports them got them from omni-build.
 */
const APP_CONFIG = fileURLToPath(new URL('../app/app.json', import.meta.url));
const APP = JSON.parse(readFileSync(APP_CONFIG, 'utf8'));
/** The colour of every pixel of the app's icon, `app/icon.png`. */
const APP_ICON = '#E8590C';

/** The OS a repackaged app's file name says it was repackaged on. */
const OS_NAMES = { darwin: 'macos', linux: 'linux', win32: 'windows' };

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
 * The platforms whose shells omni-build repackages. It can't repackage an
 * iOS shell until Phase 0's iOS spike (#1), so the stage copies those.
 */
const REPACKAGED = new Set(['android']);

/**
 * What each platform's apps must report after the repackage stage, and
 * what signed them: the app config's values where omni-build rewrites
 * them, and the shell's own elsewhere.
 *
 * @param {string} platform `ios` or `android`.
 */
export const expected = (platform) =>
  platform === 'android'
    ? {
        ...SHELL,
        id: APP.id,
        name: APP.name,
        version: APP.version,
        build: String(APP.build),
        payload: readFileSync(
          path.join(path.dirname(APP_CONFIG), APP.payload, 'payload.txt'),
          'utf8',
        ),
        icon: APP_ICON,
        // The debug key omni-build creates and signs with when it's given
        // no key: only it could have signed the app with this.
        signer: 'CN=OmniBuild Debug, O=OmniBuild',
      }
    : SHELL;

/**
 * The repackage stage: what OmniBuild users do, on any OS, with nothing but
 * the omni-build binary. Writes the result into `outDir`, named for the OS
 * it ran on, and returns its path.
 *
 * For a platform omni-build can't repackage yet, this only checks that the
 * binary runs in the toolchain-free environment, and copies the shell.
 */
export const repackage = async ({ platform, shell, outDir, log }) => {
  mkdirSync(outDir, { recursive: true });
  const extension = path.extname(shell);
  // CI repackages on three OSes and tests every result, so the name says
  // which one each came from.
  const output = path.join(
    outDir,
    `${path.basename(shell, extension)}-${OS_NAMES[process.platform] ?? process.platform}${extension}`,
  );
  if (REPACKAGED.has(platform)) {
    await run(
      omniBuildBinary(),
      ['repackage', shell, '--config', APP_CONFIG, '--out', output],
      { log, env: toolchainFree() },
    );
  } else {
    await run(omniBuildBinary(), ['--version'], { log, env: toolchainFree() });
    copyFileSync(shell, output);
  }
  return output;
};
