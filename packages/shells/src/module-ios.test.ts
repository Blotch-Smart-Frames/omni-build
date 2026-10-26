import { existsSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildIosModule } from './module-ios.ts';
import { run } from './process.ts';

// Nothing here touches the disk: Xcode is a stand-in.
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

const work = path.join('work', 'Haptics');
const derivedData = path.join(work, 'DerivedData');
const framework = (sdk: string) =>
  path.join(
    derivedData,
    'Build/Products',
    `Release-${sdk}`,
    'Haptics.framework',
  );
const xcframework = path.join('out', 'ios', 'Haptics.xcframework');

beforeEach(() => {
  vi.mocked(existsSync).mockReturnValue(true);
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('buildIosModule', () => {
  it('builds the framework for devices and the Simulator, and puts them together', async () => {
    const built = await buildIosModule({
      project: 'Haptics.xcodeproj',
      name: 'Haptics',
      out: 'out',
      work,
      log: 'commands.log',
    });

    const xcodebuild = (destination: string) =>
      `xcodebuild build -project Haptics.xcodeproj -scheme Haptics -configuration Release -destination ${destination} -derivedDataPath ${derivedData}`;
    expect(commands()).toEqual([
      `${xcodebuild('generic/platform=iOS')} CODE_SIGNING_ALLOWED=NO OTHER_LDFLAGS=$(inherited) -Wl,-adhoc_codesign`,
      xcodebuild('generic/platform=iOS Simulator'),
      `xcodebuild -create-xcframework -allow-internal-distribution -framework ${framework('iphoneos')} -framework ${framework('iphonesimulator')} -output ${xcframework}`,
    ]);
    expect(
      vi
        .mocked(run)
        .mock.calls.every(([, , options]) => options?.log === 'commands.log'),
    ).toBe(true);
    expect(built).toBe(xcframework);
    expect(mkdirSync).toHaveBeenCalledWith(path.join('out', 'ios'), {
      recursive: true,
    });
    // xcodebuild won't write over the one an earlier build made.
    expect(rmSync).toHaveBeenCalledWith(xcframework, {
      recursive: true,
      force: true,
    });
  });

  it("says when the scheme doesn't build a framework of its name", async () => {
    vi.mocked(existsSync).mockReturnValue(false);

    await expect(
      buildIosModule({
        project: 'Haptics.xcodeproj',
        name: 'Haptics',
        out: 'out',
        work,
      }),
    ).rejects.toThrow(
      `Xcode built no Haptics.framework in ${path.join(derivedData, 'Build/Products', 'Release-iphoneos')}: the scheme Haptics has to build a framework of the same name, as Xcode's Framework template names them both after the project.`,
    );
  });

  it("refuses a name that can't name its files, before building", async () => {
    await expect(
      buildIosModule({
        project: 'Haptics.xcodeproj',
        name: 'Haptic Feedback',
        out: 'out',
        work,
      }),
    ).rejects.toThrow("'Haptic Feedback' can't name a native module's files");
    expect(run).not.toHaveBeenCalled();
  });
});
