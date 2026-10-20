import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import path from 'node:path';

import type { Manifest } from './manifests.ts';

/** A packed package: its tarball's file name, and its package.json. */
export interface Packed {
  file: string;
  manifest: Manifest;
}

export interface Registry {
  /** Where npm finds it, as `--registry` takes it. */
  url: string;
  close: () => Promise<void>;
}

/**
 * An npm registry on this computer, with the packages in `dir` and nothing
 * else, so npm installs them as it would from npmjs.com, picking each
 * optional dependency by its `os` and `cpu`, without their being published.
 * It answers the two requests an install makes: a package's document, which
 * lists its versions, and a version's tarball.
 */
export const serve = async (
  dir: string,
  packages: Packed[],
): Promise<Registry> => {
  const tarballs = new Map(
    packages.map(({ file }) => [file, readFileSync(path.join(dir, file))]),
  );
  const server = createServer((request, response) => {
    // A scoped package's name comes with its slash escaped, as %2f.
    const name = decodeURIComponent(request.url!.slice(1));
    const tarball = name.startsWith('-/') && tarballs.get(name.slice(2));
    if (tarball) {
      response.writeHead(200, { 'content-type': 'application/octet-stream' });
      response.end(tarball);
      return;
    }
    const versions = packages.filter(({ manifest }) => manifest.name === name);
    if (versions.length === 0) {
      response.writeHead(404, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ error: 'Not found' }));
      return;
    }
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify(document(url, name, versions, tarballs)));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
  return {
    url,
    close: () =>
      new Promise((resolve) => {
        // npm keeps its connections alive; they'd hold close() open.
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
};

/**
 * A package's document, as npmjs.com serves it: every version's package.json,
 * with where its tarball is and the hashes npm checks it against.
 */
const document = (
  url: string,
  name: string,
  versions: Packed[],
  tarballs: Map<string, Buffer>,
) => ({
  name,
  'dist-tags': { latest: versions.at(-1)!.manifest.version },
  versions: Object.fromEntries(
    versions.map(({ file, manifest }) => {
      const tarball = tarballs.get(file)!;
      return [
        manifest.version,
        {
          ...manifest,
          _id: `${name}@${manifest.version}`,
          dist: {
            tarball: `${url}-/${file}`,
            integrity: `sha512-${createHash('sha512').update(tarball).digest('base64')}`,
            shasum: createHash('sha1').update(tarball).digest('hex'),
          },
        },
      ];
    }),
  ),
});
