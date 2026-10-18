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

import { android } from './android.js';
import { ios } from './ios.js';
import { step } from './process.js';
import { expected, repackage } from './repackage.js';
import { differences } from './shell.js';

const PACKAGE_DIR = fileURLToPath(new URL('..', import.meta.url));
const BUILD_DIR = path.join(PACKAGE_DIR, 'build');
const PLATFORMS = { ios, android };
const COMMANDS = ['build', 'repackage', 'test'];

const HELP = `Usage: npm run e2e -- [command] --platform <ios|android> [options]

Builds a fixture app shell (a real Swift or Kotlin project), repackages it
with omni-build, and runs the result on the iOS Simulator or an Android
emulator, checking the ID, name, version, payload and icon the app reports.

Each command is one stage. CI runs each on different machines; with no
command, all three run here in turn.

Commands:
  build      Build the shells with Xcode or Gradle (an Android APK and app
             bundle, an iOS app): the only stage that uses them. Writes them
             to --out (default: build/shells/<platform>).
  repackage  Run each shell (--shell) through omni-build, which gets no
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

const { values, positionals } = parseArgs({
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

const [command, ...apps] = COMMANDS.includes(positionals[0])
  ? positionals
  : [undefined, ...positionals];
if (
  values.help ||
  !Object.hasOwn(PLATFORMS, values.platform ?? '') ||
  (apps.length > 0 && command !== 'test')
) {
  console.log(HELP);
  process.exit(values.help ? 0 : 2);
}

const platform = PLATFORMS[values.platform];
const artifactsRoot = path.join(
  path.resolve(values['artifacts-dir'] ?? path.join(PACKAGE_DIR, 'artifacts')),
  values.platform,
);
const shellsDir = path.join(BUILD_DIR, 'shells', values.platform);
const repackagedDir = path.join(BUILD_DIR, 'repackaged', values.platform);

/**
 * A stage's own artifacts folder and command log, emptied first so nothing
 * in it is from an earlier run.
 */
const artifactsFor = (stage) => {
  const artifactsDir = path.join(artifactsRoot, stage);
  rmSync(artifactsDir, { recursive: true, force: true });
  mkdirSync(artifactsDir, { recursive: true });
  return { artifactsDir, log: path.join(artifactsDir, 'commands.log') };
};

const buildStage = async (outDir) => {
  step(`Building the ${platform.name} shell with ${platform.toolchain}`);
  const shells = await platform.build({
    ...artifactsFor('build'),
    buildDir: BUILD_DIR,
    outDir,
  });
  shells.forEach((shell) => console.log(shell));
  return shells;
};

const repackageStage = async (shells, outDir) => {
  step('Repackaging it with omni-build, with no toolchain in reach');
  const artifacts = artifactsFor('repackage');
  const apps = [];
  for (const shell of shells) {
    const app = await repackage({
      ...artifacts,
      platform: values.platform,
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
const testStage = async (appFiles, artifacts, started) => {
  step(`Running ${appFiles.length} app(s) on ${platform.name}`);
  const options = { ...artifacts, avd: values.avd };
  const want = expected(values.platform);
  const device = await platform.waitForDevice(started, options);
  const failures = [];
  try {
    for (const app of appFiles) {
      // With its extension: an APK and an app bundle come from each OS.
      const name = path.basename(app);
      const artifactsDir = path.join(artifacts.artifactsDir, name);
      mkdirSync(artifactsDir);
      try {
        // The platform's own tools check the package before it's
        // installed, and report what signed it. An app bundle is turned
        // into an APK to install, as an app store would.
        const { installable = app, ...verified } = await platform.verify({
          ...options,
          artifactsDir,
          app,
        });
        const report = {
          ...(await platform.runApp({
            ...options,
            artifactsDir,
            device,
            app: installable,
            appId: want.id,
          })),
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
        failures.push(`${name}: ${error.message}`);
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
  const started = await platform.startDevice({ ...artifacts, avd: values.avd });
  return { artifacts, started };
};

/** The files an earlier stage wrote to one of its default folders. */
const filesFrom = (dir, stage) => {
  const files = existsSync(dir)
    ? readdirSync(dir).filter((name) => !name.startsWith('.'))
    : [];
  if (files.length === 0) {
    throw new Error(`There's nothing in ${dir}: run the ${stage} stage first.`);
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
      let apps;
      try {
        apps = await repackageStage(await buildStage(shellsDir), repackagedDir);
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
  console.error(`\n✗ ${error.message}`);
  console.error(`\nLogs, reports and screenshots are in ${artifactsRoot}`);
  process.exitCode = 1;
}
