import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { describe, expect, it, vi } from 'vitest';

import { marker, reasonMarker } from './findings.js';
import { HELP, run } from './run.js';

// A repository whose shells are on Xcode 26.6 and API level 36.
const root = mkdtempSync(path.join(tmpdir(), 'platform-watch-'));
for (const [file, content] of Object.entries({
  '.github/workflows/ci.yml':
    'DEVELOPER_DIR: /Applications/Xcode_26.6.app/Contents/Developer\n',
  'packages/e2e/fixtures/android/app/build.gradle.kts':
    'compileSdk = 36\ntargetSdk = 36\n',
})) {
  mkdirSync(path.join(root, path.dirname(file)), { recursive: true });
  writeFileSync(path.join(root, file), content);
}

// Xcode 27 and API level 37 out; the stores require 26 and 36.
const PAGES = {
  'https://developer.apple.com/news/upcoming-requirements/':
    '<p>Since April 28, 2026</p><p>Apps must be built with Xcode 26 or later.</p>',
  'https://developer.apple.com/news/releases/rss/releases.rss':
    '<item><title>Xcode 27.1 (27A9275)</title></item>',
  'https://developer.android.com/google/play/requirements/target-sdk':
    '<p>Starting August 31 2026:</p><p>New apps and app updates must target Android 16 (API level 36) or higher.</p>',
  'https://dl.google.com/android/repository/repository2-3.xml':
    '<remotePackage path="platforms;android-37"><api-level>37</api-level></remotePackage>',
};

const json = (body, status = 200) =>
  new Response(JSON.stringify(body), { status });

/**
 * Serves PAGES, overridden by `pages`, and GitHub's API with `issues`, or a
 * `gitHubStatus` error.
 */
const fakeFetch = ({ pages = {}, issues = [], gitHubStatus } = {}) =>
  vi.fn(async (url, { method = 'GET' } = {}) => {
    if (!url.startsWith('https://api.github.com/')) {
      const page = { ...PAGES, ...pages }[url];
      return typeof page === 'number'
        ? new Response('', { status: page })
        : new Response(page);
    }
    if (gitHubStatus) {
      return json({ message: 'Bad credentials' }, gitHubStatus);
    }
    if (method === 'GET') {
      return json(url.includes('/comments') ? [] : issues);
    }
    return json({});
  });

const gitHubWrites = (fetch) =>
  fetch.mock.calls
    .filter(
      ([url, init]) =>
        url.startsWith('https://api.github.com/') && init.method !== 'GET',
    )
    .map(([url, init]) => `${init.method} ${url}`);

const env = { GITHUB_REPOSITORY: 'o/r', GITHUB_TOKEN: 't' };

const runWith = async (args, { fetch = fakeFetch(), ...options } = {}) => {
  const log = vi.fn();
  const result = await run(args, {
    root,
    env,
    fetch,
    wait: async () => {},
    log,
    ...options,
  });
  return { ...result, fetch, lines: log.mock.calls.map(([line]) => line) };
};

describe('run', () => {
  it('opens an issue for each update the shells need', async () => {
    const { status, fetch, lines } = await runWith([]);

    expect(status).toBe(0);
    expect(gitHubWrites(fetch)).toEqual([
      'POST https://api.github.com/repos/o/r/issues',
      'POST https://api.github.com/repos/o/r/issues',
    ]);
    expect(lines).toEqual([
      'ios-shell: {"major":26,"version":"26.6"}',
      'android-shell: {"compileSdk":36,"targetSdk":36}',
      'apple-requirements: {"major":26,"when":"Since April 28, 2026"}',
      'apple-releases: {"major":27,"version":"27.1"}',
      'play-target-api: {"api":36,"when":"Starting August 31 2026"}',
      'android-sdk: {"api":37}',
      'Changing on GitHub:',
      '- open an issue: Build the iOS shells with Xcode 27',
      '- open an issue: Compile and target the Android shells against API level 37',
    ]);
  });

  it('opens an issue for a page it can no longer parse', async () => {
    const fetch = fakeFetch({
      pages: {
        'https://developer.apple.com/news/upcoming-requirements/':
          '<p>Requirements have moved.</p>',
      },
    });

    const { status, lines } = await runWith([], { fetch });

    expect(status).toBe(0);
    expect(lines).toContain(
      'apple-requirements: couldn\'t parse it: no sentence saying "must be built with Xcode N"',
    );
    expect(lines).toContain(
      "- open an issue: Platform watch can't read Apple's SDK minimum requirements",
    );
  });

  it('opens an issue for a page it can no longer download', async () => {
    const fetch = fakeFetch({
      pages: {
        'https://dl.google.com/android/repository/repository2-3.xml': 404,
      },
    });

    const { lines } = await runWith([], { fetch });

    expect(lines).toContain("android-sdk: couldn't download it: HTTP 404");
    expect(lines).toContain(
      "- open an issue: Platform watch can't read the Android SDK repository",
    );
  });

  it('changes nothing when the issues are already there', async () => {
    const fetch = fakeFetch({
      issues: ['ios-xcode-27', 'android-api-37'].map((key, i) => ({
        number: i + 1,
        title: key,
        body: `${marker(key)} ${reasonMarker('released')}`,
        state: 'open',
      })),
    });

    const { lines } = await runWith([], { fetch });

    expect(gitHubWrites(fetch)).toEqual([]);
    expect(lines.at(-1)).toBe('Nothing to change on GitHub.');
  });

  it('only prints the changes in a dry run, with no token', async () => {
    const { status, fetch, lines } = await runWith(['--dry-run'], {
      env: { GITHUB_REPOSITORY: 'o/r' },
    });

    expect(status).toBe(0);
    expect(gitHubWrites(fetch)).toEqual([]);
    expect(lines.slice(-3)).toEqual([
      'Would change on GitHub:',
      '- open an issue: Build the iOS shells with Xcode 27',
      '- open an issue: Compile and target the Android shells against API level 37',
    ]);
  });

  it('uses the repository --repo names', async () => {
    const { fetch } = await runWith(['--repo', 'other/repo']);

    expect(gitHubWrites(fetch)[0]).toBe(
      'POST https://api.github.com/repos/other/repo/issues',
    );
  });

  it('fails when GitHub refuses', async () => {
    const { status, message } = await runWith([], {
      fetch: fakeFetch({ gitHubStatus: 401 }),
    });

    expect(status).toBe(1);
    expect(message).toBe(
      'platform-watch: GitHub: GET /repos/o/r/issues?labels=platform-watch&state=all&sort=created&direction=desc&per_page=100&page=1: HTTP 401: {"message":"Bad credentials"}',
    );
  });

  it('needs a repository', async () => {
    const { status, message, fetch } = await runWith([], { env: {} });

    expect(status).toBe(2);
    expect(message).toBe(
      'platform-watch: no repository: pass --repo <owner/name> or set GITHUB_REPOSITORY',
    );
    expect(fetch).not.toHaveBeenCalled();
  });

  it('needs a token to change issues', async () => {
    const { status, message } = await runWith([], {
      env: { GITHUB_REPOSITORY: 'o/r' },
    });

    expect(status).toBe(2);
    expect(message).toBe(
      'platform-watch: changing issues needs GITHUB_TOKEN. --dry-run only prints what would change',
    );
  });

  it('rejects an unknown option', async () => {
    const { status, message } = await runWith(['--bogus']);

    expect(status).toBe(2);
    expect(message).toMatch(/^platform-watch: Unknown option '--bogus'/);
  });

  it('shows its help', async () => {
    const { status, lines, fetch } = await runWith(['--help']);

    expect(status).toBe(0);
    expect(lines).toEqual([HELP]);
    expect(fetch).not.toHaveBeenCalled();
  });
});
