import { describe, expect, it, vi } from 'vitest';

import { run } from './run.js';

const binary = '/opt/omni-build';
const resolve = () => binary;

const spawnReturning = (result) =>
  vi.fn(() => ({ status: null, signal: null, ...result }));

describe('run', () => {
  it('runs the binary with the arguments on this terminal', () => {
    const spawn = spawnReturning({ status: 0 });

    run(['build', '--json'], { resolve, spawn });

    expect(spawn).toHaveBeenCalledWith(binary, ['build', '--json'], {
      stdio: 'inherit',
    });
  });

  it("exits with the binary's exit code", () => {
    expect(run([], { resolve, spawn: spawnReturning({ status: 3 }) })).toEqual({
      status: 3,
      signal: null,
    });
  });

  it('passes on the signal that killed the binary', () => {
    expect(
      run([], { resolve, spawn: spawnReturning({ signal: 'SIGTERM' }) }),
    ).toEqual({ status: 1, signal: 'SIGTERM' });
  });

  it('exits with 1 and says why when there is no binary', () => {
    const resolveNothing = () => {
      throw new Error('no binary here');
    };

    expect(run([], { resolve: resolveNothing, spawn: vi.fn() })).toEqual({
      status: 1,
      message: 'omni-build: no binary here',
    });
  });

  it('exits with 1 and says why when the binary cannot be started', () => {
    const spawn = spawnReturning({ error: new Error('spawn EACCES') });

    expect(run([], { resolve, spawn })).toEqual({
      status: 1,
      message: "omni-build: couldn't run /opt/omni-build: spawn EACCES",
    });
  });
});
