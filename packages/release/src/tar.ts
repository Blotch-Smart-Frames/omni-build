// The tar archives the release's downloads for macOS and Linux come in, in
// the ustar format POSIX defines, which every tar reads: Homebrew's, GNU's,
// BSD's and the one Windows ships. Written here rather than by running tar,
// since GNU and BSD tar take different flags for the same archive, and the
// release tool runs on Node's built-in modules alone.

/** A file or folder in an archive. */
export interface Entry {
  /** Its path in the archive, with `/` between folders and none at the end. */
  name: string;
  /** Its contents, or none for a folder. */
  data?: Buffer;
  /** Its permissions, such as 0o755. */
  mode: number;
}

const BLOCK = 512;
/**
 * ustar's longest name. Longer ones need its prefix field, which the
 * archives' names are too short for.
 */
const NAME_MAX = 100;

/** A numeric field: `width - 1` octal digits and a NUL, as ustar has them. */
const octal = (value: number, width: number) =>
  `${value.toString(8).padStart(width - 1, '0')}\0`;

/** An entry's 512-byte header. */
const header = ({ name, data, mode }: Entry, mtime: Date) => {
  const folder = data === undefined;
  const path = folder ? `${name}/` : name;
  if (Buffer.byteLength(path) > NAME_MAX) {
    throw new Error(`${path} is too long a name for a tar archive.`);
  }
  const block = Buffer.alloc(BLOCK);
  const field = (offset: number, text: string) =>
    block.write(text, offset, 'utf8');
  field(0, path);
  field(100, octal(mode, 8));
  // Owned by root, as tar records files nobody in particular owns.
  field(108, octal(0, 8));
  field(116, octal(0, 8));
  field(124, octal(data?.length ?? 0, 12));
  field(136, octal(Math.floor(mtime.getTime() / 1000), 12));
  // The checksum is summed with its own field as spaces.
  field(148, ' '.repeat(8));
  field(156, folder ? '5' : '0');
  field(257, 'ustar\0');
  field(263, '00');
  const sum = block.reduce((total, byte) => total + byte, 0);
  field(148, `${sum.toString(8).padStart(6, '0')}\0 `);
  return block;
};

/**
 * A tar archive of `entries`, in their order, each dated `mtime`. The caller
 * compresses it.
 */
export const tar = (entries: Entry[], mtime: Date): Buffer =>
  Buffer.concat([
    ...entries.flatMap((entry) => {
      const data = entry.data ?? Buffer.alloc(0);
      const padding = (BLOCK - (data.length % BLOCK)) % BLOCK;
      return [header(entry, mtime), data, Buffer.alloc(padding)];
    }),
    // Two empty blocks end the archive.
    Buffer.alloc(BLOCK * 2),
  ]);
