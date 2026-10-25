import { execFileSync } from 'node:child_process';
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { tar } from './tar.ts';

const DATE = new Date('2026-10-24T12:34:56Z');
const BINARY = Buffer.from('#!/bin/sh\necho omni\n');
/** Longer than a block, so its contents take two. */
const LICENSE = Buffer.from('MIT License\n'.repeat(50));

let dir: string;

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), 'omni-build-tar-test-'));
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

const archive = () =>
  tar(
    [
      { name: 'omni-build-x', mode: 0o755 },
      { name: 'omni-build-x/omni', data: BINARY, mode: 0o755 },
      { name: 'omni-build-x/LICENSE', data: LICENSE, mode: 0o644 },
    ],
    DATE,
  );

describe('tar', () => {
  it('writes ustar headers, each with its checksum, in blocks of 512 bytes', () => {
    const bytes = archive();
    const text = (offset: number, length: number) =>
      bytes
        .subarray(offset, offset + length)
        .toString('utf8')
        .replace(/\0.*$/s, '');

    // The folder, the binary in a block, the licence in two, and two empty
    // blocks at the end.
    expect(bytes.length).toBe(512 * (1 + 2 + 3 + 2));
    expect(text(0, 100)).toBe('omni-build-x/');
    expect(text(156, 1)).toBe('5');
    expect(text(257, 6)).toBe('ustar');
    expect(text(512, 100)).toBe('omni-build-x/omni');
    expect(text(512 + 100, 8)).toBe('0000755');
    expect(text(512 + 124, 12)).toBe(
      BINARY.length.toString(8).padStart(11, '0'),
    );
    expect(Number.parseInt(text(512 + 136, 12), 8)).toBe(DATE.getTime() / 1000);
    expect(text(512 + 156, 1)).toBe('0');
    const header = Buffer.from(bytes.subarray(512, 1024));
    const recorded = Number.parseInt(text(512 + 148, 8), 8);
    header.fill(' ', 148, 156);
    expect(recorded).toBe(header.reduce((sum, byte) => sum + byte, 0));
    expect(bytes.subarray(-1024).every((byte) => byte === 0)).toBe(true);
  });

  it("writes what tar unpacks, with the files' modes", () => {
    const file = path.join(dir, 'archive.tar');
    writeFileSync(file, archive());

    execFileSync('tar', ['-xf', file, '-C', dir]);

    expect(readFileSync(path.join(dir, 'omni-build-x', 'omni'))).toEqual(
      BINARY,
    );
    expect(readFileSync(path.join(dir, 'omni-build-x', 'LICENSE'))).toEqual(
      LICENSE,
    );
    // Windows keeps no modes.
    if (process.platform !== 'win32') {
      expect(
        statSync(path.join(dir, 'omni-build-x', 'omni')).mode & 0o777,
      ).toBe(0o755);
      expect(
        statSync(path.join(dir, 'omni-build-x', 'LICENSE')).mode & 0o777,
      ).toBe(0o644);
    }
  });

  it('refuses a name too long for ustar', () => {
    expect(() =>
      tar([{ name: 'x'.repeat(101), data: BINARY, mode: 0o644 }], DATE),
    ).toThrow(`${'x'.repeat(101)} is too long a name for a tar archive.`);
  });
});
