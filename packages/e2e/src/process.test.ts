import { mkdtempSync, readFileSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

import { afterEach, describe, expect, it, vi } from 'vitest';

import { poll, run, step } from './process.ts';

// poll()'s half-second pauses, made instant.
vi.mock('node:timers/promises', () => ({ setTimeout: vi.fn(async () => {}) }));

afterEach(() => {
  vi.restoreAllMocks();
});

// Real child processes: Node itself, running a line of JavaScript, is the
// one program every OS the tests run on has.
const node = (code: string) => ['-e', code];

const tempDir = () => mkdtempSync(path.join(tmpdir(), 'e2e-process-'));

describe('run', () => {
  it('returns the exit code and the output', async () => {
    const result = await run(
      process.execPath,
      node("process.stdout.write('out\\n'); process.stderr.write('err\\n');"),
      {},
    );

    expect(result.code).toBe(0);
    expect(result.stdout).toBe('out\n');
    // stdout and stderr arrive on separate pipes, so their order varies.
    expect(result.output).toHaveLength('out\nerr\n'.length);
    expect(result.output).toContain('out\n');
    expect(result.output).toContain('err\n');
  });

  it('appends the command and its output to the log', async () => {
    const log = path.join(tempDir(), 'commands.log');

    await run(process.execPath, node("console.log('hello')"), { log });

    expect(readFileSync(log, 'utf8')).toBe(
      `\n$ ${process.execPath} -e console.log('hello')\nhello\n`,
    );
  });

  it('runs the command in `cwd`, with `env`', async () => {
    const cwd = tempDir();

    const { stdout } = await run(
      process.execPath,
      node(
        'console.log(JSON.stringify([process.cwd(), process.env.E2E_VALUE]))',
      ),
      { cwd, env: { ...process.env, E2E_VALUE: 'set' } },
    );

    const [dir, value] = JSON.parse(stdout);
    // realpath: macOS's temporary folder is reached through a symlink.
    expect({ dir: realpathSync(dir), value }).toEqual({
      dir: realpathSync(cwd),
      value: 'set',
    });
  });

  it('throws with the end of the output when the command fails', async () => {
    const lines = node(
      'for (let i = 1; i <= 50; i++) console.log(`line ${i}`); process.exit(3);',
    );

    const error: Error = await run(process.execPath, lines, {}).catch(
      (thrown) => thrown,
    );

    expect(error.message).toMatch(
      /^`.* -e for .*` failed \(exit 3\):\nline 11\n/,
    );
    expect(error.message).toMatch(/\nline 50$/);
    expect(error.message).not.toContain('line 10\n');
  });

  // Windows has no signals: a process "killed" by one just exits.
  it.skipIf(process.platform === 'win32')(
    'names the signal that killed the command',
    async () => {
      const killed = node("process.kill(process.pid, 'SIGTERM')");

      await expect(run(process.execPath, killed, {})).rejects.toThrow(
        /failed \(SIGTERM\)/,
      );
      // With no exit code, a killed command counts as exit code 1.
      expect(
        await run(process.execPath, killed, { allowFailure: true }),
      ).toMatchObject({ code: 1 });
    },
  );

  it('returns a failed exit code with allowFailure', async () => {
    expect(
      await run(process.execPath, node('process.exit(4)'), {
        allowFailure: true,
      }),
    ).toMatchObject({ code: 4 });
  });

  it('throws when the command cannot be started', async () => {
    await expect(
      run(path.join(tempDir(), 'no-such-command'), [], {}),
    ).rejects.toThrow(/^couldn't run .*no-such-command: /);
  });
});

describe('poll', () => {
  it('tries until the attempt returns something, and returns it', async () => {
    const attempt = vi
      .fn<() => Promise<string | undefined>>()
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce('ready');

    expect(await poll(attempt, { timeoutMs: 1000, message: 'never' })).toBe(
      'ready',
    );
    expect(attempt).toHaveBeenCalledTimes(2);
    expect(sleep).toHaveBeenCalledWith(500);
  });

  it('throws the message once the time is up', async () => {
    // The deadline is set at 0, and the first attempt ends after it.
    vi.spyOn(Date, 'now').mockReturnValueOnce(0).mockReturnValue(1001);

    await expect(
      poll(async () => undefined, { timeoutMs: 1000, message: 'gave up' }),
    ).rejects.toThrow('gave up');
  });
});

describe('step', () => {
  it('prints the step on a line of its own', () => {
    const log = vi.spyOn(console, 'log').mockImplementation(() => {});

    step('Building');

    expect(log).toHaveBeenCalledWith('\n▸ Building');
  });
});
