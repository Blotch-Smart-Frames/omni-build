---
title: Installation
description: How to install the omni binary, and the platforms it's built for.
---

# Installation

:::warning[From the release after 0.0.1]

0.0.1 is on npm alone. Cargo, Homebrew and the GitHub releases start with the next release. Until then, a machine without Node builds the binary from the [repository](https://github.com/Blotch-Smart-Frames/omni-build) with `cargo build --release`.

:::

OmniBuild is one native binary, written in Rust, with no runtime and no dependencies. Every channel below installs the same binary, which a release publishes to all of them at once, from one version tag.

## npm

```sh
npm install --save-dev @blotch/omni-build
npx omni --help
```

The `@blotch/omni-build` package is a small launcher. npm installs the binary for your OS and CPU from one of the `@blotch/omni-build-*` packages, each of which declares its `os` and `cpu`, and `omni` runs it. esbuild, rspack and oxlint are distributed the same way. For every pull request, CI packs these packages from that pull request's builds and installs them on each of the platforms below, checking that npm picks the machine's own binary and that it runs.

To use a binary from somewhere else, such as one you built yourself, set `OMNI_BUILD_BINARY` to its path.

## Homebrew

On macOS and Linux:

```sh
brew install blotch-smart-frames/tap/omni-build
```

The formula, in [Blotch's tap](https://github.com/Blotch-Smart-Frames/homebrew-tap), installs the binary from the GitHub release, pinned by its SHA-256. CI installs it with Homebrew on macOS and Linux for every pull request, and audits it as Homebrew audits its own formulae.

## Cargo

```sh
cargo install omni-build --locked
```

Cargo builds `omni` from the crate's source, which takes a few minutes and needs [Rust](https://rustup.rs). [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) downloads the binary from the GitHub release instead:

```sh
cargo binstall omni-build
```

## GitHub releases

Each [release](https://github.com/Blotch-Smart-Frames/omni-build/releases) has an archive per platform: `omni-build-<Rust target>.tar.gz`, or `.zip` for Windows, holding a folder of the same name with `omni` and the licence. The latest release's are always at the same address, so a script or a CI image can download one by its name:

```sh
curl -fsSL https://github.com/Blotch-Smart-Frames/omni-build/releases/latest/download/omni-build-x86_64-unknown-linux-musl.tar.gz | tar -xz
./omni-build-x86_64-unknown-linux-musl/omni --help
```

`SHA256SUMS`, beside them, has each archive's SHA-256: `sha256sum --check --ignore-missing SHA256SUMS` checks a download. Each archive also has an attestation of the release workflow run that published it, which the GitHub CLI checks:

```sh
gh attestation verify omni-build-x86_64-unknown-linux-musl.tar.gz --repo Blotch-Smart-Frames/omni-build
```

## Platforms

| OS      | CPU                  | npm package                                                        | Rust target                                               |
| ------- | -------------------- | ------------------------------------------------------------------ | --------------------------------------------------------- |
| macOS   | Apple silicon, Intel | `@blotch/omni-build-darwin-arm64`, `@blotch/omni-build-darwin-x64` | `aarch64-apple-darwin`, `x86_64-apple-darwin`             |
| Linux   | ARM64, x64           | `@blotch/omni-build-linux-arm64`, `@blotch/omni-build-linux-x64`   | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` |
| Windows | ARM64, x64           | `@blotch/omni-build-win32-arm64`, `@blotch/omni-build-win32-x64`   | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc`       |

The Linux binaries are linked statically against musl, so one package and one archive per CPU run on glibc and musl distributions alike. CI builds and runs each of the six on a runner of that OS and CPU for every pull request, and installs it there from its npm package and from its archive.
