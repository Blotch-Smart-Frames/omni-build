import { existsSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildIos } from './ios.ts';
import { run } from './process.ts';

// Nothing here touches the disk: Xcode and ditto are stand-ins.
vi.mock('node:fs', () => ({
  existsSync: vi.fn(),
  mkdirSync: vi.fn(),
  rmSync: vi.fn(),
}));
vi.mock('./process.ts', () => ({ run: vi.fn() }));

/** The commands run, each as one line: the program and its arguments. */
const commands = () =>
  vi
    .mocked(run)
    .mock.calls.map(([command, args]) => [command, ...args].join(' '));

const work = path.join('work', 'Shell');
const derivedData = path.join(work, 'DerivedData');
const payload = path.join(work, 'ipa', 'Payload');
const xcodebuild = (configuration: string, destination: string) =>
  `xcodebuild build -project Shell.xcodeproj -scheme Shell -configuration ${configuration} -destination ${destination} -derivedDataPath ${derivedData}`;
const app = (products: string) =>
  path.join(derivedData, 'Build/Products', products, 'Shell.app');
const DEVICE_SETTINGS =
  'CODE_SIGNING_ALLOWED=NO OTHER_LDFLAGS=$(inherited) -Wl,-adhoc_codesign';

beforeEach(() => {
  vi.mocked(existsSync).mockReturnValue(true);
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('buildIos', () => {
  it('builds each configuration for the Simulator and for devices, and zips each', async () => {
    const built = await buildIos({
      project: 'Shell.xcodeproj',
      name: 'Shell',
      out: 'out',
      work,
      log: 'commands.log',
    });

    expect(commands()).toEqual([
      `${xcodebuild('Debug', 'generic/platform=iOS Simulator')} ENABLE_DEBUG_DYLIB=NO`,
      `ditto -c -k --keepParent ${app('Debug-iphonesimulator')} ${path.join('out', 'Shell-debug-simulator.zip')}`,
      `${xcodebuild('Debug', 'generic/platform=iOS')} ENABLE_DEBUG_DYLIB=NO ${DEVICE_SETTINGS}`,
      `ditto ${app('Debug-iphoneos')} ${path.join(payload, 'Shell.app')}`,
      `ditto -c -k --keepParent ${payload} ${path.join('out', 'Shell-debug.ipa')}`,
      xcodebuild('Release', 'generic/platform=iOS Simulator'),
      `ditto -c -k --keepParent ${app('Release-iphonesimulator')} ${path.join('out', 'Shell-release-simulator.zip')}`,
      `${xcodebuild('Release', 'generic/platform=iOS')} ${DEVICE_SETTINGS}`,
      `ditto ${app('Release-iphoneos')} ${path.join(payload, 'Shell.app')}`,
      `ditto -c -k --keepParent ${payload} ${path.join('out', 'Shell-release.ipa')}`,
    ]);
    expect(
      vi
        .mocked(run)
        .mock.calls.every(([, , options]) => options?.log === 'commands.log'),
    ).toBe(true);
    expect(built).toEqual([
      path.join('out', 'Shell-debug-simulator.zip'),
      path.join('out', 'Shell-debug.ipa'),
      path.join('out', 'Shell-release-simulator.zip'),
      path.join('out', 'Shell-release.ipa'),
    ]);
    expect(mkdirSync).toHaveBeenCalledWith('out', { recursive: true });
    // ditto adds to a zip that's there, and to a folder.
    expect(rmSync).toHaveBeenCalledWith(path.join('out', 'Shell-debug.ipa'), {
      force: true,
    });
    expect(rmSync).toHaveBeenCalledWith(payload, {
      recursive: true,
      force: true,
    });
  });

  it('builds only the configurations it gets', async () => {
    await buildIos({
      project: 'Shell.xcodeproj',
      name: 'Shell',
      out: 'out',
      work,
      configurations: ['release'],
    });

    expect(commands().filter((line) => line.startsWith('xcodebuild'))).toEqual([
      xcodebuild('Release', 'generic/platform=iOS Simulator'),
      `${xcodebuild('Release', 'generic/platform=iOS')} ${DEVICE_SETTINGS}`,
    ]);
  });

  it("says when the scheme doesn't build an app of its name", async () => {
    vi.mocked(existsSync).mockReturnValue(false);

    await expect(
      buildIos({ project: 'Shell.xcodeproj', name: 'Shell', out: 'out', work }),
    ).rejects.toThrow(
      `Xcode built no Shell.app in ${path.join(derivedData, 'Build/Products', 'Debug-iphonesimulator')}: the scheme Shell has to build an app of the same name, as Xcode's iOS App template names them both after the project.`,
    );
  });

  it("refuses a name that can't name its files, before building", async () => {
    await expect(
      buildIos({
        project: 'Shell.xcodeproj',
        name: 'A Shell',
        out: 'out',
        work,
      }),
    ).rejects.toThrow("'A Shell' can't name a shell's files");
    expect(run).not.toHaveBeenCalled();
  });
});
