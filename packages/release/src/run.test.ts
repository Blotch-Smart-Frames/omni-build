import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HELP, run } from './run.ts';

const pack = vi.fn(async () => []);
const check = vi.fn(async () => {});
const running = (args: string[]) => run(args, { pack, check });

beforeEach(() => {
  vi.spyOn(console, 'log').mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  pack.mockClear();
  check.mockClear();
});

describe('run', () => {
  it('packs', async () => {
    expect(
      await running(['pack', '--binaries', 'bin', '--out', 'npm']),
    ).toEqual({ status: 0 });
    expect(pack).toHaveBeenCalledWith({ binaries: 'bin', out: 'npm' });
  });

  it('checks', async () => {
    expect(await running(['check', '--packages', 'npm'])).toEqual({
      status: 0,
    });
    expect(check).toHaveBeenCalledWith({ packages: 'npm' });
  });

  it('says why a command failed', async () => {
    pack.mockRejectedValueOnce(new Error("There's no binary"));

    expect(
      await running(['pack', '--binaries', 'bin', '--out', 'npm']),
    ).toEqual({
      status: 1,
      message: "release: There's no binary",
    });
  });

  it('shows the help', async () => {
    expect(await running(['--help'])).toEqual({ status: 0 });
    expect(console.log).toHaveBeenCalledWith(HELP);
  });

  it("refuses what it doesn't understand", async () => {
    const usage = (problem: string) => ({
      status: 2,
      message: `release: ${problem}. See --help.`,
    });

    expect(await running([])).toEqual(usage('Name a command'));
    expect(await running(['publish'])).toEqual(
      usage("Unknown command 'publish'"),
    );
    expect(await running(['pack', 'extra'])).toEqual(
      usage("Unexpected argument 'extra'"),
    );
    expect(await running(['pack', '--out', 'npm'])).toEqual(
      usage('pack needs --binaries and --out'),
    );
    expect(await running(['pack', '--binaries', 'bin'])).toEqual(
      usage('pack needs --binaries and --out'),
    );
    expect(await running(['check'])).toEqual(usage('check needs --packages'));
    expect(await running(['check', '--bogus'])).toEqual({
      status: 2,
      message: expect.stringMatching(/^release: Unknown option '--bogus'/),
    });
    expect(pack).not.toHaveBeenCalled();
    expect(check).not.toHaveBeenCalled();
  });
});
