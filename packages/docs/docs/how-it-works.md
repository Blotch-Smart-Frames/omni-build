---
title: How it works
description: Why an app build doesn't need a compiler, and what OmniBuild rewrites on each platform.
---

# How it works

## The shell is the same for every app

A Lynx app binary is an ordinary native app that links the Lynx engine and carries `main.lynx.bundle` as a data file. In the shells OmniBuild uses, **no per-app value is in the Swift or Kotlin code**. The app ID, name, version, permissions, deep links, orientation and icons all live in `Info.plist`, `AndroidManifest.xml` and resources. The compiled code is byte-identical for every app.

So an app build doesn't need a compiler. It can be:

1. **Take a shell** that OmniBuild's CI compiled with Xcode and Gradle: one per platform, per engine version, per debug or release.
2. **Rewrite its configuration**, and add the app's payload and icons.
3. **Sign it.**

Every step is file-format work and cryptography. Nothing reverse-engineers Xcode or Gradle; OmniBuild replaces only their packaging steps.

## Inputs and outputs

**In:** a shell (a prebuilt `.app` or `.apk`, plus a small manifest saying what may be rewritten), an app config (ID, name, version, icons, permissions, deep links, entitlements), the payload files, and credentials.

**Out:** a signed `.apk`, `.aab`, `.app` or `.ipa`, installed on a device or uploaded to a store.

## Each step, per platform

| Step          | Android                                                                                                            | iOS                                                                                                                                                                                                                 |
| ------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Configuration | `AndroidManifest.xml`, compiled to binary XML (AXML). Rewrite the package, version, permissions and intent filters | `Info.plist`, XML or binary. Keep the `DT*` keys the shell was built with: App Store validation checks them                                                                                                         |
| Name, icons   | `resources.arsc`, and PNGs per density                                                                             | `Assets.car`, which is undocumented. App Store validation requires the icon inside a compiled asset catalog                                                                                                         |
| Payload       | `assets/main.lynx.bundle`, lazy bundles, `static/`                                                                 | The same files inside the `.app`                                                                                                                                                                                    |
| Package       | APK: a zip with 4-byte alignment. AAB: protobuf (aapt2's `Resources.proto` and `Configuration.proto` are public)   | `.app` is a folder; `.ipa` is a zip of `Payload/`                                                                                                                                                                   |
| Sign          | APK Signature Scheme v2/v3; AAB uses JAR signing. OmniBuild can generate a debug key itself                        | Apple's code signature, through [apple-codesign](https://github.com/indygreg/apple-platform-rs): CMS over code-directory hashes, entitlements, an embedded provisioning profile. Simulator builds need no signature |
| Credentials   | Create and read upload keystores (PKCS #12)                                                                        | Certificates, devices, bundle IDs and provisioning profiles through the App Store Connect API (a [decision](./roadmap.md#decisions-needed) is pending)                                                              |
| Install       | The ADB protocol over USB and Wi-Fi, built in                                                                      | usbmux and lockdown to a connected iPhone, or TestFlight                                                                                                                                                            |
| Upload        | The Google Play Developer API                                                                                      | The App Store Connect API, or Transporter                                                                                                                                                                           |

Android's formats are all public in AOSP. On iOS, `Assets.car` is the one real reverse-engineering task; the [roadmap](./roadmap.md#risks) covers the fallback.

## Why Rust, and why a standalone tool

Nothing above is specific to Angular, or even to Lynx: any app whose per-app part is data can be built this way. A standalone tool serves more than one framework, ships its own fixes when Apple or Google change a rule, and keeps its users' code in their own language.

It's written in Rust because it stands alone:

- **One static binary with no runtime.** It runs from npm, `cargo install`, Homebrew, a CI image or a plain download, with nothing else installed.
- **The hardest part exists already.** apple-codesign implements Apple's code signature and signs on Linux and Windows. Mature crates cover zip, cryptography, ASN.1/CMS and protobuf.
- **It's the same model as the tools JavaScript developers already run**: rspack, oxlint and esbuild arrive through npm as prebuilt native binaries.

## Prior art

- [xbuild](https://github.com/rust-mobile/xbuild) built APK, AAB and IPA without Android Studio or Xcode, centred on compiling Rust apps. It's unmaintained.
- [apple-codesign / rcodesign](https://github.com/indygreg/apple-platform-rs) signs and notarizes Apple binaries on Linux and Windows.
- zsign and apktool re-sign and repack existing apps; re-signing is routine in enterprise distribution.
- [Tango (`@yume-chan/adb`)](https://github.com/yume-chan/ya-webadb) implements the ADB protocol without the Android SDK.
- libimobiledevice and pymobiledevice3 talk to iPhones without Xcode.
