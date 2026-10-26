import { readFileSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import {
  GRADLE_PROJECTS,
  MODULES,
  SHELL,
  SHELLS,
  XCODE_PROJECTS,
  differences,
  shellOf,
} from './shell.ts';

describe('differences', () => {
  it('is empty when the report has every expected value', () => {
    // Values the harness doesn't expect, such as the app's own extras, are
    // fine.
    expect(differences(SHELL, { ...SHELL, extra: 'value' })).toEqual([]);
  });

  it('describes each expected value that differs or is missing', () => {
    expect(
      differences(
        { id: 'dev.omnibuild.shell', name: 'Shell', build: '1' },
        { id: 'dev.omnibuild.shell', name: 'Other' },
      ),
    ).toEqual([
      'name: expected "Shell", got "Other"',
      'build: expected "1", got null',
    ]);
  });
});

describe('shellOf', () => {
  it('tells which shell an app was made from by its name', () => {
    expect(shellOf(path.join('out', 'WebViewShell-linux.apk'))).toBe(
      'WebViewShell',
    );
    expect(shellOf('WebViewShell.ipa')).toBe('WebViewShell');
    expect(shellOf(path.join('out', 'OmniBuildShell-macos.zip'))).toBe(
      'OmniBuildShell',
    );
  });
});

describe('the shells', () => {
  /** A setting's values in each shell's file, by shell. */
  const settings = (files: Record<string, string>, pattern: RegExp) =>
    Object.fromEntries(
      Object.entries(files).map(([shell, file]) => [
        shell,
        [...readFileSync(file, 'utf8').matchAll(pattern)].map(
          ([, value]) => value,
        ),
      ]),
    );

  // The platform watch reads one shell's API levels and opens an issue to
  // raise them, and the Android Gradle plugin a level needs comes with it:
  // every shell has to move with it, or one would be left behind.
  it.each([
    ['compileSdk', /^\s*compileSdk = (\d+)$/gm, 'app/build.gradle.kts'],
    ['targetSdk', /^\s*targetSdk = (\d+)$/gm, 'app/build.gradle.kts'],
    ['minSdk', /^\s*minSdk = (\d+)$/gm, 'app/build.gradle.kts'],
    [
      'the Android Gradle plugin',
      /id\("com\.android\.application"\) version "([^"]+)"/g,
      'build.gradle.kts',
    ],
  ])('build for Android with the same %s', (_, pattern, file) => {
    const values = settings(
      Object.fromEntries(
        SHELLS.map((shell) => [shell, path.join(GRADLE_PROJECTS[shell], file)]),
      ),
      pattern,
    );

    expect(values.WebViewShell).toHaveLength(1);
    expect(values).toEqual(
      Object.fromEntries(SHELLS.map((shell) => [shell, values.WebViewShell])),
    );
  });

  // A module's binaries are built against its shell's runtime: each
  // module's against its shell's.
  it.each(
    SHELLS.flatMap((shell) =>
      (
        [
          [
            'compileSdk',
            /^\s*compileSdk = (\d+)$/gm,
            'app/build.gradle.kts',
            'module/build.gradle.kts',
          ],
          [
            'minSdk',
            /^\s*minSdk = (\d+)$/gm,
            'app/build.gradle.kts',
            'module/build.gradle.kts',
          ],
          [
            'the Android Gradle plugin',
            /id\("com\.android\.(?:application|library)"\) version "([^"]+)"/g,
            'build.gradle.kts',
            'build.gradle.kts',
          ],
        ] as const
      ).map(([setting, pattern, shellFile, moduleFile]) => ({
        shell,
        setting,
        pattern,
        shellFile,
        moduleFile,
      })),
    ),
  )(
    "and $shell's native module build for Android with the same $setting",
    ({ shell, pattern, shellFile, moduleFile }) => {
      const values = settings(
        {
          shell: path.join(GRADLE_PROJECTS[shell], shellFile),
          module: path.join(MODULES[shell].gradleProject, moduleFile),
        },
        pattern,
      );

      expect(values.shell).toHaveLength(1);
      expect(values.module).toEqual(values.shell);
    },
  );

  it('build for the same iOS version', () => {
    const values = settings(
      Object.fromEntries([
        ...SHELLS.map((shell) => [
          shell,
          path.join(XCODE_PROJECTS[shell], 'project.pbxproj'),
        ]),
        ...SHELLS.map((shell) => [
          MODULES[shell].name,
          path.join(MODULES[shell].xcodeProject, 'project.pbxproj'),
        ]),
      ]),
      /IPHONEOS_DEPLOYMENT_TARGET = ([\d.]+);/g,
    );

    expect(values.WebViewShell.length).toBeGreaterThan(0);
    expect(new Set(Object.values(values).flat()).size).toBe(1);
  });
});
