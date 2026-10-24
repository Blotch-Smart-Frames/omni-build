import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { crc32, inflateRawSync } from 'node:zlib';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { zip } from './zip.ts';

const DATE = new Date('2026-10-24T12:34:56Z');
const BINARY = Buffer.from('MZ'.padEnd(5000, 'omni'));
const LICENSE = Buffer.from('MIT License\n');

let dir: string;

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), 'omni-build-zip-test-'));
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

const archive = () =>
  zip(
    [
      { name: 'omni-build-x', mode: 0o755 },
      { name: 'omni-build-x/omni.exe', data: BINARY, mode: 0o755 },
      { name: 'omni-build-x/LICENSE', data: LICENSE, mode: 0o644 },
    ],
    DATE,
  );

/** The entries a zip's central directory lists, read as unzip reads them. */
const entries = (bytes: Buffer) => {
  const end = bytes.length - 22;
  expect(bytes.readUInt32LE(end)).toBe(0x06054b50);
  const count = bytes.readUInt16LE(end + 10);
  let at = bytes.readUInt32LE(end + 16);
  expect(at + bytes.readUInt32LE(end + 12)).toBe(end);
  return Array.from({ length: count }, () => {
    expect(bytes.readUInt32LE(at)).toBe(0x02014b50);
    const nameLength = bytes.readUInt16LE(at + 28);
    const entry = {
      name: bytes.toString('utf8', at + 46, at + 46 + nameLength),
      madeBy: bytes.readUInt16LE(at + 4),
      method: bytes.readUInt16LE(at + 10),
      time: bytes.readUInt16LE(at + 12),
      date: bytes.readUInt16LE(at + 14),
      crc: bytes.readUInt32LE(at + 16),
      size: bytes.readUInt32LE(at + 24),
      attributes: bytes.readUInt32LE(at + 38),
      offset: bytes.readUInt32LE(at + 42),
    };
    at += 46 + nameLength;
    // The local header repeats what the central one says.
    const local = entry.offset;
    expect(bytes.readUInt32LE(local)).toBe(0x04034b50);
    expect(bytes.readUInt16LE(local + 8)).toBe(entry.method);
    expect(bytes.readUInt32LE(local + 14)).toBe(entry.crc);
    const start = local + 30 + bytes.readUInt16LE(local + 26);
    const stored = bytes.subarray(
      start,
      start + bytes.readUInt32LE(local + 18),
    );
    return {
      ...entry,
      data: entry.method === 8 ? inflateRawSync(stored) : stored,
    };
  });
};

describe('zip', () => {
  it('writes each file deflated, with its CRC, mode and date', () => {
    const [folder, binary, license] = entries(archive());

    expect(folder).toMatchObject({
      name: 'omni-build-x/',
      method: 0,
      size: 0,
      // A Unix folder's mode, and MS-DOS's folder attribute.
      attributes: ((0o040755 << 16) | 0x10) >>> 0,
    });
    expect(binary).toMatchObject({
      name: 'omni-build-x/omni.exe',
      method: 8,
      madeBy: (3 << 8) | 20,
      size: BINARY.length,
      crc: crc32(BINARY),
      attributes: (0o100755 << 16) >>> 0,
      // 12:34:56, to the two seconds MS-DOS's time counts in, on 2026-10-24.
      time: (12 << 11) | (34 << 5) | 28,
      date: (46 << 9) | (10 << 5) | 24,
    });
    expect(binary.data).toEqual(BINARY);
    expect(license.data).toEqual(LICENSE);
    expect(license.attributes).toBe((0o100644 << 16) >>> 0);
  });

  it('writes what the system unpacks', () => {
    const file = path.join(dir, 'archive.zip');
    writeFileSync(file, archive());

    // bsdtar reads zips on macOS and Windows; GNU tar doesn't.
    if (process.platform === 'linux') {
      execFileSync('unzip', ['-q', file, '-d', dir]);
    } else {
      execFileSync('tar', ['-xf', file, '-C', dir]);
    }

    expect(readFileSync(path.join(dir, 'omni-build-x', 'omni.exe'))).toEqual(
      BINARY,
    );
    expect(readFileSync(path.join(dir, 'omni-build-x', 'LICENSE'))).toEqual(
      LICENSE,
    );
  });
});
