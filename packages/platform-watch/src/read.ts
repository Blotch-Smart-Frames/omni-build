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

/** fetch(), narrowed to the calls the watch makes so tests can fake it. */
export type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

/** Waits `ms` milliseconds. */
export type Wait = (ms: number) => Promise<unknown>;

/** Something the watch reads, and how to read the number it wants from it. */
export interface Source<T = unknown> {
  id: string;
  /** What the issues call it. */
  title: string;
  /**
   * A URL starting with `https://` to download, or a path from the
   * repository root.
   */
  location: string;
  /** What the parser looks for, for an issue saying it can't find it. */
  looksFor: string;
  /** Reads the value from the page or file, or throws. */
  parse: (content: string) => T;
}

export type Reading<T = unknown> =
  | { source: Source; ok: true; value: T }
  | { source: Source; ok: false; error: string };

export interface ReadOptions {
  /** The repository root, for sources in it. */
  root: string;
  fetch: Fetch;
  wait: Wait;
}

const download = async (
  url: string,
  { fetch, wait }: Pick<ReadOptions, 'fetch' | 'wait'>,
) => {
  for (let attempt = 1; ; attempt++) {
    try {
      const response = await fetch(url, {
        headers: {
          'user-agent': USER_AGENT,
          // The parsers read English sentences. Node's fetch otherwise sends
          // `Accept-Language: *`, which Google's developer site answers with
          // a translation picked at random (German, Persian, Japanese, ...).
          'accept-language': 'en',
        },
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
const describe = (error: Error) =>
  error.cause instanceof Error
    ? `${error.message}: ${error.cause.message}`
    : error.message;

// A page's title tells a reworded page from a different one (a translation,
// an error page, a sign-in wall) at a glance, so an issue about a source that
// couldn't be parsed quotes it.
const titled = (content: string) => {
  const title = content.match(/<title>([^<]*)<\/title>/)?.[1].trim();
  return title ? ` (the page's title: "${title}")` : '';
};

/**
 * Reads and parses every source. One that can't be read or parsed doesn't
 * stop the others: its reading says why, and the watch opens an issue for it.
 *
 * Returns the readings by source ID.
 */
export const readSources = async (
  sources: readonly Source[],
  { root, fetch, wait }: ReadOptions,
): Promise<Map<string, Reading>> => {
  const readings = await Promise.all(
    sources.map(async (source): Promise<Reading> => {
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
          error: `couldn't ${remote ? 'download' : 'read'} it: ${describe(error as Error)}`,
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
          error: `couldn't parse it: ${(error as Error).message}${titled(content)}`,
        };
      }
    }),
  );
  return new Map(readings.map((reading) => [reading.source.id, reading]));
};
