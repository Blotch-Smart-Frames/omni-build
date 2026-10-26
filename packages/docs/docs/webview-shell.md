---
title: The WebView shell
description: OmniBuild's own app shell, which turns a web app, a folder with an index.html, into an iOS and Android app with omni repackage.
---

# The WebView shell

:::warning[From the release after 0.0.1]

Each GitHub release of OmniBuild has the shell from the release after 0.0.1. Until then, build it from the [repository](https://github.com/Blotch-Smart-Frames/omni-build/tree/master/packages/shells/webview), as below.

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
omni repackage WebViewShell-release.apk --config app.json --out tides.apk
omni repackage WebViewShell-release.aab --config app.json --out tides.aab --signing-key upload.p12
omni repackage WebViewShell-release-simulator.zip --config app.json --out tides.zip
omni repackage WebViewShell-release.ipa --config app.json --out tides.ipa \
  --signing-key key.pem --profile tides.mobileprovision
```

Everything in [Repackaging an app](./repackaging.md) applies: the app's ID, name, version, icon, permissions, links, orientation and the rest. Nothing about your app is in the shell's Swift or Kotlin code.

## Getting it

Each [GitHub release](https://github.com/Blotch-Smart-Frames/omni-build/releases) of OmniBuild has the shell, debug and release, as the [Shell workflow](./shells.md) built it and CI ran it:

| File                                                                     | For                                                     |
| ------------------------------------------------------------------------ | ------------------------------------------------------- |
| `WebViewShell-release.apk`, `WebViewShell-debug.apk`                     | Android phones and emulators                            |
| `WebViewShell-release.aab`, `WebViewShell-debug.aab`                     | Google Play, which takes the release one                |
| `WebViewShell-release-simulator.zip`, `WebViewShell-debug-simulator.zip` | The iOS Simulator                                       |
| `WebViewShell-release.ipa`, `WebViewShell-debug.ipa`                     | iPhones, and the App Store, which takes the release one |
| `WebViewShell.json`                                                      | Each file's SHA-256                                     |

Take the shell from the release your omni is, which `omni --version` says, as CI checked that omni repackages it. Each file has an attestation of where it came from:

```sh
gh release download v1.2.3 --repo Blotch-Smart-Frames/omni-build --pattern 'WebViewShell-release.*'
gh attestation verify WebViewShell-release.apk --repo Blotch-Smart-Frames/omni-build
```

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

## Native modules

A web app reaches native code through the [native modules](./native-modules.md) among its npm dependencies, which `omni repackage` adds to the app when the app config's `packageJson` names its `package.json`. The shell's runtime is `webview@1`, and a module built for it is one the page calls:

```js
omniBuild.modules; // ['Haptics'], the app's native modules
const result = await omniBuild.call('Haptics', 'impact', { style: 'heavy' });
```

`window.omniBuild` is there before the page's own scripts run, on every page of the payload. `call` takes a module, by its class's own name, a method, and an argument, anything JSON takes, and resolves with the module's answer, or rejects with an `Error` of the module's reason, or one that says the app has no such module. Only the payload's own pages reach native code: a frame from elsewhere in them, such as an embedded video, can't, on iOS because the shell checks where each call comes from, and on Android because each call carries a token the shell gives the payload's pages alone.

### Writing a module for the WebView shell

A module's class has a method the shell calls for each of the page's calls, with the method's name and the argument as JSON. It answers once, with JSON or with why it failed, on any thread.

On iOS, a class with an Objective-C name, which the shell finds by that name, made with no arguments, and calls on the main thread:

```swift
@objc(Haptics)
public final class Haptics: NSObject {
    @objc public func call(_ method: String, argument: String, reply: @escaping (String?, String?) -> Void) {
        guard method == "impact" else { return reply(nil, "Haptics has no method \(method)") }
        UIImpactFeedbackGenerator(style: .heavy).impactOccurred()
        reply("true", nil)
    }
}
```

On Android, a class the shell makes with the activity, if it takes a `Context`, or with nothing, and calls on the main thread:

```kotlin
class Haptics(private val context: Context) {
    fun call(method: String, argument: String, reply: ValueCallback<String>, fail: ValueCallback<String>) {
        if (method != "impact") return fail.onReceiveValue("Haptics has no method $method")
        context.getSystemService(Vibrator::class.java).vibrate(VibrationEffect.createOneShot(50, 255))
        reply.onReceiveValue("true")
    }
}
```

Its `package.json` names the runtime and the class, which the page calls by its own name, without its package: `Haptics` for `com.example.haptics.Haptics`.

```json
"omniBuild": {
  "runtime": "webview@1",
  "ios": { "frameworks": ["ios/Haptics.xcframework"], "modules": ["Haptics"] },
  "android": { "dex": ["android/classes.dex"], "modules": ["com.example.haptics.Haptics"], "permissions": ["VIBRATE"] }
}
```

The [Module workflow](./native-modules.md#building-one) builds both with the Xcode and the Android Gradle plugin the shell is built with, and the end-to-end tests run such a module, `Echo`, which the test web app calls on the iOS Simulator and Android 5.0 and 14.

## Debug and release builds

A debug build of the shell (`WebViewShell-debug.*`) can be inspected: with Chrome's `chrome://inspect` on Android, and Safari's Web Inspector on iOS. A release build can't, which is what goes to the stores.

## Layout

- **Android** keeps the page clear of the status bar, the navigation bar, a display cutout and the keyboard, with white bars and dark icons, from Android 6.0. From Android 15, which draws apps under the bars, the shell pads the page instead.
- **iOS** draws the page under the status bar and the home indicator, as Safari does. A page that asks for the whole screen (`<meta name="viewport" content="viewport-fit=cover">`) keeps clear of them with `env(safe-area-inset-top)` and the like; one that doesn't is kept clear by WebKit.

## Devices

Android 5.0 and later, and iOS 17 and later, on iPhone, which iPads run as iPhone apps. On Android, the page runs in the device's own WebView, which Google Play updates. One that hasn't been updated is as old as the device: Android 5.0 shipped Chrome 37, which has no `fetch`, arrow functions or template literals. CI runs the shell on Android 5.0 and 14 emulators and the iOS Simulator.

Not yet: on Android, the page's requests for the camera, microphone and location, which need the app to ask for Android's permissions. iOS asks the user itself, given the reason in `ios.usageDescriptions` (`NSCameraUsageDescription`, `NSLocationWhenInUseUsageDescription`, ...).

## Building it

The shell is two native projects, which OmniBuild's CI builds as its users never have to. To build it yourself, with Xcode and Gradle, as the Shell workflow does:

```sh
npm run shells -w @blotch/omni-build-shells -- build --platform android --project webview/android --name WebViewShell --out out
npm run shells -w @blotch/omni-build-shells -- build --platform ios --project webview/ios/WebViewShell.xcodeproj --name WebViewShell --out out
```

`npm run e2e` builds both platforms' shells, repackages them, and runs them, as the [contributing guide](./contributing.md) says.
