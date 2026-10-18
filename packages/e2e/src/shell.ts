/**
 * The values the fixture shells are built with, and so what they must
 * report on launch. They're set in:
 *
 * - iOS: `fixtures/ios/OmniBuildShell.xcodeproj` (bundle ID, version, build)
 *   and `OmniBuildShell/Info.plist` (name)
 * - Android: `fixtures/android/app/build.gradle.kts` (application ID,
 *   version, build) and `res/values/strings.xml` (name)
 * - both: `payload.txt`, and the icons, solid squares of this colour.
 *
 * Once OmniBuild repackages the shells, the tests will also check an app
 * with different values for each of these, which only OmniBuild could have
 * put there.
 */
export const SHELL = {
  id: 'dev.omnibuild.shell',
  name: 'OmniBuild Shell',
  version: '1.0.0',
  build: '1',
  payload: 'Payload from the OmniBuild shell\n',
  icon: '#2F6FEB',
};

/**
 * Describes each value in a report (what the app reported on launch) that
 * differs from the expected one. Empty when the report matches.
 */
export const differences = (
  expected: Record<string, unknown>,
  report: Record<string, unknown>,
): string[] =>
  Object.entries(expected)
    .filter(([key, value]) => report[key] !== value)
    .map(
      ([key, value]) =>
        `${key}: expected ${JSON.stringify(value)}, got ${JSON.stringify(report[key] ?? null)}`,
    );
