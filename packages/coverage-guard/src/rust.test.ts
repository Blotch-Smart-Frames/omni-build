import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it, vi } from 'vitest';

import type { Exec } from './exec.ts';
import { rustCoverage } from './rust.ts';

const LCOV = `SF:/repo/packages/cli/src/main.rs
DA:1,1
LF:1
LH:1
end_of_record
SF:/repo/packages/cli/src/zip.rs
DA:1,1
end_of_record
`;

// cargo-llvm-cov as it writes its report: to the path after --output-path.
const cargo = (report: string) =>
  vi.fn<Exec>(async (_command, args) => {
    await writeFile(args[args.indexOf('--output-path') + 1], report);
    return { ok: true, output: '' };
  });

describe('rustCoverage', () => {
  it("lists the files in the last run's report", async () => {
    const exec = cargo(LCOV);

    expect(await rustCoverage('/repo/packages/cli', exec)).toEqual({
      ok: true,
      files: [
        '/repo/packages/cli/src/main.rs',
        '/repo/packages/cli/src/zip.rs',
      ],
    });
    const [command, args, cwd] = exec.mock.calls[0];
    expect(command).toBe('cargo');
    expect(args.slice(0, 3)).toEqual(['llvm-cov', 'report', '--lcov']);
    expect(cwd).toBe('/repo/packages/cli');
  });

  it('reads a report with Windows line endings', async () => {
    const result = await rustCoverage(
      '.',
      cargo('SF:C:\\repo\\src\\main.rs\r\nend_of_record\r\n'),
    );

    expect(result).toEqual({ ok: true, files: ['C:\\repo\\src\\main.rs'] });
  });

  it('leaves no report behind', async () => {
    const exec = cargo(LCOV);

    await rustCoverage('.', exec);

    const [, args] = exec.mock.calls[0];
    await expect(rm(args[args.indexOf('--output-path') + 1])).rejects.toThrow(
      /ENOENT/,
    );
  });

  it("says why it can't read a report, as when there's been no run", async () => {
    // Real Cargo, in a directory with no crate: it fails at once, and fails
    // the same way where cargo or cargo-llvm-cov isn't installed.
    const dir = await mkdtemp(join(tmpdir(), 'coverage-guard-test-'));
    try {
      const result = await rustCoverage(dir);

      expect(result.ok).toBe(false);
      expect(!result.ok && result.output).not.toBe('');
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
