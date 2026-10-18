---
title: Introduction
description: What OmniBuild is, who it's for, and where the project stands.
slug: /
---

# Introduction

OmniBuild builds, signs, installs and ships iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK. It's one standalone binary that repackages a prebuilt app shell with your app's config and assets, then signs it.

:::warning[Phase 0]

OmniBuild is in its first phase, the spikes. `omni-build repackage` gives an Android APK or app bundle (AAB) shell an app's ID, name, version, build number, icon and payload, and signs it. The npm package isn't published. These pages describe the design in [the plan](https://github.com/Blotch-Smart-Frames/omni-build/issues/1), and will turn into a manual as each phase lands.

:::

## Why

On the web, a developer installs Node, runs a dev server and opens a browser that's already on their machine. Building means producing files, and deploying means uploading them. Nobody installs a compiler, an SDK or an IDE.

A mobile developer has to install the platforms' toolchains: macOS and Xcode for iOS, a JDK and the Android SDK for Android, even when the app itself is a JavaScript bundle. OmniBuild removes them from building a store binary, so that shipping a mobile app needs Node, as shipping a website does.

## Who it's for

- **Apps whose per-app part is data**: a [Lynx](https://lynxjs.org) bundle, a web app in a WebView shell. Their native code is the same for every app, so it can be compiled once, by us, and reused.
- **Framework builders.** [AngularLynx](https://github.com/Blotch-Smart-Frames/AngularLynx) is the first user: its `@blotch/angular-lynx-build` builder will run OmniBuild for `ng build --platform ios|android`. See [AngularLynx](./angularlynx.md).
- **CI pipelines** on Linux, Windows or macOS runners, with nothing installed but the binary.

## What it won't do

OmniBuild doesn't compile source code. An app with its own Swift, Kotlin or C++ still needs Xcode or Gradle, as a web developer writing a browser extension in Rust still needs Rust. Platform accounts (the Apple Developer Program, the Google Play Console) and store review stay too, as hosting accounts and domains do on the web.

## Next

- [How it works](./how-it-works.md): why no compiler is needed, step by step for each platform.
- [Installation](./installation.md): how the binary will reach you.
- [Roadmap](./roadmap.md): the phases, the risks, and the decisions still open.
