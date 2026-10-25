---
title: Publishing a shell
description: How a framework builds its app shell with Xcode and Gradle in CI, checks that omni repackages each build, and publishes them with their SHA-256s, with OmniBuild's Shell workflow.
---

# Publishing a shell

:::warning[From the release after 0.0.1]

The Shell workflow is new since 0.0.1, so call it at a release's tag once the next one is out. OmniBuild's CI builds its own shells with it already.

:::

A shell is the native app OmniBuild makes apps from: one build, which keeps nothing about an app in its Swift or Kotlin code, so `omni repackage` makes any number of apps from it with no compiler. A framework builds its shell once, in its own CI, and its users download it, as they'd download any other prebuilt binary. Only that CI ever needs Xcode or Gradle.

OmniBuild's Shell workflow does the building. It's a reusable GitHub Actions workflow, `.github/workflows/shell.yml`, which builds every variant of a shell, checks that omni repackages each one, and leaves them in an artifact with a manifest of their SHA-256s, for a release to publish. OmniBuild publishes its own [WebView shell](./webview-shell.md) this way.

## The workflow

Call it from a job of your own, at a release's tag, with your shell's Xcode and Gradle projects:

```yaml
jobs:
  shell:
    uses: Blotch-Smart-Frames/omni-build/.github/workflows/shell.yml@v1.2.3
    with:
      name: MyShell
      xcode-project: ios/MyShell.xcodeproj
      gradle-project: android
```

| Input            | What it is                                                                                                                                       |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `name`           | The shell's name: its Xcode scheme and the app the scheme builds, and the start of its files' names. Letters, digits, `.`, `_` and `-`. Required |
| `xcode-project`  | The Xcode project (`.xcodeproj`). Without it, no iOS variants                                                                                    |
| `gradle-project` | The Gradle project's folder, with its wrapper. Without it, no Android variants                                                                   |
| `developer-dir`  | The Xcode to build with, as `DEVELOPER_DIR` takes one: `/Applications/Xcode_26.6.app/Contents/Developer`. The runner's default without it        |
| `macos-runner`   | The runner Xcode builds on: `macos-latest` without it                                                                                            |
| `java-version`   | The JDK Gradle runs on, from Temurin: 21 without it                                                                                              |
| `retention-days` | How long the artifact is kept: the repository's default without it                                                                               |

Xcode builds on a macOS runner and Gradle on a Linux one, side by side, and a third job checks every variant on Linux, with the omni of the workflow's version, installed from npm. The workflow's `artifact` output names the artifact with the lot: `shell-MyShell`.

## The variants

Each comes in `debug` and `release`, as Xcode's Debug and Release configurations and Gradle's debug and release build types build them:

| File                                   | What it is                                                       |
| -------------------------------------- | ---------------------------------------------------------------- |
| `<name>-<configuration>.apk`           | An Android APK                                                   |
| `<name>-<configuration>.aab`           | An Android app bundle, for Google Play                           |
| `<name>-<configuration>-simulator.zip` | An iOS app for the Simulator, a zipped `.app`                    |
| `<name>-<configuration>.ipa`           | An iOS app for devices, unsigned, with room for omni's signature |
| `<name>.json`                          | The manifest                                                     |

A debug build is the one to inspect an app with, and to install on your own devices. A release build is the one that goes to the stores.

## The manifest

`<name>.json` lists every variant, with what it is and its SHA-256, which a build tool pins it by when it downloads it, and the version of omni that repackaged them all in the check:

```json
{
  "name": "MyShell",
  "omni": "1.2.3",
  "variants": [
    {
      "file": "MyShell-release.apk",
      "platform": "android",
      "format": "apk",
      "configuration": "release",
      "sha256": "6f98d361e55de6beaadfad0f6738b8bccfe6e55b0d1666d041750dafe99f7a61",
      "size": 902927
    }
  ]
}
```

`platform` is `android` or `ios`, and `format` what omni reports a shell as in its [`repackaged` event](./build-tools.md): `apk`, `aab`, `ipa`, or `app` for a zipped `.app`.

## Publishing them

The workflow leaves publishing to you. A job after it can attach the artifact to a GitHub release, for one:

```yaml
release:
  needs: shell
  runs-on: ubuntu-latest
  permissions:
    contents: write
  steps:
    - uses: actions/download-artifact@v8
      with:
        name: ${{ needs.shell.outputs.artifact }}
        path: shell
    - run: gh release upload "$GITHUB_REF_NAME" shell/* --repo "$GITHUB_REPOSITORY"
      env:
        GH_TOKEN: ${{ github.token }}
```

Your build tool then ships the manifest, or the SHA-256s of the variants it uses, and downloads a variant the first time it needs it: from the release, checked against its SHA-256, and kept in a cache by that SHA-256, so it's downloaded once per version on each machine. Then it runs `omni repackage` on it, with the app's config.

## What a shell has to be

A project as Xcode's and Android Studio's templates make one:

- **iOS:** a scheme that builds an app of the same name, as the iOS App template names both after the project. Its extensions, App Clip and watch app come with it, as Xcode embeds them.
- **Android:** the app in the module `app`. The release build may be signed with any key, or none: omni signs every app it makes with that app's key.
- **Nothing about an app in the code**: no ID, name, version or icon that the app should have instead. omni rewrites them all from the app's [config](./repackaging.md#the-app-config).

The workflow builds the debug variants whole (`ENABLE_DEBUG_DYLIB=NO`): Xcode otherwise moves a debug build's code into a dylib for its Previews, behind an executable omni can't sign. And it builds the variants for devices unsigned, with room for the signature omni writes.
