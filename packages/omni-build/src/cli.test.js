import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

const { version } = createRequire(import.meta.url)('../package.json');

const cli = fileURLToPath(new URL('cli.js', import.meta.url));

// The binary `npm run build` makes; turbo builds it before these tests (see
// turbo.json). No platform package is installed in the repository, so the
// tests point the launcher at it the way a `cargo install` user would.
const binary = path.join(
  process.env.CARGO_TARGET_DIR ??
    fileURLToPath(new URL('../../../target', import.meta.url)),
  'release',
  process.platform === 'win32' ? 'omni-build.exe' : 'omni-build',
);

const run = (args, omniBuildBinary = binary) =>
  spawnSync(process.execPath, [cli, ...args], {
    encoding: 'utf8',
    env: { ...process.env, OMNI_BUILD_BINARY: omniBuildBinary },
  });

describe('omni-build launcher', () => {
  it('passes arguments to the binary and shows its output', () => {
    // Also checks that the crate and the npm package are at the same version.
    expect(run(['--version']).stdout).toBe(`omni-build ${version}\n`);
  });

  it("exits with the binary's exit code", () => {
    // clap rejects an unknown flag with exit code 2.
    expect(run(['--no-such-flag']).status).toBe(2);
  });

  it('exits with 1 when the binary cannot be run', () => {
    const result = run(['--version'], path.join('does', 'not', 'exist'));

    expect({ status: result.status, stderr: result.stderr }).toEqual({
      status: 1,
      stderr: expect.stringContaining("omni-build: couldn't run"),
    });
  });
});
