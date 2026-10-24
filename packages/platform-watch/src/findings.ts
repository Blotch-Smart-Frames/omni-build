/**
 * What the readings mean: each piece of work the shells need, as an issue
 * written for whoever picks it up, a person or an agent. An issue is about
 * one thing (Xcode 27 for the iOS shells, API level 37 for the Android
 * ones, a source the watch can't read), and each reason for it (released,
 * now required by the store) is added to it as it appears, so the deadline
 * Apple announces in the spring lands on the issue opened when Xcode shipped.
 */

import type { Reading, Source } from './read.ts';
import type { ApiLevels, SourceReadings, Xcode } from './sources.ts';

const WORKFLOW = '.github/workflows/platform-watch.yml';
const SOURCES_FILE = 'packages/platform-watch/src/sources.ts';
/**
 * The E2E fixture's Gradle file, whose API levels move with the shell the
 * watch reads.
 */
const FIXTURE_GRADLE = 'packages/e2e/fixtures/android/app/build.gradle.kts';

/** One reason for an issue: a line of it, or of a comment on it. */
export interface Reason {
  /** Tells the reason apart from the issue's others, through reasonMarker(). */
  id: string;
  text: string;
}

/** An issue there should be, written for whoever picks it up. */
export interface Finding {
  /** Ties the issue to the finding, through marker(). */
  key: string;
  title: string;
  context: string;
  reasons: Reason[];
  why: string;
  todo: string[];
  /** Ends the sentence "It closes this issue once ...". */
  doneWhen: string;
}

/** Ties an issue to its finding, so a later run finds it again. */
export const marker = (key: string) => `<!-- platform-watch:${key} -->`;

/** Marks a reason as already on an issue, in its body or a comment. */
export const reasonMarker = (id: string) =>
  `<!-- platform-watch-reason:${id} -->`;

const link = (source: Source) =>
  source.location.startsWith('https://')
    ? `[${source.title}](${source.location})`
    : `${source.title} (\`${source.location}\`)`;

// "Since April 28, 2026" or "Begins April 27, 2027", as the page dates the
// requirement, read into the middle of a sentence.
const dated = (when: string | null) =>
  when
    ? ` ${when.replace(/^\w+/, (word) => (word === 'Since' ? 'since' : 'starting'))}`
    : '';

/** Each reason becomes a line of the issue, or of a comment on it. */
export const reasonLines = (reasons: Reason[]): string =>
  reasons.map(({ id, text }) => `- ${text} ${reasonMarker(id)}`).join('\n');

/** The whole body of a new issue. */
export const issueBody = (finding: Finding): string =>
  [
    marker(finding.key),
    finding.context,
    reasonLines(finding.reasons),
    finding.why,
    `## What to do\n\n${finding.todo.map((step, i) => `${i + 1}. ${step}`).join('\n')}`,
    `---\n\nOpened by the daily platform watch (\`${WORKFLOW}\`). It closes this issue once ${finding.doneWhen}.`,
  ].join('\n\n');

const ios = (
  major: number,
  shell: Xcode,
  reasons: Reason[],
  readings: SourceReadings,
): Finding => {
  const { location } = readings.get('ios-shell').source;
  return {
    key: `ios-xcode-${major}`,
    title: `Build the iOS shells with Xcode ${major}`,
    context: `The iOS shells are built with Xcode ${shell.version}, set by \`DEVELOPER_DIR\` in \`${location}\`, and:`,
    reasons,
    why: "OmniBuild keeps the `DT*` keys Xcode writes into each shell's `Info.plist` (`DTXcode`, `DTSDKName`, ...), because App Store validation reads them. Meeting a new minimum means rebuilding the shells with the new Xcode; rewriting the keys would misreport the SDK they were built with.",
    todo: [
      `Point \`DEVELOPER_DIR\` in \`${location}\` at Xcode ${major}, once GitHub's macOS runner image has it ([actions/runner-images](https://github.com/actions/runner-images)).`,
      `Fix what the new SDK breaks in the Swift shells, and update other mentions of Xcode ${shell.version} in the repository.`,
      'CI builds the shells with it and runs them on the iOS Simulator: this is done when CI passes.',
    ],
    doneWhen: `the iOS shells build with Xcode ${major} or later`,
  };
};

const android = (
  api: number,
  shell: ApiLevels,
  reasons: Reason[],
  readings: SourceReadings,
): Finding => {
  const { location } = readings.get('android-shell').source;
  return {
    key: `android-api-${api}`,
    title: `Compile and target the Android shells against API level ${api}`,
    context: `The Android shells compile against API level ${shell.compileSdk} and target ${shell.targetSdk}, set in \`${location}\`, and:`,
    reasons,
    why: "Raising `targetSdk` opts an app into that Android release's behaviour changes, so OmniBuild doesn't rewrite it in the manifest: the shells are rebuilt and tested at the new level.",
    todo: [
      `Raise \`compileSdk\` and \`targetSdk\` in \`${location}\` to ${api}, and in every other Android shell's \`app/build.gradle.kts\` (the E2E fixture's, \`${FIXTURE_GRADLE}\`): \`packages/e2e\`'s tests fail until they all match. The Android Gradle plugin may need a newer version to support it.`,
      `Go through the behaviour changes for apps targeting the new level ([Android releases](https://developer.android.com/about/versions)), fix what they break in the Kotlin shells, and update other mentions of API level ${shell.targetSdk} in the repository.`,
      'CI builds the shells against it and runs them on the Android emulator: this is done when CI passes.',
    ],
    doneWhen: `the Android shells compile against and target API level ${api} or later`,
  };
};

const unreadable = ({
  source,
  error,
}: Extract<Reading, { ok: false }>): Finding => ({
  key: `watch-${source.id}`,
  title: `Platform watch can't read ${source.title}`,
  context: `The platform watch couldn't read ${link(source)}:`,
  reasons: [{ id: 'unreadable', text: error }],
  why: `It looks for ${source.looksFor}. Until it reads it again, the watch can't tell whether the shells need updating for it.`,
  todo: [
    'Find what changed: a page that moved or was reworded, a feed or file that changed shape, or a bug in the parser.',
    `Update the \`${source.id}\` source in \`${SOURCES_FILE}\` and its tests, which hold excerpts of the real page or file.`,
    'Run the watch from the Actions tab (Platform watch, Run workflow) to check it reads it.',
  ],
  doneWhen: 'it reads the source again',
});

/** A reason, and the version it calls for. */
interface VersionedReason {
  version: number;
  reason: Reason;
}

/**
 * Groups reasons by the version they call for. Usually one: the Xcode Apple
 * now requires is the one that shipped in September. A team a year behind
 * can have two.
 */
const byVersion = (entries: VersionedReason[]) => {
  const grouped = new Map<number, Reason[]>();
  for (const { version, reason } of entries) {
    grouped.set(version, [...(grouped.get(version) ?? []), reason]);
  }
  return grouped;
};

/** One finding per issue there should be. */
export const findings = (readings: SourceReadings): Finding[] => {
  const found = [...readings.values()]
    .filter((reading) => !reading.ok)
    .map(unreadable);

  const iosShell = readings.get('ios-shell');
  if (iosShell.ok) {
    const shell = iosShell.value;
    const released = readings.get('apple-releases');
    const required = readings.get('apple-requirements');
    const entries: VersionedReason[] = [];
    if (released.ok && released.value && released.value.major > shell.major) {
      entries.push({
        version: released.value.major,
        reason: {
          id: 'released',
          text: `**Released:** Xcode ${released.value.version} is out (${link(released.source)}). Apple usually requires a new Xcode for App Store uploads the April after it ships.`,
        },
      });
    }
    if (required.ok && required.value.major > shell.major) {
      entries.push({
        version: required.value.major,
        reason: {
          id: 'required',
          text: `**Required:** Apple requires Xcode ${required.value.major} for App Store uploads${dated(required.value.when)} (${link(required.source)}), and rejects uploads built with an older one.`,
        },
      });
    }
    for (const [major, reasons] of byVersion(entries)) {
      found.push(ios(major, shell, reasons, readings));
    }
  }

  const androidShell = readings.get('android-shell');
  if (androidShell.ok) {
    const shell = androidShell.value;
    const lowest = Math.min(shell.compileSdk, shell.targetSdk);
    const released = readings.get('android-sdk');
    const required = readings.get('play-target-api');
    const entries: VersionedReason[] = [];
    if (released.ok && released.value.api > lowest) {
      entries.push({
        version: released.value.api,
        reason: {
          id: 'released',
          text: `**Released:** API level ${released.value.api} is stable in ${link(released.source)}. Google Play requires apps to target a new level about a year after it ships, each August 31.`,
        },
      });
    }
    if (required.ok && required.value.api > shell.targetSdk) {
      entries.push({
        version: required.value.api,
        reason: {
          id: 'required',
          text: `**Required:** Google Play requires new apps and app updates to target API level ${required.value.api}${dated(required.value.when)} (${link(required.source)}), and rejects uploads that target less.`,
        },
      });
    }
    for (const [api, reasons] of byVersion(entries)) {
      found.push(android(api, shell, reasons, readings));
    }
  }

  return found;
};

/**
 * Whether the work an open issue asks for is done, judged from this run's
 * readings. Only the shells' own files can say an update is done; the
 * vendors' pages can't take one back.
 *
 * Returns a comment to close the issue with, or `null` to leave it open.
 */
export const resolution = (
  key: string,
  readings: SourceReadings,
): string | null => {
  const iosMatch = key.match(/^ios-xcode-(\d+)$/);
  if (iosMatch) {
    const shell = readings.get('ios-shell');
    return shell.ok && shell.value.major >= Number(iosMatch[1])
      ? `The iOS shells now build with Xcode ${shell.value.version}, so this is done.`
      : null;
  }
  const androidMatch = key.match(/^android-api-(\d+)$/);
  if (androidMatch) {
    const shell = readings.get('android-shell');
    return shell.ok &&
      Math.min(shell.value.compileSdk, shell.value.targetSdk) >=
        Number(androidMatch[1])
      ? `The Android shells now compile against API level ${shell.value.compileSdk} and target ${shell.value.targetSdk}, so this is done.`
      : null;
  }
  const watchMatch = key.match(/^watch-(.+)$/);
  const reading = watchMatch && readings.get(watchMatch[1]);
  return reading?.ok
    ? `The platform watch reads ${link(reading.source)} again, so this is done.`
    : null;
};
