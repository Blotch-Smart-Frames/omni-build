---
title: Introduction
description: What OmniBuild is, who it's for, and where the project stands.
slug: /
---

# Introduction

OmniBuild builds, signs, installs and ships iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK. It's one standalone binary that repackages a prebuilt app shell with your app's config and assets, then signs it.

:::warning[Phases 0 and 1]

OmniBuild is in its first phases. `omni-build repackage` gives an Android APK or app bundle (AAB) shell, or an iOS shell built for the Simulator or for devices, an app's ID, name, version, build number, icon, payload and permissions, and signs it, frameworks included. `omni-build key create` makes an Android upload key, `omni-build key request` and `key import` an Apple signing key without a Mac, and `omni-build install` installs an APK on an Android phone plugged in by USB, an emulator or a device on the network. Build tools read what it reports as JSON lines. The npm package isn't published. These pages describe the design in [the plan](https://github.com/Blotch-Smart-Frames/omni-build/issues/1), and will turn into a manual as each phase lands.

:::

## Why

On the web, a developer installs Node, runs a dev server and opens a browser that's already on their machine. Building means producing files, and deploying means uploading them. Nobody installs a compiler, an SDK or an IDE.

A mobile developer has to install the platforms' toolchains: macOS and Xcode for iOS, a JDK and the Android SDK for Android, even when the app itself is a JavaScript bundle. OmniBuild removes them from building a store binary, so that shipping a mobile app needs Node, as shipping a website does.

## Who it's for

- **Apps whose per-app part is data**: a [Lynx](https://lynxjs.org) bundle, a web app in a WebView shell. Their native code is the same for every app, so it can be compiled once, by us, and reused.
- **Framework builders**, whose build command can run OmniBuild for their users' apps when there's nothing to compile. See [How it works](./how-it-works.md#from-a-frameworks-build), and [Running from a build tool](./build-tools.md).
- **CI pipelines** on Linux, Windows or macOS runners, with nothing installed but the binary.

## What it won't do

OmniBuild doesn't compile source code. An app with its own Swift, Kotlin or C++ still needs Xcode or Gradle, as a web developer writing a browser extension in Rust still needs Rust. Platform accounts (the Apple Developer Program, the Google Play Console) and store review stay too, as hosting accounts and domains do on the web.

## Next

- [How it works](./how-it-works.md): why no compiler is needed, step by step for each platform.
- [Installation](./installation.md): how the binary will reach you.
- [Roadmap](./roadmap.md): the phases, the risks, and the decisions still open.
