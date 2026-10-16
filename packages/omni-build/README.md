# omni-build

Build, sign, install and ship iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK.

**Status:** planning, and not published yet. See the plan in [#1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1).

```sh
npm install --save-dev omni-build
npx omni-build --help
```

OmniBuild is a native binary written in Rust. This package is a small launcher: npm installs the binary for your OS and CPU from one of the `@omni-build/*` packages, and `omni-build` runs it.

To use a binary from somewhere else, for a platform without a prebuilt binary or one you built with `cargo install omni-build`, set `OMNI_BUILD_BINARY` to its path.
