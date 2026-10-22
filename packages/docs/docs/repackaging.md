---
title: Repackaging an app
description: Turn an app shell, an Android APK or app bundle or an iOS app, into your app with omni-build repackage, and what the app config changes.
---

# Repackaging an app

:::warning[Phase 0]

Android shells, APKs and app bundles, and iOS shells, for the Simulator or for devices. No iOS build OmniBuild signed has been through TestFlight yet. OmniBuild isn't published yet: build the binary from the [repository](https://github.com/Blotch-Smart-Frames/omni-build) with `cargo build --release`.

:::

```sh
omni-build repackage shell.apk --config app.json --out app.apk
omni-build repackage shell.aab --config app.json --out app.aab
omni-build repackage shell.zip --config app.json --out app.zip   # a zipped iOS .app, for the Simulator
omni-build repackage shell.ipa --config app.json --out app.ipa \
  --signing-key key.pem --profile app.mobileprovision            # for devices
  # and --profile again for each app extension, unless app.mobileprovision's App ID is a wildcard,
  # and for an App Clip, a watch app and its widgets
```

`repackage` reads a prebuilt app shell, gives it your app's values, and signs it. The shell is an APK, which installs on a device, an Android App Bundle (AAB), which you upload to Google Play, or an iOS app: an `.ipa` for devices, or a zipped `.app` for the Simulator. OmniBuild tells which from its contents, and writes the app in the same form. Nothing else is installed or run: no JDK, no Android SDK, no Gradle, no Xcode.

## The app config

```json
{
  "id": "com.example.tides",
  "name": "Tides",
  "version": "1.4.0",
  "build": 14,
  "icon": "icon.png",
  "payload": "dist",
  "urlSchemes": ["tides"],
  "orientation": "portrait",
  "android": {
    "adaptiveIcon": {
      "foreground": "icon-foreground.png",
      "background": "#0B3D91"
    },
    "permissions": ["INTERNET", "ACCESS_FINE_LOCATION"],
    "abis": ["arm64-v8a", "x86_64"]
  },
  "ios": {
    "usageDescriptions": {
      "NSLocationWhenInUseUsageDescription": "Shows the tides where you are."
    },
    "entitlements": {
      "aps-environment": "development",
      "com.apple.developer.associated-domains": ["applinks:tides.example.com"]
    }
  }
}
```

| Field         | Android                                                                                                                                                                                                                                                                                                          | iOS                                                                                                                                                                                                                                                                                                                                       |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`          | The manifest's `package`, and what the Android Gradle plugin derives from the application ID: content provider authorities and the permissions the app declares for itself, so two apps from one shell don't clash on a device.                                                                                  | `CFBundleIdentifier` in `Info.plist`: letters, digits, hyphens and dots.                                                                                                                                                                                                                                                                  |
| `name`        | The application's label, the name under the icon: in `resources.arsc`, in every language, when the label is a string resource, or in the manifest when it's text there. The launcher activity's own label, and its launcher intent filter's, which a launcher shows instead, get it too.                         | `CFBundleDisplayName` in `Info.plist`, the name under the icon. A translation of it in the shell's `InfoPlist.strings`, which iOS would show instead, goes.                                                                                                                                                                               |
| `version`     | `android:versionName`, the version people see.                                                                                                                                                                                                                                                                   | `CFBundleShortVersionString` in `Info.plist`, the version people see: one to three numbers separated by dots, as the App Store requires.                                                                                                                                                                                                  |
| `build`       | `android:versionCode`, which Google Play compares between uploads: from 1 to 2,100,000,000.                                                                                                                                                                                                                      | `CFBundleVersion` in `Info.plist`, which App Store Connect compares between uploads.                                                                                                                                                                                                                                                      |
| `icon`        | A square PNG, resized to each size the shell has its icon in, PNG or WebP, for the icon and the round icon. An adaptive icon, which Android 8.0 and later show, gets it too: see below.                                                                                                                          | A square PNG, resized to each size the shell has its icon in: in the asset catalog, `Assets.car`, where the home screen and the App Store take it from, and in the PNGs beside it for earlier versions of iOS. Make it opaque: App Store validation rejects an icon that isn't.                                                           |
| `payload`     | Its files go in the app's `assets/`.                                                                                                                                                                                                                                                                             | Its files go in the `.app`, beside `Info.plist`, where `Bundle.main` finds them.                                                                                                                                                                                                                                                          |
| `urlSchemes`  | An intent filter on the launcher activity, so that links such as `tides://forecast` open the app, as deep links: from a browser too (`BROWSABLE`). Schemes the manifest has already are left as they are.                                                                                                        | A URL type in `Info.plist` (`CFBundleURLTypes`), named after the app's ID, for the schemes the shell's don't have.                                                                                                                                                                                                                        |
| `orientation` | `android:screenOrientation` on the launcher activity: `portrait`, `userLandscape`, which turns to either side as the phone is held, unless its user has locked the rotation, or `unspecified`, the system's choice. Android 16 and later ignore it on large screens, such as tablets', in apps that target them. | `UISupportedInterfaceOrientations~iphone` in `Info.plist`, the orientations an iPhone reads over those for every device: portrait, landscape left and right, or all three. An iPad keeps the shell's: Apple asks iPad apps to support every orientation, so that they can share the screen, and Xcode warns that it will soon require it. |

The payload is a folder of files the app reads at run time, such as a Lynx bundle. Its files replace the shell's own files of the same name, and hidden files (`.DS_Store`) are left out.

URL schemes are added to any the shell has, so a shell that many apps are made from shouldn't have one of its own: every app would answer its links. Each is RFC 3986's letters in lowercase, which Android matches as they are, such as `tides` or `com.example.tides`. `http` and `https` are refused: a web link opens an app only once the app and its website vouch for each other. On iOS, the app's side of that is its website's domain in `ios.entitlements` (universal links); Android's, App Links, OmniBuild doesn't set up yet.

`orientation` is which way up the app's screen is on phones: `portrait`, `landscape`, on either side, or `any`, turning with the phone as apps do by default, upright or on either side but not upside down.

What only one platform has goes in `android` and `ios`:

| Field                   | What it changes                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `android.adaptiveIcon`  | The layers of the shell's adaptive icon, which launchers crop to their own shape: `foreground`, a square PNG of the whole 108dp layer with what matters in the middle 66dp, `background`, a colour (`#RRGGBB`) or a PNG, and optionally `monochrome`, which Android 13 and later tint to the system's colours (the foreground's, without it). Without `adaptiveIcon`, the layers get `icon` whole, inside the launcher's shape, and the shell's monochrome layer goes. |
| `android.permissions`   | The permissions the app asks for, `<uses-permission>` in its manifest, as `android.permission.CAMERA` or `CAMERA` for short. The shell's others go, but those under its own application ID, which its code declares for itself.                                                                                                                                                                                                                                        |
| `android.abis`          | The ABIs the app keeps native libraries for, from the shell's `lib/` folders, such as `arm64-v8a` and `x86_64`. The others go, which makes the APK smaller.                                                                                                                                                                                                                                                                                                            |
| `ios.usageDescriptions` | Why the app uses what iOS asks its user to allow, by `Info.plist` key, such as `NSCameraUsageDescription`. They replace the shell's, translations included.                                                                                                                                                                                                                                                                                                            |
| `ios.entitlements`      | The entitlements the app claims for the capabilities it uses, as an Xcode project's `.entitlements` file has them: a string, a boolean or an array of strings by key, such as `aps-environment` for push notifications, or `com.apple.developer.associated-domains` for universal links. A build for devices is signed with them, if its provisioning profile allows each: see [Signing](#signing).                                                                    |
| `ios.bundles`           | What the bundles nested in the app have of their own, by the part of their ID after the app's: its app extensions, App Clips and watch app, and the extensions in those. For now, the `entitlements` each claims, such as the App Group a notification extension shares with the app: see [Signing](#signing).                                                                                                                                                         |

Paths are relative to the config file. A field you leave out keeps the shell's value, and a field OmniBuild doesn't know is an error, so a typo doesn't pass unnoticed. On iOS, every other key in `Info.plist` stays as Xcode wrote it, the `DT*` keys among them: they record the Xcode and SDK that built the shell, which App Store validation checks.

## Signing

An APK is signed with APK Signature Schemes v2 and v3, which Android 7.0 and later verify, as `apksigner` signs it. When the shell runs on earlier versions too, which verify only a v1 (JAR) signature, it gets one of those as well. An app bundle gets a JAR signature, which is how Google Play checks an upload before it signs the APKs it makes from the bundle with the app's own key. With no key given, OmniBuild uses a debug key it creates the first time, in `~/.omni-build/android-debug-key.pem`, the way the Android SDK keeps its own in `~/.android`. APKs signed with it install on devices and emulators. Google Play takes an app signed with your own key, which you give with `--signing-key`.

```sh
export OMNI_BUILD_KEY_PASSWORD='a password'
omni-build key create upload.p12 --subject "CN=Your name, O=Your organization"
omni-build repackage shell.aab --config app.json --out app.aab --signing-key upload.p12
```

`omni-build key create` makes an upload key without a JDK: an RSA key with a certificate of its own, valid for 30 years (Google Play requires an upload key to last past 2033), in a PKCS #12 keystore under the alias `upload`. keytool, Android Studio and Gradle read it too. Keep it and its password safe: Google Play takes updates signed with it alone, unless you ask Google to reset it.

`--signing-key` takes a PKCS #12 keystore, whose password OmniBuild reads from `OMNI_BUILD_KEY_PASSWORD` rather than a flag, which anyone listing your processes could see. That includes `.p12` files from Keychain Access or OpenSSL, and the `.jks` files keytool and Android Studio have made since Java 9; an older `.jks` is in Java's own format, which keytool converts (`keytool -importkeystore -srckeystore old.jks -destkeystore new.p12 -deststoretype pkcs12`). `--signing-key` also takes a PEM file with the private key (PKCS #8) and its certificate. Either way, the key is RSA, as Google Play's upload keys and Apple's signing certificates are.

An iOS app gets Apple's code signature, as `codesign` writes it: every file sealed in `_CodeSignature/CodeResources`, and the executable signed over the seal, `Info.plist` and the entitlements. Each framework in its `Frameworks/` is signed first, the same way but without entitlements, and then each app extension, with entitlements of its own, as Xcode signs them: those in its `PlugIns/`, and the ExtensionKit extensions in its `Extensions/`, as Xcode's templates now make many (App Intents, Background Assets), then a watch app in its `Watch/`, and then an App Clip in its `AppClips/`. An extension's, watch app's or App Clip's ID stays under the app's, as iOS requires: `dev.example.app.Share` for the shell's `dev.omnibuild.shell.Share`, `dev.example.app.watchkitapp` for `dev.omnibuild.shell.watchkitapp`, and `dev.example.app.Clip` for `dev.omnibuild.shell.Clip`. It also takes the app's version and build number, which App Store validation requires, and a watch app pairs with the app's ID (`WKCompanionAppBundleIdentifier`) in place of the shell's. An extension, a watch app or an App Clip has frameworks and extensions of its own in turn, such as a watch app's widgets in its own `PlugIns/`, or the frameworks an App Clip embeds, since it downloads without the app: each is signed before the bundle it's in, as Apple signs from the inside out, and an extension's ID stays under the one it's in (`dev.example.app.watchkitapp.Widgets` for `dev.omnibuild.shell.watchkitapp.Widgets`). A watch app's `arm64_32` code for watchOS before 12 is signed with SHA-1 hashes as well as SHA-256 ones, as `codesign` signs it, and so is every architecture beside it in the same file, and the watch app's seal.

- **For the Simulator**, it's signed ad hoc, with no certificate, as Xcode signs a Simulator build. To run one, unzip it and install the `.app`:

  ```sh
  ditto -x -k app.zip app && xcrun simctl install booted app/*.app
  ```

- **For devices**, it's signed with a key Apple issued a certificate for (`--signing-key`), and a provisioning profile that allows the certificate and the app's ID (`--profile`). Export the key from Keychain Access as a `.p12` and give it as it is: OmniBuild adds the certificates above it, Apple's Worldwide Developer Relations authority (G3) and Apple's root, as `codesign` does. The profile goes in the app as `embedded.mobileprovision`, and its App ID, team and `get-task-allow` or `beta-reports-active` become the app's entitlements. Each app extension and a watch app, and the extensions in those, has an ID of its own, and needs a profile of its own too, unless a wildcard App ID covers it (`TEAM.*`), so give `--profile` once for each. Each bundle takes the profile whose App ID fits it closest, as Xcode picks them: its own App ID over a wildcard, and `TEAM.dev.example.*` over `TEAM.*`. An App Clip always needs its own: the profile of an App ID registered as an App Clip's, of the app's App ID, which allows what makes it one. It's signed as one of the app's (`com.apple.developer.on-demand-install-capable` and `com.apple.developer.parent-application-identifiers`), and the app names it in turn (`com.apple.developer.associated-appclip-app-identifiers`), as Xcode signs them.

The app also claims the entitlements in `ios.entitlements`, for the capabilities it uses, such as push notifications, and its profile has to allow each, as iOS checks them: turn the capability on for the App ID, and make the profile again. A profile allows a value it has, or one its wildcard covers (`TEAM.*`, or `*` for any), and an array when it allows each of its strings. `aps-environment` is the profile's, `development` or `production`, as Xcode sets it. Xcode's build settings that an `.entitlements` file names are filled in as Xcode fills them in: `$(AppIdentifierPrefix)` and `$(TeamIdentifierPrefix)` with the team's ID and a dot, so `$(AppIdentifierPrefix)com.example.shared` is a keychain group of the team's, and `$(CFBundleIdentifier)` and `$(PRODUCT_BUNDLE_IDENTIFIER)` with the ID of the bundle that claims it. The Simulator reads its entitlements from the shell's executable, where Xcode links them, rather than its signature, so they apply to builds for devices.

The app's extensions, App Clips and watch app, and the extensions in those, claim entitlements of their own in `ios.bundles`, as each Xcode target has its own `.entitlements` file, and each one's own profile has to allow them. Each is named by the part of its ID after the app's, which is the same in the shell: `ShellNotifications` for the shell's `dev.omnibuild.shell.ShellNotifications`, which becomes `dev.example.app.ShellNotifications`. A name the shell has no bundle for is an error, and a bundle with nothing to claim is signed with the entitlements its profile gives every program. An App Group the app shares with its notification extension, say, is claimed by both:

```json
"ios": {
  "entitlements": {
    "com.apple.security.application-groups": ["group.com.example.tides"]
  },
  "bundles": {
    "ShellNotifications": {
      "entitlements": {
        "com.apple.security.application-groups": ["group.com.example.tides"]
      }
    }
  }
}
```

## What a shell needs

On Android:

- A `minSdkVersion` of 18 (Android 4.3) or later, for an APK. Earlier versions need a v1 signature made with SHA-1, which OmniBuild doesn't write. An app bundle can have any: Google Play signs the APKs it makes from it.
- An application label and icon in its manifest. The icon's bitmaps are PNG or WebP, and an adaptive icon names a drawable for each of its layers, as Android Studio's templates make them.

On iOS:

- A build for the Simulator (`iPhoneSimulator` in its `Info.plist`'s `CFBundleSupportedPlatforms`), or for devices (`iPhoneOS`).
- A signature in its executable, if only ad hoc, for OmniBuild to replace: Xcode signs a Simulator build, and the linker reserves one in a build for devices with `OTHER_LDFLAGS=-Wl,-adhoc_codesign`, even with `CODE_SIGNING_ALLOWED=NO`.
- No code nested in its bundle besides frameworks in `Frameworks/`, app extensions in `PlugIns/` and `Extensions/`, a watch app in `Watch/` and App Clips in `AppClips/`, and in an extension, a watch app or an App Clip, only frameworks and extensions of its own, in the same folders. Libraries outside a framework need signing on their own, which OmniBuild can't do yet, and a framework has no code nested in it.
- The `.app` zipped with its files' modes, so that its executable can still run: `ditto -c -k --keepParent Shell.app shell.zip`, or `Payload/Shell.app` the same way for an `.ipa`, keeps them.
- Its translations of `Info.plist` (`InfoPlist.strings`) as property lists, as Xcode writes them unless told to keep them as text.
