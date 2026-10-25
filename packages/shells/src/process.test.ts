import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { run } from './process.ts';

/** Node, running a script: a command on every OS. */
const node = (script: string) => [process.execPath, ['-e', script]] as const;

let dir: string;

beforeEach(() => {
  dir = mkdtempSync(path.join(os.tmpdir(), 'omni-build-shells-test-'));
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
  vi.restoreAllMocks();
});

describe('run', () => {
  it('returns what the command printed, and logs it after the command', async () => {
    const log = path.join(dir, 'commands.log');
    const [command, args] = node(
      "process.stdout.write('out\\n'); process.stderr.write('err\\n')",
    );

    const result = await run(command, [...args], { log, cwd: dir });

    expect(result.code).toBe(0);
    expect(result.stdout).toBe('out\n');
    expect(result.output).toMatch(/^(out\nerr\n|err\nout\n)$/);
    const logged = readFileSync(log, 'utf8');
    expect(logged.startsWith(`\n$ ${command} -e `)).toBe(true);
    expect(logged).toContain('out\n');
    expect(logged).toContain('err\n');
  });

  it('writes the command and its output to stdout without a log', async () => {
    const write = vi
      .spyOn(process.stdout, 'write')
      .mockImplementation(() => true);
    const [command, args] = node("process.stderr.write('err\\n')");

    await run(command, [...args]);

    expect(write.mock.calls.map(([text]) => text)).toEqual([
      `\n$ ${command} -e process.stderr.write('err\\n')\n`,
      'err\n',
    ]);
  });

  it('passes the environment on', async () => {
    const [command, args] = node(
      'process.stdout.write(process.env.SHELL_TEST)',
    );

    const { stdout } = await run(command, [...args], {
      log: path.join(dir, 'log'),
      env: { ...process.env, SHELL_TEST: 'passed' },
    });

    expect(stdout).toBe('passed');
  });

  it('throws with the end of the output when the command fails', async () => {
    const [command, args] = node(
      'for (let i = 1; i <= 50; i++) console.log(i); process.exit(3)',
    );

    const failing = run(command, [...args], { log: path.join(dir, 'log') });

    await expect(failing).rejects.toThrow(/failed \(exit 3\):\n11\n12\n/);
    await expect(failing).rejects.not.toThrow(/\n10\n/);
  });

  // Windows has no signals: a process "killed" by one just exits.
  it.skipIf(process.platform === 'win32')(
    'says which signal stopped it',
    async () => {
      const [command, args] = node("process.kill(process.pid, 'SIGTERM')");
      const log = path.join(dir, 'log');

      await expect(run(command, [...args], { log })).rejects.toThrow(
        /failed \(SIGTERM\)/,
      );
      // With no exit code, a stopped command counts as exit code 1.
      expect(
        await run(command, [...args], { log, allowFailure: true }),
      ).toMatchObject({ code: 1 });
    },
  );

  it('returns a failure it was told to allow', async () => {
    const [command, args] = node("console.log('no'); process.exit(4)");

    expect(
      await run(command, [...args], {
        log: path.join(dir, 'log'),
        allowFailure: true,
      }),
    ).toEqual({ code: 4, stdout: 'no\n', output: 'no\n' });
  });

  it("throws when the command can't start", async () => {
    await expect(
      run(path.join(dir, 'missing'), [], { log: path.join(dir, 'log') }),
    ).rejects.toThrow(`couldn't run ${path.join(dir, 'missing')}: spawn`);
  });
});
