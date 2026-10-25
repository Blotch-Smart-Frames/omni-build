# @blotch/omni-build-shells

The app shells OmniBuild ships: native apps that keep nothing about an app in their Swift or Kotlin code, so `omni repackage` makes any number of apps from one build, with no compiler. Each is a Swift project, as Xcode's iOS App template makes one, and a Kotlin project, as Android Studio's Empty Views Activity template makes one.

| Shell                | What it runs                                                                                                                              |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| [`webview`](webview) | A web app: the payload's `index.html` and the files beside it, in a WebView that fills the screen ([docs](../docs/docs/webview-shell.md)) |

Xcode and Gradle build them in OmniBuild's CI, never on a user's machine. Each release of OmniBuild has the WebView shell's variants and their manifest beside its archives, as the Shell workflow built and checked them.

## The Shell workflow

`.github/workflows/shell.yml` builds any shell's variants, checks that omni repackages each one, and leaves them in an artifact, `shell-<name>`, with their manifest. It's a reusable workflow: frameworks call it for their own shells, from their repositories, and OmniBuild's CI calls it for the WebView shell and the E2E fixture.

```yaml
jobs:
  shell:
    uses: Blotch-Smart-Frames/omni-build/.github/workflows/shell.yml@v1.2.3
    with:
      name: MyShell
      xcode-project: ios/MyShell.xcodeproj
      gradle-project: android
```

| Variant                                | What it is                                                                                                           |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `<name>-<configuration>.apk`           | An Android APK                                                                                                       |
| `<name>-<configuration>.aab`           | An Android app bundle                                                                                                |
| `<name>-<configuration>-simulator.zip` | An iOS app for the Simulator, a zipped `.app`                                                                        |
| `<name>-<configuration>.ipa`           | An iOS app for devices, unsigned, with room for omni's signature                                                     |
| `<name>.json`                          | The manifest: each variant's file, platform, format, configuration, SHA-256 and size, and the omni that checked them |

Each comes in `debug` and `release`, as Xcode's Debug and Release configurations and Gradle's debug and release build types build them. The workflow runs the tool in `src/`, from OmniBuild at the commit it's called at (`job.workflow_repository` and `job.workflow_sha`), so a shell is built and checked as that version of OmniBuild does it:

```sh
npm run shells -w @blotch/omni-build-shells -- build --platform ios --project webview/ios/WebViewShell.xcodeproj --name WebViewShell --out out
npm run shells -w @blotch/omni-build-shells -- build --platform android --project webview/android --name WebViewShell --out out
npm run shells -w @blotch/omni-build-shells -- check --shells out --name WebViewShell --omni ../../target/release/omni
npm run shells -w @blotch/omni-build-shells -- --help
```

A shell is a project as Xcode's and Android Studio's templates make one: an Xcode scheme that builds an app of the same name, and a Gradle project whose app is the module `app`. `build` builds the variants of one platform:

- **iOS:** `xcodebuild` for the Simulator and for devices, each by a destination rather than an SDK, so a watch app builds for watchOS. The builds for devices are unsigned (`CODE_SIGNING_ALLOWED=NO`), with a signature's room reserved by the linker (`-Wl,-adhoc_codesign`, after the project's own flags): omni replaces a signature and never adds one. Debug builds are built whole (`ENABLE_DEBUG_DYLIB=NO`): Xcode otherwise moves the app's code into a dylib for Previews, behind a stub executable the linker leaves unsigned. Each `.app` travels as a zip, made with `ditto`, which keeps each file's mode.
- **Android:** `gradlew assembleDebug bundleDebug assembleRelease bundleRelease`, taking the one APK and app bundle Gradle writes for each build type, signed or not.

`check` repackages each variant with omni, as an app of its own (`check/app.json`: an ID, name, version, build number, icon and payload), so each of omni's rewrites runs on it, and checks that omni took it for what its name says. Then it writes the manifest. Without `--omni`, it runs the `omni` on the `PATH`. The workflow checks with the omni of its own version, from npm, and OmniBuild's CI with the one its build job made.

The `.ipa` is signed with a key and provisioning profile made for the check, `check/signing-key.p12` (password `omnibuild-check`) and `check/OmniBuildCheck.mobileprovision`: self-signed, for a team that doesn't exist (`OMNIBUILD0`), so no device would install what they sign. The profile's App ID is a wildcard, `OMNIBUILD0.*`, which covers the IDs omni gives a shell's extensions, App Clip and watch app under the app's, and it allows `com.apple.developer.on-demand-install-capable`, as an App Clip's profile does: omni gives that only to an App Clip. They're made as the E2E tests' key and profiles are (`packages/e2e/README.md`), with `-subj "/CN=OmniBuild Shell Check/OU=OMNIBUILD0/O=OmniBuild"`, `-name "OmniBuild Shell Check"` and `-passout pass:omnibuild-check`, and a profile named `OmniBuild Shell Check`, with the UUID `8293A4B5-C6D7-48E9-FA0B-1C2D3E4F5061`, whose `Entitlements` are:

```python
{'application-identifier': 'OMNIBUILD0.*', 'com.apple.developer.team-identifier': 'OMNIBUILD0',
 'get-task-allow': True, 'keychain-access-groups': ['OMNIBUILD0.*'],
 'com.apple.developer.on-demand-install-capable': True}
```

## E2E

The `shell` jobs in CI build the WebView shell and the fixture with the Shell workflow, `e2e-repackage` runs their release variants through `omni repackage` on Linux, Windows and macOS with no toolchain, and `e2e-ios` and `e2e-android` run the results on the iOS Simulator and on Android 5.0 and 14 emulators (see `packages/e2e`). Locally, the E2E harness builds the release variants with the same tool:

```sh
npm run e2e -- --platform android   # from the repository root: build, repackage and run every shell
```

## Things to know

- **Every shell moves with the platforms together.** The platform watch reads the WebView shell's Android API levels, and CI builds every iOS shell with one Xcode. `packages/e2e/src/shell.ts` lists every shell's projects, and its tests fail unless they share their API levels, Android Gradle plugin and iOS deployment target, so raising one means raising all of them, the E2E fixture's included.
- **No AndroidX.** The Android shells are plain `Activity`s on the platform's own APIs, so they run on Android 5.0, and the APK holds nothing but the shell. androidx.webkit, whose `WebViewAssetLoader` the WebView shell would otherwise use, needs Android 7.0, so the shell serves its payload the same way itself (`PayloadLoader.kt`).
- **Native projects keep their tools' formatting.** oxfmt skips `*/android` and `*/ios`, and cspell the Gradle wrappers and Xcode projects.
- **The release build is signed with the Android SDK's debug key**, so it installs without a keystore of its own. OmniBuild signs every app it makes from a shell with that app's key.
