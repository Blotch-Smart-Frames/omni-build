import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

const cli = fileURLToPath(new URL('cli.ts', import.meta.url));

const run = (args: string[]) =>
  spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8' });

// Only what cli.ts adds to run(): passing the arguments, and its status
// reaching the exit code. Nothing here builds a shell or boots a device.
describe('e2e command', () => {
  it('passes its arguments to run()', () => {
    const result = run(['--help']);

    expect(result.status).toBe(0);
    expect(result.stdout).toMatch(/^Usage: npm run e2e/);
  });

  it("exits with run()'s status", () => {
    expect(run(['--platform', 'windows']).status).toBe(2);
  });
});
