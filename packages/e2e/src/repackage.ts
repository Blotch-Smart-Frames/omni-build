import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { type RunOptions, run } from './process.ts';
import { MODULE, SHELL, shellOf } from './shell.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));
/**
 * The app the tests make from each shell. Each of its values differs from
 * the shell's, so an app that reports them got them from omni.
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
  webLinks: string[];
  orientation: string;
  translations: { fr: { name: string } };
  android: { permissions: string[]; abis: string[] };
  ios: {
    usageDescriptions: { NSCameraUsageDescription: string };
    usesNonExemptEncryption: boolean;
  };
} = JSON.parse(readFileSync(APP_CONFIG, 'utf8'));
/**
 * The native module the app depends on (`app/package.json`), as its own
 * package.json says what it adds to an app.
 */
const NATIVE_MODULE: {
  omniBuild: { android: { permissions: string[] } };
} = JSON.parse(readFileSync(MODULE.packageJson, 'utf8'));
/**
 * What the module says when the shell registers it (fixtures/module): its
 * framework's bundle ID on the Simulator, its class's name on Android, and
 * for a build for devices, which the simulator can't run, the module the
 * app's Info.plist lists.
 */
const MODULE_REPORTS = {
  simulator: 'OmniBuildModule, in dev.omnibuild.module',
  devices: 'OmniBuildModule',
  android: 'OmniBuildModule in dev.omnibuild.module.OmniBuildModule',
};
/** The colour of every pixel of the app's icon, `app/icon.png`. */
const APP_ICON = '#E8590C';
/**
 * The web app the tests make from the WebView shell: its own ID, name,
 * version and build, and a payload of a page (`fixtures/web-app/index.html`)
 * whose script reports what it finds to the console.
 */
const WEB_APP_CONFIG = fileURLToPath(
  new URL('../app/webview.json', import.meta.url),
);
const WEB_APP: {
  id: string;
  name: string;
  version: string;
  build: number;
  payload: string;
} = JSON.parse(readFileSync(WEB_APP_CONFIG, 'utf8'));
/**
 * The origin each platform's WebView shell serves the payload from: a host
 * Android keeps for an app's own files, and a scheme of the iOS shell's own.
 */
const WEB_ORIGINS = {
  android: 'https://appassets.androidplatform.net',
  ios: 'app://localhost',
};
/**
 * The link the tests open each Android app with, which only the app's own
 * URL scheme leads to it. iOS asks its user before it opens an app's link,
 * which no one is there to answer, so the iOS apps report their schemes.
 */
export const OPEN_URL = `${APP.urlSchemes[0]}://report`;
/** The orientations an iPhone turns the app to: `landscape`'s. */
const IPHONE_ORIENTATIONS =
  'UIInterfaceOrientationLandscapeLeft, UIInterfaceOrientationLandscapeRight';
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
 * A profile for each of the app's extensions, whose IDs are under the
 * app's: the app's profile is for the app's ID alone, as a profile without
 * a wildcard App ID is.
 */
const EXTENSION_PROFILE = fileURLToPath(
  new URL('../app/ShellNotifications.mobileprovision', import.meta.url),
);
const INTENTS_EXTENSION_PROFILE = fileURLToPath(
  new URL('../app/ShellIntents.mobileprovision', import.meta.url),
);
/**
 * A profile for the app's App Clip, as Apple makes one for an App ID
 * registered as an App Clip's: it allows the entitlements that make the
 * App Clip one, and name the app as its parent.
 */
const APP_CLIP_PROFILE = fileURLToPath(
  new URL('../app/ShellClip.mobileprovision', import.meta.url),
);
const WATCH_APP_PROFILE = fileURLToPath(
  new URL('../app/ShellWatch.mobileprovision', import.meta.url),
);
const WATCH_WIDGETS_PROFILE = fileURLToPath(
  new URL('../app/ShellWatchWidgets.mobileprovision', import.meta.url),
);
/** The web app's profile, for its ID, made as the others are. */
const WEB_APP_PROFILE = fileURLToPath(
  new URL('../app/WebView.mobileprovision', import.meta.url),
);
/**
 * The extensions in the iOS shell, under the shell's ID and the app's: an
 * app extension in PlugIns/, and an ExtensionKit extension in Extensions/.
 */
const EXTENSION = 'ShellNotifications';
const INTENTS_EXTENSION = 'ShellIntents';
/**
 * The App Clip's and the watch app's IDs, under the shell's and the app's,
 * as Xcode names them.
 */
const APP_CLIP = 'Clip';
const WATCH_APP = 'watchkitapp';
/** The watch app's widgets' ID, under the watch app's. */
const WATCH_WIDGETS = `${WATCH_APP}.ShellWatchWidgets`;
const SIGNER = 'OmniBuild E2E';
const TEAM = 'OMNIBUILD0';
/** The keystores' password, the iOS key's and the upload key's. */
const KEY_PASSWORD = 'omnibuild-e2e';
/** The subject of the upload key omni creates for an app bundle. */
const UPLOAD_KEY = 'CN=OmniBuild E2E Upload';

/** The OS a repackaged app's file name says it was repackaged on. */
const OS_NAMES: Partial<Record<NodeJS.Platform, string>> = {
  darwin: 'macos',
  linux: 'linux',
  win32: 'windows',
};

/**
 * The only environment variables omni gets: where the user's home and
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
      process.platform === 'win32' ? 'omni.exe' : 'omni',
    );
  if (!existsSync(binary)) {
    throw new Error(
      `There's no omni binary at ${binary}. Build one with \`npm run build -w @blotch/omni-build-cli\`, or set OMNI_BUILD_BINARY to one.`,
    );
  }
  return binary;
};

/**
 * Runs an omni command as a build tool would, with `--json`, and
 * checks that the event it ends with, its result, has the `expected`
 * values: so the stages check what omni reports, as well as what it
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
      `omni ended with ${last || 'nothing'}, where ${wrong
        .map(([name, value]) => `${name} should be ${JSON.stringify(value)}`)
        .join(' and ')}`,
    );
  }
};

/**
 * What signed an Android app: the upload key omni created for an app bundle,
 * and for an APK the debug key it creates and signs with when it's given no
 * key. Only omni could have made either.
 */
const androidSigner = (app: string) =>
  app.endsWith('.aab') ? UPLOAD_KEY : 'CN=OmniBuild Debug, O=OmniBuild';

/**
 * What an app made from the WebView shell must report, and what signed it.
 * The page reports what it found: its title, from the payload's index.html
 * rather than the shell's placeholder, the origin the shell serves it from,
 * that storage works there, and a file beside it, which it read as a web app
 * reads its data. The harness adds the ID of the app that logged it. A build
 * for devices, which the simulator can't run, gets what its files say.
 */
const expectedWebApp = (
  platform: 'ios' | 'android',
  app: string,
): Record<string, string> => {
  const payload = path.join(path.dirname(WEB_APP_CONFIG), WEB_APP.payload);
  const message = readFileSync(path.join(payload, 'message.txt'), 'utf8');
  if (app.endsWith('.ipa')) {
    return {
      id: WEB_APP.id,
      name: WEB_APP.name,
      version: WEB_APP.version,
      build: String(WEB_APP.build),
      message,
      signer: SIGNER,
      applicationIdentifier: `${TEAM}.${WEB_APP.id}`,
    };
  }
  return {
    id: WEB_APP.id,
    title: readFileSync(path.join(payload, 'index.html'), 'utf8').match(
      /<title>(.*)<\/title>/,
    )![1],
    origin: WEB_ORIGINS[platform],
    storage: 'works',
    message,
    signer: platform === 'android' ? androidSigner(app) : 'ad hoc',
  };
};

/**
 * What an app must report after the repackage stage, and what signed it:
 * the app config's values where omni rewrites them, and the shell's
 * own elsewhere.
 */
export const expected = (
  platform: 'ios' | 'android',
  app = '',
): Record<string, string> => {
  if (shellOf(app) === 'WebViewShell') {
    return expectedWebApp(platform, app);
  }
  const values = {
    id: APP.id,
    name: APP.name,
    // In French, which the shell has no name in.
    frenchName: APP.translations.fr.name,
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
      // The app's and its native module's, sorted as the app lists them,
      // and the one the shell declares for itself, under the app's ID.
      permissions: [
        ...APP.android.permissions,
        ...NATIVE_MODULE.omniBuild.android.permissions,
      ]
        .map((name) => `android.permission.${name}`)
        .concat(`${APP.id}.permission.SHELL`)
        .sort()
        .join(', '),
      abis: [...APP.android.abis].sort().join(', '),
      openedWith: OPEN_URL,
      // The hosts of the launcher activity's App Links, as aapt2 reads
      // them, whose website Android verifies.
      webLinks: APP.webLinks.join(', '),
      // `landscape`, either side up.
      orientation: 'userLandscape',
      modules: MODULE_REPORTS.android,
      signer: androidSigner(app),
    };
  }
  const cameraUsage = APP.ios.usageDescriptions.NSCameraUsageDescription;
  // What the app tells App Store Connect about its encryption, where the
  // shell says nothing.
  const nonExemptEncryption = String(APP.ios.usesNonExemptEncryption);
  if (app.endsWith('.ipa')) {
    // A build for devices, which the simulator can't run: what its files
    // say, but for the icon, which only iOS reads out of Assets.car, and
    // what the profile allowed its signature.
    return {
      id: values.id,
      name: values.name,
      frenchName: values.frenchName,
      version: values.version,
      build: values.build,
      payload: values.payload,
      cameraUsage,
      modules: MODULE_REPORTS.devices,
      signer: SIGNER,
      applicationIdentifier: `${TEAM}.${APP.id}`,
      // What app.json claims, which the profile allows: the push
      // environment is the profile's, the associated domains are its shared
      // web credentials' and then its web links', and the keychain group
      // has the team's prefix in place of $(AppIdentifierPrefix).
      pushEnvironment: 'development',
      associatedDomains: [
        'webcredentials:e2e.omnibuild.example',
        ...APP.webLinks.map((host) => `applinks:${host}`),
      ].join(', '),
      appGroups: 'group.dev.omnibuild.e2e',
      keychainGroups: `${TEAM}.dev.omnibuild.e2e.shared`,
      urlSchemes: APP.urlSchemes.join(', '),
      orientations: IPHONE_ORIENTATIONS,
      nonExemptEncryption,
      extensionId: `${APP.id}.${EXTENSION}`,
      extensionApplicationIdentifier: `${TEAM}.${APP.id}.${EXTENSION}`,
      // The App Group the extension shares with the app, as app.json's
      // ios.bundles claims it for the extension.
      extensionAppGroups: 'group.dev.omnibuild.e2e',
      intentsExtensionId: `${APP.id}.${INTENTS_EXTENSION}`,
      intentsExtensionApplicationIdentifier: `${TEAM}.${APP.id}.${INTENTS_EXTENSION}`,
      appClipId: `${APP.id}.${APP_CLIP}`,
      appClipApplicationIdentifier: `${TEAM}.${APP.id}.${APP_CLIP}`,
      // The App Clip and the app name each other in their signatures.
      appClipParent: `${TEAM}.${APP.id}`,
      associatedAppClip: `${TEAM}.${APP.id}.${APP_CLIP}`,
      // The watch app pairs with the app.
      watchAppId: `${APP.id}.${WATCH_APP}`,
      watchCompanion: APP.id,
      watchAppApplicationIdentifier: `${TEAM}.${APP.id}.${WATCH_APP}`,
      watchWidgetsId: `${APP.id}.${WATCH_WIDGETS}`,
      watchWidgetsApplicationIdentifier: `${TEAM}.${APP.id}.${WATCH_WIDGETS}`,
    };
  }
  // Signed ad hoc, as Xcode signs a simulator build, and its framework,
  // which only loads with a signature of its own, too. Its extensions' IDs
  // are under the app's, or the simulator wouldn't install it, and its App
  // Clip's, as Apple requires.
  return {
    ...values,
    cameraUsage,
    framework: SHELL.framework,
    urlSchemes: APP.urlSchemes.join(', '),
    orientations: IPHONE_ORIENTATIONS,
    nonExemptEncryption,
    extensionId: `${APP.id}.${EXTENSION}`,
    intentsExtensionId: `${APP.id}.${INTENTS_EXTENSION}`,
    appClipId: `${APP.id}.${APP_CLIP}`,
    watchAppId: `${APP.id}.${WATCH_APP}`,
    watchCompanion: APP.id,
    watchWidgetsId: `${APP.id}.${WATCH_WIDGETS}`,
    modules: MODULE_REPORTS.simulator,
    signer: 'ad hoc',
  };
};

/**
 * Installs the native module's binaries, as the Module workflow writes
 * them (`ios/`, `android/`), for the app, beside its package.json, as npm
 * would install the module's package: its package.json, and the binaries
 * it names.
 */
export const installModule = (module: string) => {
  if (!existsSync(module)) {
    throw new Error(
      `There's no native module in ${module}: run the build stage first, or give --module the folder the Module workflow wrote.`,
    );
  }
  const installed = path.join(
    path.dirname(APP_CONFIG),
    'node_modules',
    MODULE.packageName,
  );
  rmSync(installed, { recursive: true, force: true });
  cpSync(module, installed, { recursive: true });
  copyFileSync(MODULE.packageJson, path.join(installed, 'package.json'));
};

/**
 * The repackage stage: what OmniBuild users do, on any OS, with nothing but
 * the omni binary. Writes the result into `outDir`, named for the OS
 * it ran on, and returns its path.
 */
export const repackage = async ({
  shell,
  outDir,
  module,
  log,
}: {
  shell: string;
  outDir: string;
  /** The native module the fixture's app gets (see installModule()). */
  module: string;
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
  const webApp = shellOf(shell) === 'WebViewShell';
  if (!webApp) {
    installModule(module);
  }
  // A hidden folder, which no stage takes for an app.
  const keys = mkdtempSync(path.join(outDir, '.keys-'));
  try {
    let signing: string[] = [];
    if (extension === '.ipa') {
      // A build for devices has to be signed with a key and a profile, and
      // each of the fixture's extensions, the watch app, its widgets and
      // the App Clip with one of its own. The web app has none of those.
      signing = webApp
        ? ['--signing-key', SIGNING_KEY, '--profile', WEB_APP_PROFILE]
        : [
            '--signing-key',
            SIGNING_KEY,
            '--profile',
            PROFILE,
            '--profile',
            EXTENSION_PROFILE,
            '--profile',
            INTENTS_EXTENSION_PROFILE,
            '--profile',
            APP_CLIP_PROFILE,
            '--profile',
            WATCH_APP_PROFILE,
            '--profile',
            WATCH_WIDGETS_PROFILE,
          ];
    } else if (extension === '.aab') {
      // An app bundle is signed as Google Play takes one: with an upload
      // key, which omni creates first, as a developer without a JDK
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
      [
        'repackage',
        shell,
        '--config',
        webApp ? WEB_APP_CONFIG : APP_CONFIG,
        ...signing,
        '--out',
        output,
      ],
      { event: 'repackaged', out: output },
      { log, env },
    );
  } finally {
    rmSync(keys, { recursive: true, force: true });
  }
  return output;
};
