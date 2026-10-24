# @blotch/omni-build-shells

The app shells OmniBuild ships: native apps that keep nothing about an app in their Swift or Kotlin code, so `omni repackage` makes any number of apps from one build, with no compiler. Each is a Swift project, as Xcode's iOS App template makes one, and a Kotlin project, as Android Studio's Empty Views Activity template makes one.

| Shell                | What it runs                                                                                                                              |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| [`webview`](webview) | A web app: the payload's `index.html` and the files beside it, in a WebView that fills the screen ([docs](../docs/docs/webview-shell.md)) |

Xcode and Gradle build them in OmniBuild's CI, never on a user's machine. The `e2e-shell-*` jobs build each shell, `e2e-repackage` runs it through `omni repackage` on Linux, Windows and macOS with no toolchain, and `e2e-ios` and `e2e-android` run the results on the iOS Simulator and on Android 5.0 and 14 emulators (see `packages/e2e`).

## Build one

```sh
cd webview/android && ./gradlew assembleRelease bundleRelease
cd webview/ios && xcodebuild build -scheme WebViewShell -configuration Release -destination 'generic/platform=iOS Simulator'
npm run e2e -- --platform android   # from the repository root: build, repackage and run every shell
```

## Things to know

- **Every shell moves with the platforms together.** The platform watch reads the WebView shell's Android API levels, and CI builds every iOS shell with one Xcode. `packages/e2e/src/shell.ts` lists every shell's projects, and its tests fail unless they share their API levels, Android Gradle plugin and iOS deployment target, so raising one means raising all of them, the E2E fixture's included.
- **No AndroidX.** The Android shells are plain `Activity`s on the platform's own APIs, so they run on Android 5.0, and the APK holds nothing but the shell. androidx.webkit, whose `WebViewAssetLoader` the WebView shell would otherwise use, needs Android 7.0, so the shell serves its payload the same way itself (`PayloadLoader.kt`).
- **Native projects keep their tools' formatting.** oxfmt skips `*/android` and `*/ios`, and cspell the Gradle wrappers and Xcode projects.
- **The release build is signed with the Android SDK's debug key**, so it installs without a keystore of its own. OmniBuild signs every app it makes from a shell with that app's key.
