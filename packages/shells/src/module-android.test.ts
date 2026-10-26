import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { INIT_SCRIPT, buildAndroidModule } from './module-android.ts';
import { run } from './process.ts';

// Nothing here touches the disk: Gradle and d8 are stand-ins.
vi.mock('node:fs', () => ({
  cpSync: vi.fn(),
  existsSync: vi.fn(),
  mkdirSync: vi.fn(),
  rmSync: vi.fn(),
}));
vi.mock('./process.ts', () => ({ run: vi.fn() }));

/** Gradle prints its own lines among the init script's. */
const GRADLE_SAYS = [
  '> Task :haptics:omniBuildModule',
  'omni-build-d8: /sdk/build-tools/36.0.0/d8',
  'omni-build-min-sdk: 21',
  'omni-build-lib: /sdk/platforms/android-37.0/android.jar',
  'omni-build-lib: /sdk/build-tools/36.0.0/core-lambda-stubs.jar',
  'omni-build-classpath: /gradle/kotlin-stdlib-2.2.10.jar',
  'omni-build-classpath: /gradle/annotations-13.0.jar',
  '',
  'BUILD SUCCESSFUL in 4s',
].join('\n');

const saying = (stdout: string) =>
  vi
    .mocked(run)
    .mockResolvedValueOnce({ code: 0, stdout, output: stdout })
    .mockResolvedValue({ code: 0, stdout: '', output: '' });

afterEach(() => {
  vi.clearAllMocks();
});

const options = {
  project: 'android',
  library: 'haptics',
  out: 'out',
  work: 'work',
  env: { ANDROID_HOME: '/sdk' },
  log: 'commands.log',
};

describe('buildAndroidModule', () => {
  it('builds the library, and compiles its classes alone into DEX for its oldest Android', async () => {
    saying(GRADLE_SAYS);
    vi.mocked(existsSync).mockReturnValue(true);

    const built = await buildAndroidModule(options);

    const unpacked = path.resolve('work', 'aar');
    const android = path.join('out', 'android');
    expect(vi.mocked(run).mock.calls).toEqual([
      [
        path.join(path.resolve('android'), 'gradlew'),
        [
          '--init-script',
          INIT_SCRIPT,
          ':haptics:omniBuildModule',
          `-PomniBuildModuleOut=${unpacked}`,
          '--no-configuration-cache',
          '--console=plain',
        ],
        { cwd: path.resolve('android'), env: options.env, log: 'commands.log' },
      ],
      [
        '/sdk/build-tools/36.0.0/d8',
        [
          '--release',
          '--min-api',
          '21',
          '--lib',
          '/sdk/platforms/android-37.0/android.jar',
          '--lib',
          '/sdk/build-tools/36.0.0/core-lambda-stubs.jar',
          '--classpath',
          '/gradle/kotlin-stdlib-2.2.10.jar',
          '--classpath',
          '/gradle/annotations-13.0.jar',
          '--output',
          android,
          path.join(unpacked, 'classes.jar'),
        ],
        { env: options.env, log: 'commands.log' },
      ],
    ]);
    expect(built).toBe(android);
    // Nothing from an earlier build.
    expect(rmSync).toHaveBeenCalledWith(unpacked, {
      recursive: true,
      force: true,
    });
    expect(rmSync).toHaveBeenCalledWith(android, {
      recursive: true,
      force: true,
    });
    expect(mkdirSync).toHaveBeenCalledWith(android, { recursive: true });
    expect(cpSync).toHaveBeenCalledWith(
      path.join(unpacked, 'jni'),
      path.join(android, 'jni'),
      { recursive: true },
    );
  });

  it('leaves out native libraries the library has none of', async () => {
    saying(GRADLE_SAYS);
    // d8, but no jni/ in the AAR.
    vi.mocked(existsSync).mockImplementation(
      (file) => !String(file).endsWith('jni'),
    );

    await buildAndroidModule(options);

    expect(cpSync).not.toHaveBeenCalled();
  });

  it("says how to get d8 when the SDK hasn't the build tools it's in", async () => {
    saying(GRADLE_SAYS);
    vi.mocked(existsSync).mockReturnValue(false);

    await expect(buildAndroidModule(options)).rejects.toThrow(
      'There\'s no d8 at /sdk/build-tools/36.0.0/d8, in the Android SDK\'s build tools of the version the Android Gradle plugin uses: install them with the SDK Manager, or `sdkmanager "build-tools;36.0.0"`.',
    );
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("says when Gradle doesn't say how to compile the library", async () => {
    saying('BUILD SUCCESSFUL in 1s\n');

    await expect(buildAndroidModule(options)).rejects.toThrow(
      "Gradle didn't say how to compile :haptics for Android: it has to be an Android library module (com.android.library) with a release build type, as Android Studio's Android Library template makes one.",
    );
    saying('omni-build-d8: /sdk/build-tools/36.0.0/d8\n');
    await expect(buildAndroidModule(options)).rejects.toThrow(
      "Gradle didn't say how to compile :haptics for Android",
    );
  });
});
