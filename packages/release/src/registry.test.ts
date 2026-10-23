import { createHash } from 'node:crypto';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { type Registry, serve } from './registry.ts';

let dir: string;
let registry: Registry;

beforeEach(async () => {
  dir = mkdtempSync(path.join(tmpdir(), 'omni-build-registry-'));
  writeFileSync(path.join(dir, 'blotch-omni-build-1.0.0.tgz'), 'launcher');
  writeFileSync(
    path.join(dir, 'blotch-omni-build-linux-x64-1.0.0.tgz'),
    'linux',
  );
  registry = await serve(dir, [
    {
      file: 'blotch-omni-build-1.0.0.tgz',
      manifest: {
        name: '@blotch/omni-build',
        version: '1.0.0',
        bin: { omni: 'cli.js' },
      },
    },
    {
      file: 'blotch-omni-build-linux-x64-1.0.0.tgz',
      manifest: {
        name: '@blotch/omni-build-linux-x64',
        version: '1.0.0',
        os: ['linux'],
      },
    },
  ]);
});

afterEach(async () => {
  await registry.close();
  rmSync(dir, { recursive: true, force: true });
});

const get = (url: string) => fetch(new URL(url, registry.url));

describe('serve', () => {
  it("serves each package's document, as npm reads it", async () => {
    // As npm asks for a scoped package: with its slash escaped.
    const response = await get(
      encodeURIComponent('@blotch/omni-build-linux-x64'),
    );

    expect(registry.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/$/);
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      name: '@blotch/omni-build-linux-x64',
      'dist-tags': { latest: '1.0.0' },
      versions: {
        '1.0.0': {
          name: '@blotch/omni-build-linux-x64',
          version: '1.0.0',
          os: ['linux'],
          _id: '@blotch/omni-build-linux-x64@1.0.0',
          dist: {
            tarball: `${registry.url}-/blotch-omni-build-linux-x64-1.0.0.tgz`,
            integrity: `sha512-${createHash('sha512').update('linux').digest('base64')}`,
            shasum: createHash('sha1').update('linux').digest('hex'),
          },
        },
      },
    });
  });

  it('serves the tarballs the documents point to', async () => {
    const { versions } = (await (
      await get(encodeURIComponent('@blotch/omni-build'))
    ).json()) as {
      versions: Record<string, { dist: { tarball: string } }>;
    };
    const response = await get(versions['1.0.0'].dist.tarball);

    expect(response.status).toBe(200);
    expect(await response.text()).toBe('launcher');
  });

  it('has nothing else', async () => {
    for (const missing of ['left-pad', '-/left-pad-1.0.0.tgz']) {
      const response = await get(missing);

      expect(response.status).toBe(404);
      expect(await response.json()).toEqual({ error: 'Not found' });
    }
  });
});
