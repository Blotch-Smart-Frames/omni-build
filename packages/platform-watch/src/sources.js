/**
 * What the platform watch reads: the toolchain the app shells are built
 * with, from this repository, and what Apple and Google require and have
 * released, from their own pages.
 *
 * None of the vendors' pages is an API, so each parser looks for the
 * sentence or element that carries the number, and throws when
 * it isn't there rather than guessing. A page that changes its wording then
 * shows up as an issue saying the watch can't read it, instead of the watch
 * going quiet.
 */

// Pages are HTML, but each requirement reads as one sentence once the tags
// are gone. Apple writes "App&nbsp;Store&nbsp;Connect", and wraps "Xcode 26"
// in a link, so tags become spaces and runs of whitespace collapse.
const text = (html) =>
  html
    .replaceAll(/<[^>]*>/g, ' ')
    .replaceAll(/&nbsp;|&#160;/g, ' ')
    .replaceAll(/\s+/g, ' ');

const DATE =
  /\b(?:Since|Begins|Beginning|Starting) [A-Z][a-z]+ \d{1,2},? \d{4}/g;

/**
 * The date phrase closest before `index` ("Since April 28, 2026", "Begins
 * ..."), which both vendors put just above the requirement it dates. `null`
 * when there's none: the number matters more than the date.
 */
const dateBefore = (page, index) =>
  [...page.matchAll(DATE)].findLast((match) => match.index < index)?.[0] ??
  null;

/**
 * The newest "must be built with Xcode N" on Apple's upcoming requirements
 * page. The page keeps past requirements too (Xcode 14.1, 15, ...), so the
 * highest N is the one in force or announced.
 *
 * @returns {{ major: number, when: string | null }}
 */
export const parseAppleRequirements = (html) => {
  const page = text(html);
  const newest = [...page.matchAll(/built (?:with|using) Xcode (\d+)/g)]
    .map((match) => ({ major: Number(match[1]), index: match.index }))
    .toSorted((a, b) => b.major - a.major)[0];
  if (!newest) {
    throw new Error('no sentence saying "must be built with Xcode N"');
  }
  return { major: newest.major, when: dateBefore(page, newest.index) };
};

/**
 * The newest Xcode release in Apple's developer releases feed, ignoring
 * betas and release candidates. The feed holds only the last few weeks, so
 * a feed without an Xcode release in it is normal and gives `null`.
 *
 * @returns {{ major: number, version: string } | null}
 */
export const parseAppleReleases = (rss) => {
  const titles = [
    ...rss.matchAll(
      /<item>[\s\S]*?<title>(?:<!\[CDATA\[)?(.*?)(?:\]\]>)?<\/title>/g,
    ),
  ].map((match) => match[1].trim());
  // Every software release is titled "Name version (build)". Without one,
  // the feed has changed shape, and "no Xcode release" would be a guess.
  if (!titles.some((title) => / \([0-9A-Za-z]+\)$/.test(title))) {
    throw new Error('no items titled like "Xcode 27.1 (27A9275)"');
  }
  return (
    titles
      .map((title) => title.match(/^Xcode (\d+(?:\.\d+)*) \([0-9A-Za-z]+\)$/))
      .filter(Boolean)
      .map(([, version]) => ({ major: Number.parseInt(version), version }))
      .toSorted((a, b) => b.major - a.major)[0] ?? null
  );
};

/**
 * The API level Google Play requires new apps and app updates to target, on
 * its target API level page. Wear OS, TV and the other device types have
 * lower levels in the same paragraph, which this skips: the shells are phone
 * and tablet apps.
 *
 * @returns {{ api: number, when: string | null }}
 */
export const parsePlayTargetApi = (html) => {
  const page = text(html);
  const newest = [
    ...page.matchAll(
      /New apps and app updates must target (?:Android [\d.]+ \(API level (\d+)\)|API level (\d+))/g,
    ),
  ]
    .map((match) => ({ api: Number(match[1] ?? match[2]), index: match.index }))
    .toSorted((a, b) => b.api - a.api)[0];
  if (!newest) {
    throw new Error(
      'no sentence saying "New apps and app updates must target Android N (API level N)"',
    );
  }
  return { api: newest.api, when: dateBefore(page, newest.index) };
};

/**
 * The newest stable Android API level in the repository the SDK manager
 * installs from. Every channel lists its platforms there; previews carry a
 * codename (`CANARY`, `CinnamonBun`) or a suffix in their path
 * (`android-37.2-beta3`), and extension SDKs an `-ext` one. Minor releases
 * (`android-36.1`) count as their major level.
 *
 * @returns {{ api: number }}
 */
export const parseAndroidSdk = (xml) => {
  const stable = [
    ...xml.matchAll(
      /<remotePackage path="platforms;android-(\d+)(?:\.\d+)?">([\s\S]*?)<\/remotePackage>/g,
    ),
  ]
    .filter(([, , details]) => !/<codename>[^<]+<\/codename>/.test(details))
    .map(([, api]) => Number(api));
  if (stable.length === 0) {
    throw new Error('no stable "platforms;android-N" package');
  }
  return { api: Math.max(...stable) };
};

/**
 * The Xcode the iOS shells are built with: the one `DEVELOPER_DIR` selects
 * from those on GitHub's macOS runner.
 *
 * @returns {{ major: number, version: string }}
 */
export const parseXcodePin = (workflow) => {
  const versions = new Set(
    [
      ...workflow.matchAll(
        /DEVELOPER_DIR: \/Applications\/Xcode_(\d+(?:\.\d+)*)\.app\b/g,
      ),
    ].map(([, version]) => version),
  );
  if (versions.size !== 1) {
    throw new Error(
      `expected one "DEVELOPER_DIR: /Applications/Xcode_N.app/..." line, found ${versions.size} versions`,
    );
  }
  const [version] = versions;
  return { major: Number.parseInt(version), version };
};

/**
 * The API levels the Android shells compile and run against. The lower one
 * is what still needs raising.
 *
 * @returns {{ compileSdk: number, targetSdk: number }}
 */
export const parseGradleSdk = (gradle) => {
  const level = (name) => {
    const match = gradle.match(new RegExp(`\\b${name} = (\\d+)\\b`));
    if (!match) {
      throw new Error(`no "${name} = N" line`);
    }
    return Number(match[1]);
  };
  return { compileSdk: level('compileSdk'), targetSdk: level('targetSdk') };
};

/**
 * Everything the watch reads. A `location` starting with `https://` is
 * fetched; anything else is a path from the repository root.
 *
 * The shells' toolchain is read from the E2E fixtures, the only shells there
 * are yet. When the release shells arrive (Phase 2 of #1), their files take
 * these two places.
 */
export const SOURCES = [
  {
    id: 'ios-shell',
    title: "the iOS shells' Xcode version",
    location: '.github/workflows/ci.yml',
    looksFor:
      'the Xcode the iOS shells are built with, in a `DEVELOPER_DIR: /Applications/Xcode_N.app/Contents/Developer` line',
    parse: parseXcodePin,
  },
  {
    id: 'android-shell',
    title: "the Android shells' API levels",
    location: 'packages/e2e/fixtures/android/app/build.gradle.kts',
    looksFor: 'the `compileSdk = N` and `targetSdk = N` lines',
    parse: parseGradleSdk,
  },
  {
    id: 'apple-requirements',
    title: "Apple's SDK minimum requirements",
    location: 'https://developer.apple.com/news/upcoming-requirements/',
    looksFor:
      'the Xcode App Store uploads must be built with, in a sentence like "Apps uploaded to App Store Connect must be built with Xcode 26 or later"',
    parse: parseAppleRequirements,
  },
  {
    id: 'apple-releases',
    title: "Apple's developer releases feed",
    location: 'https://developer.apple.com/news/releases/rss/releases.rss',
    looksFor: 'Xcode releases, in RSS items titled like "Xcode 27.1 (27A9275)"',
    parse: parseAppleReleases,
  },
  {
    id: 'play-target-api',
    title: "Google Play's target API level requirements",
    location:
      'https://developer.android.com/google/play/requirements/target-sdk',
    looksFor:
      'the API level Play requires, in a sentence like "New apps and app updates must target Android 16 (API level 36) or higher"',
    parse: parsePlayTargetApi,
  },
  {
    id: 'android-sdk',
    title: 'the Android SDK repository',
    location: 'https://dl.google.com/android/repository/repository2-3.xml',
    looksFor:
      'stable platforms, in `<remotePackage path="platforms;android-N">` elements without a codename',
    parse: parseAndroidSdk,
  },
];
