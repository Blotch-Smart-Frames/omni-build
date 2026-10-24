import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HELP, run } from './run.ts';

const pack = vi.fn(async () => []);
const check = vi.fn(async () => {});
const publish = vi.fn(async () => {});
const archive = vi.fn(() => new Map<string, string>());
const checkArchive = vi.fn(async () => {});
const formula = vi.fn(() => {});
const running = (args: string[]) =>
  run(args, { pack, check, publish, archive, checkArchive, formula });

beforeEach(() => {
  vi.spyOn(console, 'log').mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  pack.mockClear();
  check.mockClear();
  publish.mockClear();
  archive.mockClear();
  checkArchive.mockClear();
  formula.mockClear();
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

  it('publishes, for a tag or none', async () => {
    expect(
      await running(['publish', '--packages', 'npm', '--tag', 'v1.2.3']),
    ).toEqual({ status: 0 });
    expect(publish).toHaveBeenCalledWith({ packages: 'npm', tag: 'v1.2.3' });
    expect(await running(['publish', '--packages', 'npm'])).toEqual({
      status: 0,
    });
    expect(publish).toHaveBeenLastCalledWith({
      packages: 'npm',
      tag: undefined,
    });
  });

  it('archives', async () => {
    expect(
      await running(['archive', '--binaries', 'bin', '--out', 'archives']),
    ).toEqual({ status: 0 });
    expect(archive).toHaveBeenCalledWith({ binaries: 'bin', out: 'archives' });
  });

  it('checks an archive', async () => {
    expect(await running(['check-archive', '--archives', 'archives'])).toEqual({
      status: 0,
    });
    expect(checkArchive).toHaveBeenCalledWith({ archives: 'archives' });
  });

  it('writes the formula, for a URL or none', async () => {
    expect(
      await running([
        'formula',
        '--archives',
        'archives',
        '--out',
        'omni-build.rb',
        '--url',
        'file:///archives',
      ]),
    ).toEqual({ status: 0 });
    expect(formula).toHaveBeenCalledWith({
      archives: 'archives',
      out: 'omni-build.rb',
      url: 'file:///archives',
    });
    expect(
      await running(['formula', '--archives', 'archives', '--out', 'f.rb']),
    ).toEqual({ status: 0 });
    expect(formula).toHaveBeenLastCalledWith({
      archives: 'archives',
      out: 'f.rb',
      url: undefined,
    });
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
    expect(await running(['deploy'])).toEqual(
      usage("Unknown command 'deploy'"),
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
    expect(await running(['publish', '--tag', 'v1.2.3'])).toEqual(
      usage('publish needs --packages'),
    );
    expect(await running(['archive', '--binaries', 'bin'])).toEqual(
      usage('archive needs --binaries and --out'),
    );
    expect(await running(['archive', '--out', 'archives'])).toEqual(
      usage('archive needs --binaries and --out'),
    );
    expect(await running(['check-archive'])).toEqual(
      usage('check-archive needs --archives'),
    );
    expect(await running(['formula', '--archives', 'archives'])).toEqual(
      usage('formula needs --archives and --out'),
    );
    expect(await running(['formula', '--out', 'f.rb'])).toEqual(
      usage('formula needs --archives and --out'),
    );
    expect(await running(['check', '--bogus'])).toEqual({
      status: 2,
      message: expect.stringMatching(/^release: Unknown option '--bogus'/),
    });
    expect(pack).not.toHaveBeenCalled();
    expect(check).not.toHaveBeenCalled();
    expect(publish).not.toHaveBeenCalled();
    expect(archive).not.toHaveBeenCalled();
    expect(checkArchive).not.toHaveBeenCalled();
    expect(formula).not.toHaveBeenCalled();
  });
});
