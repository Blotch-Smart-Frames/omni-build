import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

import type { Variant } from './variants.ts';

/**
 * What a shell's release has, in `<name>.json` beside its variants: each
 * one's file, what it is and its SHA-256, which a build tool pins it by
 * when it downloads it.
 */
export interface Manifest {
  name: string;
  /** The version of omni that repackaged every variant in the check. */
  omni: string;
  variants: (Variant & { sha256: string; size: number })[];
}

/** The manifest's file, in the folder with the variants. */
export const manifestFile = (dir: string, name: string) =>
  path.join(dir, `${name}.json`);

/**
 * Writes the manifest of the shell `name`'s variants, which are in `dir`,
 * and returns it.
 */
export const writeManifest = ({
  dir,
  name,
  omni,
  variants,
}: {
  dir: string;
  name: string;
  omni: string;
  variants: Variant[];
}): Manifest => {
  const manifest: Manifest = {
    name,
    omni,
    variants: variants.map((variant) => {
      const bytes = readFileSync(path.join(dir, variant.file));
      return {
        ...variant,
        sha256: createHash('sha256').update(bytes).digest('hex'),
        size: bytes.length,
      };
    }),
  };
  writeFileSync(
    manifestFile(dir, name),
    `${JSON.stringify(manifest, undefined, 2)}\n`,
  );
  return manifest;
};
