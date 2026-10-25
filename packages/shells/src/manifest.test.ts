import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { manifestFile, writeManifest } from './manifest.ts';
import { variants } from './variants.ts';

let dir: string;

beforeEach(() => {
  dir = mkdtempSync(path.join(os.tmpdir(), 'omni-build-shells-test-'));
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

describe('writeManifest', () => {
  it("writes each variant's file, what it is, its SHA-256 and size, and returns them", () => {
    const shells = variants('Shell', 'android', ['release']);
    writeFileSync(path.join(dir, 'Shell-release.apk'), 'an APK');
    writeFileSync(path.join(dir, 'Shell-release.aab'), 'an app bundle');

    const manifest = writeManifest({
      dir,
      name: 'Shell',
      omni: '1.2.3',
      variants: shells,
    });

    const sha256 = (text: string) =>
      createHash('sha256').update(text).digest('hex');
    expect(manifest).toEqual({
      name: 'Shell',
      omni: '1.2.3',
      variants: [
        {
          file: 'Shell-release.apk',
          platform: 'android',
          format: 'apk',
          configuration: 'release',
          sha256: sha256('an APK'),
          size: 6,
        },
        {
          file: 'Shell-release.aab',
          platform: 'android',
          format: 'aab',
          configuration: 'release',
          sha256: sha256('an app bundle'),
          size: 13,
        },
      ],
    });
    expect(manifestFile(dir, 'Shell')).toBe(path.join(dir, 'Shell.json'));
    expect(readFileSync(path.join(dir, 'Shell.json'), 'utf8')).toBe(
      `${JSON.stringify(manifest, undefined, 2)}\n`,
    );
  });
});
