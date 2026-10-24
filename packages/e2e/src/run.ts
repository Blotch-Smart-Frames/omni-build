import {
  existsSync,
  mkdirSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

import { android } from './android.ts';
import { ios } from './ios.ts';
import { step } from './process.ts';
import { expected, repackage as repackageShell } from './repackage.ts';
import { type ShellName, differences, shellOf } from './shell.ts';

/** A stage's own artifacts folder, and the log of the commands it runs. */
export interface Artifacts {
  artifactsDir: string;
  log: string;
}

/** The emulator to boot, with --avd. iOS ignores it. */
export interface DeviceOptions extends Artifacts {
  avd?: string;
}

/**
 * What run() needs of a platform (ios.ts, android.ts) to build its shells
 * and run apps on a device. `Started` is what startDevice() leaves for
 * waitForDevice(), and `Device` the device the apps then run on. Each app
 * comes with the shell it was made from, which says how it reports.
 */
export interface Platform<Started = unknown, Device = unknown> {
  name: string;
  toolchain: string;
  /** Builds every shell into `outDir`, and returns their paths. */
  build(
    options: Artifacts & { buildDir: string; outDir: string },
  ): Promise<string[]>;
  /**
   * Checks an app with the platform's own tools, and returns what signed it,
   * what else they read in it that the app can't report the same way on
   * every device (an Android app's `webLinks`), and, when it isn't the app
   * itself, the file to install. For an app the device can't run, it
   * returns what the app's files say (`report`), which is checked in place
   * of what the app would report.
   */
  verify(options: DeviceOptions & { app: string; shell: ShellName }): Promise<{
    installable?: string;
    signer?: string;
    webLinks?: string;
    report?: Record<string, unknown>;
  }>;
  /** Starts the device booting, without waiting for it. */
  startDevice(options: DeviceOptions): Promise<Started>;
  waitForDevice(started: Started, options: DeviceOptions): Promise<Device>;
  /** Installs and launches the app, and returns what it reported. */
  runApp(
    options: DeviceOptions & {
      device: Device;
      app: string;
      appId: string;
      shell: ShellName;
    },
  ): Promise<Record<string, unknown>>;
  stopDevice(device: Started | Device, options: Artifacts): Promise<unknown>;
}

export type PlatformId = 'ios' | 'android';

const PACKAGE_DIR = fileURLToPath(new URL('..', import.meta.url));
const PLATFORMS: Record<PlatformId, Platform> = { ios, android };
const COMMANDS = ['build', 'repackage', 'test'];

export const HELP = `Usage: npm run e2e -- [command] --platform <ios|android> [options]

Builds the app shells (real Swift or Kotlin projects: the fixture, and
OmniBuild's WebView shell), repackages them with omni, and runs the results
on the iOS Simulator or an Android emulator, checking what each app reports:
the fixture its ID, name, version, payload and icon, and the WebView shell's
web app what its page finds.

Each command is one stage. CI runs each on different machines; with no
command, all three run here in turn.

Commands:
  build      Build the shells with Xcode or Gradle (an Android APK and app
             bundle, an iOS app for the simulator and one for devices, of
             each shell): the only stage that uses them. Writes them to --out
             (default: build/shells/<platform>).
  repackage  Run each shell (--shell) through omni, which gets no
             PATH and no toolchain variables. Writes to --out (default:
             build/repackaged/<platform>).
  test [app...]
             Install and launch each app (default: every file in
             build/repackaged/<platform>), and check what it reports.

Options:
  --platform <ios|android>  Required.
  --shell <file>            repackage: a shell, as often as there are
                            shells. Default: the ones build wrote.
  --out <dir>               build, repackage: where to write.
  --avd <name>              test, Android: boot this emulator unless it's
                            already running (\`emulator -list-avds\` lists
                            them). Without it, the tests use the one device
                            adb sees, or $ANDROID_SERIAL.
  --artifacts-dir <dir>     Where the logs, reports and screenshots go.
                            Default: packages/e2e/artifacts.
  --keep-running            test: leave the simulator or emulator running.
  --help                    Show this.`;

export interface RunOptions {
  /** By `--platform` value. */
  platforms?: Record<PlatformId, Platform>;
  /** The folder the stages' default build/ and artifacts/ folders are in. */
  packageDir?: string;
  /** Runs one shell through omni, as repackage() in repackage.ts. */
  repackage?: typeof repackageShell;
}

/**
 * Runs the stage `args` ask for, or all three (see HELP), and returns the
 * exit status.
 *
 * The options default to the real platforms, folders and omni binary,
 * and exist for tests.
 */
export const run = async (
  args: string[],
  {
    platforms = PLATFORMS,
    packageDir = PACKAGE_DIR,
    repackage = repackageShell,
  }: RunOptions = {},
): Promise<number> => {
  const buildDir = path.join(packageDir, 'build');
  const { values, positionals } = parseArgs({
    args,
    allowPositionals: true,
    options: {
      platform: { type: 'string' },
      shell: { type: 'string', multiple: true },
      out: { type: 'string' },
      avd: { type: 'string' },
      'artifacts-dir': { type: 'string' },
      'keep-running': { type: 'boolean', default: false },
      help: { type: 'boolean', default: false },
    },
  });

  const command = COMMANDS.includes(positionals[0])
    ? positionals[0]
    : undefined;
  const apps = command ? positionals.slice(1) : positionals;
  const isPlatform = (name?: string): name is PlatformId =>
    Object.hasOwn(platforms, name ?? '');
  if (
    values.help ||
    !isPlatform(values.platform) ||
    (apps.length > 0 && command !== 'test')
  ) {
    console.log(HELP);
    return values.help ? 0 : 2;
  }

  // A const, so the functions below see it narrowed by the check above.
  const platformId = values.platform;
  const platform = platforms[platformId];
  const artifactsRoot = path.join(
    path.resolve(values['artifacts-dir'] ?? path.join(packageDir, 'artifacts')),
    platformId,
  );
  const shellsDir = path.join(buildDir, 'shells', platformId);
  const repackagedDir = path.join(buildDir, 'repackaged', platformId);

  /**
   * A stage's own artifacts folder and command log, emptied first so nothing
   * in it is from an earlier run.
   */
  const artifactsFor = (stage: string): Artifacts => {
    const artifactsDir = path.join(artifactsRoot, stage);
    rmSync(artifactsDir, { recursive: true, force: true });
    mkdirSync(artifactsDir, { recursive: true });
    return { artifactsDir, log: path.join(artifactsDir, 'commands.log') };
  };

  const buildStage = async (outDir: string) => {
    step(`Building the ${platform.name} shells with ${platform.toolchain}`);
    const shells = await platform.build({
      ...artifactsFor('build'),
      buildDir,
      outDir,
    });
    shells.forEach((shell) => console.log(shell));
    return shells;
  };

  const repackageStage = async (shells: string[], outDir: string) => {
    step('Repackaging them with omni, with no toolchain in reach');
    const artifacts = artifactsFor('repackage');
    const apps: string[] = [];
    for (const shell of shells) {
      const app = await repackage({
        ...artifacts,
        shell,
        outDir,
      });
      console.log(app);
      apps.push(app);
    }
    return apps;
  };

  /**
   * Runs each app on one device, and throws with every failure once all have
   * run. `started` is what the platform's startDevice() returned.
   */
  const testStage = async (
    appFiles: string[],
    artifacts: Artifacts,
    started: unknown,
  ) => {
    step(`Running ${appFiles.length} app(s) on ${platform.name}`);
    const options = { ...artifacts, avd: values.avd };
    const device = await platform.waitForDevice(started, options);
    const failures: string[] = [];
    try {
      for (const app of appFiles) {
        // With its extension: an APK and an app bundle come from each OS.
        const name = path.basename(app);
        const artifactsDir = path.join(artifacts.artifactsDir, name);
        mkdirSync(artifactsDir);
        try {
          const want = expected(platformId, app);
          const shell = shellOf(app);
          // The platform's own tools check the package before it's
          // installed, and report what signed it. An app bundle is turned
          // into an APK to install, as an app store would.
          const {
            installable = app,
            report: files,
            ...verified
          } = await platform.verify({
            ...options,
            artifactsDir,
            app,
            shell,
          });
          const report = {
            ...(files ??
              (await platform.runApp({
                ...options,
                artifactsDir,
                device,
                app: installable,
                appId: want.id,
                shell,
              }))),
            ...verified,
          };
          writeFileSync(
            path.join(artifactsDir, 'report.json'),
            `${JSON.stringify(report, undefined, 2)}\n`,
          );
          const problems = differences(want, report);
          if (problems.length > 0) {
            throw new Error(
              `it didn't report what it was built with:\n  ${problems.join('\n  ')}`,
            );
          }
          console.log(`✓ ${name} reported every expected value.`);
        } catch (error) {
          console.error(`✗ ${name}`);
          failures.push(`${name}: ${(error as Error).message}`);
        }
      }
    } finally {
      if (!values['keep-running']) {
        await platform.stopDevice(device, options);
      }
    }
    if (failures.length > 0) {
      throw new Error(failures.join('\n\n'));
    }
  };

  /** Starts the device for the test stage, so it boots while other work runs. */
  const startTest = async () => {
    const artifacts = artifactsFor('test');
    const started = await platform.startDevice({
      ...artifacts,
      avd: values.avd,
    });
    return { artifacts, started };
  };

  /** The files an earlier stage wrote to one of its default folders. */
  const filesFrom = (dir: string, stage: string) => {
    const files = existsSync(dir)
      ? readdirSync(dir).filter((name) => !name.startsWith('.'))
      : [];
    if (files.length === 0) {
      throw new Error(
        `There's nothing in ${dir}: run the ${stage} stage first.`,
      );
    }
    return files.map((name) => path.join(dir, name));
  };

  try {
    switch (command) {
      case 'build':
        await buildStage(path.resolve(values.out ?? shellsDir));
        break;
      case 'repackage':
        await repackageStage(
          values.shell
            ? values.shell.map((shell) => path.resolve(shell))
            : filesFrom(shellsDir, 'build'),
          path.resolve(values.out ?? repackagedDir),
        );
        break;
      case 'test': {
        const appFiles =
          apps.length > 0
            ? apps.map((app) => path.resolve(app))
            : filesFrom(repackagedDir, 'repackage');
        const { artifacts, started } = await startTest();
        await testStage(appFiles, artifacts, started);
        break;
      }
      default: {
        const { artifacts, started } = await startTest();
        let apps: string[];
        try {
          apps = await repackageStage(
            await buildStage(shellsDir),
            repackagedDir,
          );
        } catch (error) {
          // The device started booting first; don't leave it running for a
          // test stage that won't come.
          if (!values['keep-running']) {
            await platform.stopDevice(started, artifacts);
          }
          throw error;
        }
        await testStage(apps, artifacts, started);
      }
    }
  } catch (error) {
    console.error(`\n✗ ${(error as Error).message}`);
    console.error(`\nLogs, reports and screenshots are in ${artifactsRoot}`);
    return 1;
  }
  return 0;
};
