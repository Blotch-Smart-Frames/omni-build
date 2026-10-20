import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type OmniBuildEvent,
  OmniBuildError,
  omniBuild,
  resolveBinary,
} from './index.ts';

// The binary `npm run build` makes; turbo builds it before these tests (see
// turbo.json), so the events are checked against what it really reports.
const binary = path.join(
  process.env.CARGO_TARGET_DIR ??
    fileURLToPath(new URL('../../../target', import.meta.url)),
  'release',
  process.platform === 'win32' ? 'omni-build.exe' : 'omni-build',
);
const shell = fileURLToPath(
  new URL('../../cli/tests/fixtures/shell.apk', import.meta.url),
);

/**
 * Node, running `script` in place of omni-build. The `--json` that follows
 * goes to the script, after `--`, rather than to Node.
 */
const fake = (script: string) =>
  omniBuild(['-e', script, '--'], { binary: process.execPath });

describe('omniBuild', () => {
  let home: string;
  let env: NodeJS.ProcessEnv;

  beforeEach(() => {
    // A home folder of its own, for the debug key omni-build creates.
    home = mkdtempSync(path.join(tmpdir(), 'omni-build-'));
    env = { ...process.env, HOME: home, USERPROFILE: home };
  });

  afterEach(() => {
    rmSync(home, { recursive: true, force: true });
    vi.unstubAllEnvs();
  });

  it('hands over each event, and resolves with the result', async () => {
    const out = path.join(home, 'app.apk');
    const events: OmniBuildEvent[] = [];

    const result = await omniBuild(['repackage', shell, '--out', out], {
      binary,
      env,
      onEvent: (event) => events.push(event),
    });

    const key = path.join(home, '.omni-build', 'android-debug-key.pem');
    expect(events).toEqual([
      {
        event: 'key-created',
        key: 'debug',
        path: key,
        message: `Created a debug key for signing Android apps: ${key}`,
      },
      {
        event: 'repackaged',
        shell,
        out,
        platform: 'android',
        format: 'apk',
        message: `Repackaged ${shell} into ${out}`,
      },
    ]);
    expect(result).toBe(events[1]);
  });

  it('runs the binary resolveBinary() finds', async () => {
    vi.stubEnv('OMNI_BUILD_BINARY', binary);
    const out = path.join(home, 'app.apk');

    const result = await omniBuild(['repackage', shell, '--out', out], {
      env,
      cwd: home,
    });

    expect(resolveBinary()).toBe(binary);
    expect(result).toMatchObject({ event: 'repackaged', out });
  });

  it("rejects with omni-build's reason", async () => {
    const events: OmniBuildEvent[] = [];

    const error = await omniBuild(['install', 'missing.apk'], {
      binary,
      env,
      onEvent: (event) => events.push(event),
    }).catch((error: unknown) => error);

    expect(events).toEqual([
      {
        event: 'error',
        message: expect.stringMatching(/^couldn't read the app missing\.apk: /),
      },
    ]);
    expect(error).toBeInstanceOf(OmniBuildError);
    expect(error).toHaveProperty('message', events[0]!.message);
    expect(error).toHaveProperty('name', 'OmniBuildError');
  });

  it('rejects with what omni-build said about its arguments', async () => {
    await expect(omniBuild(['--no-such-flag'], { binary })).rejects.toThrow(
      /^omni-build exited with 2: error: unexpected argument '--no-such-flag'/,
    );
  });

  it("rejects when there's no binary to run", async () => {
    // No binary named, and no platform package in this repository.
    vi.stubEnv('OMNI_BUILD_BINARY', '');

    await expect(omniBuild(['--version'])).rejects.toThrow(/isn't installed/);
    await expect(
      omniBuild(['--version'], { binary: path.join(home, 'nothing') }),
    ).rejects.toThrow(/^couldn't run .*nothing: /);
  });

  it('stops omni-build when the tool stops following it', async () => {
    const out = path.join(home, 'app.apk');
    const thrown = new Error('the tool failed');

    await expect(
      omniBuild(['repackage', shell, '--out', out], {
        binary,
        env,
        onEvent: () => {
          throw thrown;
        },
      }),
    ).rejects.toBe(thrown);
    // --version prints text, whatever the flags.
    await expect(omniBuild(['--version'], { binary })).rejects.toThrow(
      /^omni-build wrote a line that isn't JSON: omni-build \d/,
    );
  });

  it('is what the package exports, compiled', async () => {
    // By the package's name, as a build tool imports it: through the exports
    // in package.json to dist/index.js, which turbo builds before these
    // tests. A variable, so the type check doesn't need dist/ built.
    const name = 'omni-build';
    const built = (await import(name)) as Record<string, unknown>;

    expect(Object.keys(built).sort()).toEqual([
      'OmniBuildError',
      'omniBuild',
      'resolveBinary',
    ]);
  });

  it('rejects when omni-build stops without a result', async () => {
    await expect(fake('')).rejects.toThrow('omni-build reported nothing');
    // Windows has no signals: a process that kills itself exits with 1.
    await expect(fake('process.kill(process.pid, "SIGKILL")')).rejects.toThrow(
      /^omni-build (was stopped by SIGKILL|exited with 1)$/,
    );
  });
});
