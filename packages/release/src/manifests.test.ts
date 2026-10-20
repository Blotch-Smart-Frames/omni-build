import { describe, expect, it } from 'vitest';

import {
  binaryName,
  launcherManifest,
  packageName,
  platformManifest,
  platformReadme,
} from './manifests.ts';

const LAUNCHER = {
  name: 'omni-build',
  version: '1.2.3',
  homepage: 'https://example.com/omni-build',
  bugs: { url: 'https://example.com/omni-build/issues' },
  license: 'MIT',
  repository: {
    type: 'git',
    url: 'git+https://example.com/omni-build.git',
    directory: 'packages/omni-build',
  },
  bin: { 'omni-build': './dist/cli.js' },
  scripts: { build: 'tsc', prepack: 'npm run build' },
};

describe('platformManifest', () => {
  it('is installed only on its OS and CPU, at the launcher version', () => {
    expect(platformManifest('linux-x64', LAUNCHER)).toEqual({
      name: '@omni-build/linux-x64',
      version: '1.2.3',
      description:
        "The omni-build binary for Linux on x64. Install omni-build, which installs this package on the machines it's for",
      homepage: LAUNCHER.homepage,
      bugs: LAUNCHER.bugs,
      license: 'MIT',
      repository: { ...LAUNCHER.repository, directory: 'packages/cli' },
      os: ['linux'],
      cpu: ['x64'],
      files: ['omni-build'],
      preferUnplugged: true,
      publishConfig: { access: 'public' },
    });
  });

  it("holds Windows's binary as an .exe", () => {
    expect(platformManifest('win32-arm64', LAUNCHER)).toMatchObject({
      description: expect.stringMatching(
        /^The omni-build binary for Windows on arm64\./,
      ),
      os: ['win32'],
      cpu: ['arm64'],
      files: ['omni-build.exe'],
    });
    expect(binaryName('darwin-arm64')).toBe('omni-build');
  });
});

describe('launcherManifest', () => {
  it('lists every platform at its own version, without the scripts', () => {
    expect(launcherManifest(LAUNCHER, ['darwin-arm64', 'linux-x64'])).toEqual({
      name: 'omni-build',
      version: '1.2.3',
      homepage: LAUNCHER.homepage,
      bugs: LAUNCHER.bugs,
      license: 'MIT',
      repository: LAUNCHER.repository,
      bin: { 'omni-build': './dist/cli.js' },
      optionalDependencies: {
        '@omni-build/darwin-arm64': '1.2.3',
        '@omni-build/linux-x64': '1.2.3',
      },
    });
    // The repository's own package.json is left as it is.
    expect(LAUNCHER.scripts).toBeDefined();
  });
});

describe('platformReadme', () => {
  it('sends people to the launcher', () => {
    expect(platformReadme('darwin-x64')).toMatch(
      /^# @omni-build\/darwin-x64\n\nThe \[omni-build\]\(https:\/\/www\.npmjs\.com\/package\/omni-build\) binary for macOS on x64\.\n\nInstall `omni-build` rather than this package\./,
    );
  });

  it('names an OS it has no name for by its key', () => {
    expect(platformReadme('freebsd-x64')).toContain(
      ' binary for freebsd on x64.',
    );
    expect(packageName('freebsd-x64')).toBe('@omni-build/freebsd-x64');
  });
});
