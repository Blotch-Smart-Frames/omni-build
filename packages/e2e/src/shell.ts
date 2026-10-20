/**
 * The values the fixture shells are built with, and so what they must
 * report on launch. They're set in:
 *
 * - iOS: `fixtures/ios/OmniBuildShell.xcodeproj` (bundle ID, version, build,
 *   and the app extensions', the App Clip's and the watch app's bundle IDs) and `OmniBuildShell/Info.plist`
 *   (name, camera usage), which `en.lproj/InfoPlist.strings` translates
 *   into English
 * - Android: `fixtures/android/app/build.gradle.kts` (application ID,
 *   version, build), `res/values/strings.xml` (name, as the application's
 *   label and the launcher activity's), and the manifest (provider,
 *   permissions); the ABIs are `src/main/jniLibs`' folders
 * - both: `payload.txt`, and the icons, solid squares of this colour (on
 *   Android, a WebP per density and an adaptive icon, whose layers are
 *   vectors).
 *
 * The apps the tests make from them have different values for each of
 * these (`app/app.json`), so an app that reports one got it from OmniBuild.
 * The iOS shell's framework is the same in the app: it shows the framework
 * loads once OmniBuild has signed it. See expected() in repackage.ts.
 */
export const SHELL = {
  id: 'dev.omnibuild.shell',
  name: 'OmniBuild Shell',
  version: '1.0.0',
  build: '1',
  payload: 'Payload from the OmniBuild shell\n',
  icon: '#2F6FEB',
  // Android's.
  provider: 'dev.omnibuild.shell.provider',
  permissions:
    'android.permission.CAMERA, android.permission.INTERNET, dev.omnibuild.shell.permission.SHELL',
  abis: 'arm64-v8a, armeabi-v7a, x86, x86_64',
  // iOS's.
  cameraUsage: "The shell's camera.",
  framework: 'ShellKit',
  extensionId: 'dev.omnibuild.shell.ShellNotifications',
  intentsExtensionId: 'dev.omnibuild.shell.ShellIntents',
  appClipId: 'dev.omnibuild.shell.Clip',
  watchAppId: 'dev.omnibuild.shell.watchkitapp',
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
