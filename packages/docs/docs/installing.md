---
title: Installing on a device
description: Install an APK on an Android phone, emulator or device on the network with omni install, over ADB, without the Android SDK, pair with a phone for wireless debugging with omni pair, or have one take apps over Wi-Fi with omni tcpip.
---

# Installing on a device

:::warning[Phase 1]

Android phones plugged in by USB, Android emulators, and Android devices on the network, with Android 11's wireless debugging or listening for ADB. OmniBuild doesn't find phones with wireless debugging on the network by itself yet: name them by the address they show. On iOS, install through TestFlight for now. Install OmniBuild from npm, as [Installation](./installation.md) shows.

:::

```sh
omni install app.apk                              # on the one phone plugged in, or emulator running
omni install app.apk --device R5CT1234      # on one of several phones, by its serial number
omni install app.apk --device emulator-5556       # on one of several emulators
omni install app.apk --device 192.168.1.20:5555   # on a device on the network
omni install app.apk --device 192.168.1.20:41235  # on a phone with wireless debugging, once paired
```

`install` sends an APK to a device over ADB, the protocol the Android SDK's `adb` speaks to devices, and has the device's package manager install it, as `adb install` does. Nothing else is installed or run: no Android SDK, no `adb`. From Android 7.0 on, the APK goes straight to the package manager. Earlier versions can't take it that way, so it's copied to `/data/local/tmp` first, installed from there and deleted, as `adb` does with them. An app with the same ID is updated, keeping its data, when it's signed with the same key.

## Finding the device

Without `--device`, OmniBuild looks for phones plugged into this computer with USB debugging on, and for emulators running on it where `adb` does, on the ports emulators listen on (5555, 5557, and so on to 5585), and installs on the one it finds. When it finds several, it lists them, and `--device` names one as `adb` does. A phone is named by its serial number, as `adb devices` lists it, or by where it's plugged in, such as `usb:001-3`, if it has none. `emulator-5554` is the emulator whose console is on port 5554, the number in its window's title, and which listens for ADB on the next port, 5555.

A device on the network is named by its address and the port it listens on: the one a phone with [wireless debugging](#wireless-debugging) shows, or 5555 once [`omni tcpip`](#without-the-cable), or `adb tcpip 5555`, has told it to listen.

## Phones plugged in by USB

A phone takes apps over USB once USB debugging is on: tap **Build number** in **Settings › About phone** seven times to show **Developer options**, then turn on **USB debugging** there. OmniBuild talks to the phone through the system's own USB support, with nothing from the Android SDK. On macOS that's all. Elsewhere:

- **Linux** lets only root open a USB device until a udev rule gives it to its user. Android's rules do, for every phone maker's devices: install them with `android-sdk-platform-tools-common` on Debian and Ubuntu, or `android-udev` on Arch.
- **Windows** lets OmniBuild open a phone through WinUSB, so the phone's ADB interface needs a WinUSB driver, such as Google's USB driver. Windows installs WinUSB itself for a phone that asks for it. Some phone makers' own drivers aren't WinUSB, and OmniBuild says so when it can't open the phone.

One program at a time can use a phone. `adb`'s server keeps every phone it finds, so if you've run `adb`, stop it with `adb kill-server` first.

## Wireless debugging

Android 11 and later take apps over Wi-Fi with no cable at all, encrypted, from a computer they're paired with. On the phone, turn on **Wireless debugging** in **Developer options**, tap **Pair device with pairing code**, and pair with the address and port, and the code, that it shows. Then install at the IP address and port the **Wireless debugging** screen itself shows, which is another port:

```sh
omni pair 192.168.1.20:37099 482913                # where the phone listens for pairing, and its code
omni install app.apk --device 192.168.1.20:41235  # where Wireless debugging listens
```

`pair` does what `adb pair` does. OmniBuild and the phone each prove they know the code, without sending it, over TLS, and OmniBuild gives the phone its [ADB key](#the-adb-key): the phone lists this computer under **Paired devices**, by the user and computer the key belongs to, and trusts the key from then on. The phone shows a new code, on a new port, each time it starts pairing.

`install` then connects as it does to any device on the network, and the phone asks for TLS 1.3, in which OmniBuild proves it holds the key, so everything sent is encrypted. A phone that doesn't know the key refuses, and OmniBuild says to pair first. The port Wireless debugging listens on changes each time it's turned on, and `adb` finds it over the network by itself, which OmniBuild doesn't yet: read it from the phone's screen. The phone and this computer need to be on the same network.

## Without the cable

A phone plugged in can also be told to listen for ADB on the network, as phones before Android 11 can, and then takes apps over Wi-Fi, unplugged, until it restarts:

```sh
omni tcpip                                   # the one phone plugged in, on port 5555
omni tcpip --device R5CT1234 --port 5556     # one of several phones, on another port
omni install app.apk --device 192.168.1.20:5555
```

`tcpip` does what `adb tcpip 5555` does, and finds the address you'd otherwise look up in the phone's Wi-Fi settings. Over USB, it reads the phone's address on Wi-Fi, or on Ethernet if it has no Wi-Fi, and has the phone's ADB restart listening on the port. Then it connects to the phone there, to check that this computer reaches it, and says where it listens: unplug the phone, and name it with `--device` and that address. A device on the network already moves to another port the same way, named by its address.

The phone and this computer have to be on the same network, and one that lets its devices reach each other, which many guest and office networks don't: OmniBuild says so when it can't reach the phone. Nothing on the network is encrypted, as with `adb tcpip`, so use it on a network you trust, or [wireless debugging](#wireless-debugging) on Android 11 and later. A phone that restarts stops listening: plug it in and run `tcpip` again. Emulators are on the network already, as `emulator-5554`.

## The ADB key

A device lets a computer install apps only once it trusts the computer's key. OmniBuild uses `adb`'s own, `~/.android/adbkey`, so a device that trusts your `adb` trusts OmniBuild, and the other way round. Without one, OmniBuild creates it there, as `adb` would, with its public key beside it in `adbkey.pub`. The first time, a device asks whether to allow the computer: allow it, and OmniBuild carries on. Emulators don't ask. Pairing for wireless debugging gives a phone the same key, so a phone paired with `adb` takes apps from OmniBuild too.

## When an install fails

OmniBuild gives the device's reason, such as `INSTALL_FAILED_OLDER_SDK` for an app that needs a newer version of Android. The one you'll meet first is `INSTALL_FAILED_UPDATE_INCOMPATIBLE`: the device has the app signed with another key, such as the Android SDK's debug key when Android Studio installed it, and Android only updates an app with the key it was signed with. Uninstall it from the device, and install again.

An app bundle (AAB) doesn't install: it's for Google Play, which makes the APKs each device installs from it. To try an app on a device, [repackage](./repackaging.md) the APK shell.
