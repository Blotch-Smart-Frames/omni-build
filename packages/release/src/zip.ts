// The zip archives the release's downloads for Windows come in, since
// Windows opens zips itself, in Explorer and with PowerShell's
// Expand-Archive. Written here rather than by running zip, which Windows
// doesn't have, since the release tool runs on Node's built-in modules
// alone. Only what a few files need: deflated, with no ZIP64 and no
// encryption. A file of 4 GiB or more would need ZIP64, and fails to write
// (writeUInt32LE refuses the size), where the binaries are a few MiB.

import { crc32, deflateRawSync } from 'node:zlib';

import type { Entry } from './tar.ts';

const LOCAL_HEADER = 0x04034b50;
const CENTRAL_HEADER = 0x02014b50;
const END_OF_CENTRAL_DIRECTORY = 0x06054b50;
/** Version 2.0, the first with folders and deflate, which is all these use. */
const VERSION_NEEDED = 20;
/**
 * Made on Unix, so the upper half of each entry's external attributes is
 * its mode, which unzip on macOS and Linux restores.
 */
const MADE_BY = (3 << 8) | VERSION_NEEDED;
const STORED = 0;
const DEFLATED = 8;
/** MS-DOS's attribute for a folder, which Windows reads. */
const DOS_FOLDER = 0x10;

/** A date in MS-DOS's format, which zip keeps: its time and date fields. */
const dosTime = (date: Date) => ({
  time:
    (date.getUTCHours() << 11) |
    (date.getUTCMinutes() << 5) |
    (date.getUTCSeconds() >> 1),
  date:
    ((date.getUTCFullYear() - 1980) << 9) |
    ((date.getUTCMonth() + 1) << 5) |
    date.getUTCDate(),
});

/**
 * A zip archive of `entries`, in their order, each dated `mtime`: its files
 * deflated and its folders stored, as their names and a `/`.
 */
export const zip = (entries: Entry[], mtime: Date): Buffer => {
  const { time, date } = dosTime(mtime);
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const { name, data, mode } of entries) {
    const folder = data === undefined;
    const path = Buffer.from(folder ? `${name}/` : name);
    const contents = data ?? Buffer.alloc(0);
    const compressed = folder
      ? contents
      : deflateRawSync(contents, { level: 9 });
    const crc = crc32(contents);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(LOCAL_HEADER, 0);
    local.writeUInt16LE(VERSION_NEEDED, 4);
    local.writeUInt16LE(0, 6);
    local.writeUInt16LE(folder ? STORED : DEFLATED, 8);
    local.writeUInt16LE(time, 10);
    local.writeUInt16LE(date, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(compressed.length, 18);
    local.writeUInt32LE(contents.length, 22);
    local.writeUInt16LE(path.length, 26);
    local.writeUInt16LE(0, 28);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(CENTRAL_HEADER, 0);
    central.writeUInt16LE(MADE_BY, 4);
    local.copy(central, 6, 4, 30);
    // No comment, on the first disk, as binary data.
    central.writeUInt16LE(0, 32);
    central.writeUInt16LE(0, 34);
    central.writeUInt16LE(0, 36);
    const kind = folder ? 0o040000 : 0o100000;
    central.writeUInt32LE(
      (((kind | mode) << 16) | (folder ? DOS_FOLDER : 0)) >>> 0,
      38,
    );
    central.writeUInt32LE(offset, 42);

    locals.push(local, path, compressed);
    centrals.push(central, path);
    offset += local.length + path.length + compressed.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(END_OF_CENTRAL_DIRECTORY, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
};
