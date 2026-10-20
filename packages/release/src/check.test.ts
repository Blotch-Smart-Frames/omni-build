import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { check } from './check.ts';
import { INDEX } from './pack.ts';
import type { Exec, ExecOptions } from './process.ts';

let packages: string;
/** Each command check ran, without npm's settings, and where. */
let commands: string[];
let options: ExecOptions[];

/** The packages `pack` would leave: the launcher, and Linux's and macOS's. */
const packed = (
  names = ['@omni-build/linux-x64', '@omni-build/darwin-arm64', 'omni-build'],
) => {
  const index = names.map((name) => ({
    file: `${name.replace(/\W/g, '-')}.tgz`,
    manifest: { name, version: '1.2.3' },
  }));
  for (const { file } of index) {
    writeFileSync(path.join(packages, file), 'tarball');
  }
  writeFileSync(path.join(packages, INDEX), JSON.stringify(index));
};

/**
 * npm and Node in the project: the install puts `installs` in
 * node_modules/@omni-build, omni-build prints `version`, and the API
 * reports `result`, writing the app.
 */
const fakeExec = ({
  installs = ['linux-x64'],
  version = 'omni-build 1.2.3\n',
  result = { event: 'repackaged' },
} = {}): Exec =>
  vi.fn(async (_node, args, execOptions = {}) => {
    options.push(execOptions);
    const settings = args.indexOf('--registry');
    commands.push(
      settings === -1
        ? args.slice(0, 2).join(' ')
        : [...args.slice(1, settings), ...args.slice(settings + 9)].join(' '),
    );
    const project = execOptions.cwd!;
    if (args[1] === 'install') {
      for (const key of installs) {
        mkdirSync(path.join(project, 'node_modules', '@omni-build', key), {
          recursive: true,
        });
      }
      return '';
    }
    if (args[1] === 'exec') {
      return version;
    }
    const out = JSON.parse(args[2].split('omniBuild(')[1].split(')')[0]).at(-1);
    writeFileSync(out, 'app');
    return `${JSON.stringify(result)}\n`;
  });

beforeEach(() => {
  packages = mkdtempSync(path.join(tmpdir(), 'omni-build-check-test-'));
  commands = [];
  options = [];
  packed();
  vi.stubEnv('OMNI_BUILD_BINARY', '/somewhere/omni-build');
  vi.stubEnv('npm_config_local_prefix', '/repo');
});

afterEach(() => {
  rmSync(packages, { recursive: true, force: true });
  vi.unstubAllEnvs();
});

const checking = (exec: Exec, platform = 'linux') =>
  check({ packages, exec, platform, arch: 'x64', log: () => {} });

describe('check', () => {
  it('installs omni-build from the packages, and runs it two ways', async () => {
    const logged: string[] = [];

    await check({
      packages,
      exec: fakeExec(),
      platform: 'linux',
      arch: 'x64',
      log: (line) => logged.push(line),
    });

    expect(commands).toEqual([
      'install omni-build@1.2.3',
      'exec --offline -- omni-build --version',
      '--input-type=module -e',
    ]);
    expect(logged).toEqual([
      "npm installed omni-build 1.2.3 with @omni-build/linux-x64, and no other platform's package",
      'omni-build --version runs it: omni-build 1.2.3',
      'omniBuild() runs it, as a build tool would: it repackaged a shell',
    ]);
    // In a project of its own, with a home of its own, and nothing that
    // would lead it to another binary or another npm's settings.
    const [{ cwd, env }] = options;
    expect(path.basename(cwd!)).toBe('project');
    expect(env).toMatchObject({
      HOME: expect.stringMatching(/home$/),
      USERPROFILE: env!.HOME,
    });
    expect(env).not.toHaveProperty('OMNI_BUILD_BINARY');
    expect(env).not.toHaveProperty('npm_config_local_prefix');
    expect(options.every((each) => each.cwd === cwd)).toBe(true);
  });

  it('installs from a registry with the packages and nothing else', async () => {
    let registry = '';
    const exec: Exec = async (_node, args, execOptions) => {
      if (args[1] !== 'install') {
        return fakeExec()(_node, args, execOptions);
      }
      registry = args[args.indexOf('--registry') + 1];
      const response = await fetch(
        new URL(encodeURIComponent('@omni-build/linux-x64'), registry),
      );
      const document = (await response.json()) as Record<string, unknown>;
      expect(document['dist-tags']).toEqual({ latest: '1.2.3' });
      return fakeExec()(_node, args, execOptions);
    };

    await checking(exec);

    expect(registry).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/$/);
    // Closed once it's done.
    await expect(fetch(registry)).rejects.toThrow();
  });

  it('fails when npm installs the wrong platforms, or none', async () => {
    await expect(
      checking(fakeExec({ installs: ['linux-x64', 'darwin-arm64'] })),
    ).rejects.toThrow(
      'npm installed @omni-build/darwin-arm64, @omni-build/linux-x64, where it should have installed @omni-build/linux-x64 alone.',
    );
    await expect(checking(fakeExec({ installs: [] }))).rejects.toThrow(
      'npm installed no platform package, where it should have installed @omni-build/linux-x64 alone.',
    );
  });

  it("fails when what npm installed doesn't run", async () => {
    await expect(
      checking(fakeExec({ version: 'omni-build 1.0.0\n' })),
    ).rejects.toThrow(
      'omni-build --version printed "omni-build 1.0.0", not "omni-build 1.2.3".',
    );
    await expect(
      checking(fakeExec({ result: { event: 'error' } })),
    ).rejects.toThrow(
      'omniBuild() ended with {"event":"error"}, not having repackaged the shell.',
    );
  });

  it('needs the launcher, and a package for this machine', async () => {
    await expect(checking(fakeExec(), 'freebsd')).rejects.toThrow(
      `There's no @omni-build/freebsd-x64 in ${packages}: omni-build has no binary for this machine.`,
    );
    packed(['@omni-build/linux-x64']);
    await expect(checking(fakeExec())).rejects.toThrow(
      `There's no omni-build package in ${packages}.`,
    );
    expect(commands).toEqual([]);
  });
});
