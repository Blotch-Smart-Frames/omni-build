import {
  existsSync,
  mkdtempSync,
  readdirSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { run } from './process.ts';
import { expected, omniBuild, repackage } from './repackage.ts';
import { SHELL } from './shell.ts';

// omni-build, reporting to a build tool that each command worked: the
// keystore `key create` was given, or the app `repackage` wrote.
vi.mock('./process.ts', () => ({
  run: vi.fn(async (_command: string, args: string[]) => {
    const event =
      args[0] === 'key'
        ? { event: 'key-created', key: 'upload', path: args[2] }
        : { event: 'repackaged', out: args[args.indexOf('--out') + 1] };
    return { code: 0, stdout: `${JSON.stringify(event)}\n`, output: '' };
  }),
}));

// The real file system, but with existsSync() a spy, so a test can say the
// release binary in target/ is there whether or not this checkout built it.
vi.mock('node:fs', async (importOriginal) => {
  const fs = await importOriginal<typeof import('node:fs')>();
  return { ...fs, existsSync: vi.fn(fs.existsSync) };
});

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));
const APP_CONFIG = fileURLToPath(new URL('../app/app.json', import.meta.url));
/** All omni-build gets of the environment: the home and temporary folders, and the keystores' password. */
const ENV = {
  HOME: '/home/me',
  TMPDIR: '/tmp/me',
  OMNI_BUILD_KEY_PASSWORD: 'omnibuild-e2e',
};
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
  const values = {
    id: 'dev.omnibuild.e2e.app',
    name: 'OmniBuild E2E',
    version: '2.5.0',
    build: '42',
    payload: 'Payload from the OmniBuild E2E app\n',
  };

  it("is the app's values for Android, which omni-build repackages", () => {
    expect(expected('android')).toEqual({
      ...values,
      icon: '#E8590C',
      provider: 'dev.omnibuild.e2e.app.provider',
      permissions:
        'android.permission.INTERNET, android.permission.VIBRATE, dev.omnibuild.e2e.app.permission.SHELL',
      abis: 'arm64-v8a, x86_64',
      openedWith: 'omnibuild-e2e://report',
      signer: 'CN=OmniBuild Debug, O=OmniBuild',
    });
    // An app bundle, signed with the upload key omni-build created.
    expect(expected('android', 'OmniBuildShell-linux.aab').signer).toBe(
      'CN=OmniBuild E2E Upload',
    );
  });

  it("is the app's values for an iOS simulator build, signed ad hoc", () => {
    expect(expected('ios', 'OmniBuildShell-linux.zip')).toEqual({
      ...values,
      icon: '#E8590C',
      cameraUsage: "Scans the codes on OmniBuild's test cards.",
      framework: SHELL.framework,
      urlSchemes: 'omnibuild-e2e',
      extensionId: 'dev.omnibuild.e2e.app.ShellNotifications',
      signer: 'ad hoc',
    });
  });

  it("is what an iOS build for devices' files say, signed with the test key", () => {
    expect(expected('ios', 'OmniBuildShell-linux.ipa')).toEqual({
      ...values,
      cameraUsage: "Scans the codes on OmniBuild's test cards.",
      signer: 'OmniBuild E2E',
      applicationIdentifier: 'OMNIBUILD0.dev.omnibuild.e2e.app',
      urlSchemes: 'omnibuild-e2e',
      extensionId: 'dev.omnibuild.e2e.app.ShellNotifications',
      extensionApplicationIdentifier:
        'OMNIBUILD0.dev.omnibuild.e2e.app.ShellNotifications',
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
      ['repackage', shell, '--config', APP_CONFIG, '--out', app, '--json'],
      { log: 'commands.log', env: ENV },
    );
  });

  it('runs an iOS shell through it the same way', async () => {
    const shell = path.join('shells', 'OmniBuildShell.zip');
    onPlatform('darwin');

    const app = await repackage({ shell, outDir, log: 'commands.log' });

    expect(app).toBe(path.join(outDir, 'OmniBuildShell-macos.zip'));
    expect(run).toHaveBeenCalledWith(
      binary,
      ['repackage', shell, '--config', APP_CONFIG, '--out', app, '--json'],
      { log: 'commands.log', env: ENV },
    );
  });

  it("signs an iOS build for devices with the test key, and the app's and its extension's profiles", async () => {
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
        fileURLToPath(new URL('../app/signing-key.p12', import.meta.url)),
        '--profile',
        fileURLToPath(
          new URL('../app/OmniBuildE2E.mobileprovision', import.meta.url),
        ),
        '--profile',
        fileURLToPath(
          new URL('../app/ShellNotifications.mobileprovision', import.meta.url),
        ),
        '--out',
        app,
        '--json',
      ],
      { log: 'commands.log', env: ENV },
    );
  });

  it('signs an app bundle with an upload key it has omni-build create', async () => {
    const shell = path.join('shells', 'OmniBuildShell.aab');

    const app = await repackage({ shell, outDir, log: 'commands.log' });

    const [create, sign] = vi.mocked(run).mock.calls;
    const keystore = create[1][2];
    expect(path.dirname(path.dirname(keystore))).toBe(outDir);
    expect(path.basename(path.dirname(keystore))).toMatch(/^\.keys-/);
    expect(create).toEqual([
      binary,
      [
        'key',
        'create',
        keystore,
        '--subject',
        'CN=OmniBuild E2E Upload',
        '--json',
      ],
      { log: 'commands.log', env: ENV },
    ]);
    expect(sign).toEqual([
      binary,
      [
        'repackage',
        shell,
        '--config',
        APP_CONFIG,
        '--signing-key',
        keystore,
        '--out',
        app,
        '--json',
      ],
      { log: 'commands.log', env: ENV },
    ]);
    // Deleted, so no stage takes it for an app.
    expect(readdirSync(outDir)).toEqual([]);
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

describe('omniBuild', () => {
  const options = { log: 'commands.log' };

  it("fails when the command's result isn't what was expected", async () => {
    vi.mocked(run).mockResolvedValueOnce({
      code: 0,
      stdout:
        '{"event":"key-created","key":"debug","path":"key.pem"}\n{"event":"repackaged","out":"other.apk"}\n',
      output: '',
    });

    await expect(
      omniBuild(
        ['repackage'],
        { event: 'repackaged', out: 'app.apk' },
        options,
      ),
    ).rejects.toThrow(
      'omni-build ended with {"event":"repackaged","out":"other.apk"}, where out should be "app.apk"',
    );
  });

  it('fails when omni-build reports something other than JSON, or nothing', async () => {
    for (const stdout of ['Repackaged shell.apk into app.apk\n', '']) {
      vi.mocked(run).mockResolvedValueOnce({ code: 0, stdout, output: stdout });

      await expect(
        omniBuild(
          ['repackage'],
          { event: 'repackaged', out: 'app.apk' },
          options,
        ),
      ).rejects.toThrow(
        `omni-build ended with ${stdout.trim() || 'nothing'}, where event should be "repackaged" and out should be "app.apk"`,
      );
    }
  });
});
