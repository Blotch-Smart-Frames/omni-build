import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

const cli = fileURLToPath(new URL('cli.ts', import.meta.url));

const run = (args: string[]) =>
  spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8' });

// Only what cli.ts adds to run(): passing the arguments, and the result
// reaching the terminal and the exit code. Nothing here packs or installs.
describe('release command', () => {
  it('passes its arguments to run() and prints what it logs', () => {
    const result = run(['--help']);

    expect(result.status).toBe(0);
    expect(result.stdout).toMatch(/^Usage: npm run release/);
  });

  it("prints run()'s message on stderr and exits with its status", () => {
    const result = run(['publish']);

    expect(result.status).toBe(2);
    expect(result.stderr).toBe(
      "release: Unknown command 'publish'. See --help.\n",
    );
  });
});
