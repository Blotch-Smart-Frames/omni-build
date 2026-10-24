---
title: The WebView shell
description: OmniBuild's own app shell, which turns a web app, a folder with an index.html, into an iOS and Android app with omni repackage.
---

# The WebView shell

:::warning[Phase 1]

The shell is built and tested in OmniBuild's CI, and will be published with each release once the shell release pipeline lands. Until then, build it from the [repository](https://github.com/Blotch-Smart-Frames/omni-build/tree/master/packages/shells/webview) with Xcode and Gradle, as below.

:::

The WebView shell is an app shell OmniBuild ships: a native app that shows a web app in a WebView filling the screen. Your web app's files are its payload, so `omni repackage` turns any web app into an iOS and Android app, with no framework, compiler or SDK:

```json
{
  "id": "com.example.tides",
  "name": "Tides",
  "version": "1.4.0",
  "build": 14,
  "icon": "icon.png",
  "payload": "dist"
}
```

```sh
omni repackage WebViewShell.apk --config app.json --out tides.apk
omni repackage WebViewShell.aab --config app.json --out tides.aab --signing-key upload.p12
omni repackage WebViewShell.zip --config app.json --out tides.zip   # for the Simulator
omni repackage WebViewShell.ipa --config app.json --out tides.ipa \
  --signing-key key.pem --profile tides.mobileprovision
```

Everything in [Repackaging an app](./repackaging.md) applies: the app's ID, name, version, icon, permissions, links, orientation and the rest. Nothing about your app is in the shell's Swift or Kotlin code.

## The payload

`payload` is the folder your web app's build writes, with an `index.html` at its top. The app opens that page on launch, and the page loads the rest by relative URLs, as on a web server. OmniBuild puts the folder in the APK's `assets/`, and at the top of the iOS app's bundle.

| Platform | The page's address                                        | Its origin                              |
| -------- | --------------------------------------------------------- | --------------------------------------- |
| Android  | `https://appassets.androidplatform.net/assets/index.html` | `https://appassets.androidplatform.net` |
| iOS      | `app://localhost/index.html`                              | `app://localhost`                       |

A page opened from a `file://` URL has no origin of its own, and browsers refuse it most of what a web app needs. So the shell serves the payload itself, from an address of its own that no website uses: Android's is the host androidx.webkit's `WebViewAssetLoader` uses, and iOS's a scheme of the shell's own. The page gets a secure origin, and `fetch`, ES modules, `localStorage` and IndexedDB work as on the web. A URL that ends in `/` serves that folder's `index.html`, a missing file is a 404, and files are served with the web's types (`text/javascript`, `application/wasm`, ...), which module scripts need.

## Links

Links to other websites open in the browser, Safari on iOS, and `mailto:` and `tel:` links in the app that handles them. Pages of the payload, and frames inside a page, such as an embedded video, stay in the app. The back button on Android, and the edge swipe on iOS, go back in the page's history, and leave the app when there's none.

`alert()`, `confirm()` and `prompt()` show the system's dialogs, and `<input type="file">` the system's picker.

## The console

What the page logs to its console goes to the system's log, with no web inspector attached:

- **Android:** to logcat, under the tag `OmniBuildWebView`. Warnings and errors say which file and line they came from.

  ```sh
  adb logcat -s OmniBuildWebView
  ```

- **iOS:** to the unified log, under your app's ID as its subsystem and the category `console`, with uncaught errors and unhandled promise rejections. Console.app shows it for a device, and for the Simulator:

  ```sh
  xcrun simctl spawn booted log stream --predicate 'subsystem == "com.example.tides" AND category == "console"'
  ```

The messages are public in the log, as they are to anyone with a web inspector, so don't log what the device's owner mustn't see.

## Debug and release builds

A debug build of the shell can be inspected: Chrome's `chrome://inspect` for a debuggable Android build, and Safari's Web Inspector for an iOS build from Xcode's Debug configuration. A release build can't, which is what goes to the stores.

## Layout

- **Android** keeps the page clear of the status bar, the navigation bar, a display cutout and the keyboard, with white bars and dark icons, from Android 6.0. From Android 15, which draws apps under the bars, the shell pads the page instead.
- **iOS** draws the page under the status bar and the home indicator, as Safari does. A page that asks for the whole screen (`<meta name="viewport" content="viewport-fit=cover">`) keeps clear of them with `env(safe-area-inset-top)` and the like; one that doesn't is kept clear by WebKit.

## Devices

Android 5.0 and later, and iOS 17 and later, on iPhone, which iPads run as iPhone apps. On Android, the page runs in the device's own WebView, which Google Play updates. One that hasn't been updated is as old as the device: Android 5.0 shipped Chrome 37, which has no `fetch`, arrow functions or template literals. CI runs the shell on Android 5.0 and 14 emulators and the iOS Simulator.

Not yet: on Android, the page's requests for the camera, microphone and location, which need the app to ask for Android's permissions. iOS asks the user itself, given the reason in `ios.usageDescriptions` (`NSCameraUsageDescription`, `NSLocationWhenInUseUsageDescription`, ...).

## Building it

The shell is two native projects, which OmniBuild's CI builds as its users never have to:

```sh
cd packages/shells/webview/android
./gradlew assembleRelease bundleRelease   # app/build/outputs/apk/release/app-release.apk and bundle/release/app-release.aab

cd packages/shells/webview/ios
xcodebuild build -scheme WebViewShell -configuration Release -destination 'generic/platform=iOS Simulator'
xcodebuild build -scheme WebViewShell -configuration Release -destination 'generic/platform=iOS' \
  CODE_SIGNING_ALLOWED=NO OTHER_LDFLAGS=-Wl,-adhoc_codesign   # unsigned, with room for OmniBuild's signature
```

A zipped `.app` is the Simulator shell (`ditto -c -k --keepParent WebViewShell.app WebViewShell.zip`), and a zip of a `Payload` folder with the `.app` the shell for devices (`WebViewShell.ipa`). `npm run e2e` builds both platforms' shells, repackages them, and runs them, as the [contributing guide](./contributing.md) says.
