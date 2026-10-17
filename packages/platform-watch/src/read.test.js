import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { describe, expect, it, vi } from 'vitest';

import { readSources } from './read.js';

const remote = {
  id: 'page',
  location: 'https://example.com/page',
  parse: (content) => ({ length: content.length }),
};

const local = {
  id: 'file',
  location: 'config/file.txt',
  parse: (content) => content.trim(),
};

const root = mkdtempSync(path.join(tmpdir(), 'platform-watch-'));
writeFileSync(path.join(root, 'file.txt'), 'pinned\n');

const read = (sources, fetch, wait = vi.fn(async () => {})) =>
  readSources(sources, { root, fetch, wait });

describe('readSources', () => {
  it('downloads a page and parses it', async () => {
    const fetch = vi.fn(async () => new Response('<html>'));

    const readings = await read([remote], fetch);

    expect(readings.get('page')).toEqual({
      source: remote,
      ok: true,
      value: { length: 6 },
    });
    expect(fetch).toHaveBeenCalledWith(remote.location, {
      headers: {
        'user-agent': expect.stringContaining('platform-watch'),
        'accept-language': 'en',
      },
      signal: expect.any(AbortSignal),
    });
  });

  it('reads a file from the repository and parses it', async () => {
    const readings = await read([{ ...local, location: 'file.txt' }]);

    expect(readings.get('file')).toMatchObject({ ok: true, value: 'pinned' });
  });

  it('tries a download again before giving up', async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(new Response('', { status: 503 }))
      .mockResolvedValueOnce(new Response('<html>'));
    const wait = vi.fn(async () => {});

    const readings = await read([remote], fetch, wait);

    expect(readings.get('page').ok).toBe(true);
    expect(wait).toHaveBeenCalledWith(10_000);
  });

  it('says why after three failed downloads', async () => {
    const fetch = vi.fn(async () => new Response('', { status: 404 }));
    const wait = vi.fn(async () => {});

    const readings = await read([remote], fetch, wait);

    expect(readings.get('page')).toEqual({
      source: remote,
      ok: false,
      error: "couldn't download it: HTTP 404",
    });
    expect(fetch).toHaveBeenCalledTimes(3);
    expect(wait.mock.calls).toEqual([[10_000], [20_000]]);
  });

  it("includes the network error behind Node's fetch failure", async () => {
    const fetch = vi.fn(async () => {
      throw new TypeError('fetch failed', {
        cause: new Error('getaddrinfo ENOTFOUND example.com'),
      });
    });

    const readings = await read([remote], fetch);

    expect(readings.get('page').error).toBe(
      "couldn't download it: fetch failed: getaddrinfo ENOTFOUND example.com",
    );
  });

  it('says why a file could not be read', async () => {
    const readings = await read([local]);

    expect(readings.get('file').error).toMatch(/^couldn't read it: ENOENT/);
  });

  it('says why a source could not be parsed', async () => {
    const source = {
      ...local,
      location: 'file.txt',
      parse: () => {
        throw new Error('no pin');
      },
    };

    const readings = await read([source]);

    expect(readings.get('file')).toEqual({
      source,
      ok: false,
      error: "couldn't parse it: no pin",
    });
  });

  it("quotes the title of a page it couldn't parse", async () => {
    const source = {
      ...remote,
      parse: () => {
        throw new Error('no sentence');
      },
    };
    const fetch = vi.fn(
      async () =>
        new Response(
          '<html><head><title> Google Play の対象 API レベル要件を満たす </title></head></html>',
        ),
    );

    const readings = await read([source], fetch);

    expect(readings.get('page').error).toBe(
      'couldn\'t parse it: no sentence (the page\'s title: "Google Play の対象 API レベル要件を満たす")',
    );
  });

  it('reads every source, whatever happens to the others', async () => {
    const fetch = vi.fn(async () => new Response('', { status: 500 }));

    const readings = await read(
      [remote, { ...local, location: 'file.txt' }],
      fetch,
    );

    expect([...readings.keys()]).toEqual(['page', 'file']);
    expect(readings.get('file').ok).toBe(true);
  });
});
