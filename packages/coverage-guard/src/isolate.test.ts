import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it, vi } from 'vitest';

import type { Exec } from './exec.ts';
import { escapeGlob, isolate } from './isolate.ts';

// A package with one module of each kind the guard tells apart, which
// these tests run real vitest in.
const FIXTURE = join(import.meta.dirname, '../fixtures/isolation');

const isolateFixture = (name: string) =>
  isolate(FIXTURE, `src/${name}.ts`, `src/${name}.test.ts`);

// Each starts a vitest of its own.
describe('isolate', { timeout: 60_000 }, () => {
  it('passes a module its own test covers in full', async () => {
    expect(await isolateFixture('sign')).toEqual({ kind: 'covered' });
  });

  it('says how much of a module its own test leaves out', async () => {
    // The fixture's threshold fails the run, as the packages' do.
    expect(await isolateFixture('half')).toEqual({
      kind: 'short',
      shortfalls: ['50% of branches'],
    });
  });

  it("says a module the package's coverage settings exclude isn't measured", async () => {
    expect(await isolateFixture('excluded')).toEqual({ kind: 'unmeasured' });
  });

  it('says a failing test fails, with its output', async () => {
    const result = await isolateFixture('broken');

    expect(result.kind).toBe('failed');
    expect(result.kind === 'failed' && result.output).toMatch(
      /expected 'broken' to be 'working'/,
    );
  });

  it('measures only the module, running only its test, and leaves no report behind', async () => {
    const exec = vi.fn<Exec>(async () => ({ ok: true, output: '' }));

    await isolate(FIXTURE, 'src/sign.ts', 'src/sign.test.ts', exec);

    const [command, args, cwd] = exec.mock.calls[0];
    expect(command).toBe(process.execPath);
    expect(cwd).toBe(FIXTURE);
    expect(args.slice(1, 3)).toEqual([
      'run',
      join(FIXTURE, 'src/sign.test.ts'),
    ]);
    expect(args).toContain('--coverage.include=src/sign.ts');
    const reports = args
      .find((arg) => arg.startsWith('--coverage.reportsDirectory='))!
      .split('=')[1];
    await expect(rm(reports)).rejects.toThrow(/ENOENT/);
  });

  it('fails a package without vitest', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'coverage-guard-test-'));
    try {
      expect(await isolate(dir, 'src/a.ts', 'src/a.test.ts')).toEqual({
        kind: 'failed',
        output: `Can't find vitest from ${dir}.`,
      });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe('escapeGlob', () => {
  it('escapes glob characters, so a file name matches only itself', () => {
    expect(escapeGlob('src/pages/[id].tsx')).toBe('src/pages/\\[id\\].tsx');
    expect(escapeGlob('src/{a,b}/*.ts')).toBe('src/\\{a,b\\}/\\*.ts');
    expect(escapeGlob('src/run.ts')).toBe('src/run.ts');
  });
});
