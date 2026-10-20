---
title: Installing on a device
description: Install an APK on an Android emulator or device with omni-build install, over ADB, without the Android SDK.
---

# Installing on a device

:::warning[Phase 1]

Android emulators, and Android devices listening for ADB on the network. Phones connected by USB, and Android 11's wireless debugging, which pairs with a code first, come next. On iOS, install through TestFlight for now. OmniBuild isn't published yet: build the binary from the [repository](https://github.com/Blotch-Smart-Frames/omni-build) with `cargo build --release`.

:::

```sh
omni-build install app.apk                              # on the one emulator running
omni-build install app.apk --device emulator-5556       # on one of several
omni-build install app.apk --device 192.168.1.20:5555   # on a device on the network
```

`install` sends an APK to a device over ADB, the protocol the Android SDK's `adb` speaks to devices, and has the device's package manager install it, as `adb install` does. Nothing else is installed or run: no Android SDK, no `adb`. The device runs Android 7.0 or later, as the apps OmniBuild signs need. An app with the same ID is updated, keeping its data, when it's signed with the same key.

## Finding the device

Without `--device`, OmniBuild looks for emulators running on this computer where `adb` does, on the ports emulators listen on (5555, 5557, and so on to 5585), and installs on the one it finds. When several are running, it lists them, and `--device` names one as `adb` does: `emulator-5554` is the emulator whose console is on port 5554, the number in its window's title, and which listens for ADB on the next port, 5555.

A device on the network is named by its address and the port it listens on, which is 5555 once `adb tcpip 5555` has told it to listen.

## The ADB key

A device lets a computer install apps only once it trusts the computer's key. OmniBuild uses `adb`'s own, `~/.android/adbkey`, so a device that trusts your `adb` trusts OmniBuild, and the other way round. Without one, OmniBuild creates it there, as `adb` would, with its public key beside it in `adbkey.pub`. The first time, a device asks whether to allow the computer: allow it, and OmniBuild carries on. Emulators don't ask.

## When an install fails

OmniBuild gives the device's reason, such as `INSTALL_FAILED_OLDER_SDK` for an app that needs a newer version of Android. The one you'll meet first is `INSTALL_FAILED_UPDATE_INCOMPATIBLE`: the device has the app signed with another key, such as the Android SDK's debug key when Android Studio installed it, and Android only updates an app with the key it was signed with. Uninstall it from the device, and install again.

An app bundle (AAB) doesn't install: it's for Google Play, which makes the APKs each device installs from it. To try an app on a device, [repackage](./repackaging.md) the APK shell.
