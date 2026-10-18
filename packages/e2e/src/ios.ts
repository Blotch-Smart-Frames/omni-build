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

/**
 * Builds the shell for the simulator with Xcode, zips it into `outDir`, and
 * returns the zip's path, the only shell.
 */
const build: Platform['build'] = async ({ buildDir, outDir, log }) => {
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
      'iphonesimulator',
      '-destination',
      'generic/platform=iOS Simulator',
      '-derivedDataPath',
      derivedData,
    ],
    { log },
  );
  const app = path.join(
    derivedData,
    'Build/Products/Release-iphonesimulator',
    `${SHELL}.app`,
  );

  // A .app is a folder, and its executable's mode wouldn't survive a
  // workflow artifact or a Windows file system on the way to the repackage
  // stage. So the shell travels as a zip, which ditto writes with every
  // file's mode.
  mkdirSync(outDir, { recursive: true });
  const shell = path.join(outDir, `${SHELL}.zip`);
  rmSync(shell, { force: true });
  await run('ditto', ['-c', '-k', '--keepParent', app, shell], { log });
  return [shell];
};

/**
 * Boots the E2E simulator, first creating it on the newest iOS runtime, and
 * returns its UDID. On GitHub's runners a simulator sometimes fails to boot
 * ("Data Migration Failed"), so as in AngularLynx's E2E tests, it falls back
 * to an older runtime when that happens.
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
  const unzipped = mkdtempSync(path.join(os.tmpdir(), 'omni-build-e2e-'));
  try {
    await run('ditto', ['-x', '-k', app, unzipped], { log });
    const bundle = readdirSync(unzipped).find((name) => name.endsWith('.app'));
    if (!bundle) {
      throw new Error(`${app} has no .app inside.`);
    }
    // A fresh install empties the data container, so the report can't be
    // one an earlier run left there.
    await simctl(['uninstall', udid, appId], { log, allowFailure: true });
    await simctl(['install', udid, path.join(unzipped, bundle)], { log });
  } finally {
    rmSync(unzipped, { recursive: true, force: true });
  }
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
  // Nothing yet: the shell is copied, still signed as Xcode built it.
  verify: async () => ({}),
  // Booting is the slowest part, so it starts first and goes on while the
  // other stages work.
  startDevice: ({ log }) => bootSimulator({ log, wait: false }),
  waitForDevice: (_started, { log }) => bootSimulator({ log, wait: true }),
  runApp,
  stopDevice: (udid, { log }) =>
    simctl(['shutdown', udid], { log, allowFailure: true }),
};
