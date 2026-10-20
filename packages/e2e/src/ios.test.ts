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
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ios } from './ios.ts';
import { type RunResult, run } from './process.ts';

// Nothing here touches the disk or a simulator: Xcode, simctl and ditto are
// all stand-ins.
vi.mock('node:fs', () => ({
  copyFileSync: vi.fn(),
  existsSync: vi.fn(),
  mkdirSync: vi.fn(),
  mkdtempSync: vi.fn(() => '/tmp/omni-build-e2e-1'),
  readFileSync: vi.fn(),
  readdirSync: vi.fn(),
  rmSync: vi.fn(),
  statSync: vi.fn(),
  writeFileSync: vi.fn(),
}));
vi.mock('node:os', () => ({
  default: { homedir: () => '/home/me', tmpdir: () => '/tmp' },
}));
vi.mock('node:timers/promises', () => ({ setTimeout: vi.fn(async () => {}) }));
// The real poll(), so the condition the harness polls for is tested too.
vi.mock('./process.ts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./process.ts')>()),
  run: vi.fn(),
}));

const PROJECT = fileURLToPath(
  new URL('../fixtures/ios/OmniBuildShell.xcodeproj', import.meta.url),
);
const IOS_26 = 'com.apple.CoreSimulator.SimRuntime.iOS-26-0';
const IOS_27 = 'com.apple.CoreSimulator.SimRuntime.iOS-27-0';
const IPHONE = 'com.apple.CoreSimulator.SimDeviceType.iPhone-17';

const artifacts = { artifactsDir: 'artifacts', log: 'commands.log' };

/** What each command prints, by its program's name and arguments. */
let respond: (program: string, args: string[]) => Partial<RunResult> | void;

/** The commands run, each as one line: the program and its arguments. */
const commands = () =>
  vi
    .mocked(run)
    .mock.calls.map(([command, args]) => [command, ...args].join(' '));

/** A runtime in `simctl list --json`, which runs an iPhone unless told not to. */
const runtime = (identifier: string, fields: Record<string, unknown> = {}) => ({
  identifier,
  name: identifier.replace(/.*\.iOS-(\d+)-(\d+)$/, 'iOS $1.$2'),
  isAvailable: true,
  supportedDeviceTypes: [
    { identifier: 'iPad', productFamily: 'iPad' },
    { identifier: IPHONE, productFamily: 'iPhone' },
  ],
  ...fields,
});

/** Answers `simctl list` with this, and `simctl create` with a new UDID. */
const simulators = (
  list: { runtimes: unknown[]; devices?: unknown },
  bootStatuses: number[] = [],
) => {
  respond = (_, args) => {
    switch (args[1]) {
      case 'list':
        return { stdout: JSON.stringify({ devices: {}, ...list }) };
      case 'create':
        return { stdout: `NEW-UDID\n` };
      case 'bootstatus':
        return { code: bootStatuses.shift() ?? 0 };
      default:
        return undefined;
    }
  };
};

beforeEach(() => {
  respond = () => {};
  vi.mocked(run).mockImplementation(async (command, args) => ({
    code: 0,
    stdout: '',
    output: '',
    ...respond(command, args),
  }));
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe('build', () => {
  it('builds the shell for the simulator and for devices with Xcode, and zips each', async () => {
    const shells = await ios.build({
      ...artifacts,
      buildDir: 'build',
      outDir: 'out',
    });

    const derivedData = path.join('build', 'ios');
    const xcodebuild = `xcodebuild build -project ${PROJECT} -scheme OmniBuildShell -configuration Release`;
    const simulator = path.join(
      derivedData,
      'Build/Products/Release-iphonesimulator',
      'OmniBuildShell.app',
    );
    const device = path.join(
      derivedData,
      'Build/Products/Release-iphoneos',
      'OmniBuildShell.app',
    );
    const payload = path.join('build', 'ios-ipa', 'Payload');
    const zip = path.join('out', 'OmniBuildShell.zip');
    const ipa = path.join('out', 'OmniBuildShell.ipa');
    expect(commands()).toEqual([
      `${xcodebuild} -sdk iphonesimulator -destination generic/platform=iOS Simulator -derivedDataPath ${derivedData}`,
      `${xcodebuild} -sdk iphoneos -destination generic/platform=iOS -derivedDataPath ${derivedData} CODE_SIGNING_ALLOWED=NO OTHER_LDFLAGS=-Wl,-adhoc_codesign`,
      `ditto -c -k --keepParent ${simulator} ${zip}`,
      `ditto ${device} ${path.join(payload, 'OmniBuildShell.app')}`,
      `ditto -c -k --keepParent ${payload} ${ipa}`,
    ]);
    expect(mkdirSync).toHaveBeenCalledWith('out', { recursive: true });
    expect(rmSync).toHaveBeenCalledWith(zip, { force: true });
    expect(rmSync).toHaveBeenCalledWith(ipa, { force: true });
    expect(rmSync).toHaveBeenCalledWith(payload, {
      recursive: true,
      force: true,
    });
    expect(shells).toEqual([zip, ipa]);
  });
});

describe('startDevice', () => {
  it('creates the simulator on the newest iOS runtime and starts it booting', async () => {
    simulators({
      runtimes: [
        runtime(IOS_26),
        runtime(IOS_27),
        // Neither runs the app: not an installed runtime, not iOS.
        runtime('com.apple.CoreSimulator.SimRuntime.iOS-28-0', {
          isAvailable: false,
        }),
        runtime('com.apple.CoreSimulator.SimRuntime.watchOS-12-0'),
      ],
    });

    expect(await ios.startDevice(artifacts)).toBe('NEW-UDID');
    expect(commands()).toEqual([
      'xcrun simctl list --json',
      `xcrun simctl create OmniBuild E2E ${IPHONE} ${IOS_27}`,
      'xcrun simctl boot NEW-UDID',
    ]);
    // Booting again fails harmlessly when it's already booting or booted.
    expect(vi.mocked(run).mock.calls[2][2]).toEqual({
      log: 'commands.log',
      allowFailure: true,
    });
  });

  it('reuses the simulator it made before', async () => {
    simulators({
      runtimes: [runtime(IOS_27)],
      devices: {
        [IOS_27]: [
          { name: 'iPhone 17', udid: 'THEIRS' },
          { name: 'OmniBuild E2E', udid: 'OURS' },
        ],
      },
    });

    expect(await ios.startDevice(artifacts)).toBe('OURS');
    expect(commands()).toEqual([
      'xcrun simctl list --json',
      'xcrun simctl boot OURS',
    ]);
  });

  it('says how to install a runtime when there is none', async () => {
    simulators({
      runtimes: [
        runtime('com.apple.CoreSimulator.SimRuntime.watchOS-12-0'),
        runtime(IOS_27, { isAvailable: false }),
      ],
    });

    await expect(ios.startDevice(artifacts)).rejects.toThrow(
      'No iOS Simulator runtime is installed. Install one in Xcode → Settings → Components.',
    );
  });
});

describe('waitForDevice', () => {
  it('waits for the simulator to boot', async () => {
    simulators({ runtimes: [runtime(IOS_27)] });

    expect(await ios.waitForDevice('NEW-UDID', artifacts)).toBe('NEW-UDID');
    expect(commands()).toEqual([
      'xcrun simctl list --json',
      `xcrun simctl create OmniBuild E2E ${IPHONE} ${IOS_27}`,
      'xcrun simctl boot NEW-UDID',
      'xcrun simctl bootstatus NEW-UDID -b',
    ]);
    expect(console.log).toHaveBeenCalledWith(
      'Device: OmniBuild E2E on iOS 27.0 (NEW-UDID)',
    );
  });

  it('skips a runtime that runs no iPhone', async () => {
    simulators({
      runtimes: [
        runtime(IOS_26),
        runtime(IOS_27, {
          supportedDeviceTypes: [{ identifier: 'iPad', productFamily: 'iPad' }],
        }),
        runtime('com.apple.CoreSimulator.SimRuntime.iOS-28-0', {
          supportedDeviceTypes: undefined,
        }),
      ],
    });

    await ios.waitForDevice('NEW-UDID', artifacts);

    expect(commands()).toContain(
      `xcrun simctl create OmniBuild E2E ${IPHONE} ${IOS_26}`,
    );
  });

  it('falls back to an older runtime when the simulator fails to boot', async () => {
    simulators({ runtimes: [runtime(IOS_26), runtime(IOS_27)] }, [1, 0]);

    await ios.waitForDevice('NEW-UDID', artifacts);

    expect(commands()).toEqual([
      'xcrun simctl list --json',
      `xcrun simctl create OmniBuild E2E ${IPHONE} ${IOS_27}`,
      'xcrun simctl boot NEW-UDID',
      'xcrun simctl bootstatus NEW-UDID -b',
      'xcrun simctl delete NEW-UDID',
      `xcrun simctl create OmniBuild E2E ${IPHONE} ${IOS_26}`,
      'xcrun simctl boot NEW-UDID',
      'xcrun simctl bootstatus NEW-UDID -b',
    ]);
    expect(console.warn).toHaveBeenCalledWith(
      '! OmniBuild E2E failed to boot on iOS 27.0; trying an older runtime.',
    );
  });

  it('gives up when it boots on no runtime', async () => {
    simulators({ runtimes: [runtime(IOS_26), runtime(IOS_27)] }, [1, 1]);

    await expect(ios.waitForDevice('NEW-UDID', artifacts)).rejects.toThrow(
      'OmniBuild E2E failed to boot on every installed iOS runtime.',
    );
  });
});

describe('runApp', () => {
  const appId = 'dev.omnibuild.e2e.app';
  const options = { ...artifacts, device: 'UDID', app: 'app.zip', appId };
  const unzipped = '/tmp/omni-build-e2e-1';
  const reportFile = path.join('/container', 'Documents/omni-build-e2e.json');
  const crashReports = path.join('/home/me', 'Library/Logs/DiagnosticReports');

  beforeEach(() => {
    respond = (_, args) => {
      if (args.includes('get_app_container')) {
        return { stdout: '/container\n' };
      }
      if (args.includes('log')) {
        return { stdout: 'what the app logged\n' };
      }
      return undefined;
    };
    vi.mocked(readdirSync).mockReturnValueOnce([
      '__MACOSX',
      'OmniBuildShell.app',
    ] as never);
  });

  it('installs the app fresh, launches it, and returns its report', async () => {
    // The report appears on the second look.
    vi.mocked(existsSync).mockReturnValueOnce(false).mockReturnValueOnce(true);
    vi.mocked(readFileSync).mockReturnValue('{"id":"dev.omnibuild.e2e.app"}');

    expect(await ios.runApp(options)).toEqual({ id: 'dev.omnibuild.e2e.app' });
    expect(mkdtempSync).toHaveBeenCalledWith(
      path.join('/tmp', 'omni-build-e2e-'),
    );
    expect(commands()).toEqual([
      `ditto -x -k app.zip ${unzipped}`,
      `xcrun simctl uninstall UDID ${appId}`,
      `xcrun simctl install UDID ${path.join(unzipped, 'OmniBuildShell.app')}`,
      `xcrun simctl get_app_container UDID ${appId} data`,
      `xcrun simctl launch UDID ${appId}`,
      `xcrun simctl io UDID screenshot ${path.join('artifacts', 'screenshot.png')}`,
    ]);
    expect(rmSync).toHaveBeenCalledWith(unzipped, {
      recursive: true,
      force: true,
    });
    expect(readFileSync).toHaveBeenCalledWith(reportFile, 'utf8');
  });

  it('cleans up and says so when the zip holds no app', async () => {
    vi.mocked(readdirSync)
      .mockReset()
      .mockReturnValueOnce(['__MACOSX'] as never);

    await expect(ios.runApp(options)).rejects.toThrow(
      'app.zip has no .app inside.',
    );
    expect(rmSync).toHaveBeenCalledWith(unzipped, {
      recursive: true,
      force: true,
    });
    expect(commands()).toEqual([`ditto -x -k app.zip ${unzipped}`]);
  });

  it("saves the app's log and new crash reports when it never reports", async () => {
    // Launched at 1000; the minute is up after the first look.
    vi.spyOn(Date, 'now')
      .mockReturnValueOnce(1000)
      .mockReturnValueOnce(1000)
      .mockReturnValue(61_001);
    vi.mocked(existsSync).mockImplementation((file) => file === crashReports);
    vi.mocked(readdirSync).mockReturnValueOnce([
      'OmniBuildShell-2026-10-18-120000.ips',
      'OmniBuildShell-2026-10-17-090000.ips',
      'Safari-2026-10-18-120000.ips',
    ] as never);
    vi.mocked(statSync).mockImplementation(
      (file) =>
        ({
          mtimeMs: String(file).includes('10-18') ? 2000 : 500,
        }) as never,
    );

    await expect(ios.runApp(options)).rejects.toThrow(
      `${appId} didn't save its report within a minute of launching: it may have crashed. Its log and any crash report are in artifacts.`,
    );
    expect(commands()).toContain(
      'xcrun simctl spawn UDID log show --last 10m --style compact --predicate process == "OmniBuildShell"',
    );
    expect(writeFileSync).toHaveBeenCalledWith(
      path.join('artifacts', 'app.log'),
      'what the app logged\n',
    );
    expect(vi.mocked(copyFileSync).mock.calls).toEqual([
      [
        path.join(crashReports, 'OmniBuildShell-2026-10-18-120000.ips'),
        path.join('artifacts', 'OmniBuildShell-2026-10-18-120000.ips'),
      ],
    ]);
  });

  it('saves only the log when there are no crash reports', async () => {
    // The app reports, and then the screenshot fails.
    vi.mocked(existsSync).mockImplementation((file) => file === reportFile);
    vi.mocked(readFileSync).mockReturnValue('{}');
    const answer = respond;
    respond = (program, args) => {
      if (args.includes('screenshot')) {
        throw new Error('no screen');
      }
      return answer(program, args);
    };

    await expect(ios.runApp(options)).rejects.toThrow('no screen');
    expect(writeFileSync).toHaveBeenCalledWith(
      path.join('artifacts', 'app.log'),
      'what the app logged\n',
    );
    expect(copyFileSync).not.toHaveBeenCalled();
  });
});

describe('verify', () => {
  const unzipped = '/tmp/omni-build-e2e-1';

  /** codesign's details of a signature, which it prints to stderr. */
  const signedBy = (details: string) => {
    respond = (program, args) =>
      program === 'codesign' && args[0] === '--display' && args.length === 3
        ? { output: details }
        : undefined;
  };

  it("checks a simulator build's signature with codesign", async () => {
    signedBy('Identifier=dev.omnibuild.e2e.app\nSignature=adhoc\n');
    vi.mocked(readdirSync).mockReturnValueOnce(['OmniBuildShell.app'] as never);

    expect(await ios.verify({ ...artifacts, app: 'app.zip' })).toEqual({
      signer: 'ad hoc',
    });
    const bundle = path.join(unzipped, 'OmniBuildShell.app');
    expect(commands()).toEqual([
      `ditto -x -k app.zip ${unzipped}`,
      `codesign --verify --strict --deep --verbose=2 ${bundle}`,
      `codesign --display --verbose=2 ${bundle}`,
    ]);
    expect(rmSync).toHaveBeenCalledWith(unzipped, {
      recursive: true,
      force: true,
    });
  });

  it('reports the certificate that signed it, or nothing for no signature', async () => {
    vi.mocked(readdirSync).mockReturnValue(['OmniBuildShell.app'] as never);

    signedBy('Authority=OmniBuild E2E\nAuthority=Its issuer\n');
    const signed = await ios.verify({ ...artifacts, app: 'app.zip' });
    signedBy('Identifier=dev.omnibuild.e2e.app\n');
    const unsigned = await ios.verify({ ...artifacts, app: 'app.zip' });

    expect(signed).toEqual({ signer: 'OmniBuild E2E' });
    expect(unsigned).toEqual({ signer: undefined });
  });

  it("reads what a build for devices' files say, which the simulator can't run", async () => {
    vi.mocked(readdirSync).mockReturnValue(['OmniBuildShell.app'] as never);
    vi.mocked(readFileSync).mockReturnValue('the payload');
    const bundle = path.join(unzipped, 'Payload', 'OmniBuildShell.app');
    const info = {
      CFBundleIdentifier: 'dev.omnibuild.e2e.app',
      CFBundleName: 'OmniBuildShell',
      CFBundleShortVersionString: '2.5.0',
      CFBundleVersion: '42',
      CFBundleURLTypes: [
        { CFBundleURLSchemes: ['first', 'second'] },
        { CFBundleURLName: 'no schemes' },
      ],
    };
    let display = { CFBundleDisplayName: 'OmniBuild E2E' };
    respond = (program, args) => {
      if (program === 'plutil') {
        return { stdout: JSON.stringify({ ...info, ...display }) };
      }
      if (args.includes('--entitlements')) {
        return {
          stdout:
            '<dict>\n\t<key>application-identifier</key>\n\t<string>TEAM.dev.omnibuild.e2e.app</string>\n</dict>',
        };
      }
      return args[0] === '--display'
        ? { output: 'Authority=OmniBuild E2E\n' }
        : undefined;
    };

    const verified = await ios.verify({ ...artifacts, app: 'app.ipa' });
    display = {} as typeof display;
    respond = (
      (previous) => (program: string, args: string[]) =>
        args.includes('--entitlements')
          ? { stdout: '<dict/>' }
          : previous(program, args)
    )(respond);
    const nameless = await ios.verify({ ...artifacts, app: 'app.ipa' });

    expect(verified).toEqual({
      signer: 'OmniBuild E2E',
      report: {
        id: 'dev.omnibuild.e2e.app',
        name: 'OmniBuild E2E',
        version: '2.5.0',
        build: '42',
        payload: 'the payload',
        applicationIdentifier: 'TEAM.dev.omnibuild.e2e.app',
        urlSchemes: 'first, second',
      },
    });
    expect(commands().slice(0, 5)).toEqual([
      `ditto -x -k app.ipa ${unzipped}`,
      `codesign --verify --strict --deep --verbose=2 ${bundle}`,
      `codesign --display --verbose=2 ${bundle}`,
      `plutil -convert json -o - ${path.join(bundle, 'Info.plist')}`,
      `codesign --display --entitlements - --xml ${bundle}`,
    ]);
    expect(readFileSync).toHaveBeenCalledWith(
      path.join(bundle, 'payload.txt'),
      'utf8',
    );
    // Without a display name, the bundle's name is under the icon.
    expect(nameless.report).toMatchObject({
      name: 'OmniBuildShell',
      applicationIdentifier: undefined,
    });
  });

  it("reads each nested bundle's own ID and what its profile allowed it", async () => {
    const bundle = path.join(unzipped, 'Payload', 'OmniBuildShell.app');
    // An app extension, an ExtensionKit extension in a folder of its own,
    // and an App Clip.
    const folders: Record<string, string[]> = {
      [path.join(bundle, 'PlugIns')]: ['README', 'ShellNotifications.appex'],
      [path.join(bundle, 'Extensions')]: ['ShellIntents.appex'],
      [path.join(bundle, 'AppClips')]: ['ShellClip.app'],
    };
    const nested = Object.entries(folders).map(([folder, names]) =>
      path.join(folder, names.at(-1) ?? ''),
    );
    vi.mocked(readdirSync).mockImplementation(
      (folder) => (folders[String(folder)] ?? ['OmniBuildShell.app']) as never,
    );
    vi.mocked(existsSync).mockImplementation((file) => String(file) in folders);
    vi.mocked(readFileSync).mockReturnValue('the payload');
    /** The ID of the bundle a command names, by the nested one it names. */
    const id = (args: string[]) => {
      const named = nested.find((folder) =>
        args.some((arg) => arg.startsWith(folder)),
      );
      return named
        ? `dev.omnibuild.e2e.app.${path.parse(named).name}`
        : 'dev.omnibuild.e2e.app';
    };
    const clip = 'TEAM.dev.omnibuild.e2e.app.ShellClip';
    /** The entitlements codesign prints, which name the app and its App Clip to each other. */
    const entitlements = (bundleId: string) => {
      const named =
        bundleId === 'dev.omnibuild.e2e.app'
          ? `<key>com.apple.developer.associated-appclip-app-identifiers</key>\n\t<array>\n\t\t<string>${clip}</string>\n\t</array>`
          : bundleId.endsWith('ShellClip')
            ? '<key>com.apple.developer.parent-application-identifiers</key><array><string>TEAM.dev.omnibuild.e2e.app</string></array>'
            : '';
      return `<dict><key>application-identifier</key><string>TEAM.${bundleId}</string>${named}</dict>`;
    };
    respond = (program, args) => {
      if (program === 'plutil') {
        return { stdout: JSON.stringify({ CFBundleIdentifier: id(args) }) };
      }
      if (args.includes('--entitlements')) {
        return { stdout: entitlements(id(args)) };
      }
      return args[0] === '--display' ? { output: '' } : undefined;
    };

    const verified = await ios.verify({ ...artifacts, app: 'app.ipa' });

    expect(verified.report).toMatchObject({
      id: 'dev.omnibuild.e2e.app',
      applicationIdentifier: 'TEAM.dev.omnibuild.e2e.app',
      extensionId: 'dev.omnibuild.e2e.app.ShellNotifications',
      extensionApplicationIdentifier:
        'TEAM.dev.omnibuild.e2e.app.ShellNotifications',
      intentsExtensionId: 'dev.omnibuild.e2e.app.ShellIntents',
      intentsExtensionApplicationIdentifier:
        'TEAM.dev.omnibuild.e2e.app.ShellIntents',
      appClipId: 'dev.omnibuild.e2e.app.ShellClip',
      appClipApplicationIdentifier: clip,
      appClipParent: 'TEAM.dev.omnibuild.e2e.app',
      associatedAppClip: clip,
    });
  });

  it('reads what iOS shows in English, over what Info.plist says', async () => {
    vi.mocked(readdirSync).mockReturnValue(['OmniBuildShell.app'] as never);
    vi.mocked(readFileSync).mockReturnValue('the payload');
    const bundle = path.join(unzipped, 'Payload', 'OmniBuildShell.app');
    const strings = path.join(bundle, 'en.lproj', 'InfoPlist.strings');
    vi.mocked(existsSync).mockImplementation((file) => file === strings);
    respond = (program, args) => {
      if (program === 'plutil') {
        return {
          stdout: JSON.stringify(
            args.includes(strings)
              ? { NSCameraUsageDescription: 'The camera, in English.' }
              : {
                  CFBundleDisplayName: 'OmniBuild E2E',
                  NSCameraUsageDescription: 'The camera.',
                },
          ),
        };
      }
      return args[0] === '--display' ? { output: '' } : { stdout: '<dict/>' };
    };

    const verified = await ios.verify({ ...artifacts, app: 'app.ipa' });

    expect(verified.report).toMatchObject({
      name: 'OmniBuild E2E',
      cameraUsage: 'The camera, in English.',
    });
    expect(commands()).toContain(`plutil -convert json -o - ${strings}`);
  });
});

describe('stopDevice', () => {
  it('shuts the simulator down', async () => {
    await ios.stopDevice('UDID', artifacts);

    expect(commands()).toEqual(['xcrun simctl shutdown UDID']);
    expect(vi.mocked(run).mock.calls[0][2]).toEqual({
      log: 'commands.log',
      allowFailure: true,
    });
  });
});
