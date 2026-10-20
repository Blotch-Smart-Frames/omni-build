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
  "payload": "dist"
}
```

| Field     | Android                                                                                                                                                                                                                         | iOS                                                                                                                                                                                                                                                                             |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`      | The manifest's `package`, and what the Android Gradle plugin derives from the application ID: content provider authorities and the permissions the app declares for itself, so two apps from one shell don't clash on a device. | `CFBundleIdentifier` in `Info.plist`: letters, digits, hyphens and dots.                                                                                                                                                                                                        |
| `name`    | The application's label, the name under the icon: in `resources.arsc`, in every language, when the label is a string resource, or in the manifest when it's text there.                                                         | `CFBundleDisplayName` in `Info.plist`, the name under the icon.                                                                                                                                                                                                                 |
| `version` | `android:versionName`, the version people see.                                                                                                                                                                                  | `CFBundleShortVersionString` in `Info.plist`, the version people see: one to three numbers separated by dots, as the App Store requires.                                                                                                                                        |
| `build`   | `android:versionCode`, which Google Play compares between uploads: from 1 to 2,100,000,000.                                                                                                                                     | `CFBundleVersion` in `Info.plist`, which App Store Connect compares between uploads.                                                                                                                                                                                            |
| `icon`    | A square PNG, resized to each size the shell has its icon in, for the icon and the round icon.                                                                                                                                  | A square PNG, resized to each size the shell has its icon in: in the asset catalog, `Assets.car`, where the home screen and the App Store take it from, and in the PNGs beside it for earlier versions of iOS. Make it opaque: App Store validation rejects an icon that isn't. |
| `payload` | Its files go in the app's `assets/`.                                                                                                                                                                                            | Its files go in the `.app`, beside `Info.plist`, where `Bundle.main` finds them.                                                                                                                                                                                                |

The payload is a folder of files the app reads at run time, such as a Lynx bundle. Its files replace the shell's own files of the same name, and hidden files (`.DS_Store`) are left out.

Paths are relative to the config file. A field you leave out keeps the shell's value, and a field OmniBuild doesn't know is an error, so a typo doesn't pass unnoticed. On iOS, every other key in `Info.plist` stays as Xcode wrote it, the `DT*` keys among them: they record the Xcode and SDK that built the shell, which App Store validation checks.

## Signing

An APK is signed with APK Signature Schemes v2 and v3, which Android 7.0 and later verify. An app bundle gets a JAR signature, which is how Google Play checks an upload before it signs the APKs it makes from the bundle with the app's own key. With no key given, OmniBuild uses a debug key it creates the first time, in `~/.omni-build/android-debug-key.pem`, the way the Android SDK keeps its own in `~/.android`. APKs signed with it install on devices and emulators. Google Play takes an app signed with your own key, which you give with `--signing-key`.

```sh
export OMNI_BUILD_KEY_PASSWORD='a password'
omni-build key create upload.p12 --subject "CN=Your name, O=Your organization"
omni-build repackage shell.aab --config app.json --out app.aab --signing-key upload.p12
```

`omni-build key create` makes an upload key without a JDK: an RSA key with a certificate of its own, valid for 30 years (Google Play requires an upload key to last past 2033), in a PKCS #12 keystore under the alias `upload`. keytool, Android Studio and Gradle read it too. Keep it and its password safe: Google Play takes updates signed with it alone, unless you ask Google to reset it.

`--signing-key` takes a PKCS #12 keystore, whose password OmniBuild reads from `OMNI_BUILD_KEY_PASSWORD` rather than a flag, which anyone listing your processes could see. That includes `.p12` files from Keychain Access or OpenSSL, and the `.jks` files keytool and Android Studio have made since Java 9; an older `.jks` is in Java's own format, which keytool converts (`keytool -importkeystore -srckeystore old.jks -destkeystore new.p12 -deststoretype pkcs12`). `--signing-key` also takes a PEM file with the private key (PKCS #8) and its certificate. Either way, the key is RSA, as Google Play's upload keys and Apple's signing certificates are.

An iOS app gets Apple's code signature, as `codesign` writes it: every file sealed in `_CodeSignature/CodeResources`, and the executable signed over the seal, `Info.plist` and the entitlements.

- **For the Simulator**, it's signed ad hoc, with no certificate, as Xcode signs a Simulator build. To run one, unzip it and install the `.app`:

  ```sh
  ditto -x -k app.zip app && xcrun simctl install booted app/*.app
  ```

- **For devices**, it's signed with a key Apple issued a certificate for (`--signing-key`), and a provisioning profile that allows the certificate and the app's ID (`--profile`). Export the key from Keychain Access as a `.p12` and give it as it is: OmniBuild adds the certificates above it, Apple's Worldwide Developer Relations authority (G3) and Apple's root, as `codesign` does. The profile goes in the app as `embedded.mobileprovision`, and its App ID, team and `get-task-allow` or `beta-reports-active` become the app's entitlements. Other capabilities, such as push notifications, come later.

## What a shell needs

On Android:

- A `minSdkVersion` of 24 (Android 7.0) or later. Earlier versions need a v1 signature, which OmniBuild doesn't write.
- An application label and icon in its manifest, with the icon as PNGs. Adaptive icons aren't supported yet.
- No label of its own on the launcher activity, which would show instead of the app's name.

On iOS:

- A build for the Simulator (`iPhoneSimulator` in its `Info.plist`'s `CFBundleSupportedPlatforms`), or for devices (`iPhoneOS`).
- A signature in its executable, if only ad hoc, for OmniBuild to replace: Xcode signs a Simulator build, and the linker reserves one in a build for devices with `OTHER_LDFLAGS=-Wl,-adhoc_codesign`, even with `CODE_SIGNING_ALLOWED=NO`.
- No code nested in its bundle (`Frameworks/`, `PlugIns/`), which OmniBuild can't sign yet.
- An app icon of a single size, Xcode's default, in its asset catalog. When an icon set has every size, Xcode can pack the smaller ones in with other images, which OmniBuild can't rewrite yet.
- The `.app` zipped with its files' modes, so that its executable can still run: `ditto -c -k --keepParent Shell.app shell.zip`, or `Payload/Shell.app` the same way for an `.ipa`, keeps them.
- No localized display name (`CFBundleDisplayName` in an `InfoPlist.strings`), which would show instead of the app's name.
