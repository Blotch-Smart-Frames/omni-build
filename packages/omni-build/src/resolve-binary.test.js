import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { resolveBinary } from './resolve-binary.js';

const installedIn = (dir) => (name) => path.join(dir, name, 'package.json');

const notInstalled = (name) => {
  throw new Error(`Cannot find module '${name}/package.json'`);
};

describe('resolveBinary', () => {
  it('returns OMNI_BUILD_BINARY when it is set', () => {
    expect(
      resolveBinary({
        env: { OMNI_BUILD_BINARY: '/opt/omni-build' },
        resolvePackageJson: notInstalled,
      }),
    ).toBe('/opt/omni-build');
  });

  it('returns the binary in the platform package for this machine', () => {
    expect(
      resolveBinary({
        platform: 'darwin',
        arch: 'arm64',
        env: {},
        resolvePackageJson: installedIn('node_modules'),
      }),
    ).toBe(path.join('node_modules', '@omni-build/darwin-arm64', 'omni-build'));
  });

  it('returns the .exe on Windows', () => {
    expect(
      resolveBinary({
        platform: 'win32',
        arch: 'x64',
        env: {},
        resolvePackageJson: installedIn('node_modules'),
      }),
    ).toBe(
      path.join('node_modules', '@omni-build/win32-x64', 'omni-build.exe'),
    );
  });

  it('throws for a platform with no prebuilt binary', () => {
    expect(() =>
      resolveBinary({
        platform: 'freebsd',
        arch: 'x64',
        env: {},
        resolvePackageJson: installedIn('node_modules'),
      }),
    ).toThrow(/no prebuilt omni-build binary for freebsd-x64/);
  });

  it('throws when npm left the platform package out', () => {
    expect(() =>
      resolveBinary({
        platform: 'linux',
        arch: 'x64',
        env: {},
        resolvePackageJson: notInstalled,
      }),
    ).toThrow(/@omni-build\/linux-x64, .* isn't installed/);
  });
});
