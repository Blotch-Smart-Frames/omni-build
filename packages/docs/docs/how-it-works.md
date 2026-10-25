---
title: How it works
description: Why an app build doesn't need a compiler, and what OmniBuild rewrites on each platform.
---

# How it works

## The shell is the same for every app

An app whose per-app part is data, such as a [Lynx](https://lynxjs.org) bundle or a web app in a WebView, is an ordinary native app: a shell that links its runtime and carries the app's code as data files. In the shells OmniBuild uses, **no per-app value is in the Swift or Kotlin code**. The app ID, name, version, permissions, deep links, orientation and icons all live in `Info.plist`, `AndroidManifest.xml` and resources. The compiled code is byte-identical for every app.

So an app build doesn't need a compiler. It can be:

1. **Take a shell** compiled once in CI with Xcode and Gradle: one per platform, per runtime version, per debug or release.
2. **Rewrite its configuration**, and add the app's payload and icons.
3. **Sign it.**

Every step is file-format work and cryptography. Nothing reverse-engineers Xcode or Gradle; OmniBuild replaces only their packaging steps.

## Inputs and outputs

**In:** a shell (a prebuilt `.app` or `.apk`, plus a small manifest saying what may be rewritten), an app config (ID, name, version, icons, permissions, deep links, entitlements), the payload files, and credentials.

**Out:** a signed `.apk`, `.aab`, `.app` or `.ipa`, installed on a device or uploaded to a store.

## Each step, per platform

| Step          | Android                                                                                                                                      | iOS                                                                                                                                                                                                                        |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Configuration | `AndroidManifest.xml`, compiled to binary XML (AXML). Rewrite the package, version, permissions and intent filters                           | `Info.plist`, XML or binary. Keep the `DT*` keys the shell was built with: App Store validation checks them                                                                                                                |
| Name, icons   | `resources.arsc`, and PNGs per density                                                                                                       | `Assets.car`, which is undocumented. App Store validation requires the icon inside a compiled asset catalog, so OmniBuild replaces the icon's pixels in the shell's                                                        |
| Payload       | `assets/main.lynx.bundle`, lazy bundles, `static/`                                                                                           | The same files inside the `.app`                                                                                                                                                                                           |
| Package       | APK: a zip with 4-byte alignment. AAB: protobuf (aapt2's `Resources.proto` and `Configuration.proto` are public)                             | `.app` is a folder; `.ipa` is a zip of `Payload/`                                                                                                                                                                          |
| Sign          | APK Signature Scheme v2/v3, and v1 (JAR signing) for Android before 7.0; AAB uses JAR signing. OmniBuild can generate a debug key itself     | Apple's code signature, which OmniBuild writes as codesign does: CMS over code-directory hashes, entitlements, an embedded provisioning profile. Simulator builds are signed ad hoc                                        |
| Credentials   | Create and read upload keystores (PKCS #12), as keytool does                                                                                 | A key and its signing request for developer.apple.com, or certificates, App IDs, capabilities and provisioning profiles through the App Store Connect API, with `omni provision` (see [decisions](./roadmap.md#decisions)) |
| Install       | The ADB protocol, built in: over USB to phones, TCP to emulators and devices on the network, and TLS to phones paired for wireless debugging | usbmux and lockdown to a connected iPhone, or TestFlight                                                                                                                                                                   |
| Upload        | The Google Play Developer API                                                                                                                | The App Store Connect API, or Transporter                                                                                                                                                                                  |

Android's formats are all public in AOSP. On iOS, `Assets.car` is the one real reverse-engineering task. OmniBuild doesn't write a catalog of its own: actool compiled the shell's with the icon in every size iOS needs, so OmniBuild replaces only the pixels of each, and every key and value actool chose stays. The [roadmap](./roadmap.md#risks) covers what's still unproven.

## Why Rust, and why a standalone tool

Nothing above is specific to one framework, or even to Lynx: any app whose per-app part is data can be built this way. A standalone tool serves more than one framework, ships its own fixes when Apple or Google change a rule, and keeps its users' code in their own language.

It's written in Rust because it stands alone:

- **One static binary with no runtime.** It runs from npm, `cargo install`, Homebrew, a CI image or a plain download, with nothing else installed.
- **The formats are within reach.** Mature crates cover cryptography, ASN.1/CMS and compression, and OmniBuild writes the rest, Apple's code signature among them, checking each against the vendors' own tools. apple-codesign showed Apple's signature can be written on Linux and Windows.
- **It's the same model as the tools JavaScript developers already run**: rspack, oxlint and esbuild arrive through npm as prebuilt native binaries.

## From a framework's build

A framework's build command runs OmniBuild when there's nothing to compile, and keeps its own Xcode and Gradle path for the rest:

```text
build for android
   │
   ├─ native source code (an ejected app, or modules that need compiling)? → Xcode or Gradle
   └─ otherwise → download the prebuilt shell (cached by checksum)
                → OmniBuild: rewrite config, add the app's files, sign
                → *.apk, *.aab
```

The framework maps its own options to OmniBuild's [app config](./repackaging.md#the-app-config), and runs the binary from the `@blotch/omni-build` npm package, reading what it reports as JSON lines (see [Running from a build tool](./build-tools.md)). Shells are downloaded rather than bundled in an npm package: a release asset pinned by SHA-256 and cached per user. There are release and debug variants per platform, iOS device and Simulator shells, and one universal Android shell from which only the requested ABIs are kept.

## Native modules from npm

A prebuilt shell can't compile a module's source, so the plan is for module packages to ship binaries built against the shell's runtime version, and declare them in `package.json`:

```jsonc
"omniBuild": {
  "runtime": "lynx@4.1",
  "ios": { "frameworks": ["ios/Camera.xcframework"], "modules": ["CameraModule"] },
  "android": { "dex": "android/classes.dex", "jniLibs": "android/jni", "modules": ["dev.example.CameraModule"], "manifest": "android/AndroidManifest.xml" }
}
```

OmniBuild collects these from the app's dependencies, embeds the binaries and writes the module list into the app's configuration. The shell registers each module by name at startup, so nothing is generated and nothing is compiled. This is [Phase 4](./roadmap.md#phases).

## Prior art

- [xbuild](https://github.com/rust-mobile/xbuild) built APK, AAB and IPA without Android Studio or Xcode, centred on compiling Rust apps. It's unmaintained.
- [apple-codesign / rcodesign](https://github.com/indygreg/apple-platform-rs) signs and notarizes Apple binaries on Linux and Windows.
- zsign and apktool re-sign and repack existing apps; re-signing is routine in enterprise distribution.
- [Tango (`@yume-chan/adb`)](https://github.com/yume-chan/ya-webadb) implements the ADB protocol without the Android SDK.
- libimobiledevice and pymobiledevice3 talk to iPhones without Xcode.
