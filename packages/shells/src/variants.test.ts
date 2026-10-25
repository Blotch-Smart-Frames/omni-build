import { describe, expect, it } from 'vitest';

import { capitalized, variants } from './variants.ts';

describe('variants', () => {
  it("names an Android shell's APKs and app bundles, debug and release", () => {
    expect(variants('WebViewShell', 'android')).toEqual([
      {
        file: 'WebViewShell-debug.apk',
        platform: 'android',
        format: 'apk',
        configuration: 'debug',
      },
      {
        file: 'WebViewShell-debug.aab',
        platform: 'android',
        format: 'aab',
        configuration: 'debug',
      },
      {
        file: 'WebViewShell-release.apk',
        platform: 'android',
        format: 'apk',
        configuration: 'release',
      },
      {
        file: 'WebViewShell-release.aab',
        platform: 'android',
        format: 'aab',
        configuration: 'release',
      },
    ]);
  });

  it("names an iOS shell's apps for the Simulator and for devices", () => {
    expect(
      variants('WebViewShell', 'ios').map(({ file, format }) => [file, format]),
    ).toEqual([
      ['WebViewShell-debug-simulator.zip', 'app'],
      ['WebViewShell-debug.ipa', 'ipa'],
      ['WebViewShell-release-simulator.zip', 'app'],
      ['WebViewShell-release.ipa', 'ipa'],
    ]);
  });

  it('names those of the configurations it gets', () => {
    expect(
      variants('Shell_2.x', 'ios', ['release']).map(({ file }) => file),
    ).toEqual(['Shell_2.x-release-simulator.zip', 'Shell_2.x-release.ipa']);
  });

  it.each(['My Shell', '-shell', 'shells/web', ''])(
    "refuses a name that can't name files: '%s'",
    (name) => {
      expect(() => variants(name, 'android')).toThrow(
        `'${name}' can't name a shell's files: use letters, digits, '.', '_' and '-', starting with a letter or digit.`,
      );
    },
  );
});

describe('capitalized', () => {
  it('names a configuration as Xcode and Gradle do', () => {
    expect(capitalized('debug')).toBe('Debug');
    expect(capitalized('release')).toBe('Release');
  });
});
