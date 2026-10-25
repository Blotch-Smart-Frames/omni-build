# omni-build

Build, sign, install and ship iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK. One binary, `omni`, repackages a prebuilt app shell with your app's config and assets, then signs it, on Linux, Windows and macOS alike.

```sh
cargo install omni-build --locked   # builds omni from source
cargo binstall omni-build           # or downloads the prebuilt binary from the GitHub release
omni --help
```

OmniBuild is also on npm as [`@blotch/omni-build`](https://www.npmjs.com/package/@blotch/omni-build), on Homebrew as `blotch-smart-frames/tap/omni-build`, and a download per platform from [GitHub releases](https://github.com/Blotch-Smart-Frames/omni-build/releases).

- [Documentation](https://blotch-smart-frames.github.io/omni-build/)
- [Repository](https://github.com/Blotch-Smart-Frames/omni-build), with the plan in [issue #1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1)
