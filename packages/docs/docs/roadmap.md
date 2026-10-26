---
title: Roadmap
description: OmniBuild's phases, the risks the plan names, and the decisions it has made.
---

# Roadmap

The full plan, with its reasoning, is [issue #1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1). Android goes before iOS: every Android format is public, Google doesn't restrict how binaries are built, and its toolchain is the slower one today.

## Phases

| Phase                          | Scope                                                                                                                                                       | Done when                                                                                                                       |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| **0. Spikes**                  | Android: rewrite a prebuilt shell's manifest and resources, sign v2/v3 and as an AAB. iOS: rewrite `Info.plist`, write an `Assets.car` with the icon, sign. | The Android APK passes the end-to-end flows and the AAB is accepted on a Play internal track; TestFlight accepts the iOS build. |
| **1. Android**                 | The CLI contract and npm distribution. APK and AAB writing, signing, ADB install. The shell release pipeline.                                               | A machine without a JDK or Android SDK builds a signed AAB and APK, and installs the APK on a phone.                            |
| **2. iOS**                     | `.app` and `.ipa` writing, `Assets.car`, signing, signing identities through the App Store Connect API, device install.                                     | A Linux CI job produces an `.ipa` that TestFlight accepts.                                                                      |
| **3. Upload**                  | Upload to TestFlight and a Play track.                                                                                                                      | `omni` publishes a build to both stores' test tracks.                                                                           |
| **4. Prebuilt native modules** | The `package.json` manifest, shell-side registration, a build workflow for module authors.                                                                  | An app adds a third-party module with `npm install` and builds without Xcode or Gradle.                                         |

## What's in place

The repository already gates every pull request on what OmniBuild will need:

- **Release builds for all six targets**, each run on a runner of its own OS and CPU.
- **100% test coverage** of the Rust and TypeScript code.
- **End-to-end tests on real platforms.** Real Swift and Kotlin shells are built with Xcode and Gradle, run through the `omni` binary on Linux, Windows and macOS with no `PATH` and no toolchain variables, then launched on the iOS Simulator and an Android emulator, where each app reports its own ID, name, version, payload and icon. `omni install` installs each Android app on the emulator, over ADB, and on Android 14 over wireless debugging's TLS. `omni repackage` gives the Android APK and app bundle shells an app's ID, name, version, build number, icon and payload and signs them, then Google's tools check the results (`apksigner` and `zipalign` for the APK, `bundletool` and `jarsigner` for the bundle, which is installed as the APK bundletool makes from it, as Google Play would). It gives the iOS shells the same values, the icon in its asset catalog (`Assets.car`) among them, and signs them: the Simulator build ad hoc, and the build for devices, an `.ipa`, with a test key and provisioning profile. `codesign` checks both, and the Simulator runs the first.

## Risks

- **`Assets.car` is undocumented.** Readers exist; writers are rare. Rather than write a catalog of its own, OmniBuild replaces the icon's pixels in the one actool compiled into the shell, keeping every key and value actool chose. The iOS Simulator reads the result, as the end-to-end tests check, but App Store validation hasn't seen one yet: the TestFlight upload is the test.
- **Apple changes validation or policy.** Repackaged apps aren't a path Apple documents. The shell really was built with Xcode and re-signing is routine, but nothing guarantees it stays accepted, so a TestFlight upload runs in CI to catch a rejection before users do.
- **Signing identities.** Without Xcode, certificates and provisioning profiles come from developer.apple.com, where OmniBuild's `key request` and `key import` get the certificate by hand, or from the App Store Connect API, with the user's own key, which `omni provision` uses. Its requests follow Apple's published description of the API, but no real account has been provisioned with it yet.
- **Going unmaintained**, as xbuild did. Apple and Google change SDK minimums, privacy manifests, signing rules or device protocols every year. The scope stays at packaging, signing, installing and uploading, and CI runs against the newest platform images.
- **Bugs in the signer** would produce binaries that install but fail store checks. Every output is checked with the vendors' own tools in CI (`apksigner verify`, `bundletool validate`, `codesign --verify`), never on users' machines.

## Decisions

- **Signing identities:** both ways, so users can choose. OmniBuild makes the key and its signing request for developer.apple.com, with no API key (`omni key request`), or gets the certificate and the profiles through the App Store Connect API, with the user's own key, behind a command of its own (`omni provision`), reusing what the account has.
- **Wireless debugging:** `omni tcpip`, which switches a phone plugged in by USB to ADB over the network, and Android 11's wireless debugging: `omni pair` pairs with a code, and installing goes over TLS, through rustls with a pure-Rust crypto provider.
- **Native modules:** prebuilt binaries in npm packages by default, which `omni repackage` adds from the app's dependencies, and a hosted build for native code without them, eventually as a paid service.
- **Dependencies:** fine, as long as none makes OmniBuild depend on a platform. Nothing may need a Mac, or any other OS, to build or run it.
