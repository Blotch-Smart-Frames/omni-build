import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { PLATFORMS } from './platforms.ts';

// The parts of a Rust target triple each Node platform and CPU must map to.
// Linux is musl, not glibc: its binary has to be static to run on both (see
// platforms.ts).
const OS_TARGETS: Record<string, string> = {
  darwin: 'apple-darwin',
  linux: 'unknown-linux-musl',
  win32: 'pc-windows-msvc',
};
const CPU_TARGETS: Record<string, string> = {
  arm64: 'aarch64',
  x64: 'x86_64',
};

describe('PLATFORMS', () => {
  it.each(Object.entries(PLATFORMS))(
    'builds %s for the Rust target of that OS and CPU',
    (key, target) => {
      const [platform, arch] = key.split('-');

      expect(target).toBe(`${CPU_TARGETS[arch]}-${OS_TARGETS[platform]}`);
    },
  );

  it('is the list of targets CI builds and runs a release binary for', () => {
    // CI's build job is what proves each published binary builds and runs,
    // so a platform added here and not there would ship untried.
    const workflow = readFileSync(
      new URL('../../../.github/workflows/ci.yml', import.meta.url),
      'utf8',
    );
    const built = [...workflow.matchAll(/^ +- target: (\S+)$/gm)].map(
      ([, target]) => target,
    );

    expect(built.toSorted()).toEqual(Object.values(PLATFORMS).toSorted());
  });
});
