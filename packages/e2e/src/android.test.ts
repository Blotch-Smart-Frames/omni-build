import { spawn } from 'node:child_process';
import { type Hash, createHash } from 'node:crypto';
import {
  closeSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  openSync,
  readdirSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { android } from './android.ts';
import { type RunResult, run } from './process.ts';

// Nothing here touches the disk, a device or the network: the Android SDK,
// the JDK, Gradle and the emulator are all stand-ins.
vi.mock('node:fs', () => ({
  closeSync: vi.fn(),
  copyFileSync: vi.fn(),
  existsSync: vi.fn(),
  mkdirSync: vi.fn(),
  openSync: vi.fn(() => 7),
  readdirSync: vi.fn(),
  writeFileSync: vi.fn(),
}));
vi.mock('node:child_process', () => ({ spawn: vi.fn() }));
vi.mock('node:crypto', () => ({ createHash: vi.fn() }));
vi.mock('node:os', () => ({ default: { homedir: () => '/home/me' } }));
vi.mock('node:timers/promises', () => ({ setTimeout: vi.fn(async () => {}) }));
// The real poll(), so the conditions the harness polls for are tested too.
vi.mock('./process.ts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./process.ts')>()),
  run: vi.fn(),
}));

const PROJECT = fileURLToPath(new URL('../fixtures/android/', import.meta.url));
const BUILD_DIR = fileURLToPath(new URL('../build/', import.meta.url));
const BUNDLETOOL_JAR = path.join(BUILD_DIR, 'bundletool-1.18.3.jar');
const BUNDLETOOL_URL =
  'https://github.com/google/bundletool/releases/download/1.18.3/bundletool-all-1.18.3.jar';
const BUNDLETOOL_SHA256 =
  'a099cfa1543f55593bc2ed16a70a7c67fe54b1747bb7301f37fdfd6d91028e29';
const KEYSTORE = path.join(BUILD_DIR, 'play-signing.keystore');
const PLATFORM = process.platform;

const artifacts = { artifactsDir: 'artifacts', log: 'commands.log' };

/** The files existsSync() finds. */
let existing: Set<string>;
/** What each command prints, by its program's name and arguments. */
let respond: (program: string, args: string[]) => Partial<RunResult> | void;

/** The commands run, each as one line: the program's name and arguments. */
const commands = () =>
  vi
    .mocked(run)
    .mock.calls.map(([command, args]) =>
      [path.basename(command), ...args].join(' '),
    );

/** `adb devices`, listing these serials and states. */
const adbDevices = (...devices: [string, string][]) => ({
  stdout: `List of devices attached\n${devices.map((device) => `${device.join('\t')}\n`).join('')}\n`,
});

const onPlatform = (platform: string) => {
  Object.defineProperty(process, 'platform', { value: platform });
};

beforeEach(() => {
  vi.stubEnv('ANDROID_HOME', '/sdk');
  vi.stubEnv('ANDROID_SDK_ROOT', undefined);
  vi.stubEnv('JAVA_HOME', undefined);
  vi.stubEnv('ANDROID_SERIAL', undefined);
  existing = new Set(['/sdk']);
  vi.mocked(existsSync).mockImplementation((file) =>
    existing.has(String(file)),
  );
  respond = () => {};
  vi.mocked(run).mockImplementation(async (command, args) => ({
    code: 0,
    stdout: '',
    output: '',
    ...respond(path.basename(command), args),
  }));
  vi.mocked(spawn).mockReturnValue({ unref: vi.fn() } as never);
  vi.spyOn(console, 'log').mockImplementation(() => {});
});

afterEach(() => {
  onPlatform(PLATFORM);
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe('build', () => {
  it('builds a release APK and app bundle with Gradle, and copies them out', async () => {
    const shells = await android.build({
      ...artifacts,
      buildDir: 'build',
      outDir: 'out',
    });

    expect(run).toHaveBeenCalledWith(
      path.join(PROJECT, 'gradlew'),
      ['assembleRelease', 'bundleRelease', '--console=plain'],
      {
        cwd: PROJECT,
        env: expect.objectContaining({ ANDROID_HOME: '/sdk' }),
        log: 'commands.log',
      },
    );
    expect(mkdirSync).toHaveBeenCalledWith('out', { recursive: true });
    const outputs = path.join(PROJECT, 'app/build/outputs');
    expect(vi.mocked(copyFileSync).mock.calls).toEqual([
      [
        path.join(outputs, 'apk/release/app-release.apk'),
        path.join('out', 'OmniBuildShell.apk'),
      ],
      [
        path.join(outputs, 'bundle/release/app-release.aab'),
        path.join('out', 'OmniBuildShell.aab'),
      ],
    ]);
    expect(shells).toEqual([
      path.join('out', 'OmniBuildShell.apk'),
      path.join('out', 'OmniBuildShell.aab'),
    ]);
  });
});

describe('the Android SDK', () => {
  const sdkGradleGets = async () => {
    await android.build({ ...artifacts, buildDir: 'build', outDir: 'out' });
    return vi.mocked(run).mock.calls[0][2].env?.ANDROID_HOME;
  };

  it('is where ANDROID_SDK_ROOT says, without ANDROID_HOME', async () => {
    vi.stubEnv('ANDROID_HOME', undefined);
    vi.stubEnv('ANDROID_SDK_ROOT', '/old-sdk');
    existing = new Set(['/old-sdk']);

    expect(await sdkGradleGets()).toBe('/old-sdk');
  });

  it.each([
    ['darwin', 'Library/Android/sdk'],
    ['linux', 'Android/Sdk'],
  ])(
    "is where Android Studio installs it on %s, when the variables don't lead to one",
    async (platform, folder) => {
      vi.stubEnv('ANDROID_HOME', '/removed');
      onPlatform(platform);
      existing = new Set([path.join('/home/me', folder)]);

      expect(await sdkGradleGets()).toBe(path.join('/home/me', folder));
    },
  );

  it('says how to point to one when there is none', async () => {
    existing = new Set();

    await expect(sdkGradleGets()).rejects.toThrow(
      'No Android SDK found. Set ANDROID_HOME to its folder (Android Studio installs one).',
    );
  });
});

describe('verify', () => {
  describe('an APK', () => {
    const buildTools = path.join('/sdk', 'build-tools');

    beforeEach(() => {
      existing.add(buildTools);
      vi.mocked(readdirSync).mockReturnValue([
        '9.0.0',
        '36.0.0',
        '30.0.3',
      ] as never);
    });

    it.each([
      ['up to build tools 36', 'Signer #1 certificate DN: CN=Me, O=Us'],
      ['from build tools 37', 'V3.0 Signer: certificate DN: CN=Me, O=Us'],
    ])(
      "checks its alignment and signatures with the newest build tools, and reads its signer's name, %s",
      async (_, line) => {
        respond = (program) =>
          program === 'apksigner'
            ? { stdout: `Verifies\nVerified using v2 scheme: true\n${line}\n` }
            : undefined;

        expect(await android.verify({ ...artifacts, app: 'app.apk' })).toEqual({
          signer: 'CN=Me, O=Us',
        });
        expect(vi.mocked(run).mock.calls.map(([command]) => command)).toEqual([
          path.join(buildTools, '36.0.0', 'zipalign'),
          path.join(buildTools, '36.0.0', 'apksigner'),
        ]);
        expect(commands()).toEqual([
          'zipalign -c -P 16 4 app.apk',
          'apksigner verify --verbose --print-certs app.apk',
        ]);
      },
    );

    it('has no signer when apksigner names none', async () => {
      expect(await android.verify({ ...artifacts, app: 'app.apk' })).toEqual({
        signer: undefined,
      });
    });

    it.each([
      ['no build tools folder', () => existing.delete(buildTools)],
      ['an empty one', () => vi.mocked(readdirSync).mockReturnValue([])],
    ])('says how to install the build tools with %s', async (_, arrange) => {
      arrange();

      await expect(
        android.verify({ ...artifacts, app: 'app.apk' }),
      ).rejects.toThrow(
        `No build tools in ${buildTools}. Install them with \`sdkmanager 'build-tools;36.0.0'\`.`,
      );
    });
  });

  describe('an app bundle', () => {
    const verified = {
      stdout:
        'sm  1234 META-INF/MANIFEST.MF\n      - Signed by "CN=Me, O=Us"\n\njar verified.\n',
    };

    beforeEach(() => {
      existing.add(BUNDLETOOL_JAR).add(KEYSTORE);
      respond = (program) => (program === 'jarsigner' ? verified : undefined);
    });

    it('checks it as Google Play would, and makes the APK to install from it', async () => {
      const result = await android.verify({ ...artifacts, app: 'app.aab' });

      expect(result).toEqual({
        installable: path.join('artifacts', 'universal.apk'),
        signer: 'CN=Me, O=Us',
      });
      expect(commands()).toEqual([
        'jarsigner -verify -verbose -certs app.aab',
        `java -jar ${BUNDLETOOL_JAR} validate --bundle=app.aab`,
        `java -jar ${BUNDLETOOL_JAR} build-apks --bundle=app.aab --output=${path.join('artifacts', 'universal.apks')} --mode=universal --overwrite --ks=${KEYSTORE} --ks-pass=pass:android --ks-key-alias=play`,
        `unzip -o ${path.join('artifacts', 'universal.apks')} universal.apk -d artifacts`,
      ]);
    });

    it("uses JAVA_HOME's JDK when it's set", async () => {
      vi.stubEnv('JAVA_HOME', '/jdk');

      await android.verify({ ...artifacts, app: 'app.aab' });

      expect(vi.mocked(run).mock.calls[0][0]).toBe(
        path.join('/jdk', 'bin', 'jarsigner'),
      );
    });

    it('downloads bundletool and makes a signing key the first time', async () => {
      existing.delete(BUNDLETOOL_JAR);
      existing.delete(KEYSTORE);
      const jar = new Uint8Array([1, 2, 3]);
      vi.stubGlobal(
        'fetch',
        vi.fn(async () => new Response(jar)),
      );
      const update = vi.fn(() => ({ digest: () => BUNDLETOOL_SHA256 }));
      vi.mocked(createHash).mockReturnValue({ update } as unknown as Hash);

      await android.verify({ ...artifacts, app: 'app.aab' });

      expect(fetch).toHaveBeenCalledWith(BUNDLETOOL_URL);
      expect(createHash).toHaveBeenCalledWith('sha256');
      expect(update).toHaveBeenCalledWith(Buffer.from(jar));
      expect(mkdirSync).toHaveBeenCalledWith(BUILD_DIR, { recursive: true });
      expect(writeFileSync).toHaveBeenCalledWith(
        BUNDLETOOL_JAR,
        Buffer.from(jar),
      );
      expect(commands()).toContain(
        `keytool -genkeypair -keystore ${KEYSTORE} -storepass android -alias play -keyalg RSA -keysize 2048 -validity 10000 -dname CN=Play signing stand-in`,
      );
    });

    it("refuses a bundletool download that isn't the pinned release", async () => {
      existing.delete(BUNDLETOOL_JAR);
      vi.stubGlobal(
        'fetch',
        vi.fn(async () => new Response('tampered')),
      );
      vi.mocked(createHash).mockReturnValue({
        update: () => ({ digest: () => 'abc' }),
      } as unknown as Hash);

      await expect(
        android.verify({ ...artifacts, app: 'app.aab' }),
      ).rejects.toThrow(
        `${BUNDLETOOL_URL} has SHA-256 abc, not ${BUNDLETOOL_SHA256}.`,
      );
      expect(writeFileSync).not.toHaveBeenCalled();
    });

    it('says when bundletool cannot be downloaded', async () => {
      existing.delete(BUNDLETOOL_JAR);
      vi.stubGlobal(
        'fetch',
        vi.fn(async () => new Response('', { status: 404 })),
      );

      await expect(
        android.verify({ ...artifacts, app: 'app.aab' }),
      ).rejects.toThrow(`Couldn't download ${BUNDLETOOL_URL}: 404`);
    });

    it('fails a bundle jarsigner finds unsigned', async () => {
      // jarsigner exits with 0 either way.
      respond = () => ({ stdout: 'no manifest.\n\njar is unsigned.\n' });

      await expect(
        android.verify({ ...artifacts, app: 'app.aab' }),
      ).rejects.toThrow("jarsigner didn't verify app.aab's signature.");
    });

    it('has no signer when jarsigner names none', async () => {
      respond = (program) =>
        program === 'jarsigner' ? { stdout: 'jar verified.\n' } : undefined;

      expect(
        await android.verify({ ...artifacts, app: 'app.aab' }),
      ).toMatchObject({ signer: undefined });
    });
  });
});

describe('startDevice', () => {
  it('uses the connected device without --avd', async () => {
    expect(await android.startDevice(artifacts)).toEqual({ started: false });
    expect(run).not.toHaveBeenCalled();
  });

  it("uses the AVD's emulator when it's already running", async () => {
    respond = (_, args) => {
      if (args[0] === 'devices') {
        // A phone and an offline emulator, which aren't it, then the one.
        return adbDevices(
          ['R58M', 'device'],
          ['emulator-5556', 'offline'],
          ['emulator-5554', 'device'],
        );
      }
      return { stdout: 'test-avd\nOK\n' };
    };

    expect(
      await android.startDevice({ ...artifacts, avd: 'test-avd' }),
    ).toEqual({ started: false });
    expect(commands()).toEqual([
      'adb devices',
      'adb -s emulator-5554 emu avd name',
    ]);
    expect(spawn).not.toHaveBeenCalled();
  });

  it('starts the emulator without waiting for it', async () => {
    respond = (_, args) =>
      args[0] === 'devices'
        ? adbDevices(['emulator-5554', 'device'])
        : { stdout: 'another-avd\nOK\n' };

    expect(
      await android.startDevice({ ...artifacts, avd: 'test-avd' }),
    ).toEqual({ started: true, serial: 'emulator-5584' });
    expect(openSync).toHaveBeenCalledWith(
      path.join('artifacts', 'emulator.log'),
      'w',
    );
    expect(spawn).toHaveBeenCalledWith(
      path.join('/sdk', 'emulator/emulator'),
      [
        '-avd',
        'test-avd',
        '-port',
        '5584',
        '-no-snapshot-save',
        '-no-boot-anim',
        '-no-audio',
      ],
      { detached: true, stdio: ['ignore', 7, 7] },
    );
    expect(vi.mocked(spawn).mock.results[0].value.unref).toHaveBeenCalled();
    expect(closeSync).toHaveBeenCalledWith(7);
  });
});

describe('waitForDevice', () => {
  /** Answers `adb devices` with these and getprop with each boot state in turn. */
  const devicesBooting = (
    devices: [string, string][],
    ...bootStates: string[]
  ) => {
    respond = (_, args) => {
      if (args[0] === 'devices') {
        return adbDevices(...devices);
      }
      if (args.includes('getprop')) {
        return { stdout: `${bootStates.shift() ?? '1'}\n` };
      }
      return { stdout: 'test-avd\nOK\n' };
    };
  };

  it('waits for the emulator it started to boot', async () => {
    devicesBooting([], '', '0', '1');

    expect(
      await android.waitForDevice(
        { started: true, serial: 'emulator-5584' },
        artifacts,
      ),
    ).toEqual({ serial: 'emulator-5584', started: true });
    expect(commands()).toEqual(
      Array(3).fill('adb -s emulator-5584 shell getprop sys.boot_completed'),
    );
    // Polled without a log, which it would bury.
    expect(vi.mocked(run).mock.calls[0][2]).toEqual({ allowFailure: true });
    expect(console.log).toHaveBeenCalledWith('Device: emulator-5584');
  });

  it("finds the AVD's emulator running", async () => {
    devicesBooting([['emulator-5554', 'device']]);

    expect(
      await android.waitForDevice(
        { started: false },
        { ...artifacts, avd: 'test-avd' },
      ),
    ).toEqual({ serial: 'emulator-5554', started: false });
  });

  it('uses ANDROID_SERIAL otherwise', async () => {
    vi.stubEnv('ANDROID_SERIAL', 'R58M');
    devicesBooting([]);

    expect(
      await android.waitForDevice(
        { started: false },
        { ...artifacts, avd: 'test-avd' },
      ),
    ).toEqual({ serial: 'R58M', started: false });
  });

  it('uses the one connected device otherwise', async () => {
    devicesBooting([['R58M', 'device']]);

    expect(await android.waitForDevice({ started: false }, artifacts)).toEqual({
      serial: 'R58M',
      started: false,
    });
  });

  it('says how to get a device when none is connected', async () => {
    devicesBooting([['emulator-5554', 'offline']]);

    await expect(
      android.waitForDevice({ started: false }, artifacts),
    ).rejects.toThrow(
      'No Android device is connected. Boot an emulator, or pass --avd <name> to have the tests boot one (`emulator -list-avds` lists them).',
    );
  });

  it('asks which device when several are connected', async () => {
    devicesBooting([
      ['R58M', 'device'],
      ['emulator-5554', 'device'],
    ]);

    await expect(
      android.waitForDevice({ started: false }, artifacts),
    ).rejects.toThrow(
      '2 Android devices are connected (R58M, emulator-5554). Set ANDROID_SERIAL to the one to test on.',
    );
  });
});

describe('runApp', () => {
  const device = { serial: 'emulator-5584', started: true };
  const appId = 'dev.omnibuild.e2e.app';
  const activity = `${appId}/dev.omnibuild.shell.MainActivity`;
  const options = { ...artifacts, device, app: 'app.apk', appId };

  /**
   * Answers the report search (logcat's raw dump) with each of `dumps` in
   * turn, and the full log with `log`.
   */
  const logging = (dumps: string[], log = 'the whole log\n') => {
    respond = (_, args) => {
      if (args.includes('resolve-activity')) {
        return { stdout: `priority=0 preferredOrder=0\n${activity}\n` };
      }
      if (args.includes('raw')) {
        return { stdout: dumps.shift() ?? '' };
      }
      if (args.includes('time')) {
        return { stdout: log };
      }
      return undefined;
    };
  };

  it('installs the app fresh, launches it, and returns its report', async () => {
    logging(['', 'Starting\n{"id":"dev.omnibuild.e2e.app"}\n']);

    expect(await android.runApp(options)).toEqual({
      id: 'dev.omnibuild.e2e.app',
    });
    const adb = 'adb -s emulator-5584';
    expect(commands()).toEqual([
      `${adb} uninstall ${appId}`,
      `${adb} install app.apk`,
      `${adb} logcat -c`,
      `${adb} shell cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.LAUNCHER ${appId}`,
      `${adb} shell am start -W -n ${activity}`,
      `${adb} logcat -d -v raw -s OmniBuildE2E:I`,
      `${adb} logcat -d -v raw -s OmniBuildE2E:I`,
      `${adb} shell screencap -p /data/local/tmp/omni-build-e2e.png`,
      `${adb} pull /data/local/tmp/omni-build-e2e.png ${path.join('artifacts', 'screenshot.png')}`,
      `${adb} logcat -d -v time`,
    ]);
    expect(writeFileSync).toHaveBeenCalledWith(
      path.join('artifacts', 'logcat.log'),
      'the whole log\n',
    );
  });

  it('adds the crash to the error when the app never reports', async () => {
    logging(
      [],
      [
        'I/ActivityManager: Start proc',
        'E/AndroidRuntime: FATAL EXCEPTION: main',
        'E/AndroidRuntime: java.lang.RuntimeException',
        '',
      ].join('\n'),
    );
    // A minute passes after the first look at the log.
    vi.spyOn(Date, 'now').mockReturnValueOnce(0).mockReturnValue(60_001);

    await expect(android.runApp(options)).rejects.toThrow(
      `${appId} didn't log its report within a minute of launching. The device's log is in artifacts.\nE/AndroidRuntime: FATAL EXCEPTION: main\nE/AndroidRuntime: java.lang.RuntimeException\n`,
    );
  });

  it('gives the error as it is when the log has no crash', async () => {
    logging([]);
    vi.spyOn(Date, 'now').mockReturnValueOnce(0).mockReturnValue(60_001);

    await expect(android.runApp(options)).rejects.toThrow(
      new RegExp(`launching\\. The device's log is in artifacts\\.$`),
    );
  });
});

describe('stopDevice', () => {
  it('shuts down an emulator the harness started', async () => {
    await android.stopDevice(
      { serial: 'emulator-5584', started: true },
      artifacts,
    );

    expect(commands()).toEqual(['adb -s emulator-5584 emu kill']);
  });

  it('leaves a device that was already there as it was', async () => {
    await android.stopDevice({ serial: 'R58M', started: false }, artifacts);

    expect(run).not.toHaveBeenCalled();
  });
});
