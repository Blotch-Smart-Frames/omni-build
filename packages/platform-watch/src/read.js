import { readFile } from 'node:fs/promises';
import path from 'node:path';

// Apple and Google serve these pages to anyone, but a named client is easier
// for them to tell apart from abuse, and to reach if it misbehaves.
const USER_AGENT =
  'omni-build-platform-watch (+https://github.com/Blotch-Smart-Frames/omni-build)';

// A failed read opens an issue, so a blip in the vendors' CDNs or the
// runner's network gets a few more tries first. Retrying a 404 is wasted
// but harmless, and keeps the rule simple.
const ATTEMPTS = 3;
const RETRY_DELAY_MS = 10_000;
const TIMEOUT_MS = 60_000;

const download = async (url, { fetch, wait }) => {
  for (let attempt = 1; ; attempt++) {
    try {
      const response = await fetch(url, {
        headers: { 'user-agent': USER_AGENT },
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      return await response.text();
    } catch (error) {
      if (attempt === ATTEMPTS) {
        throw error;
      }
      await wait(attempt * RETRY_DELAY_MS);
    }
  }
};

// Node's fetch throws a bare "fetch failed" and keeps what went wrong (DNS,
// TLS, a reset connection) in `cause`.
const describe = (error) =>
  error.cause ? `${error.message}: ${error.cause.message}` : error.message;

/**
 * @typedef {{ source: object, ok: true, value: unknown }
 *   | { source: object, ok: false, error: string }} Reading
 */

/**
 * Reads and parses every source. One that can't be read or parsed doesn't
 * stop the others: its reading says why, and the watch opens an issue for it.
 *
 * @param {object[]} sources
 * @param {object} options
 * @param {string} options.root The repository root, for sources in it.
 * @param {typeof fetch} options.fetch
 * @param {(ms: number) => Promise<void>} options.wait
 * @returns {Promise<Map<string, Reading>>} By source ID.
 */
export const readSources = async (sources, { root, fetch, wait }) => {
  const readings = await Promise.all(
    sources.map(async (source) => {
      const remote = source.location.startsWith('https://');
      let content;
      try {
        content = remote
          ? await download(source.location, { fetch, wait })
          : await readFile(path.join(root, source.location), 'utf8');
      } catch (error) {
        return {
          source,
          ok: false,
          error: `couldn't ${remote ? 'download' : 'read'} it: ${describe(error)}`,
        };
      }
      // A parser that throws for any reason, its own bug included, is a
      // source the watch can't read, and gets an issue like any other.
      try {
        return { source, ok: true, value: source.parse(content) };
      } catch (error) {
        return {
          source,
          ok: false,
          error: `couldn't parse it: ${error.message}`,
        };
      }
    }),
  );
  return new Map(readings.map((reading) => [reading.source.id, reading]));
};
