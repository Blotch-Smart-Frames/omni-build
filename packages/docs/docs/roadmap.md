---
title: Roadmap
description: OmniBuild's phases, the risks the plan names, and the decisions still open.
---

# Roadmap

The full plan, with its reasoning, is [issue #1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1). Android goes before iOS: every Android format is public, Google doesn't restrict how binaries are built, and its toolchain is the slower one today.

## Phases

| Phase                          | Scope                                                                                                                                                       | Done when                                                                                                                       |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| **0. Spikes**                  | Android: rewrite a prebuilt shell's manifest and resources, sign v2/v3 and as an AAB. iOS: rewrite `Info.plist`, write an `Assets.car` with the icon, sign. | The Android APK passes the end-to-end flows and the AAB is accepted on a Play internal track; TestFlight accepts the iOS build. |
| **1. Android**                 | The CLI contract and npm distribution. APK and AAB writing, signing, ADB install. The shell release pipeline.                                               | A machine without a JDK or Android SDK builds a signed AAB and APK, and installs the APK on a phone.                            |
| **2. iOS**                     | `.app` and `.ipa` writing, `Assets.car`, signing, signing identities through the App Store Connect API, device install.                                     | A Linux CI job produces an `.ipa` that TestFlight accepts.                                                                      |
| **3. Upload**                  | Upload to TestFlight and a Play track.                                                                                                                      | `omni-build` publishes a build to both stores' test tracks.                                                                     |
| **4. Prebuilt native modules** | The `package.json` manifest, shell-side registration, a build workflow for module authors.                                                                  | An app adds a third-party module with `npm install` and builds without Xcode or Gradle.                                         |

## What's in place

The repository already gates every pull request on what OmniBuild will need:

- **Release builds for all six targets**, each run on a runner of its own OS and CPU.
- **100% test coverage** of the Rust and TypeScript code.
- **End-to-end tests on real platforms.** Real Swift and Kotlin shells are built with Xcode and Gradle, run through the `omni-build` binary on Linux, Windows and macOS with no `PATH` and no toolchain variables, then launched on the iOS Simulator and an Android emulator, where each app reports its own ID, name, version, payload and icon. `omni-build install` installs each Android app on the emulator, over ADB. `omni-build repackage` gives the Android APK and app bundle shells an app's ID, name, version, build number, icon and payload and signs them, then Google's tools check the results (`apksigner` and `zipalign` for the APK, `bundletool` and `jarsigner` for the bundle, which is installed as the APK bundletool makes from it, as Google Play would). It gives the iOS shells the same values, the icon in its asset catalog (`Assets.car`) among them, and signs them: the Simulator build ad hoc, and the build for devices, an `.ipa`, with a test key and provisioning profile. `codesign` checks both, and the Simulator runs the first.

## Risks

- **`Assets.car` is undocumented.** Readers exist; writers are rare. Rather than write a catalog of its own, OmniBuild replaces the icon's pixels in the one actool compiled into the shell, keeping every key and value actool chose. The iOS Simulator reads the result, as the end-to-end tests check, but App Store validation hasn't seen one yet: the TestFlight upload is the test.
- **Apple changes validation or policy.** Repackaged apps aren't a path Apple documents. The shell really was built with Xcode and re-signing is routine, but nothing guarantees it stays accepted, so a TestFlight upload runs in CI to catch a rejection before users do.
- **Signing identities.** Without Xcode, certificates and provisioning profiles come from developer.apple.com, where OmniBuild's `key request` and `key import` get the certificate by hand, or from the App Store Connect API, with the user's own key.
- **Going unmaintained**, as xbuild did. Apple and Google change SDK minimums, privacy manifests, signing rules or device protocols every year. The scope stays at packaging, signing, installing and uploading, and CI runs against the newest platform images.
- **Bugs in the signer** would produce binaries that install but fail store checks. Every output is checked with the vendors' own tools in CI (`apksigner verify`, `bundletool validate`, `codesign --verify`), never on users' machines.

## Decisions needed

- **Signing identities:** whether OmniBuild may create Apple certificates and profiles with the user's API key.
