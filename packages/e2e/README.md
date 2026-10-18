# @omni-build/e2e

End-to-end tests on real platforms: real Swift and Kotlin app shells, repackaged by omni-build on Linux, Windows and macOS, then installed and launched on the iOS Simulator and an Android emulator. They run on every pull request.

Each fixture is a stand-in for the app shells OmniBuild repackages. As in a real shell, nothing that differs between apps is in the Swift or Kotlin code: the ID, name, version, icon and payload are data in the bundle. On launch, the app reads those values from its own bundle and reports them, and the harness checks the report.

## The three stages

They follow how OmniBuild is meant to be used ([#1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1)), and CI runs each on different machines:

| Stage       | Where it runs in CI                 | What it does                                                                                                              |
| ----------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `build`     | macOS (Xcode), Linux (Gradle)       | Compiles the fixture shells. The only stage with a native toolchain: OmniBuild's own CI makes the shells, never its users |
| `repackage` | Linux, Windows and macOS            | Runs each shell through the omni-build release binary, which gets no `PATH` and no toolchain variables (below)            |
| `test`      | macOS (Simulator), Linux (emulator) | Installs and launches every repackaged app, one per OS it was repackaged on, and checks what each reports                 |

**Every shell goes through `omni-build repackage`**, which tells an iOS app from an APK or an AAB by its contents. Gradle builds an APK and an app bundle (AAB), and omni-build gives each the ID, name, version, build number, icon and payload in `app/`, and signs it with the debug key it creates in the home folder. Each value differs from the shell's, so an app that reports it got it from omni-build. The `test` stage checks the results with Google's own tools first. For an APK, that's `zipalign` for the alignment Android needs to map stored entries, and `apksigner` for every signature scheme. An app bundle gets what Google Play does with an upload: `jarsigner` checks its signature and `bundletool validate` its contents, then `bundletool build-apks` makes the universal APK that's installed. bundletool is downloaded once, pinned by its SHA-256, and signs that APK with a key of its own, as Play signs with the app's. Xcode builds the iOS shell for the Simulator, and omni-build gives it the same values but the icon, which needs an `Assets.car` it can't write yet. It doesn't sign iOS apps yet either: the Simulator runs them with the signature Xcode gave the shell, which no longer matches their files, so the `test` stage has no signature to check. `expected()` in `src/repackage.ts` says what each platform's apps must report: the app's values where omni-build rewrites them, and the shell's elsewhere.

**No toolchain in reach.** GitHub's Linux and Windows runners come with a JDK and the Android SDK, and its macOS runners with Xcode. So `repackage` runs omni-build with only `HOME`, `USERPROFILE`, the temporary folder variables and `SystemRoot`: no `PATH` (it can't start a program by name) and no `JAVA_HOME`, `ANDROID_HOME` or `DEVELOPER_DIR`. A step that quietly relied on a vendor tool fails in CI, not on a user's machine without one.

## Run it

```sh
npm run e2e -- --platform ios                    # All three stages. Needs Xcode with an iOS Simulator runtime
npm run e2e -- --platform android --avd <name>   # Boots that emulator (`emulator -list-avds`) and shuts it down after
npm run e2e -- --platform android                # Uses the one device adb sees, or $ANDROID_SERIAL
npm run e2e -- build --platform ios              # One stage; then `repackage`, then `test`
npm run e2e -- --help                            # Every option
```

`npm run e2e` from the repository root builds the omni-build binary first. The harness runs `target/release/omni-build`, or the binary `OMNI_BUILD_BINARY` names.

Android needs a JDK (17 or later, for Gradle, apksigner, jarsigner and bundletool) and the Android SDK with API 37 and build tools 35 or later (for `zipalign -P`), found through `ANDROID_HOME` or Android Studio's default location. The iOS tests create and reuse their own simulator, "OmniBuild E2E", so your own simulators stay untouched.

## What it checks

| Value     | iOS: where it comes from                                      | Android: where it comes from                                                        |
| --------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `id`      | `CFBundleIdentifier`                                          | the package name in the binary manifest                                             |
| `name`    | `CFBundleDisplayName`                                         | the application label, in `resources.arsc`                                          |
| `version` | `CFBundleShortVersionString`                                  | `versionName`                                                                       |
| `build`   | `CFBundleVersion`                                             | `versionCode`                                                                       |
| `payload` | `payload.txt` in the `.app`                                   | `assets/payload.txt` in the APK                                                     |
| `icon`    | the centre pixel of the icon PNG named in `CFBundleIconFiles` | the centre pixel of the manifest's `android:icon`                                   |
| `signer`  | not yet checked: omni-build doesn't sign iOS apps yet         | the signing certificate's name, from `apksigner` or, for an app bundle, `jarsigner` |

The app also has to install and launch at all, which checks the package's structure and signature. The shells' own values are in `src/shell.ts`, and must match the fixtures.

## Debug a failure

Each stage writes to `artifacts/<platform>/<stage>/` (in CI, the `e2e-logs-*` and `e2e-results-*` workflow artifacts). `commands.log` has every command and its output. The `test` stage adds a folder per app, named for its file, with `report.json`, what the app reported, and `screenshot.png`, the app on screen. When an app never reports, iOS adds `app.log` and any crash report, and Android's `logcat.log` has the crash, which the error also quotes.

## Things to know

- **No Maestro, on purpose.** The app reports what it found itself (a file in its data container on iOS, a logcat line on Android), so nothing has to drive the UI. That's faster and steadier, and needs nothing installed beyond the platforms' own tools. A test that has to interact with an app (tapping through a Lynx shell, accepting a permission) is where Maestro would come in.
- **The iOS shell travels as a zip of the `.app`**, made with `ditto`, which keeps each file's mode. A `.app` is a folder, and its executable's mode wouldn't survive a workflow artifact or a Windows file system.
- **The iOS icon check reads the PNGs Xcode writes next to `Assets.car`.** The home screen uses the copy inside `Assets.car`, which no public API reads: `actool` stores app icons as a kind of asset `UIImage(named:)` doesn't load. That copy needs checking outside the app, with `assetutil --info`.
- **The fixtures are native projects**, so they keep Xcode's and Android Studio's formatting: oxfmt skips `fixtures/`.
- **The release APK is signed with the Android SDK's debug key**, so it installs without a keystore of its own. omni-build re-signs it with its own.
- **The harness uses only Node's built-in modules**, and Node runs its TypeScript as it is, stripping the types (Node 22.18 or later), so CI runs it without `npm ci` or a build.
- **The harness has unit tests too**, a `.test.ts` beside each module, which CI's coverage job holds to 100%. They stand in for Xcode, Gradle, the JDK, adb, simctl and the emulator, so they run on any machine in a fraction of a second and catch a broken harness before the slow jobs do. Run them with `npm test -w @omni-build/e2e`. As in the other packages, `cli.ts` only wires `run.ts` to the process, so the tests reach everything else in-process.
