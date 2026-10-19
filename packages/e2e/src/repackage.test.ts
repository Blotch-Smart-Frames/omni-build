import { existsSync, mkdtempSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { run } from './process.ts';
import { expected, repackage } from './repackage.ts';
import { SHELL } from './shell.ts';

vi.mock('./process.ts', () => ({
  run: vi.fn(async () => ({ code: 0, stdout: '', output: '' })),
}));

// The real file system, but with existsSync() a spy, so a test can say the
// release binary in target/ is there whether or not this checkout built it.
vi.mock('node:fs', async (importOriginal) => {
  const fs = await importOriginal<typeof import('node:fs')>();
  return { ...fs, existsSync: vi.fn(fs.existsSync) };
});

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));
const APP_CONFIG = fileURLToPath(new URL('../app/app.json', import.meta.url));
const PLATFORM = process.platform;

const onPlatform = (platform: string) => {
  Object.defineProperty(process, 'platform', { value: platform });
};

// Read before the tests stub TMPDIR.
const TEMP = tmpdir();
const tempDir = () => mkdtempSync(path.join(TEMP, 'e2e-repackage-'));

let binary: string;
let outDir: string;

beforeEach(() => {
  const dir = tempDir();
  binary = path.join(dir, 'omni-build');
  writeFileSync(binary, '');
  outDir = path.join(dir, 'out');
  vi.stubEnv('OMNI_BUILD_BINARY', binary);
  // A user's environment, which omni-build gets only the home and temporary
  // folders of.
  vi.stubEnv('HOME', '/home/me');
  vi.stubEnv('TMPDIR', '/tmp/me');
  vi.stubEnv('USERPROFILE', undefined);
  vi.stubEnv('TEMP', undefined);
  vi.stubEnv('TMP', undefined);
  vi.stubEnv('SystemRoot', undefined);
  vi.stubEnv('PATH', '/usr/bin');
  vi.stubEnv('JAVA_HOME', '/jdk');
  vi.stubEnv('ANDROID_HOME', '/sdk');
  onPlatform('linux');
});

afterEach(() => {
  onPlatform(PLATFORM);
  vi.unstubAllEnvs();
  vi.clearAllMocks();
});

describe('expected', () => {
  it("is the app's values for Android, which omni-build repackages", () => {
    expect(expected('android')).toEqual({
      ...SHELL,
      id: 'dev.omnibuild.e2e.app',
      name: 'OmniBuild E2E',
      version: '2.5.0',
      build: '42',
      payload: 'Payload from the OmniBuild E2E app\n',
      icon: '#E8590C',
      signer: 'CN=OmniBuild Debug, O=OmniBuild',
    });
  });

  it("is the app's values for an iOS simulator build, signed ad hoc", () => {
    expect(expected('ios', 'OmniBuildShell-linux.zip')).toEqual({
      ...SHELL,
      id: 'dev.omnibuild.e2e.app',
      name: 'OmniBuild E2E',
      version: '2.5.0',
      build: '42',
      payload: 'Payload from the OmniBuild E2E app\n',
      icon: '#E8590C',
      signer: 'ad hoc',
    });
  });

  it("is what an iOS build for devices' files say, signed with the test key", () => {
    expect(expected('ios', 'OmniBuildShell-linux.ipa')).toEqual({
      id: 'dev.omnibuild.e2e.app',
      name: 'OmniBuild E2E',
      version: '2.5.0',
      build: '42',
      payload: 'Payload from the OmniBuild E2E app\n',
      signer: 'OmniBuild E2E',
      applicationIdentifier: 'OMNIBUILD0.dev.omnibuild.e2e.app',
    });
  });
});

describe('repackage', () => {
  it('runs an Android shell through omni-build with no toolchain in reach', async () => {
    const shell = path.join('shells', 'OmniBuildShell.apk');

    const app = await repackage({
      shell,
      outDir,
      log: 'commands.log',
    });

    expect(app).toBe(path.join(outDir, 'OmniBuildShell-linux.apk'));
    expect(statSync(outDir).isDirectory()).toBe(true);
    expect(run).toHaveBeenCalledWith(
      binary,
      ['repackage', shell, '--config', APP_CONFIG, '--out', app],
      { log: 'commands.log', env: { HOME: '/home/me', TMPDIR: '/tmp/me' } },
    );
  });

  it('runs an iOS shell through it the same way', async () => {
    const shell = path.join('shells', 'OmniBuildShell.zip');
    onPlatform('darwin');

    const app = await repackage({ shell, outDir, log: 'commands.log' });

    expect(app).toBe(path.join(outDir, 'OmniBuildShell-macos.zip'));
    expect(run).toHaveBeenCalledWith(
      binary,
      ['repackage', shell, '--config', APP_CONFIG, '--out', app],
      { log: 'commands.log', env: { HOME: '/home/me', TMPDIR: '/tmp/me' } },
    );
  });

  it('signs an iOS build for devices with the test key and profile', async () => {
    const shell = path.join('shells', 'OmniBuildShell.ipa');
    onPlatform('darwin');

    const app = await repackage({ shell, outDir, log: 'commands.log' });

    expect(app).toBe(path.join(outDir, 'OmniBuildShell-macos.ipa'));
    expect(run).toHaveBeenCalledWith(
      binary,
      [
        'repackage',
        shell,
        '--config',
        APP_CONFIG,
        '--signing-key',
        fileURLToPath(new URL('../app/signing-key.pem', import.meta.url)),
        '--profile',
        fileURLToPath(
          new URL('../app/OmniBuildE2E.mobileprovision', import.meta.url),
        ),
        '--out',
        app,
      ],
      { log: 'commands.log', env: { HOME: '/home/me', TMPDIR: '/tmp/me' } },
    );
  });

  it('names an app for an OS it has no name for by its platform', async () => {
    onPlatform('freebsd');

    expect(
      await repackage({
        shell: 'OmniBuildShell.aab',
        outDir,
        log: 'commands.log',
      }),
    ).toBe(path.join(outDir, 'OmniBuildShell-freebsd.aab'));
  });

  it("runs this repository's release build without OMNI_BUILD_BINARY", async () => {
    vi.stubEnv('OMNI_BUILD_BINARY', undefined);
    vi.mocked(existsSync).mockReturnValueOnce(true).mockReturnValueOnce(true);

    await repackage({
      shell: 'OmniBuildShell.apk',
      outDir,
      log: 'commands.log',
    });
    onPlatform('win32');
    const app = await repackage({
      shell: 'OmniBuildShell.apk',
      outDir,
      log: 'commands.log',
    });

    expect(app).toBe(path.join(outDir, 'OmniBuildShell-windows.apk'));
    expect(vi.mocked(run).mock.calls.map(([command]) => command)).toEqual([
      path.join(REPO_ROOT, 'target/release', 'omni-build'),
      path.join(REPO_ROOT, 'target/release', 'omni-build.exe'),
    ]);
  });

  it('says how to get a binary when there is none', async () => {
    const missing = path.join(tempDir(), 'omni-build');
    vi.stubEnv('OMNI_BUILD_BINARY', missing);

    await expect(
      repackage({
        shell: 'OmniBuildShell.apk',
        outDir,
        log: 'commands.log',
      }),
    ).rejects.toThrow(
      `There's no omni-build binary at ${missing}. Build one with \`npm run build -w @omni-build/cli\`, or set OMNI_BUILD_BINARY to one.`,
    );
    expect(run).not.toHaveBeenCalled();
  });
});
