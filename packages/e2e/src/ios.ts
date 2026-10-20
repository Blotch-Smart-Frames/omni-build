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
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';

import type { Artifacts, Platform } from './run.ts';
import { type RunOptions, poll, run } from './process.ts';

const PROJECT = fileURLToPath(
  new URL('../fixtures/ios/OmniBuildShell.xcodeproj', import.meta.url),
);
/** The scheme, and the name of the .app and of its process. */
const SHELL = 'OmniBuildShell';
/** The simulator the tests create and reuse, so a developer's own stay untouched. */
const DEVICE_NAME = 'OmniBuild E2E';
/** Where the app saves its report, in its data container (ShellReport.swift). */
const REPORT_FILE = 'Documents/omni-build-e2e.json';

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

/** Where each SDK's builds run: anywhere, on a simulator or a device. */
const DESTINATIONS = {
  iphonesimulator: 'generic/platform=iOS Simulator',
  iphoneos: 'generic/platform=iOS',
};

/** Builds the shell with Xcode for one SDK, and returns the .app's path. */
const xcodebuild = async (
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
      PROJECT,
      '-scheme',
      SHELL,
      '-configuration',
      'Release',
      '-sdk',
      sdk,
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
    `${SHELL}.app`,
  );
};

/**
 * Builds the shell with Xcode for the simulator and for devices, zips each
 * into `outDir`, and returns their paths: the simulator's a zipped .app,
 * and the devices' an .ipa, a zip of a Payload folder with the .app.
 */
const build: Platform['build'] = async ({ buildDir, outDir, log }) => {
  const simulator = await xcodebuild('iphonesimulator', { buildDir, log });
  // Unsigned, as CI has no certificate Apple issued, but with room for a
  // signature, which the linker reserves when asked: omni-build replaces
  // a signature, and never adds one.
  const device = await xcodebuild('iphoneos', { buildDir, log }, [
    'CODE_SIGNING_ALLOWED=NO',
    'OTHER_LDFLAGS=-Wl,-adhoc_codesign',
  ]);

  // A .app is a folder, and its executable's mode wouldn't survive a
  // workflow artifact or a Windows file system on the way to the repackage
  // stage. So the shells travel as zips, which ditto writes with every
  // file's mode.
  mkdirSync(outDir, { recursive: true });
  const zip = path.join(outDir, `${SHELL}.zip`);
  const ipa = path.join(outDir, `${SHELL}.ipa`);
  rmSync(zip, { force: true });
  rmSync(ipa, { force: true });
  await run('ditto', ['-c', '-k', '--keepParent', simulator, zip], { log });
  const payload = path.join(buildDir, 'ios-ipa', 'Payload');
  rmSync(payload, { recursive: true, force: true });
  await run('ditto', [device, path.join(payload, `${SHELL}.app`)], { log });
  await run('ditto', ['-c', '-k', '--keepParent', payload, ipa], { log });
  return [zip, ipa];
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
const verify: Platform['verify'] = ({ app, log }) =>
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
    // The entitlements in a bundle's signature, and the first App ID in
    // one of them, which is a string or an array of them.
    const entitlements = async (signed: string) =>
      (
        await run(
          'codesign',
          ['--display', '--entitlements', '-', '--xml', signed],
          { log },
        )
      ).stdout;
    const appId = (xml: string, key: string) =>
      xml.match(
        new RegExp(
          `<key>${key.replaceAll('.', '\\.')}</key>\\s*(?:<array>\\s*)?<string>(.*?)</string>`,
        ),
      )?.[1];
    // The bundle of this kind in one of the app's folders for them: its
    // own ID, and the entitlements its profile allowed its signature.
    const bundleIn = async (folder: string, kind: string) => {
      const nested = existsSync(folder)
        ? readdirSync(folder).find((name) => name.endsWith(kind))
        : undefined;
      if (!nested) {
        return undefined;
      }
      const signed = path.join(folder, nested);
      return {
        id: JSON.parse(
          (
            await run(
              'plutil',
              ['-convert', 'json', '-o', '-', path.join(signed, 'Info.plist')],
              { log },
            )
          ).stdout,
        ).CFBundleIdentifier,
        entitlements: await entitlements(signed),
      };
    };
    // An app extension, an ExtensionKit extension, which Xcode puts in a
    // folder of its own, and an App Clip.
    const plugIn = await bundleIn(path.join(bundle, 'PlugIns'), '.appex');
    const intents = await bundleIn(path.join(bundle, 'Extensions'), '.appex');
    const appClip = await bundleIn(path.join(bundle, 'AppClips'), '.app');
    const own = await entitlements(bundle);
    // What iOS shows in English: the strings the app has in English, over
    // Info.plist's.
    const strings = path.join(bundle, 'en.lproj', 'InfoPlist.strings');
    const shown = existsSync(strings)
      ? {
          ...info,
          ...JSON.parse(
            (
              await run('plutil', ['-convert', 'json', '-o', '-', strings], {
                log,
              })
            ).stdout,
          ),
        }
      : info;
    return {
      signer,
      report: {
        id: info.CFBundleIdentifier,
        name: shown.CFBundleDisplayName ?? shown.CFBundleName,
        cameraUsage: shown.NSCameraUsageDescription,
        version: info.CFBundleShortVersionString,
        build: info.CFBundleVersion,
        payload: readFileSync(path.join(bundle, 'payload.txt'), 'utf8'),
        applicationIdentifier: appId(own, 'application-identifier'),
        urlSchemes: (
          (info.CFBundleURLTypes ?? []) as { CFBundleURLSchemes?: string[] }[]
        )
          .flatMap((type) => type.CFBundleURLSchemes ?? [])
          .join(', '),
        extensionId: plugIn?.id,
        extensionApplicationIdentifier: appId(
          plugIn?.entitlements ?? '',
          'application-identifier',
        ),
        intentsExtensionId: intents?.id,
        intentsExtensionApplicationIdentifier: appId(
          intents?.entitlements ?? '',
          'application-identifier',
        ),
        appClipId: appClip?.id,
        appClipApplicationIdentifier: appId(
          appClip?.entitlements ?? '',
          'application-identifier',
        ),
        // The App Clip and the app each name the other.
        appClipParent: appId(
          appClip?.entitlements ?? '',
          'com.apple.developer.parent-application-identifiers',
        ),
        associatedAppClip: appId(
          own,
          'com.apple.developer.associated-appclip-app-identifiers',
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
 * never reported.
 */
const saveAppLogs = async ({
  udid,
  artifactsDir,
  log,
  since,
}: Artifacts & { udid: string; since: number }) => {
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
      `process == "${SHELL}"`,
    ],
    { log, allowFailure: true },
  );
  writeFileSync(path.join(artifactsDir, 'app.log'), appLog.stdout);

  // Simulator apps' crash reports go to the Mac's own folder.
  const reports = path.join(os.homedir(), 'Library/Logs/DiagnosticReports');
  if (existsSync(reports)) {
    for (const name of readdirSync(reports)) {
      const file = path.join(reports, name);
      if (name.startsWith(SHELL) && statSync(file).mtimeMs >= since) {
        copyFileSync(file, path.join(artifactsDir, name));
      }
    }
  }
};

/**
 * Installs the zipped app on the simulator, launches it, and returns the
 * report it saves on launch.
 */
const runApp: Platform<string, string>['runApp'] = async ({
  device: udid,
  app,
  appId,
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
  const container = (
    await simctl(['get_app_container', udid, appId, 'data'], { log })
  ).stdout.trim();
  await simctl(['launch', udid, appId], { log });

  const reportFile = path.join(container, REPORT_FILE);
  try {
    const report = await poll(
      async () =>
        existsSync(reportFile)
          ? JSON.parse(readFileSync(reportFile, 'utf8'))
          : undefined,
      {
        timeoutMs: 60_000,
        message: `${appId} didn't save its report within a minute of launching: it may have crashed. Its log and any crash report are in ${artifactsDir}.`,
      },
    );
    // The report is saved before the first frame; give SwiftUI a moment to
    // draw it.
    await sleep(1000);
    await simctl(
      ['io', udid, 'screenshot', path.join(artifactsDir, 'screenshot.png')],
      { log },
    );
    return report;
  } catch (error) {
    await saveAppLogs({ udid, artifactsDir, log, since });
    throw error;
  }
};

/**
 * Building the iOS shell and running it on the iOS Simulator. See run.ts.
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
