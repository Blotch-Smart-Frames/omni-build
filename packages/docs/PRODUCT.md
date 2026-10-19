# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Docusaurus 3 (classic preset) in `packages/docs`, an npm workspace of the OmniBuild monorepo. TypeScript and TSX, as the repository's other npm packages are. Landing-page motion comes from React Bits (https://reactbits.dev), copied into the package as source, which is how React Bits is distributed. Hosted on GitHub Pages at `blotch-smart-frames.github.io/omni-build`, deployed from `master` by a workflow.

## Users

- **Web developers who want to ship a mobile app** whose per-app part is data: a Lynx bundle, a web app in a WebView shell. They work on Linux, Windows or macOS, know Node and npm, and don't want to install Xcode, a JDK or the Android SDK to produce a store binary.
- **Framework and tooling authors** who would call OmniBuild from their own build commands and CI.
- **Contributors** who want to understand the plan, the file formats and the risks before helping.

## Product Purpose

OmniBuild builds, signs, installs and ships iOS and Android apps with no native toolchain. A developer with only Node (or only the binary) can turn their app's config and assets into a signed `.apk`, `.aab`, `.app` or `.ipa`. Success is a machine with no JDK, Android SDK or Xcode producing a store-accepted binary.

## Positioning

A general app packager, not a Lynx-only tool. It works for any app whose per-app part is data (a Lynx bundle, a web app in a WebView shell). It's framework-agnostic: the frameworks that build with it are its users, and the site names none of them.

Its mechanism: it doesn't compile anything. It takes a shell that OmniBuild's own CI compiled with Xcode and Gradle, rewrites the shell's configuration (`Info.plist`, binary `AndroidManifest.xml`, `resources.arsc`, `Assets.car`), adds the app's payload and icons, then signs it (Apple code signature, APK Signature Scheme v2/v3). Every step is file-format work and cryptography. It ships as one static Rust binary through npm (one package per OS and CPU, as rspack and oxlint do), `cargo install`, Homebrew and GitHub releases.

## Operating Context

- Developers install it from npm and run it from a framework builder (`ng build --platform ios|android`) or directly.
- CI pipelines on Linux, Windows and macOS runners.
- Platform accounts (Apple Developer Program, Google Play Console) and store review remain, as hosting accounts do on the web.
- The prior art readers will compare it with: xbuild (unmaintained), apple-codesign/rcodesign, zsign, apktool, Tango (`@yume-chan/adb`), libimobiledevice, Expo Go.

## Capabilities and Constraints

- **Status: Phase 0, the spikes.** `omni-build repackage` gives an Android APK or app bundle (AAB) shell an app's ID, name, version, build number, icon and payload, and signs it, and gives an iOS shell built for the Simulator the same values but the icon, unsigned; nothing else is built. The plan is issue #1 (https://github.com/Blotch-Smart-Frames/omni-build/issues/1). The npm package is not published.
- What exists today: the repository, the npm launcher, CI gating 100% coverage and release builds on six targets, and an end-to-end pipeline that builds real Swift and Kotlin shells, runs them through the binary on Linux, Windows and macOS with no `PATH` or toolchain variables, and launches them on the iOS Simulator and an Android emulator. The binary repackages both shells; the iOS one doesn't get the icon or a signature yet.
- Planned phases: 0 spikes, 1 Android, 2 iOS, 3 upload, 4 prebuilt native modules. Android goes first.
- Out of scope: compiling source code (Swift, Kotlin, C++). Custom native code still means Xcode or Gradle.
- Release targets: aarch64 and x86_64 for macOS, Linux (musl, static) and Windows.

## Brand Commitments

- Name: **OmniBuild** in prose, `omni-build` for the binary and npm package, `@omni-build/*` for scoped packages.
- Voice: plain, specific and honest about status, like the README and the plan. No hype and no claims the plan doesn't back (speed numbers, store acceptance, features not built).
- Nothing may look like it belongs to Apple or Google: no platform logos, no imitation of their sites or product art. The site's one-bit desktop world (chosen by the user) keeps the medium's generic grammar and drops everything that is Apple's own: no Apple logo, no Chicago or Geneva typefaces, no Finder, Trash or happy-computer icons.
- The landing page's main action is to follow the project on GitHub; the second is to read the plan. An install command may appear only labelled as not published yet.

## Evidence on Hand

- `README.md`, issue #1 (the plan: goal, what users install today and after, formats per platform, distribution, prior art, risks, phases).
- `packages/e2e/README.md`: the three-stage end-to-end pipeline and the values each fixture reports.
- `.github/workflows/ci.yml`: the six release targets and the CI gate.
- No users, benchmarks, testimonials, store acceptances or download numbers exist. Never invent them.

## Product Principles

1. Honest about status: every claim on the site is something the repository or the plan says.
2. Data, not compilers: explain the mechanism (repackage, rewrite, sign) rather than promise outcomes.
3. The web's ease is the yardstick: Node and a phone, as Node and a browser.
4. General first: Lynx and WebView apps are examples, never the boundary, and no framework is named.
