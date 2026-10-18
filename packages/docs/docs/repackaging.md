---
title: Repackaging an app
description: Turn an Android app shell into your app with omni-build repackage, and what the app config changes.
---

# Repackaging an app

:::warning[Phase 0]

Only Android APK shells so far, signed with a debug key. OmniBuild isn't published yet: build the binary from the [repository](https://github.com/Blotch-Smart-Frames/omni-build) with `cargo build --release`.

:::

```sh
omni-build repackage shell.apk --config app.json --out app.apk
```

`repackage` reads a prebuilt app shell, gives it your app's values, and signs it. Nothing else is installed or run: no JDK, no Android SDK, no Gradle.

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

| Field     | What it changes in the APK                                                                                                                                                                                                      |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`      | The manifest's `package`, and what the Android Gradle plugin derives from the application ID: content provider authorities and the permissions the app declares for itself, so two apps from one shell don't clash on a device. |
| `name`    | The application's label, the name under the icon: in `resources.arsc`, in every language, when the label is a string resource, or in the manifest when it's text there.                                                         |
| `version` | `android:versionName`, the version people see.                                                                                                                                                                                  |
| `build`   | `android:versionCode`, which Google Play compares between uploads: from 1 to 2,100,000,000.                                                                                                                                     |
| `icon`    | A square PNG, resized to each size the shell has its icon in, for the icon and the round icon.                                                                                                                                  |
| `payload` | A folder whose files go in the APK's `assets/`, such as a Lynx bundle, replacing the shell's own files of the same name. Hidden files (`.DS_Store`) are left out.                                                               |

Paths are relative to the config file. A field you leave out keeps the shell's value, and a field OmniBuild doesn't know is an error, so a typo doesn't pass unnoticed.

## Signing

`repackage` signs with APK Signature Schemes v2 and v3, which Android 7.0 and later verify. With no key given, it uses a debug key it creates the first time, in `~/.omni-build/android-debug-key.pem`, the way the Android SDK keeps its own in `~/.android`. Apps signed with it install on devices and emulators; signing with your own upload key, for Google Play, comes with [Phase 2](./roadmap.md).

## What a shell needs

- A `minSdkVersion` of 24 (Android 7.0) or later. Earlier versions need a v1 signature, which OmniBuild doesn't write.
- An application label and icon in its manifest, with the icon as PNGs. Adaptive icons aren't supported yet.
- No label of its own on the launcher activity, which would show instead of the app's name.
