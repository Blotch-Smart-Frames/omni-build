import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { type RunOptions, run } from './process.ts';
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
  urlSchemes: string[];
  android: { permissions: string[]; abis: string[] };
  ios: { usageDescriptions: { NSCameraUsageDescription: string } };
} = JSON.parse(readFileSync(APP_CONFIG, 'utf8'));
/** The colour of every pixel of the app's icon, `app/icon.png`. */
const APP_ICON = '#E8590C';
/**
 * The link the tests open each Android app with, which only the app's own
 * URL scheme leads to it. iOS asks its user before it opens an app's link,
 * which no one is there to answer, so the iOS apps report their schemes.
 */
export const OPEN_URL = `${APP.urlSchemes[0]}://report`;
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
/**
 * A profile for the app's extension, whose ID is under the app's: the
 * app's profile is for the app's ID alone, as a profile without a wildcard
 * App ID is.
 */
const EXTENSION_PROFILE = fileURLToPath(
  new URL('../app/ShellNotifications.mobileprovision', import.meta.url),
);
/** The app extension in the iOS shell, under the shell's ID and the app's. */
const EXTENSION = 'ShellNotifications';
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
 * Runs an omni-build command as a build tool would, with `--json`, and
 * checks that the event it ends with, its result, has the `expected`
 * values: so the stages check what omni-build reports, as well as what it
 * makes.
 */
export const omniBuild = async (
  args: string[],
  expected: Record<string, string>,
  options: RunOptions,
) => {
  const { stdout } = await run(omniBuildBinary(), [...args, '--json'], options);
  const last = stdout.trimEnd().split('\n').at(-1)!;
  let event: Record<string, unknown> = {};
  try {
    event = JSON.parse(last);
  } catch {
    // Wrong in every value, as the error below says.
  }
  const wrong = Object.entries(expected).filter(
    ([name, value]) => event[name] !== value,
  );
  if (wrong.length > 0) {
    throw new Error(
      `omni-build ended with ${last || 'nothing'}, where ${wrong
        .map(([name, value]) => `${name} should be ${JSON.stringify(value)}`)
        .join(' and ')}`,
    );
  }
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
      // Under the app's ID, as Gradle would have put them for it.
      provider: `${APP.id}.provider`,
      // The app's, sorted as the app lists them, and the one the shell
      // declares for itself, under the app's ID.
      permissions: [
        ...APP.android.permissions.map((name) => `android.permission.${name}`),
        `${APP.id}.permission.SHELL`,
      ]
        .sort()
        .join(', '),
      abis: [...APP.android.abis].sort().join(', '),
      openedWith: OPEN_URL,
      // An app bundle is signed with the upload key omni-build created for
      // it, and an APK with the debug key it creates and signs with when
      // it's given no key: only omni-build could have made either.
      signer: app.endsWith('.aab')
        ? UPLOAD_KEY
        : 'CN=OmniBuild Debug, O=OmniBuild',
    };
  }
  const cameraUsage = APP.ios.usageDescriptions.NSCameraUsageDescription;
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
      cameraUsage,
      signer: SIGNER,
      applicationIdentifier: `${TEAM}.${APP.id}`,
      urlSchemes: APP.urlSchemes.join(', '),
      extensionId: `${APP.id}.${EXTENSION}`,
      extensionApplicationIdentifier: `${TEAM}.${APP.id}.${EXTENSION}`,
    };
  }
  // Signed ad hoc, as Xcode signs a simulator build, and its framework,
  // which only loads with a signature of its own, too. Its extension's ID
  // is under the app's, or the simulator wouldn't install it.
  return {
    ...values,
    cameraUsage,
    framework: SHELL.framework,
    urlSchemes: APP.urlSchemes.join(', '),
    extensionId: `${APP.id}.${EXTENSION}`,
    signer: 'ad hoc',
  };
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
  const env = { ...toolchainFree(), OMNI_BUILD_KEY_PASSWORD: KEY_PASSWORD };
  // A hidden folder, which no stage takes for an app.
  const keys = mkdtempSync(path.join(outDir, '.keys-'));
  try {
    let signing: string[] = [];
    if (extension === '.ipa') {
      // A build for devices has to be signed with a key and a profile, and
      // its extension with one of its own.
      signing = [
        '--signing-key',
        SIGNING_KEY,
        '--profile',
        PROFILE,
        '--profile',
        EXTENSION_PROFILE,
      ];
    } else if (extension === '.aab') {
      // An app bundle is signed as Google Play takes one: with an upload
      // key, which omni-build creates first, as a developer without a JDK
      // would.
      const keystore = path.join(keys, 'upload.p12');
      await omniBuild(
        ['key', 'create', keystore, '--subject', UPLOAD_KEY],
        { event: 'key-created', path: keystore },
        { log, env },
      );
      signing = ['--signing-key', keystore];
    }
    await omniBuild(
      ['repackage', shell, '--config', APP_CONFIG, ...signing, '--out', output],
      { event: 'repackaged', out: output },
      { log, env },
    );
  } finally {
    rmSync(keys, { recursive: true, force: true });
  }
  return output;
};
