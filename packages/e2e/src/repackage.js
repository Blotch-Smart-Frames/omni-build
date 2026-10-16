import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { run } from './process.js';
import { SHELL } from './shell.js';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

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
 * What every app must report after the repackage stage: for now the shell's
 * own values, since repackage() passes the shell through unchanged. They
 * become an app's own values when omni-build repackages the shell with them.
 */
export const EXPECTED = SHELL;

/**
 * The repackage stage: what OmniBuild users do, on any OS, with nothing but
 * the omni-build binary. Writes the result into `outDir`, named for the OS
 * it ran on, and returns its path.
 *
 * omni-build has no repackaging command yet, so for now this only checks
 * that the binary runs in the toolchain-free environment, and copies the
 * shell. Phase 0's first command (#1) replaces the copy.
 */
export const repackage = async ({ shell, outDir, log }) => {
  await run(omniBuildBinary(), ['--version'], { log, env: toolchainFree() });

  mkdirSync(outDir, { recursive: true });
  const extension = path.extname(shell);
  // CI repackages on three OSes and tests every result, so the name says
  // which one each came from.
  const output = path.join(
    outDir,
    `${path.basename(shell, extension)}-${OS_NAMES[process.platform] ?? process.platform}${extension}`,
  );
  copyFileSync(shell, output);
  return output;
};
