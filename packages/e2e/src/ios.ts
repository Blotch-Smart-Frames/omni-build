import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

import type { Artifacts, Platform } from './run.ts';
import { type RunOptions, poll, run } from './process.ts';
import { SHELLS, type ShellName, XCODE_PROJECTS } from './shell.ts';

/** The simulator the tests create and reuse, so a developer's own stay untouched. */
const DEVICE_NAME = 'OmniBuild E2E';
/**
 * Where the fixture app saves its report, in its data container
 * (ShellReport.swift).
 */
const REPORT_FILE = 'Documents/omni-build-e2e.json';
/**
 * The category the WebView shell logs its page's console under, in the
 * unified log, where the web app's report goes (ConsoleHandler.swift).
 */
const CONSOLE_CATEGORY = 'console';

const simctl = (args: string[], options: RunOptions) =>
  run('xcrun', ['simctl', ...args], options);

/** What the tests read of `simctl list --json`. */
interface SimctlList {
  runtimes: {
    identifier: string;
    name: string;
    isAvailable: boolean;
    supportedDeviceTypes?: { identifier: string; productFamily: string }[];
  }[];
  /** By runtime identifier. */
  devices: Record<string, { name: string; udid: string }[] | undefined>;
}

/**
 * Where each SDK's builds run: anywhere, on a simulator or a device. The
 * destination alone picks the SDK, so that each target builds for its own
 * platform: the watch app for watchOS, where `-sdk iphoneos` would build it
 * for iOS.
 */
const DESTINATIONS = {
  iphonesimulator: 'generic/platform=iOS Simulator',
  iphoneos: 'generic/platform=iOS',
};

/**
 * Builds a shell with Xcode for one SDK, and returns the .app's path. Each
 * shell's scheme, and its .app, are named after it.
 */
const xcodebuild = async (
  shell: ShellName,
  sdk: keyof typeof DESTINATIONS,
  { buildDir, log }: { buildDir: string; log: string },
  settings: string[] = [],
) => {
  const derivedData = path.join(buildDir, 'ios');
  await run(
    'xcodebuild',
    [
      'build',
      '-project',
      XCODE_PROJECTS[shell],
      '-scheme',
      shell,
      '-configuration',
      'Release',
      '-destination',
      DESTINATIONS[sdk],
      '-derivedDataPath',
      derivedData,
      ...settings,
    ],
    { log },
  );
  return path.join(
    derivedData,
    `Build/Products/Release-${sdk}`,
    `${shell}.app`,
  );
};

/**
 * Builds each shell with Xcode for the simulator and for devices, zips each
 * into `outDir`, and returns their paths: the simulator's a zipped .app,
 * and the devices' an .ipa, a zip of a Payload folder with the .app.
 */
const build: Platform['build'] = async ({ buildDir, outDir, log }) => {
  mkdirSync(outDir, { recursive: true });
  const shells: string[] = [];
  for (const shell of SHELLS) {
    const simulator = await xcodebuild(shell, 'iphonesimulator', {
      buildDir,
      log,
    });
    // Unsigned, as CI has no certificate Apple issued, but with room for a
    // signature, which the linker reserves when asked: omni replaces
    // a signature, and never adds one.
    const device = await xcodebuild(shell, 'iphoneos', { buildDir, log }, [
      'CODE_SIGNING_ALLOWED=NO',
      'OTHER_LDFLAGS=-Wl,-adhoc_codesign',
    ]);

    // A .app is a folder, and its executable's mode wouldn't survive a
    // workflow artifact or a Windows file system on the way to the
    // repackage stage. So the shells travel as zips, which ditto writes
    // with every file's mode.
    const zip = path.join(outDir, `${shell}.zip`);
    const ipa = path.join(outDir, `${shell}.ipa`);
    rmSync(zip, { force: true });
    rmSync(ipa, { force: true });
    await run('ditto', ['-c', '-k', '--keepParent', simulator, zip], { log });
    const payload = path.join(buildDir, 'ios-ipa', 'Payload');
    rmSync(payload, { recursive: true, force: true });
    await run('ditto', [device, path.join(payload, `${shell}.app`)], { log });
    await run('ditto', ['-c', '-k', '--keepParent', payload, ipa], { log });
    shells.push(zip, ipa);
  }
  return shells;
};

/**
 * Unzips an app into a folder of its own, passes `use` the .app (at the
 * top of the zip, or in Payload/ in an .ipa), and deletes the folder.
 */
const withApp = async <T>(
  app: string,
  log: string,
  use: (bundle: string) => Promise<T>,
) => {
  const unzipped = mkdtempSync(path.join(os.tmpdir(), 'omni-build-e2e-'));
  try {
    await run('ditto', ['-x', '-k', app, unzipped], { log });
    const folder = app.endsWith('.ipa')
      ? path.join(unzipped, 'Payload')
      : unzipped;
    const bundle = readdirSync(folder).find((name) => name.endsWith('.app'));
    if (!bundle) {
      throw new Error(`${app} has no .app inside.`);
    }
    return await use(path.join(folder, bundle));
  } finally {
    rmSync(unzipped, { recursive: true, force: true });
  }
};

/**
 * Checks an app's signature with codesign, as strictly as it checks one
 * for the App Store, and returns what signed it. A build for devices
 * can't run on the simulator, so for one it reads what the app's files
 * say instead, which the harness checks in place of what the app reports.
 */
const verify: Platform['verify'] = ({ app, shell, log }) =>
  withApp(app, log, async (bundle) => {
    await run(
      'codesign',
      ['--verify', '--strict', '--deep', '--verbose=2', bundle],
      { log },
    );
    // codesign prints what it displays to stderr.
    const { output } = await run(
      'codesign',
      ['--display', '--verbose=2', bundle],
      { log },
    );
    const signer =
      output.match(/^Authority=(.*)$/m)?.[1] ??
      (output.includes('Signature=adhoc') ? 'ad hoc' : undefined);
    if (!app.endsWith('.ipa')) {
      return { signer };
    }
    const info = JSON.parse(
      (
        await run(
          'plutil',
          ['-convert', 'json', '-o', '-', path.join(bundle, 'Info.plist')],
          { log },
        )
      ).stdout,
    );
    // The entitlements in a bundle's signature, and what one of them claims
    // for a key: a string, or an array of them, joined.
    const entitlements = async (signed: string) =>
      (
        await run(
          'codesign',
          ['--display', '--entitlements', '-', '--xml', signed],
          { log },
        )
      ).stdout;
    const claimed = (xml: string, key: string) => {
      const value =
        xml.match(
          new RegExp(
            `<key>${key.replaceAll('.', '\\.')}</key>\\s*(<string>.*?</string>|<array>[^]*?</array>)`,
          ),
        )?.[1] ?? '';
      return (
        [...value.matchAll(/<string>(.*?)<\/string>/g)]
          .map(([, string]) => string)
          .join(', ') || undefined
      );
    };
    // The web app: its values, a file from its payload, and the App ID its
    // profile gave its signature. The WebView shell has no extensions.
    if (shell === 'WebViewShell') {
      return {
        signer,
        report: {
          id: info.CFBundleIdentifier,
          name: info.CFBundleDisplayName ?? info.CFBundleName,
          version: info.CFBundleShortVersionString,
          build: info.CFBundleVersion,
          message: readFileSync(path.join(bundle, 'message.txt'), 'utf8'),
          applicationIdentifier: claimed(
            await entitlements(bundle),
            'application-identifier',
          ),
        },
      };
    }
    // The bundle of this kind in one of the app's folders for them: its
    // Info.plist, and the entitlements its profile allowed its signature.
    const bundleIn = async (folder: string, kind: string) => {
      const nested = existsSync(folder)
        ? readdirSync(folder).find((name) => name.endsWith(kind))
        : undefined;
      if (!nested) {
        return undefined;
      }
      const signed = path.join(folder, nested);
      const info: Record<string, string | undefined> = JSON.parse(
        (
          await run(
            'plutil',
            ['-convert', 'json', '-o', '-', path.join(signed, 'Info.plist')],
            { log },
          )
        ).stdout,
      );
      return {
        folder: signed,
        id: info.CFBundleIdentifier,
        info,
        entitlements: await entitlements(signed),
      };
    };
    // An app extension, an ExtensionKit extension, which Xcode puts in a
    // folder of its own, an App Clip and a watch app, and the watch app's
    // widgets, an extension in its own PlugIns/.
    const plugIn = await bundleIn(path.join(bundle, 'PlugIns'), '.appex');
    const intents = await bundleIn(path.join(bundle, 'Extensions'), '.appex');
    const appClip = await bundleIn(path.join(bundle, 'AppClips'), '.app');
    const watchApp = await bundleIn(path.join(bundle, 'Watch'), '.app');
    const watchWidgets =
      watchApp &&
      (await bundleIn(path.join(watchApp.folder, 'PlugIns'), '.appex'));
    const own = await entitlements(bundle);
    // A translation of the app's Info.plist, in its language's folder, if
    // the app has one.
    const translation = async (
      language: string,
    ): Promise<Record<string, string | undefined>> => {
      const strings = path.join(
        bundle,
        `${language}.lproj`,
        'InfoPlist.strings',
      );
      return existsSync(strings)
        ? JSON.parse(
            (
              await run('plutil', ['-convert', 'json', '-o', '-', strings], {
                log,
              })
            ).stdout,
          )
        : {};
    };
    // What iOS shows in English: the strings the app has in English, over
    // Info.plist's.
    const shown = { ...info, ...(await translation('en')) };
    return {
      signer,
      report: {
        id: info.CFBundleIdentifier,
        name: shown.CFBundleDisplayName ?? shown.CFBundleName,
        // The name on an iPhone set to French.
        frenchName: (await translation('fr')).CFBundleDisplayName,
        cameraUsage: shown.NSCameraUsageDescription,
        version: info.CFBundleShortVersionString,
        build: info.CFBundleVersion,
        payload: readFileSync(path.join(bundle, 'payload.txt'), 'utf8'),
        applicationIdentifier: claimed(own, 'application-identifier'),
        // What the app claims for its capabilities, as its profile allowed.
        pushEnvironment: claimed(own, 'aps-environment'),
        associatedDomains: claimed(
          own,
          'com.apple.developer.associated-domains',
        ),
        appGroups: claimed(own, 'com.apple.security.application-groups'),
        keychainGroups: claimed(own, 'keychain-access-groups'),
        urlSchemes: (
          (info.CFBundleURLTypes ?? []) as { CFBundleURLSchemes?: string[] }[]
        )
          .flatMap((type) => type.CFBundleURLSchemes ?? [])
          .join(', '),
        // The orientations an iPhone turns the app to: its own
        // (`~iphone`), over those for every device, as iOS reads them.
        orientations: (
          (info['UISupportedInterfaceOrientations~iphone'] ??
            info.UISupportedInterfaceOrientations) as string[] | undefined
        )?.join(', '),
        // Whether it uses encryption that export regulations don't exempt.
        nonExemptEncryption: (
          info.ITSAppUsesNonExemptEncryption as boolean | undefined
        )?.toString(),
        extensionId: plugIn?.id,
        extensionApplicationIdentifier: claimed(
          plugIn?.entitlements ?? '',
          'application-identifier',
        ),
        // The App Group it shares with the app, which it claims itself.
        extensionAppGroups: claimed(
          plugIn?.entitlements ?? '',
          'com.apple.security.application-groups',
        ),
        intentsExtensionId: intents?.id,
        intentsExtensionApplicationIdentifier: claimed(
          intents?.entitlements ?? '',
          'application-identifier',
        ),
        appClipId: appClip?.id,
        appClipApplicationIdentifier: claimed(
          appClip?.entitlements ?? '',
          'application-identifier',
        ),
        // The App Clip and the app each name the other.
        appClipParent: claimed(
          appClip?.entitlements ?? '',
          'com.apple.developer.parent-application-identifiers',
        ),
        associatedAppClip: claimed(
          own,
          'com.apple.developer.associated-appclip-app-identifiers',
        ),
        // The watch app, and the app it pairs with.
        watchAppId: watchApp?.id,
        watchCompanion: watchApp?.info.WKCompanionAppBundleIdentifier,
        watchAppApplicationIdentifier: claimed(
          watchApp?.entitlements ?? '',
          'application-identifier',
        ),
        // Its widgets, under the watch app.
        watchWidgetsId: watchWidgets?.id,
        watchWidgetsApplicationIdentifier: claimed(
          watchWidgets?.entitlements ?? '',
          'application-identifier',
        ),
      },
    };
  });

/**
 * Boots the E2E simulator, first creating it on the newest iOS runtime, and
 * returns its UDID. On GitHub's runners a simulator sometimes fails to boot
 * ("Data Migration Failed"), so it falls back to an older runtime when that
 * happens.
 */
const bootSimulator = async ({ log, wait }: { log: string; wait: boolean }) => {
  const list: SimctlList = JSON.parse(
    (await simctl(['list', '--json'], { log })).stdout,
  );
  const runtimes = list.runtimes
    .filter(
      (runtime) => runtime.isAvailable && runtime.identifier.includes('iOS'),
    )
    .reverse();
  if (runtimes.length === 0) {
    throw new Error(
      'No iOS Simulator runtime is installed. Install one in Xcode → Settings → Components.',
    );
  }

  for (const runtime of runtimes) {
    // Each runtime runs only some device types, and the full device type
    // list isn't in release order, so take an iPhone this runtime supports.
    const deviceType = runtime.supportedDeviceTypes?.find(
      (type) => type.productFamily === 'iPhone',
    );
    if (!deviceType) {
      continue;
    }
    const existing = list.devices[runtime.identifier]?.find(
      (device) => device.name === DEVICE_NAME,
    );
    const udid =
      existing?.udid ??
      (
        await simctl(
          ['create', DEVICE_NAME, deviceType.identifier, runtime.identifier],
          { log },
        )
      ).stdout.trim();
    // Fails harmlessly when an earlier call already started the boot, or
    // the simulator is already running.
    await simctl(['boot', udid], { log, allowFailure: true });
    if (!wait) {
      return udid;
    }
    const status = await simctl(['bootstatus', udid, '-b'], {
      log,
      allowFailure: true,
    });
    if (status.code === 0) {
      console.log(`Device: ${DEVICE_NAME} on ${runtime.name} (${udid})`);
      return udid;
    }
    console.warn(
      `! ${DEVICE_NAME} failed to boot on ${runtime.name}; trying an older runtime.`,
    );
    await simctl(['delete', udid], { log, allowFailure: true });
  }
  throw new Error(
    `${DEVICE_NAME} failed to boot on every installed iOS runtime.`,
  );
};

/**
 * Saves what the app logged, and any crash report, for a run in which it
 * never reported. Its process, and its crash reports, are named after the
 * shell's .app.
 */
const saveAppLogs = async ({
  udid,
  shell,
  artifactsDir,
  log,
  since,
}: Artifacts & { udid: string; shell: ShellName; since: number }) => {
  const appLog = await simctl(
    [
      'spawn',
      udid,
      'log',
      'show',
      '--last',
      '10m',
      '--style',
      'compact',
      '--predicate',
      `process == "${shell}"`,
    ],
    { log, allowFailure: true },
  );
  writeFileSync(path.join(artifactsDir, 'app.log'), appLog.stdout);

  // Simulator apps' crash reports go to the Mac's own folder.
  const reports = path.join(os.homedir(), 'Library/Logs/DiagnosticReports');
  if (existsSync(reports)) {
    for (const name of readdirSync(reports)) {
      const file = path.join(reports, name);
      if (name.startsWith(shell) && statSync(file).mtimeMs >= since) {
        copyFileSync(file, path.join(artifactsDir, name));
      }
    }
  }
};

/** An entry of the unified log, as `log show --style ndjson` prints it. */
interface LogEntry {
  subsystem?: string;
  eventMessage?: string;
}

/**
 * The report the web app's page logs to its console, which the WebView
 * shell sends to the unified log, from the process `pid`. The ID is the
 * log's subsystem, which the shell sets to its bundle's ID as it runs.
 */
const webAppReport = async (udid: string, pid: string) => {
  // No log: this runs every half second until the report is there.
  const shown = await simctl(
    [
      'spawn',
      udid,
      'log',
      'show',
      '--last',
      '5m',
      '--style',
      'ndjson',
      '--predicate',
      `processID == ${pid} AND category == "${CONSOLE_CATEGORY}"`,
    ],
    { allowFailure: true },
  );
  const entry = shown.stdout
    .split('\n')
    .filter((line) => line.startsWith('{'))
    .map((line) => JSON.parse(line) as LogEntry)
    .find((candidate) => candidate.eventMessage?.startsWith('{'));
  return entry && { ...JSON.parse(entry.eventMessage!), id: entry.subsystem };
};

/**
 * Installs the zipped app on the simulator, launches it, and returns the
 * report it makes on launch: the one the fixture saves in its data
 * container, or the one the WebView shell's page logs.
 */
const runApp: Platform<string, string>['runApp'] = async ({
  device: udid,
  app,
  appId,
  shell,
  artifactsDir,
  log,
}) => {
  const since = Date.now();
  await withApp(app, log, async (bundle) => {
    // A fresh install empties the data container, so the report can't be
    // one an earlier run left there.
    await simctl(['uninstall', udid, appId], { log, allowFailure: true });
    await simctl(['install', udid, bundle], { log });
  });
  let read: () => Promise<Record<string, unknown> | undefined>;
  if (shell === 'WebViewShell') {
    // simctl names the process it started, "dev.omnibuild.e2e.web: 41657",
    // so the report can only be this launch's.
    const launched = await simctl(['launch', udid, appId], { log });
    const pid = /: (\d+)\s*$/.exec(launched.stdout)?.[1];
    if (!pid) {
      throw new Error(
        `simctl launched ${appId} without naming its process: ${launched.stdout.trim()}`,
      );
    }
    read = () => webAppReport(udid, pid);
  } else {
    const container = (
      await simctl(['get_app_container', udid, appId, 'data'], { log })
    ).stdout.trim();
    await simctl(['launch', udid, appId], { log });
    const reportFile = path.join(container, REPORT_FILE);
    read = async () =>
      existsSync(reportFile)
        ? JSON.parse(readFileSync(reportFile, 'utf8'))
        : undefined;
  }

  try {
    const report = await poll(read, {
      timeoutMs: 60_000,
      message: `${appId} didn't report within a minute of launching: it may have crashed. Its log and any crash report are in ${artifactsDir}.`,
    });
    // The report comes before the first frame, or as the page has loaded;
    // give the view a moment to draw it.
    await sleep(1000);
    await simctl(
      ['io', udid, 'screenshot', path.join(artifactsDir, 'screenshot.png')],
      { log },
    );
    return report;
  } catch (error) {
    await saveAppLogs({ udid, shell, artifactsDir, log, since });
    throw error;
  }
};

/**
 * Building the iOS shells and running them on the iOS Simulator. See run.ts.
 * The device is the simulator's UDID.
 */
export const ios: Platform<string, string> = {
  name: 'iOS',
  toolchain: 'Xcode',
  build,
  verify,
  // Booting is the slowest part, so it starts first and goes on while the
  // other stages work.
  startDevice: ({ log }) => bootSimulator({ log, wait: false }),
  waitForDevice: (_started, { log }) => bootSimulator({ log, wait: true }),
  runApp,
  stopDevice: (udid, { log }) =>
    simctl(['shutdown', udid], { log, allowFailure: true }),
};
