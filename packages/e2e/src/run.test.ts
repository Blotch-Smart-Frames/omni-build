import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { expected } from './repackage.ts';
import { HELP, type Platform, run } from './run.ts';

// What the Android apps must report, and what signed them, which the
// platform's own tools report rather than the app.
const { signer, ...REPORTED } = expected('android');

let packageDir: string;
let platform: Platform<string, string>;
let repackage: ReturnType<typeof fakeRepackage>;
/** Every console line, in order, prefixed with the stream it went to. */
let output: string[];

const fakeRepackage = () =>
  vi.fn(async ({ shell, outDir }: { shell: string; outDir: string }) =>
    path.join(outDir, `repackaged-${path.basename(shell)}`),
  );

/** A platform whose every step works, and whose apps report as they should. */
const fakePlatform = (): Platform<string, string> => ({
  name: 'Android',
  toolchain: 'Gradle',
  build: vi.fn(async ({ outDir }) => [
    path.join(outDir, 'Shell.apk'),
    path.join(outDir, 'Shell.aab'),
  ]),
  // Each app signed as it should be: an app bundle with an upload key.
  verify: vi.fn(async ({ app }) => ({
    signer: expected('android', app).signer,
  })),
  startDevice: vi.fn(async () => 'starting'),
  waitForDevice: vi.fn(async () => 'device'),
  runApp: vi.fn(async () => REPORTED),
  stopDevice: vi.fn(async () => {}),
});

const runWith = (...args: string[]) =>
  run(args, {
    platforms: { android: platform, ios: fakePlatform() },
    packageDir,
    repackage,
  });

/** A stage's default artifacts folder, and its command log. */
const artifactsOf = (stage: string) => {
  const artifactsDir = path.join(packageDir, 'artifacts', 'android', stage);
  return { artifactsDir, log: path.join(artifactsDir, 'commands.log') };
};

/** Files in a folder the build or repackage stage writes to by default. */
const writeFiles = (dir: string, ...names: string[]) => {
  mkdirSync(dir, { recursive: true });
  for (const name of names) {
    writeFileSync(path.join(dir, name), '');
  }
  return names.map((name) => path.join(dir, name));
};

beforeEach(() => {
  packageDir = mkdtempSync(path.join(tmpdir(), 'e2e-run-'));
  platform = fakePlatform();
  repackage = fakeRepackage();
  output = [];
  vi.spyOn(console, 'log').mockImplementation((line) => {
    output.push(`log: ${line}`);
  });
  vi.spyOn(console, 'error').mockImplementation((line) => {
    output.push(`error: ${line}`);
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('run', () => {
  it('shows its help', async () => {
    expect(await runWith('--help')).toBe(0);
    expect(output).toEqual([`log: ${HELP}`]);
  });

  it.each([
    ['no platform', []],
    ['an unknown platform', ['--platform', 'windows']],
    [
      'apps for a stage other than test',
      ['build', '--platform', 'android', 'app.apk'],
    ],
    ['apps with no stage', ['--platform', 'android', 'app.apk']],
  ])('shows its help and fails with %s', async (_, args) => {
    expect(await runWith(...args)).toBe(2);
    expect(output).toEqual([`log: ${HELP}`]);
  });

  describe('build', () => {
    it('builds the shells into build/shells, in an emptied artifacts folder', async () => {
      const [stale] = writeFiles(artifactsOf('build').artifactsDir, 'old.log');

      expect(await runWith('build', '--platform', 'android')).toBe(0);
      const outDir = path.join(packageDir, 'build', 'shells', 'android');
      expect(platform.build).toHaveBeenCalledWith({
        ...artifactsOf('build'),
        buildDir: path.join(packageDir, 'build'),
        outDir,
      });
      expect(existsSync(stale)).toBe(false);
      expect(output).toEqual([
        'log: \n▸ Building the Android shell with Gradle',
        `log: ${path.join(outDir, 'Shell.apk')}`,
        `log: ${path.join(outDir, 'Shell.aab')}`,
      ]);
    });

    it('builds them into --out, and keeps artifacts in --artifacts-dir', async () => {
      const elsewhere = path.join(packageDir, 'elsewhere');

      await runWith(
        'build',
        '--platform',
        'android',
        '--out',
        path.join(elsewhere, 'shells'),
        '--artifacts-dir',
        path.join(elsewhere, 'artifacts'),
      );

      expect(platform.build).toHaveBeenCalledWith(
        expect.objectContaining({
          artifactsDir: path.join(elsewhere, 'artifacts', 'android', 'build'),
          outDir: path.join(elsewhere, 'shells'),
        }),
      );
    });
  });

  describe('repackage', () => {
    it('repackages each --shell into build/repackaged', async () => {
      expect(
        await runWith(
          'repackage',
          '--platform',
          'android',
          '--shell',
          path.join(packageDir, 'a.apk'),
          '--shell',
          path.join(packageDir, 'a.aab'),
        ),
      ).toBe(0);

      const outDir = path.join(packageDir, 'build', 'repackaged', 'android');
      expect(repackage.mock.calls).toEqual([
        [
          {
            ...artifactsOf('repackage'),
            shell: path.join(packageDir, 'a.apk'),
            outDir,
          },
        ],
        [
          {
            ...artifactsOf('repackage'),
            shell: path.join(packageDir, 'a.aab'),
            outDir,
          },
        ],
      ]);
      expect(output).toEqual([
        'log: \n▸ Repackaging it with omni, with no toolchain in reach',
        `log: ${path.join(outDir, 'repackaged-a.apk')}`,
        `log: ${path.join(outDir, 'repackaged-a.aab')}`,
      ]);
    });

    it('repackages what the build stage wrote, into --out', async () => {
      const shells = path.join(packageDir, 'build', 'shells', 'android');
      // A file the OS added, which isn't a shell.
      writeFiles(shells, '.DS_Store', 'Shell.apk');

      await runWith(
        'repackage',
        '--platform',
        'android',
        '--out',
        path.join(packageDir, 'out'),
      );

      expect(repackage).toHaveBeenCalledExactlyOnceWith(
        expect.objectContaining({
          shell: path.join(shells, 'Shell.apk'),
          outDir: path.join(packageDir, 'out'),
        }),
      );
    });

    it.each([
      ['never ran', () => {}],
      [
        'wrote nothing',
        () =>
          writeFiles(
            path.join(packageDir, 'build', 'shells', 'android'),
            '.DS_Store',
          ),
      ],
    ])('says to run the build stage when it %s', async (_, arrange) => {
      arrange();

      expect(await runWith('repackage', '--platform', 'android')).toBe(1);
      expect(output).toEqual([
        `error: \n✗ There's nothing in ${path.join(packageDir, 'build', 'shells', 'android')}: run the build stage first.`,
        `error: \nLogs, reports and screenshots are in ${path.join(packageDir, 'artifacts', 'android')}`,
      ]);
      expect(repackage).not.toHaveBeenCalled();
    });
  });

  describe('test', () => {
    it('runs each app on the device and checks its report', async () => {
      const apk = path.join(packageDir, 'App-linux.apk');
      const aab = path.join(packageDir, 'App-linux.aab');
      // An app bundle is installed as the APK made from it.
      const bundleSigner = expected('android', aab).signer;
      vi.mocked(platform.verify).mockImplementation(async ({ app }) =>
        app === aab
          ? { installable: 'universal.apk', signer: bundleSigner }
          : { signer },
      );

      expect(await runWith('test', '--platform', 'android', apk, aab)).toBe(0);

      const test = artifactsOf('test');
      const options = { ...test, avd: undefined };
      expect(platform.startDevice).toHaveBeenCalledWith(options);
      expect(platform.waitForDevice).toHaveBeenCalledWith('starting', options);
      expect(vi.mocked(platform.runApp).mock.calls).toEqual([
        [
          {
            ...options,
            artifactsDir: path.join(test.artifactsDir, 'App-linux.apk'),
            device: 'device',
            app: apk,
            appId: 'dev.omnibuild.e2e.app',
          },
        ],
        [
          {
            ...options,
            artifactsDir: path.join(test.artifactsDir, 'App-linux.aab'),
            device: 'device',
            app: 'universal.apk',
            appId: 'dev.omnibuild.e2e.app',
          },
        ],
      ]);
      expect(
        JSON.parse(
          readFileSync(
            path.join(test.artifactsDir, 'App-linux.apk', 'report.json'),
            'utf8',
          ),
        ),
      ).toEqual({ ...REPORTED, signer });
      expect(platform.stopDevice).toHaveBeenCalledWith('device', options);
      expect(output).toEqual([
        'log: \n▸ Running 2 app(s) on Android',
        'log: ✓ App-linux.apk reported every expected value.',
        'log: ✓ App-linux.aab reported every expected value.',
      ]);
    });

    it("checks what an app's files say when the device can't run it", async () => {
      const app = path.join(packageDir, 'App-linux.ipa');
      vi.mocked(platform.verify).mockResolvedValue({
        signer,
        report: REPORTED,
      });

      expect(await runWith('test', '--platform', 'android', app)).toBe(0);

      expect(platform.runApp).not.toHaveBeenCalled();
      expect(
        JSON.parse(
          readFileSync(
            path.join(
              artifactsOf('test').artifactsDir,
              'App-linux.ipa',
              'report.json',
            ),
            'utf8',
          ),
        ),
      ).toEqual({ ...REPORTED, signer });
    });

    it('runs what the repackage stage wrote', async () => {
      const [app] = writeFiles(
        path.join(packageDir, 'build', 'repackaged', 'android'),
        'App-macos.apk',
      );

      await runWith('test', '--platform', 'android');

      expect(platform.runApp).toHaveBeenCalledWith(
        expect.objectContaining({ app }),
      );
    });

    it('boots the --avd and leaves it running with --keep-running', async () => {
      await runWith(
        'test',
        '--platform',
        'android',
        '--avd',
        'test-avd',
        '--keep-running',
        path.join(packageDir, 'App.apk'),
      );

      expect(platform.startDevice).toHaveBeenCalledWith({
        ...artifactsOf('test'),
        avd: 'test-avd',
      });
      expect(platform.stopDevice).not.toHaveBeenCalled();
    });

    it('runs every app, then fails with what each got wrong', async () => {
      vi.mocked(platform.verify)
        .mockRejectedValueOnce(new Error('not signed'))
        .mockResolvedValue({ signer });
      vi.mocked(platform.runApp)
        .mockResolvedValueOnce({ ...REPORTED, name: 'OmniBuild Shell' })
        .mockResolvedValue(REPORTED);

      expect(
        await runWith(
          'test',
          '--platform',
          'android',
          path.join(packageDir, 'A.aab'),
          path.join(packageDir, 'B.apk'),
          path.join(packageDir, 'C.apk'),
        ),
      ).toBe(1);

      expect(platform.stopDevice).toHaveBeenCalled();
      expect(output).toEqual([
        'log: \n▸ Running 3 app(s) on Android',
        'error: ✗ A.aab',
        'error: ✗ B.apk',
        'log: ✓ C.apk reported every expected value.',
        `error: \n✗ A.aab: not signed\n\nB.apk: it didn't report what it was built with:\n  name: expected "OmniBuild E2E", got "OmniBuild Shell"`,
        `error: \nLogs, reports and screenshots are in ${path.join(packageDir, 'artifacts', 'android')}`,
      ]);
    });

    it('says to run the repackage stage when it never ran', async () => {
      expect(await runWith('test', '--platform', 'android')).toBe(1);
      expect(output[0]).toBe(
        `error: \n✗ There's nothing in ${path.join(packageDir, 'build', 'repackaged', 'android')}: run the repackage stage first.`,
      );
      expect(platform.startDevice).not.toHaveBeenCalled();
    });
  });

  describe('with no stage', () => {
    it('starts the device, builds, repackages, and tests, here in turn', async () => {
      expect(await runWith('--platform', 'android')).toBe(0);

      // The device boots while the shells build.
      const [started, built, ran] = [
        platform.startDevice,
        platform.build,
        platform.runApp,
      ].map((step) => vi.mocked(step).mock.invocationCallOrder[0]);
      expect(started).toBeLessThan(built);
      expect(built).toBeLessThan(ran);
      const repackaged = path.join(
        packageDir,
        'build',
        'repackaged',
        'android',
      );
      expect(repackage).toHaveBeenCalledWith(
        expect.objectContaining({
          shell: path.join(
            packageDir,
            'build',
            'shells',
            'android',
            'Shell.apk',
          ),
          outDir: repackaged,
        }),
      );
      expect(platform.runApp).toHaveBeenCalledWith(
        expect.objectContaining({
          app: path.join(repackaged, 'repackaged-Shell.apk'),
        }),
      );
    });

    it('stops the device it started when a stage before the tests fails', async () => {
      vi.mocked(platform.build).mockRejectedValue(new Error('Xcode failed'));

      expect(await runWith('--platform', 'android')).toBe(1);

      expect(platform.stopDevice).toHaveBeenCalledWith(
        'starting',
        artifactsOf('test'),
      );
      expect(output.at(-2)).toBe('error: \n✗ Xcode failed');
      expect(platform.runApp).not.toHaveBeenCalled();
    });

    it('leaves it running with --keep-running', async () => {
      vi.mocked(platform.build).mockRejectedValue(new Error('Xcode failed'));

      expect(await runWith('--platform', 'android', '--keep-running')).toBe(1);

      expect(platform.stopDevice).not.toHaveBeenCalled();
    });
  });
});
