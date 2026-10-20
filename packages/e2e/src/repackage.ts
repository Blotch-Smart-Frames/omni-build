import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from 'node:fs';
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
/**
 * What an iOS build for devices is signed with: a key with a self-signed
 * certificate, in a keystore as Keychain Access exports one, and a
 * provisioning profile allowing it and the app's ID, for a team that
 * doesn't exist. Both were made for these tests, as README.md says: no
 * device would install the app, but codesign checks the signature all the
 * same.
 */
const SIGNING_KEY = fileURLToPath(
  new URL('../app/signing-key.p12', import.meta.url),
);
const PROFILE = fileURLToPath(
  new URL('../app/OmniBuildE2E.mobileprovision', import.meta.url),
);
const SIGNER = 'OmniBuild E2E';
const TEAM = 'OMNIBUILD0';
/** The keystores' password, the iOS key's and the upload key's. */
const KEY_PASSWORD = 'omnibuild-e2e';
/** The subject of the upload key omni-build creates for an app bundle. */
const UPLOAD_KEY = 'CN=OmniBuild E2E Upload';

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

export const toolchainFree = () =>
  Object.fromEntries(
    KEPT_VARIABLES.filter((name) => process.env[name] !== undefined).map(
      (name) => [name, process.env[name]],
    ),
  );

/**
 * The binary under test: the one OMNI_BUILD_BINARY names (as for the npm
 * launcher), or the release build in this repository's target/.
 */
export const omniBuildBinary = () => {
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
 * What an app must report after the repackage stage, and what signed it:
 * the app config's values where omni-build rewrites them, and the shell's
 * own elsewhere.
 */
export const expected = (
  platform: 'ios' | 'android',
  app = '',
): Record<string, string> => {
  const values = {
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
  };
  if (platform === 'android') {
    return {
      ...values,
      // An app bundle is signed with the upload key omni-build created for
      // it, and an APK with the debug key it creates and signs with when
      // it's given no key: only omni-build could have made either.
      signer: app.endsWith('.aab')
        ? UPLOAD_KEY
        : 'CN=OmniBuild Debug, O=OmniBuild',
    };
  }
  if (app.endsWith('.ipa')) {
    // A build for devices, which the simulator can't run: what its files
    // say, but for the icon, which only iOS reads out of Assets.car, and
    // what the profile allowed its signature.
    return {
      id: values.id,
      name: values.name,
      version: values.version,
      build: values.build,
      payload: values.payload,
      signer: SIGNER,
      applicationIdentifier: `${TEAM}.${APP.id}`,
    };
  }
  // Signed ad hoc, as Xcode signs a simulator build.
  return { ...values, signer: 'ad hoc' };
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
  const binary = omniBuildBinary();
  const env = { ...toolchainFree(), OMNI_BUILD_KEY_PASSWORD: KEY_PASSWORD };
  // A hidden folder, which no stage takes for an app.
  const keys = mkdtempSync(path.join(outDir, '.keys-'));
  try {
    let signing: string[] = [];
    if (extension === '.ipa') {
      // A build for devices has to be signed with a key and a profile.
      signing = ['--signing-key', SIGNING_KEY, '--profile', PROFILE];
    } else if (extension === '.aab') {
      // An app bundle is signed as Google Play takes one: with an upload
      // key, which omni-build creates first, as a developer without a JDK
      // would.
      const keystore = path.join(keys, 'upload.p12');
      await run(binary, ['key', 'create', keystore, '--subject', UPLOAD_KEY], {
        log,
        env,
      });
      signing = ['--signing-key', keystore];
    }
    await run(
      binary,
      ['repackage', shell, '--config', APP_CONFIG, ...signing, '--out', output],
      { log, env },
    );
  } finally {
    rmSync(keys, { recursive: true, force: true });
  }
  return output;
};
