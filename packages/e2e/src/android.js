import { spawn } from 'node:child_process';
import {
  closeSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  openSync,
  readdirSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';

import { poll, run } from './process.js';

const PROJECT = fileURLToPath(new URL('../fixtures/android/', import.meta.url));
/** The tag the app logs its report under (ShellReport.kt). */
const LOG_TAG = 'OmniBuildE2E';
/**
 * The console port of an emulator the harness boots itself, which fixes its
 * adb serial (emulator-5584). Clear of the 5554 that a first emulator gets.
 */
const EMULATOR_PORT = 5584;

/**
 * The Android SDK: where ANDROID_HOME (or the older ANDROID_SDK_ROOT) says,
 * or where Android Studio installs it. Gradle needs it, and adb and the
 * emulator are in it, often without being on the PATH.
 */
const androidSdk = () => {
  const candidates = [
    process.env.ANDROID_HOME,
    process.env.ANDROID_SDK_ROOT,
    path.join(
      os.homedir(),
      process.platform === 'darwin' ? 'Library/Android/sdk' : 'Android/Sdk',
    ),
  ];
  const sdk = candidates.find((dir) => dir && existsSync(dir));
  if (!sdk) {
    throw new Error(
      'No Android SDK found. Set ANDROID_HOME to its folder (Android Studio installs one).',
    );
  }
  return sdk;
};

const adb = (args, options) =>
  run(path.join(androidSdk(), 'platform-tools/adb'), args, options);

/** The newest build tools in the SDK, where apksigner and zipalign are. */
const buildTools = () => {
  const dir = path.join(androidSdk(), 'build-tools');
  const [newest] = (existsSync(dir) ? readdirSync(dir) : []).toSorted((a, b) =>
    b.localeCompare(a, 'en', { numeric: true }),
  );
  if (!newest) {
    throw new Error(
      `No build tools in ${dir}. Install them with \`sdkmanager 'build-tools;36.0.0'\`.`,
    );
  }
  return path.join(dir, newest);
};

/**
 * Checks an APK with the Android SDK's own tools, which OmniBuild's users
 * never need, as Google Play would: every stored entry aligned for the
 * device to map it (zipalign), and every signature valid (apksigner, which
 * needs Java). Returns the name on the signing certificate.
 */
const verify = async ({ app, log }) => {
  await run(path.join(buildTools(), 'zipalign'), ['-c', '-P', '16', '4', app], {
    log,
  });
  const verified = await run(
    path.join(buildTools(), 'apksigner'),
    ['verify', '--verbose', '--print-certs', app],
    { log },
  );
  return {
    signer: verified.stdout.match(/^Signer #1 certificate DN: (.*)$/m)?.[1],
  };
};

/**
 * Builds the shell's release APK with Gradle, copies it into `outDir`, and
 * returns the copy's path.
 */
const build = async ({ outDir, log }) => {
  await run(
    path.join(PROJECT, 'gradlew'),
    ['assembleRelease', '--console=plain'],
    {
      cwd: PROJECT,
      env: { ...process.env, ANDROID_HOME: androidSdk() },
      log,
    },
  );
  mkdirSync(outDir, { recursive: true });
  const shell = path.join(outDir, 'OmniBuildShell.apk');
  copyFileSync(
    path.join(PROJECT, 'app/build/outputs/apk/release/app-release.apk'),
    shell,
  );
  return shell;
};

/** The serials of the devices adb sees, booted or not. */
const devices = async (log) =>
  (await adb(['devices'], { log })).stdout
    .split('\n')
    .slice(1)
    .map((line) => line.split('\t'))
    .filter(([, state]) => state === 'device')
    .map(([serial]) => serial);

/** The serial of the running emulator of this AVD, if one is running. */
const emulatorOf = async (avd, log) => {
  for (const serial of await devices(log)) {
    if (serial.startsWith('emulator-')) {
      const name = await adb(['-s', serial, 'emu', 'avd', 'name'], {
        log,
        allowFailure: true,
      });
      if (name.stdout.split('\n')[0].trim() === avd) {
        return serial;
      }
    }
  }
  return undefined;
};

/**
 * With --avd, starts that emulator unless it's already running, without
 * waiting for it to boot. Without, uses whichever device is connected (as
 * in CI, where android-emulator-runner boots one).
 */
const startDevice = async ({ avd, artifactsDir, log }) => {
  if (!avd || (await emulatorOf(avd, log))) {
    return { started: false };
  }
  const emulatorLog = openSync(path.join(artifactsDir, 'emulator.log'), 'w');
  spawn(
    path.join(androidSdk(), 'emulator/emulator'),
    [
      '-avd',
      avd,
      '-port',
      String(EMULATOR_PORT),
      // Keep the AVD's saved state as it was, so every run boots the same.
      '-no-snapshot-save',
      '-no-boot-anim',
      '-no-audio',
    ],
    { detached: true, stdio: ['ignore', emulatorLog, emulatorLog] },
  ).unref();
  closeSync(emulatorLog);
  return { started: true, serial: `emulator-${EMULATOR_PORT}` };
};

/** Picks the device to test on, waits for it to boot, and returns it. */
const waitForDevice = async (started, { avd, log }) => {
  let serial = started.serial;
  if (!serial && avd) {
    serial = await emulatorOf(avd, log);
  }
  if (!serial && process.env.ANDROID_SERIAL) {
    serial = process.env.ANDROID_SERIAL;
  }
  if (!serial) {
    const connected = await devices(log);
    if (connected.length !== 1) {
      throw new Error(
        connected.length === 0
          ? 'No Android device is connected. Boot an emulator, or pass --avd <name> to have the tests boot one (`emulator -list-avds` lists them).'
          : `${connected.length} Android devices are connected (${connected.join(', ')}). Set ANDROID_SERIAL to the one to test on.`,
      );
    }
    [serial] = connected;
  }

  await poll(
    async () => {
      // No log: this runs every half second until the device is up.
      const booted = await adb(
        ['-s', serial, 'shell', 'getprop', 'sys.boot_completed'],
        { allowFailure: true },
      );
      return booted.stdout.trim() === '1' ? true : undefined;
    },
    {
      timeoutMs: 5 * 60_000,
      message: `${serial} didn't finish booting within 5 minutes.`,
    },
  );
  console.log(`Device: ${serial}`);
  return { serial, started: started.started };
};

/**
 * The crash from the app's log, for when it never reports: the lines from
 * the last FATAL EXCEPTION on.
 */
const crashIn = (logcat) => {
  const lines = logcat.split('\n');
  const start = lines.findLastIndex((line) => line.includes('FATAL EXCEPTION'));
  return start === -1 ? '' : `\n${lines.slice(start, start + 20).join('\n')}`;
};

/**
 * Installs the APK on the device, launches it, and returns the report it
 * logs on launch.
 */
const runApp = async ({ device, app, appId, artifactsDir, log }) => {
  const onDevice = (args, options) =>
    adb(['-s', device.serial, ...args], { log, ...options });

  // A fresh install, so the device can't run an earlier build, and so a
  // shell signed with another key replaces one already installed.
  await onDevice(['uninstall', appId], { allowFailure: true });
  await onDevice(['install', app]);
  await onDevice(['logcat', '-c'], { allowFailure: true });
  // The launcher activity's name, which the application ID doesn't give.
  const resolved = await onDevice([
    'shell',
    'cmd',
    'package',
    'resolve-activity',
    '--brief',
    '-a',
    'android.intent.action.MAIN',
    '-c',
    'android.intent.category.LAUNCHER',
    appId,
  ]);
  const activity = resolved.stdout.trim().split('\n').at(-1).trim();
  await onDevice(['shell', 'am', 'start', '-W', '-n', activity]);

  const saveLogcat = async () => {
    const logcat = await onDevice(['logcat', '-d', '-v', 'time'], {
      log: undefined,
      allowFailure: true,
    });
    writeFileSync(path.join(artifactsDir, 'logcat.log'), logcat.stdout);
    return logcat.stdout;
  };
  try {
    const report = await poll(
      async () => {
        const logged = await onDevice(
          ['logcat', '-d', '-v', 'raw', '-s', `${LOG_TAG}:I`],
          { log: undefined },
        );
        const line = logged.stdout
          .split('\n')
          .find((candidate) => candidate.startsWith('{'));
        return line ? JSON.parse(line) : undefined;
      },
      {
        timeoutMs: 60_000,
        message: `${appId} didn't log its report within a minute of launching. The device's log is in ${artifactsDir}.`,
      },
    );
    // The report is logged before the first frame; give the view a moment
    // to draw it.
    await sleep(1000);
    const onDeviceScreenshot = '/data/local/tmp/omni-build-e2e.png';
    await onDevice(['shell', 'screencap', '-p', onDeviceScreenshot]);
    await onDevice([
      'pull',
      onDeviceScreenshot,
      path.join(artifactsDir, 'screenshot.png'),
    ]);
    await saveLogcat();
    return report;
  } catch (error) {
    error.message += crashIn(await saveLogcat());
    throw error;
  }
};

/** Building the Android shell and running it on an emulator or device. See run.js. */
export const android = {
  name: 'Android',
  toolchain: 'Gradle',
  build,
  verify,
  startDevice,
  waitForDevice,
  runApp,
  // Only an emulator the harness booted; a device that was already there
  // stays as it was.
  stopDevice: async (device, { log }) => {
    if (device.started) {
      await adb(['-s', device.serial, 'emu', 'kill'], {
        log,
        allowFailure: true,
      });
    }
  },
};
