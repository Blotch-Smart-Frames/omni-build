import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildAndroid } from './android.ts';
import { run } from './process.ts';

// Gradle is a stand-in; the files it would write are written here.
vi.mock('./process.ts', () => ({ run: vi.fn() }));

let dir: string;
let project: string;
let out: string;

/** Writes a file where Gradle writes the app module's outputs. */
const output = (file: string, contents = file) => {
  const written = path.join(project, 'app/build/outputs', file);
  mkdirSync(path.dirname(written), { recursive: true });
  writeFileSync(written, contents);
};

/** Every variant, as Gradle names them when the release build is signed. */
const outputEverything = () => {
  output('apk/debug/app-debug.apk');
  output('apk/debug/output-metadata.json');
  output('bundle/debug/app-debug.aab');
  output('apk/release/app-release.apk');
  output('bundle/release/app-release.aab');
};

beforeEach(() => {
  dir = mkdtempSync(path.join(os.tmpdir(), 'omni-build-shells-test-'));
  project = path.join(dir, 'android');
  out = path.join(dir, 'out');
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
  vi.clearAllMocks();
});

describe('buildAndroid', () => {
  it('builds each configuration with Gradle, and copies the APK and app bundle under the shell name', async () => {
    outputEverything();
    const env = { ANDROID_HOME: '/sdk' };

    const built = await buildAndroid({
      // Relative, as on a command line: Gradle runs in the project's
      // folder, so the wrapper is found by its whole path.
      project: path.relative(process.cwd(), project),
      name: 'Shell',
      out,
      env,
      log: 'commands.log',
    });

    expect(run).toHaveBeenCalledExactlyOnceWith(
      path.join(project, 'gradlew'),
      [
        'assembleDebug',
        'bundleDebug',
        'assembleRelease',
        'bundleRelease',
        '--console=plain',
      ],
      { cwd: project, env, log: 'commands.log' },
    );
    expect(built).toEqual(
      [
        'Shell-debug.apk',
        'Shell-debug.aab',
        'Shell-release.apk',
        'Shell-release.aab',
      ].map((file) => path.join(out, file)),
    );
    expect(readFileSync(path.join(out, 'Shell-release.aab'), 'utf8')).toBe(
      'bundle/release/app-release.aab',
    );
    expect(readdirSync(out)).toHaveLength(4);
  });

  it("takes the release APK Gradle names unsigned, when there's no key for it", async () => {
    output('apk/release/app-release-unsigned.apk');
    output('bundle/release/app-release.aab');

    await buildAndroid({
      project,
      name: 'Shell',
      out,
      configurations: ['release'],
    });

    expect(vi.mocked(run).mock.calls[0][1]).toEqual([
      'assembleRelease',
      'bundleRelease',
      '--console=plain',
    ]);
    expect(readFileSync(path.join(out, 'Shell-release.apk'), 'utf8')).toBe(
      'apk/release/app-release-unsigned.apk',
    );
  });

  it("says when Gradle wrote no app where the app module's go", async () => {
    await expect(buildAndroid({ project, name: 'Shell', out })).rejects.toThrow(
      `Gradle wrote no .apk files to ${path.join(project, 'app/build/outputs', 'apk', 'debug')}, where it writes the app module's one: the app has to be the module app, as Android Studio's templates make it.`,
    );
  });

  it('says when Gradle wrote more than one, which it does for split APKs', async () => {
    outputEverything();
    output('apk/debug/app-x86-debug.apk');

    await expect(buildAndroid({ project, name: 'Shell', out })).rejects.toThrow(
      `Gradle wrote 2 .apk files to ${path.join(project, 'app/build/outputs', 'apk', 'debug')}`,
    );
  });

  it("refuses a name that can't name its files, before building", async () => {
    await expect(
      buildAndroid({ project, name: 'A Shell', out }),
    ).rejects.toThrow("'A Shell' can't name a shell's files");
    expect(run).not.toHaveBeenCalled();
  });
});
