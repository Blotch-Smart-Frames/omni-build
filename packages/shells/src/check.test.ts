import {
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { check } from './check.ts';
import { type RunResult, run } from './process.ts';
import { variants } from './variants.ts';

// omni is a stand-in, which answers as the real one does with --json.
vi.mock('./process.ts', () => ({ run: vi.fn() }));

const CHECK = fileURLToPath(new URL('../check/', import.meta.url));

let shells: string;

/** What omni answers to `repackage`, by the shell's file name. */
let repackaged: (shell: string) => Partial<RunResult>;

/** The event omni reports for a variant it repackaged as it should. */
const repackagedEvent = (shell: string) => {
  const variant = [
    ...variants('Shell', 'android'),
    ...variants('Shell', 'ios'),
  ].find(({ file }) => path.basename(shell) === file);
  return JSON.stringify({
    event: 'repackaged',
    shell,
    out: 'out',
    platform: variant?.platform,
    format: variant?.format,
    message: `Repackaged ${shell} into out`,
  });
};

/** Writes a platform's variants of the shell Shell to `shells`. */
const build = (platform: 'android' | 'ios') =>
  variants('Shell', platform).forEach(({ file }) =>
    writeFileSync(path.join(shells, file), file),
  );

/** The repackage commands' shells, in order. */
const repackagedShells = () =>
  vi
    .mocked(run)
    .mock.calls.filter(([, args]) => args[0] === 'repackage')
    .map(([, args]) => path.basename(args[2]));

beforeEach(() => {
  shells = mkdtempSync(path.join(os.tmpdir(), 'omni-build-shells-test-'));
  repackaged = (shell) => ({ code: 0, stdout: `${repackagedEvent(shell)}\n` });
  vi.mocked(run).mockImplementation(async (_, args) => {
    const answer =
      args[0] === '--version'
        ? { code: 0, stdout: 'omni 1.2.3\n' }
        : repackaged(args[2]);
    return { code: 0, stdout: '', output: answer.stdout ?? '', ...answer };
  });
  vi.spyOn(console, 'log').mockImplementation(() => {});
});

afterEach(() => {
  rmSync(shells, { recursive: true, force: true });
  vi.restoreAllMocks();
  vi.mocked(run).mockReset();
});

describe('check', () => {
  it("repackages every variant of both platforms as the check's app, and writes their manifest", async () => {
    build('android');
    build('ios');

    const manifest = await check({
      shells,
      name: 'Shell',
      omni: '/bin/omni',
      log: 'commands.log',
    });

    expect(repackagedShells()).toEqual([
      'Shell-debug.apk',
      'Shell-debug.aab',
      'Shell-release.apk',
      'Shell-release.aab',
      'Shell-debug-simulator.zip',
      'Shell-debug.ipa',
      'Shell-release-simulator.zip',
      'Shell-release.ipa',
    ]);
    const [version, apk] = vi.mocked(run).mock.calls;
    expect(version).toEqual([
      '/bin/omni',
      ['--version'],
      { log: 'commands.log' },
    ]);
    const [, args, options] = apk;
    expect(args.slice(0, 2)).toEqual(['repackage', '--json']);
    expect(args.slice(2)).toEqual([
      path.join(shells, 'Shell-debug.apk'),
      '--config',
      path.join(CHECK, 'app.json'),
      '--out',
      expect.stringMatching(/Shell-debug\.apk$/),
    ]);
    expect(options).toMatchObject({
      log: 'commands.log',
      allowFailure: true,
      env: { OMNI_BUILD_KEY_PASSWORD: 'omnibuild-check' },
    });
    // The .ipa is signed with the check's key and wildcard profile.
    const ipa = vi
      .mocked(run)
      .mock.calls.find(([, args]) => args[2]?.endsWith('Shell-debug.ipa'));
    expect(ipa?.[1].slice(-4)).toEqual([
      '--signing-key',
      path.join(CHECK, 'signing-key.p12'),
      '--profile',
      path.join(CHECK, 'OmniBuildCheck.mobileprovision'),
    ]);
    // Each app it made is gone, and the variants and manifest are left.
    const out = path.dirname(apk[1][6]);
    expect(existsSync(out)).toBe(false);
    expect(readdirSync(shells)).toHaveLength(9);
    expect(manifest.omni).toBe('1.2.3');
    expect(manifest.variants.map(({ file }) => file)).toEqual(
      repackagedShells(),
    );
    expect(
      JSON.parse(readFileSync(path.join(shells, 'Shell.json'), 'utf8')),
    ).toEqual(manifest);
    expect(console.log).toHaveBeenCalledWith(
      '✓ omni 1.2.3 repackaged Shell-release.ipa',
    );
  });

  it('checks the one platform there is', async () => {
    build('ios');

    const manifest = await check({ shells, name: 'Shell', omni: 'omni' });

    expect(manifest.variants.map(({ platform }) => platform)).toEqual([
      'ios',
      'ios',
      'ios',
      'ios',
    ]);
  });

  it('says when a platform is missing some of its variants', async () => {
    build('android');
    rmSync(path.join(shells, 'Shell-release.aab'));

    await expect(
      check({ shells, name: 'Shell', omni: 'omni' }),
    ).rejects.toThrow(
      `${shells} has Shell-debug.apk, Shell-debug.aab, Shell-release.apk, but not Shell-release.aab: build every variant.`,
    );
    expect(run).not.toHaveBeenCalled();
  });

  it("says when there's no variant of the shell", async () => {
    await expect(
      check({ shells, name: 'Shell', omni: 'omni' }),
    ).rejects.toThrow(
      `There's no Shell shell in ${shells}: build it first (Shell-debug.apk, Shell-debug.aab, Shell-release.apk, Shell-release.aab, Shell-debug-simulator.zip, Shell-debug.ipa, Shell-release-simulator.zip, Shell-release.ipa).`,
    );
  });

  it("says when what it runs isn't omni", async () => {
    build('android');
    vi.mocked(run).mockResolvedValue({
      code: 0,
      stdout: 'v22.0.0\n',
      output: 'v22.0.0\n',
    });

    await expect(
      check({ shells, name: 'Shell', omni: 'node' }),
    ).rejects.toThrow("node isn't omni: `--version` printed v22.0.0\n");
  });

  it("says why omni couldn't repackage a variant", async () => {
    build('android');
    repackaged = (shell) =>
      shell.endsWith('Shell-release.apk')
        ? {
            code: 1,
            stdout: `${JSON.stringify({ event: 'error', message: "it isn't a zip" })}\n`,
          }
        : { code: 0, stdout: `${repackagedEvent(shell)}\n` };

    await expect(
      check({ shells, name: 'Shell', omni: 'omni' }),
    ).rejects.toThrow(
      "omni couldn't repackage Shell-release.apk: it isn't a zip",
    );
    expect(existsSync(path.join(shells, 'Shell.json'))).toBe(false);
  });

  it("shows what omni printed when it reported nothing, as an omni without --json doesn't", async () => {
    build('android');
    repackaged = () => ({
      code: 2,
      stdout: '',
      output: "error: unexpected argument '--json' found\n",
    });

    await expect(
      check({ shells, name: 'Shell', omni: 'omni' }),
    ).rejects.toThrow(
      "omni couldn't repackage Shell-debug.apk: error: unexpected argument '--json' found",
    );
  });

  it('says when omni took a variant for something its name says it is not', async () => {
    build('android');
    repackaged = (shell) => ({
      code: 0,
      stdout: `${repackagedEvent(shell).replace('"format":"apk"', '"format":"aab"')}\n`,
    });

    await expect(
      check({ shells, name: 'Shell', omni: 'omni' }),
    ).rejects.toThrow(
      'omni repackaged Shell-debug.apk as android aab, where its name says android apk.',
    );
  });
});
