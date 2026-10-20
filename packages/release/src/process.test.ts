import { existsSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { exec, npmCommand } from './process.ts';

const node = (script: string) => exec(process.execPath, ['-e', script]);

describe('exec', () => {
  it("returns the command's stdout", async () => {
    expect(
      await exec(process.execPath, ['-e', 'console.log(process.cwd())'], {
        cwd: path.parse(process.cwd()).root,
        env: { ...process.env },
      }),
    ).toBe(`${path.parse(process.cwd()).root}\n`);
  });

  it('throws with all the command printed when it fails', async () => {
    await expect(
      node('console.log("out"); console.error("err"); process.exit(3)'),
    ).rejects.toThrow(/failed \(exit 3\):\nout\nerr$/);
    // Windows has no signals: a process that kills itself exits with 1.
    await expect(node('process.kill(process.pid, "SIGKILL")')).rejects.toThrow(
      /failed \((SIGKILL|exit 1)\)/,
    );
  });

  it("throws when the command can't start", async () => {
    await expect(exec(path.join('no', 'such', 'command'), [])).rejects.toThrow(
      /^couldn't run no.such.command: /,
    );
  });
});

describe('npmCommand', () => {
  it('runs the npm that npm run names', () => {
    expect(
      npmCommand({
        env: { npm_execpath: '/npm/bin/npm-cli.js' },
        execPath: '/bin/node',
      }),
    ).toEqual(['/bin/node', '/npm/bin/npm-cli.js']);
  });

  it('runs the npm installed beside Node otherwise', () => {
    expect(
      npmCommand({ env: {}, execPath: '/node/bin/node', platform: 'linux' }),
    ).toEqual([
      '/node/bin/node',
      path.join(
        '/node/bin',
        '..',
        'lib',
        'node_modules',
        'npm',
        'bin',
        'npm-cli.js',
      ),
    ]);
    expect(
      npmCommand({
        env: {},
        execPath: 'C:\\node\\node.exe',
        platform: 'win32',
      })[1],
    ).toBe(
      path.join(
        path.dirname('C:\\node\\node.exe'),
        'node_modules',
        'npm',
        'bin',
        'npm-cli.js',
      ),
    );
    // This machine's Node has its npm there, which is what CI runs.
    expect(existsSync(npmCommand({ env: {} })[1])).toBe(true);
  });
});
