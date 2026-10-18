import { describe, expect, it } from 'vitest';

import { exec } from './exec.ts';

// Real processes: this Node, running a line of script each.
const node = (script: string) =>
  exec(process.execPath, ['-e', script], process.cwd());

describe('exec', () => {
  it('says a command succeeded, with what it printed', async () => {
    expect(await node('console.log("out"); console.error("err")')).toEqual({
      ok: true,
      output: 'out\nerr\n',
    });
  });

  it('succeeds with no output for a command that prints nothing', async () => {
    expect(await node('')).toEqual({ ok: true, output: '' });
  });

  it('says a command failed, with what it printed, rather than throwing', async () => {
    expect(await node('console.error("broken"); process.exit(3)')).toEqual({
      ok: false,
      output: 'broken\n',
    });
  });

  it('says why a command that never started failed', async () => {
    const result = await exec('coverage-guard-no-such-command', [], '.');

    expect(result.ok).toBe(false);
    expect(result.output).toMatch(/ENOENT/);
  });

  it('runs the command in the directory given', async () => {
    const { output } = await exec(
      process.execPath,
      ['-e', 'console.log(process.cwd())'],
      import.meta.dirname,
    );

    expect(output.trim()).toBe(import.meta.dirname);
  });
});
