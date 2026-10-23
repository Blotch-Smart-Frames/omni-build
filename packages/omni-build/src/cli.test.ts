import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

const { version } = createRequire(import.meta.url)('../package.json') as {
  version: string;
};

// The compiled launcher npm users get, rather than cli.ts, so these tests
// also catch a build that leaves its imports pointing at .ts files. turbo
// builds it before them, as it does the binary below (see turbo.json).
const cli = fileURLToPath(new URL('../dist/cli.js', import.meta.url));

// The binary `npm run build` makes; turbo builds it before these tests (see
// turbo.json). No platform package is installed in the repository, so the
// tests point the launcher at it the way a `cargo install` user would.
const binary = path.join(
  process.env.CARGO_TARGET_DIR ??
    fileURLToPath(new URL('../../../target', import.meta.url)),
  'release',
  process.platform === 'win32' ? 'omni.exe' : 'omni',
);

const run = (args: string[], omniBuildBinary = binary) =>
  spawnSync(process.execPath, [cli, ...args], {
    encoding: 'utf8',
    env: { ...process.env, OMNI_BUILD_BINARY: omniBuildBinary },
  });

describe('omni launcher', () => {
  it('passes arguments to the binary and shows its output', () => {
    // Also checks that the crate and the npm package are at the same version.
    expect(run(['--version']).stdout).toBe(`omni ${version}\n`);
  });

  it("exits with the binary's exit code", () => {
    // clap rejects an unknown flag with exit code 2.
    expect(run(['--no-such-flag']).status).toBe(2);
  });

  it('exits with 1 when the binary cannot be run', () => {
    const result = run(['--version'], path.join('does', 'not', 'exist'));

    expect({ status: result.status, stderr: result.stderr }).toEqual({
      status: 1,
      stderr: expect.stringContaining("omni: couldn't run"),
    });
  });
});
