import { describe, expect, it } from 'vitest';

import { type Finding, findings, issueBody, resolution } from './findings.ts';
import type { Reading } from './read.ts';
import { SOURCES, type SourceReadings } from './sources.ts';

class Broken {
  error: string;

  constructor(error: string) {
    this.error = error;
  }
}

// Where things stood in October 2026: Xcode 27 and API level 37 out, the
// stores requiring Xcode 26 and API level 36.
const OCTOBER_2026 = {
  'ios-shell': { major: 26, version: '26.6' },
  'android-shell': { compileSdk: 36, targetSdk: 36 },
  'apple-requirements': { major: 26, when: 'Since April 28, 2026' },
  'apple-releases': { major: 27, version: '27.1' },
  'play-target-api': { api: 36, when: 'Starting August 31 2026' },
  'android-sdk': { api: 37 },
};

const UP_TO_DATE = {
  ...OCTOBER_2026,
  'ios-shell': { major: 27, version: '27.1' },
  'android-shell': { compileSdk: 37, targetSdk: 37 },
};

/** Readings of every source, with `values` by ID; a Broken one failed. */
const readingsOf = (values: Record<string, unknown>) =>
  new Map<string, Reading>(
    SOURCES.map((source) => {
      const value = values[source.id];
      return [
        source.id,
        value instanceof Broken
          ? { source, ok: false, error: value.error }
          : { source, ok: true, value },
      ];
    }),
  ) as SourceReadings;

const keysOf = (found: Finding[]) => found.map((finding) => finding.key);
const reasonsOf = (finding: Finding) =>
  finding.reasons.map((reason) => reason.id);

describe('findings', () => {
  it('finds nothing when the shells are up to date', () => {
    expect(findings(readingsOf(UP_TO_DATE))).toEqual([]);
  });

  it('finds a new Xcode and a new Android API level when they ship', () => {
    const [ios, android] = findings(readingsOf(OCTOBER_2026));

    expect(ios.title).toBe('Build the iOS shells with Xcode 27');
    expect(issueBody(ios)).toMatchInlineSnapshot(`
      "<!-- platform-watch:ios-xcode-27 -->

      The iOS shells are built with Xcode 26.6, set by \`DEVELOPER_DIR\` in \`.github/workflows/ci.yml\`, and:

      - **Released:** Xcode 27.1 is out ([Apple's developer releases feed](https://developer.apple.com/news/releases/rss/releases.rss)). Apple usually requires a new Xcode for App Store uploads the April after it ships. <!-- platform-watch-reason:released -->

      OmniBuild keeps the \`DT*\` keys Xcode writes into each shell's \`Info.plist\` (\`DTXcode\`, \`DTSDKName\`, ...), because App Store validation reads them. Meeting a new minimum means rebuilding the shells with the new Xcode; rewriting the keys would misreport the SDK they were built with.

      ## What to do

      1. Point \`DEVELOPER_DIR\` in \`.github/workflows/ci.yml\` at Xcode 27, once GitHub's macOS runner image has it ([actions/runner-images](https://github.com/actions/runner-images)).
      2. Fix what the new SDK breaks in the Swift shells, and update other mentions of Xcode 26.6 in the repository.
      3. CI builds the shells with it and runs them on the iOS Simulator: this is done when CI passes.

      ---

      Opened by the daily platform watch (\`.github/workflows/platform-watch.yml\`). It closes this issue once the iOS shells build with Xcode 27 or later."
    `);
    expect(android.title).toBe(
      'Compile and target the Android shells against API level 37',
    );
    expect(issueBody(android)).toMatchInlineSnapshot(`
      "<!-- platform-watch:android-api-37 -->

      The Android shells compile against API level 36 and target 36, set in \`packages/shells/webview/android/app/build.gradle.kts\`, and:

      - **Released:** API level 37 is stable in [the Android SDK repository](https://dl.google.com/android/repository/repository2-3.xml). Google Play requires apps to target a new level about a year after it ships, each August 31. <!-- platform-watch-reason:released -->

      Raising \`targetSdk\` opts an app into that Android release's behaviour changes, so OmniBuild doesn't rewrite it in the manifest: the shells are rebuilt and tested at the new level.

      ## What to do

      1. Raise \`compileSdk\` and \`targetSdk\` in \`packages/shells/webview/android/app/build.gradle.kts\` to 37, and in every other Android shell's \`app/build.gradle.kts\` (the E2E fixture's, \`packages/e2e/fixtures/android/app/build.gradle.kts\`): \`packages/e2e\`'s tests fail until they all match. The Android Gradle plugin may need a newer version to support it.
      2. Go through the behaviour changes for apps targeting the new level ([Android releases](https://developer.android.com/about/versions)), fix what they break in the Kotlin shells, and update other mentions of API level 36 in the repository.
      3. CI builds the shells against it and runs them on the Android emulator: this is done when CI passes.

      ---

      Opened by the daily platform watch (\`.github/workflows/platform-watch.yml\`). It closes this issue once the Android shells compile against and target API level 37 or later."
    `);
  });

  it("adds Apple's deadline to the Xcode that shipped", () => {
    const [ios] = findings(
      readingsOf({
        ...OCTOBER_2026,
        'apple-requirements': { major: 27, when: 'Begins April 27, 2027' },
      }),
    );

    expect(ios.key).toBe('ios-xcode-27');
    expect(reasonsOf(ios)).toEqual(['released', 'required']);
    expect(ios.reasons[1].text).toBe(
      "**Required:** Apple requires Xcode 27 for App Store uploads starting April 27, 2027 ([Apple's SDK minimum requirements](https://developer.apple.com/news/upcoming-requirements/)), and rejects uploads built with an older one.",
    );
  });

  it("adds Google Play's deadline to the API level that shipped", () => {
    const [android] = findings(
      readingsOf({
        ...UP_TO_DATE,
        'android-shell': { compileSdk: 36, targetSdk: 36 },
        'play-target-api': { api: 37, when: 'Starting August 31 2027' },
      }),
    );

    expect(android.key).toBe('android-api-37');
    expect(reasonsOf(android)).toEqual(['released', 'required']);
    expect(android.reasons[1].text).toBe(
      "**Required:** Google Play requires new apps and app updates to target API level 37 starting August 31 2027 ([Google Play's target API level requirements](https://developer.android.com/google/play/requirements/target-sdk)), and rejects uploads that target less.",
    );
  });

  it('dates a requirement already in force, or leaves the date out', () => {
    const shells = {
      ...UP_TO_DATE,
      'ios-shell': { major: 25, version: '25.4' },
      'android-shell': { compileSdk: 35, targetSdk: 35 },
    };

    const [ios, android] = findings(
      readingsOf({
        ...shells,
        'apple-releases': null,
        'android-sdk': { api: 35 },
        'play-target-api': { api: 36, when: null },
      }),
    );

    expect(ios.reasons[0].text).toContain(
      'for App Store uploads since April 28, 2026 (',
    );
    expect(android.reasons[0].text).toContain('target API level 36 (');
  });

  it('opens one issue per version when the shells are further behind', () => {
    const found = findings(
      readingsOf({
        ...OCTOBER_2026,
        'apple-requirements': { major: 27, when: 'Since April 27, 2027' },
        'apple-releases': { major: 28, version: '28.0' },
      }),
    );

    expect(keysOf(found)).toEqual([
      'ios-xcode-28',
      'ios-xcode-27',
      'android-api-37',
    ]);
  });

  it('counts the lower of compileSdk and targetSdk', () => {
    const found = findings(
      readingsOf({
        ...UP_TO_DATE,
        'android-shell': { compileSdk: 37, targetSdk: 36 },
      }),
    );

    expect(keysOf(found)).toEqual(['android-api-37']);
  });

  it('finds no new Xcode in a feed without an Xcode release', () => {
    expect(
      findings(readingsOf({ ...OCTOBER_2026, 'apple-releases': null })),
    ).toHaveLength(1);
  });

  it('opens an issue for a source the watch could not read', () => {
    const [watch] = findings(
      readingsOf({
        ...UP_TO_DATE,
        'apple-requirements': new Broken(
          'couldn\'t parse it: no sentence saying "must be built with Xcode N"',
        ),
      }),
    );

    expect(watch.title).toBe(
      "Platform watch can't read Apple's SDK minimum requirements",
    );
    expect(issueBody(watch)).toMatchInlineSnapshot(`
      "<!-- platform-watch:watch-apple-requirements -->

      The platform watch couldn't read [Apple's SDK minimum requirements](https://developer.apple.com/news/upcoming-requirements/):

      - couldn't parse it: no sentence saying "must be built with Xcode N" <!-- platform-watch-reason:unreadable -->

      It looks for the Xcode App Store uploads must be built with, in a sentence like "Apps uploaded to App Store Connect must be built with Xcode 26 or later". Until it reads it again, the watch can't tell whether the shells need updating for it.

      ## What to do

      1. Find what changed: a page that moved or was reworded, a feed or file that changed shape, or a bug in the parser.
      2. Update the \`apple-requirements\` source in \`packages/platform-watch/src/sources.ts\` and its tests, which hold excerpts of the real page or file.
      3. Run the watch from the Actions tab (Platform watch, Run workflow) to check it reads it.

      ---

      Opened by the daily platform watch (\`.github/workflows/platform-watch.yml\`). It closes this issue once it reads the source again."
    `);
  });

  it('names the file for a source in the repository', () => {
    const [watch] = findings(
      readingsOf({
        ...UP_TO_DATE,
        'ios-shell': new Broken("couldn't parse it: no pin"),
      }),
    );

    expect(watch.context).toBe(
      "The platform watch couldn't read the iOS shells' Xcode version (`.github/workflows/ci.yml`):",
    );
  });

  it('skips the checks whose shell it could not read', () => {
    const found = findings(
      readingsOf({
        ...OCTOBER_2026,
        'ios-shell': new Broken('a'),
        'play-target-api': new Broken('b'),
        'android-sdk': new Broken('c'),
      }),
    );

    expect(keysOf(found)).toEqual([
      'watch-ios-shell',
      'watch-play-target-api',
      'watch-android-sdk',
    ]);
  });

  it('skips the checks whose vendor sources it could not read', () => {
    const found = findings(
      readingsOf({
        ...OCTOBER_2026,
        'android-shell': new Broken('a'),
        'apple-releases': new Broken('b'),
        'apple-requirements': new Broken('c'),
      }),
    );

    expect(keysOf(found)).toEqual([
      'watch-android-shell',
      'watch-apple-requirements',
      'watch-apple-releases',
    ]);
  });
});

describe('resolution', () => {
  it('closes an Xcode issue once the shells build with it', () => {
    expect(resolution('ios-xcode-27', readingsOf(UP_TO_DATE))).toBe(
      'The iOS shells now build with Xcode 27.1, so this is done.',
    );
  });

  it('keeps an Xcode issue open until then', () => {
    expect(resolution('ios-xcode-27', readingsOf(OCTOBER_2026))).toBeNull();
    expect(
      resolution(
        'ios-xcode-27',
        readingsOf({ ...UP_TO_DATE, 'ios-shell': new Broken('a') }),
      ),
    ).toBeNull();
  });

  it('closes an API level issue once the shells compile and target it', () => {
    expect(resolution('android-api-37', readingsOf(UP_TO_DATE))).toBe(
      'The Android shells now compile against API level 37 and target 37, so this is done.',
    );
  });

  it('keeps an API level issue open until then', () => {
    expect(
      resolution(
        'android-api-37',
        readingsOf({
          ...UP_TO_DATE,
          'android-shell': { compileSdk: 37, targetSdk: 36 },
        }),
      ),
    ).toBeNull();
    expect(
      resolution(
        'android-api-37',
        readingsOf({ ...UP_TO_DATE, 'android-shell': new Broken('a') }),
      ),
    ).toBeNull();
  });

  it('closes a watch issue once the source reads again', () => {
    expect(resolution('watch-android-sdk', readingsOf(UP_TO_DATE))).toBe(
      'The platform watch reads [the Android SDK repository](https://dl.google.com/android/repository/repository2-3.xml) again, so this is done.',
    );
  });

  it('keeps a watch issue open while the source is unreadable', () => {
    expect(
      resolution(
        'watch-android-sdk',
        readingsOf({ ...UP_TO_DATE, 'android-sdk': new Broken('a') }),
      ),
    ).toBeNull();
  });

  it('leaves alone an issue it knows nothing about', () => {
    expect(resolution('watch-removed-source', readingsOf(UP_TO_DATE))).toBe(
      null,
    );
    expect(resolution('something-else', readingsOf(UP_TO_DATE))).toBeNull();
  });
});
