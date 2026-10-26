import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HELP, run } from './run.ts';

const buildIos = vi.fn(async () => []);
const buildAndroid = vi.fn(async () => []);
const check = vi.fn(async () => ({
  name: 'Shell',
  omni: '1.2.3',
  variants: [],
}));
const buildIosModule = vi.fn(async () => '');
const buildAndroidModule = vi.fn(async () => '');
const running = (args: string[]) =>
  run(args, {
    buildIos,
    buildAndroid,
    check,
    buildIosModule,
    buildAndroidModule,
  });

const BUILD = ['--project', 'project', '--name', 'Shell', '--out', 'out'];

beforeEach(() => {
  vi.spyOn(console, 'log').mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  buildIos.mockClear();
  buildAndroid.mockClear();
  check.mockClear();
  buildIosModule.mockClear();
  buildAndroidModule.mockClear();
});

describe('run', () => {
  it('builds an iOS shell, in derived data of its own that lasts between runs', async () => {
    expect(await running(['build', '--platform', 'ios', ...BUILD])).toEqual({
      status: 0,
    });
    expect(buildIos).toHaveBeenCalledWith({
      project: 'project',
      name: 'Shell',
      out: 'out',
      work: path.join(os.tmpdir(), 'omni-build-shells', 'Shell'),
    });
    expect(buildAndroid).not.toHaveBeenCalled();
  });

  it('builds an Android shell', async () => {
    expect(await running(['build', '--platform', 'android', ...BUILD])).toEqual(
      { status: 0 },
    );
    expect(buildAndroid).toHaveBeenCalledWith({
      project: 'project',
      name: 'Shell',
      out: 'out',
    });
  });

  it('checks the shells with omni on the PATH, or the one it gets', async () => {
    expect(
      await running(['check', '--shells', 'out', '--name', 'Shell']),
    ).toEqual({ status: 0 });
    expect(check).toHaveBeenCalledWith({
      shells: 'out',
      name: 'Shell',
      omni: 'omni',
    });
    await running([
      'check',
      '--shells',
      'out',
      '--name',
      'Shell',
      '--omni',
      '/bin/omni',
    ]);
    expect(check).toHaveBeenLastCalledWith({
      shells: 'out',
      name: 'Shell',
      omni: '/bin/omni',
    });
  });

  it("builds a native module's binaries for each platform", async () => {
    const module = ['--project', 'project', '--out', 'out'];

    expect(
      await running([
        'module',
        '--platform',
        'ios',
        ...module,
        '--name',
        'Haptics',
      ]),
    ).toEqual({ status: 0 });
    expect(
      await running([
        'module',
        '--platform',
        'android',
        ...module,
        '--library',
        'haptics',
      ]),
    ).toEqual({ status: 0 });

    const work = (what: string) =>
      path.join(os.tmpdir(), 'omni-build-modules', what);
    expect(buildIosModule).toHaveBeenCalledWith({
      project: 'project',
      name: 'Haptics',
      out: 'out',
      work: work('Haptics'),
    });
    expect(buildAndroidModule).toHaveBeenCalledWith({
      project: 'project',
      library: 'haptics',
      out: 'out',
      work: work('haptics'),
    });
  });

  it('prints the help', async () => {
    expect(await running(['--help'])).toEqual({ status: 0 });
    expect(console.log).toHaveBeenCalledWith(HELP);
  });

  it.each<[string[], string]>([
    [[], 'Name a command'],
    [['deploy'], "Unknown command 'deploy'"],
    [['build', 'extra'], "Unexpected argument 'extra'"],
    [
      ['build', '--platform', 'ios'],
      'build needs --platform, --project, --name and --out',
    ],
    [
      ['build', '--platform', 'windows', ...BUILD],
      "--platform is ios or android, not 'windows'",
    ],
    [['check', '--shells', 'out'], 'check needs --shells and --name'],
    ...[
      ['--platform', 'ios', '--project', 'project', '--out', 'out'],
      [
        '--platform',
        'android',
        '--project',
        'project',
        '--name',
        'Haptics',
        '--out',
        'out',
      ],
      [
        '--platform',
        'windows',
        '--project',
        'project',
        '--name',
        'Haptics',
        '--library',
        'haptics',
        '--out',
        'out',
      ],
    ].map((args): [string[], string] => [
      ['module', ...args],
      'module needs --platform ios, --project, --name and --out, or --platform android, --project, --library and --out',
    ]),
  ])('refuses %j: %s', async (args, problem) => {
    expect(await running(args)).toEqual({
      status: 2,
      message: `shells: ${problem}. See --help.`,
    });
  });

  it('refuses an option it has no use for', async () => {
    expect(await running(['build', '--fast'])).toEqual({
      status: 2,
      message: expect.stringMatching(/^shells: Unknown option '--fast'/),
    });
  });

  it('fails with the reason a command failed', async () => {
    check.mockRejectedValueOnce(new Error('omni broke'));

    expect(
      await running(['check', '--shells', 'out', '--name', 'Shell']),
    ).toEqual({ status: 1, message: 'shells: omni broke' });
  });
});
