import { spawn } from 'node:child_process';
import { type Hash, createHash } from 'node:crypto';
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readdirSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildAndroid } from '../../shells/src/android.ts';
import { android, appLinkHosts } from './android.ts';
import { type RunResult, run } from './process.ts';
import { omniBuild } from './repackage.ts';

// Nothing here touches the disk, a device or the network: the Android SDK,
// the JDK, Gradle and the emulator are all stand-ins.
vi.mock('node:fs', () => ({
  closeSync: vi.fn(),
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
// Gradle runs in packages/shells, whose tests check how.
vi.mock('../../shells/src/android.ts', () => ({
  buildAndroid: vi.fn(async ({ name }: { name: string }) => [
    `out/${name}-release.apk`,
    `out/${name}-release.aab`,
  ]),
}));
// omniBuild() runs the binary through the run() above, as the real one
// does, but leaves checking its result to repackage.test.ts.
vi.mock('./repackage.ts', async () => {
  const { run: runMock } = await import('./process.ts');
  return {
    omniBuild: vi.fn((args: string[], _expected: unknown, options: object) =>
      runMock('/repo/target/release/omni', [...args, '--json'], options),
    ),
    toolchainFree: () => ({ HOME: '/home/me' }),
    OPEN_URL: 'omnibuild-e2e://report',
  };
});

const FIXTURE = fileURLToPath(new URL('../fixtures/android/', import.meta.url));
const WEBVIEW_SHELL = fileURLToPath(
  new URL('../../shells/webview/android/', import.meta.url),
);
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
  it("builds each shell's release APK and app bundle with Gradle, with the Android SDK", async () => {
    const shells = await android.build({
      ...artifacts,
      buildDir: 'build',
      outDir: 'out',
    });

    const building = (project: string, name: string) => [
      {
        project,
        name,
        out: 'out',
        configurations: ['release'],
        env: expect.objectContaining({ ANDROID_HOME: '/sdk' }),
        log: 'commands.log',
      },
    ];
    expect(vi.mocked(buildAndroid).mock.calls).toEqual([
      building(FIXTURE, 'OmniBuildShell'),
      building(WEBVIEW_SHELL, 'WebViewShell'),
    ]);
    expect(shells).toEqual([
      'out/OmniBuildShell-release.apk',
      'out/OmniBuildShell-release.aab',
      'out/WebViewShell-release.apk',
      'out/WebViewShell-release.aab',
    ]);
  });
});

describe('the Android SDK', () => {
  const sdkGradleGets = async () => {
    await android.build({ ...artifacts, buildDir: 'build', outDir: 'out' });
    return vi.mocked(buildAndroid).mock.calls[0][0].env?.ANDROID_HOME;
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

/**
 * aapt2's dump of a manifest with an App Links filter for two hosts, and
 * a filter for a URL scheme, which isn't one.
 */
const APP_LINKS = [
  'N: android=http://schemas.android.com/apk/res/android (line=1)',
  '  E: manifest (line=1)',
  '      E: application (line=2)',
  '          E: activity (line=3)',
  '              E: intent-filter (line=3)',
  '                A: http://schemas.android.com/apk/res/android:autoVerify(0x010104ee)=true',
  '                  E: action (line=3)',
  '                    A: http://schemas.android.com/apk/res/android:name(0x01010003)="android.intent.action.VIEW" (Raw: "android.intent.action.VIEW")',
  '                  E: data (line=3)',
  '                    A: http://schemas.android.com/apk/res/android:scheme(0x01010027)="https" (Raw: "https")',
  '                  E: data (line=3)',
  '                    A: http://schemas.android.com/apk/res/android:host(0x01010028)="tides.example.com" (Raw: "tides.example.com")',
  '                  E: data (line=3)',
  '                    A: http://schemas.android.com/apk/res/android:host(0x01010028)="*.example.com" (Raw: "*.example.com")',
  '              E: intent-filter (line=3)',
  '                  E: data (line=3)',
  '                    A: http://schemas.android.com/apk/res/android:scheme(0x01010027)="tides" (Raw: "tides")',
  '                  E: data (line=3)',
  '                    A: http://schemas.android.com/apk/res/android:host(0x01010028)="forecast" (Raw: "forecast")',
  '          E: provider (line=9)',
].join('\n');

describe('appLinkHosts', () => {
  it("reads the hosts of the intent filters Android verifies, from aapt2's dump", () => {
    expect(appLinkHosts(APP_LINKS)).toBe('tides.example.com, *.example.com');
  });

  it('has none without an App Links filter', () => {
    expect(appLinkHosts(APP_LINKS.replace('autoVerify', 'other'))).toBe(
      undefined,
    );
  });
});

describe('verify', () => {
  describe('an APK', () => {
    const buildTools = path.join('/sdk', 'build-tools');

    /** apksigner's verdict on each scheme, for an APK it verifies. */
    const schemes = (v1 = true) =>
      [
        'Verifies',
        `Verified using v1 scheme (JAR signing): ${v1}`,
        'Verified using v2 scheme (APK Signature Scheme v2): true',
        'Verified using v3 scheme (APK Signature Scheme v3): true',
        'Verified using v3.1 scheme (APK Signature Scheme v3.1): false',
      ].join('\n');

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
            ? { stdout: `${schemes()}\n${line}\n` }
            : undefined;

        expect(
          await android.verify({
            ...artifacts,
            shell: 'OmniBuildShell',
            app: 'app.apk',
          }),
        ).toEqual({
          signer: 'CN=Me, O=Us',
        });
        expect(vi.mocked(run).mock.calls.map(([command]) => command)).toEqual([
          path.join(buildTools, '36.0.0', 'zipalign'),
          path.join(buildTools, '36.0.0', 'apksigner'),
          path.join(buildTools, '36.0.0', 'aapt2'),
        ]);
        expect(commands()).toEqual([
          'zipalign -c -P 16 4 app.apk',
          'apksigner verify --verbose --print-certs app.apk',
          'aapt2 dump xmltree --file AndroidManifest.xml app.apk',
        ]);
      },
    );

    it('has no signer when apksigner names none', async () => {
      respond = () => ({ stdout: schemes() });

      expect(
        await android.verify({
          ...artifacts,
          shell: 'OmniBuildShell',
          app: 'app.apk',
        }),
      ).toEqual({
        signer: undefined,
      });
    });

    it.each([
      ['without v1, which Android before 7.0 needs', schemes(false), 'v1'],
      ['without any', 'Verifies\n', 'v1, v2, v3'],
    ])('says which signatures it lacks, %s', async (_, stdout, missing) => {
      respond = () => ({ stdout });

      await expect(
        android.verify({
          ...artifacts,
          shell: 'OmniBuildShell',
          app: 'app.apk',
        }),
      ).rejects.toThrow(`apksigner found no ${missing} signature in app.apk.`);
    });

    it.each([
      ['no build tools folder', () => existing.delete(buildTools)],
      ['an empty one', () => vi.mocked(readdirSync).mockReturnValue([])],
    ])('says how to install the build tools with %s', async (_, arrange) => {
      arrange();

      await expect(
        android.verify({
          ...artifacts,
          shell: 'OmniBuildShell',
          app: 'app.apk',
        }),
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
      existing
        .add(BUNDLETOOL_JAR)
        .add(KEYSTORE)
        .add(path.join('/sdk', 'build-tools'));
      vi.mocked(readdirSync).mockReturnValue(['36.0.0'] as never);
      respond = (program) => (program === 'jarsigner' ? verified : undefined);
    });

    it('checks it as Google Play would, and makes the APK to install from it', async () => {
      respond = (program) =>
        program === 'jarsigner'
          ? verified
          : program === 'aapt2'
            ? { stdout: APP_LINKS }
            : undefined;

      const result = await android.verify({
        ...artifacts,
        shell: 'OmniBuildShell',
        app: 'app.aab',
      });

      expect(result).toEqual({
        installable: path.join('artifacts', 'universal.apk'),
        signer: 'CN=Me, O=Us',
        webLinks: 'tides.example.com, *.example.com',
      });
      expect(commands()).toEqual([
        'jarsigner -verify -verbose -certs app.aab',
        `java -jar ${BUNDLETOOL_JAR} validate --bundle=app.aab`,
        `java -jar ${BUNDLETOOL_JAR} build-apks --bundle=app.aab --output=${path.join('artifacts', 'universal.apks')} --mode=universal --overwrite --ks=${KEYSTORE} --ks-pass=pass:android --ks-key-alias=play`,
        `unzip -o ${path.join('artifacts', 'universal.apks')} universal.apk -d artifacts`,
        `aapt2 dump xmltree --file AndroidManifest.xml ${path.join('artifacts', 'universal.apk')}`,
      ]);
    });

    it("uses JAVA_HOME's JDK when it's set", async () => {
      vi.stubEnv('JAVA_HOME', '/jdk');

      await android.verify({
        ...artifacts,
        shell: 'OmniBuildShell',
        app: 'app.aab',
      });

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

      await android.verify({
        ...artifacts,
        shell: 'OmniBuildShell',
        app: 'app.aab',
      });

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
        android.verify({
          ...artifacts,
          shell: 'OmniBuildShell',
          app: 'app.aab',
        }),
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
        android.verify({
          ...artifacts,
          shell: 'OmniBuildShell',
          app: 'app.aab',
        }),
      ).rejects.toThrow(`Couldn't download ${BUNDLETOOL_URL}: 404`);
    });

    it('fails a bundle jarsigner finds unsigned', async () => {
      // jarsigner exits with 0 either way.
      respond = () => ({ stdout: 'no manifest.\n\njar is unsigned.\n' });

      await expect(
        android.verify({
          ...artifacts,
          shell: 'OmniBuildShell',
          app: 'app.aab',
        }),
      ).rejects.toThrow("jarsigner didn't verify app.aab's signature.");
    });

    it('has no signer when jarsigner names none', async () => {
      respond = (program) =>
        program === 'jarsigner' ? { stdout: 'jar verified.\n' } : undefined;

      expect(
        await android.verify({
          ...artifacts,
          shell: 'OmniBuildShell',
          app: 'app.aab',
        }),
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
  const options = {
    ...artifacts,
    device,
    app: 'app.apk',
    appId,
    shell: 'OmniBuildShell' as const,
  };

  /**
   * Answers the report search (logcat's raw dump) with each of `dumps` in
   * turn, and the full log with `log`.
   */
  const logging = (dumps: string[], log = 'the whole log\n') => {
    respond = (_, args) => {
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
      'omni install app.apk --device emulator-5584 --json',
      `${adb} logcat -c`,
      `${adb} shell am start -W -a android.intent.action.VIEW -d omnibuild-e2e://report ${appId}`,
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

  it('installs with omni without a PATH or toolchain variables', async () => {
    logging(['{}\n']);

    await android.runApp(options);

    expect(omniBuild).toHaveBeenCalledWith(
      ['install', 'app.apk', '--device', 'emulator-5584'],
      { event: 'installed', device: 'emulator-5584' },
      { log: 'commands.log', env: { HOME: '/home/me' } },
    );
    expect(vi.mocked(run).mock.calls[1][0]).toBe('/repo/target/release/omni');
  });

  it('installs on a device on the network with omni', async () => {
    logging(['{}\n']);

    await android.runApp({
      ...options,
      device: { serial: '192.168.1.20:5555', started: false },
    });

    expect(commands()[1]).toBe(
      'omni install app.apk --device 192.168.1.20:5555 --json',
    );
  });

  it("installs on a phone on USB with adb, whose server holds the phone's USB interface", async () => {
    logging(['{}\n']);

    await android.runApp({
      ...options,
      device: { serial: 'R58M123ABC', started: false },
    });

    expect(commands()[1]).toBe('adb -s R58M123ABC install app.apk');
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

  describe('made from the WebView shell', () => {
    const webApp = {
      ...options,
      appId: 'dev.omnibuild.e2e.web',
      shell: 'WebViewShell' as const,
    };

    /**
     * Answers the search of the shell's log (logcat's threadtime dump) with
     * each of `dumps` in turn, and each running process's name, by its ID,
     * as /proc has it: nothing for a process that's gone.
     */
    const consoleLogging = (
      dumps: string[],
      names: Record<string, string> = {
        4733: 'dev.omnibuild.e2e.web\0',
        3739: 'dev.omnibuild.e2e.web\0',
      },
    ) => {
      respond = (_, args) => {
        if (args.includes('threadtime')) {
          return { stdout: dumps.shift() ?? '' };
        }
        if (args.includes('cat')) {
          const pid = /^\/proc\/(\d+)\/cmdline$/.exec(args[4])![1];
          return { stdout: names[pid] ?? '' };
        }
        return undefined;
      };
    };

    it("launches the shell's activity under the app's ID, and returns what the page logged, with the ID of the app that logged it", async () => {
      consoleLogging([
        '--------- beginning of main\n',
        [
          '--------- beginning of main',
          '10-24 23:20:25.100  4733  4733 W OmniBuildWebView: Not the report (https://appassets.androidplatform.net/assets/app.js:3)',
          '10-24 23:20:25.962  4733  4733 I OmniBuildWebView: {"origin":"https://appassets.androidplatform.net"}',
          '',
        ].join('\n'),
      ]);

      expect(await android.runApp(webApp)).toEqual({
        origin: 'https://appassets.androidplatform.net',
        id: 'dev.omnibuild.e2e.web',
      });
      const adb = 'adb -s emulator-5584';
      const search = `${adb} logcat -d -v threadtime -s OmniBuildWebView:I`;
      expect(commands()).toEqual([
        `${adb} uninstall dev.omnibuild.e2e.web`,
        'omni install app.apk --device emulator-5584 --json',
        `${adb} logcat -c`,
        `${adb} shell am start -W -n dev.omnibuild.e2e.web/dev.omnibuild.webview.MainActivity`,
        search,
        search,
        `${adb} shell cat /proc/4733/cmdline 2>/dev/null`,
        `${adb} shell screencap -p /data/local/tmp/omni-build-e2e.png`,
        `${adb} pull /data/local/tmp/omni-build-e2e.png ${path.join('artifacts', 'screenshot.png')}`,
        `${adb} logcat -d -v time`,
      ]);
    });

    it("reads Android 5.0's log, whose lines end with a carriage return too", async () => {
      consoleLogging([
        '--------- beginning of main\r\n10-24 23:29:27.773  3739  3739 I OmniBuildWebView: {"storage":"works"}\r\n',
      ]);

      expect(await android.runApp(webApp)).toEqual({
        storage: 'works',
        id: 'dev.omnibuild.e2e.web',
      });
    });

    it("passes over an earlier run's report, from a process that's gone, for this run's", async () => {
      const earlier =
        '10-24 23:29:20.000  3604  3604 I OmniBuildWebView: {"run":"earlier"}';
      consoleLogging([
        // Only the earlier run's report, which Android 5.0 can leave in
        // the log: its process went with its app.
        `${earlier}\r\n`,
        `${earlier}\r\n10-24 23:29:27.773  3739  3739 I OmniBuildWebView: {"run":"this"}\r\n`,
      ]);

      expect(await android.runApp(webApp)).toEqual({
        run: 'this',
        id: 'dev.omnibuild.e2e.web',
      });
      expect(commands()).toContain(
        'adb -s emulator-5584 shell cat /proc/3604/cmdline 2>/dev/null',
      );
    });
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
