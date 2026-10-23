---
title: Installation
description: How to install the omni binary, and the platforms it's built for.
---

# Installation

:::warning[npm only, for now]

OmniBuild is on npm. Cargo, Homebrew and GitHub releases come later: until then, a machine without Node builds the binary from the [repository](https://github.com/Blotch-Smart-Frames/omni-build) with `cargo build --release`.

:::

OmniBuild is one native binary, written in Rust, with no runtime and no dependencies. The same binary reaches every audience.

## npm

```sh
npm install --save-dev @blotch/omni-build
npx omni --help
```

The `@blotch/omni-build` package is a small launcher. npm installs the binary for your OS and CPU from one of the `@blotch/omni-build-*` packages, each of which declares its `os` and `cpu`, and `omni` runs it. esbuild, rspack and oxlint are distributed the same way. For every pull request, CI packs these packages from that pull request's builds and installs them on each of the platforms below, checking that npm picks the machine's own binary and that it runs.

To use a binary from somewhere else, such as one you built yourself, set `OMNI_BUILD_BINARY` to its path.

## Without Node

Still to come:

- **Cargo:** `cargo install omni-build`
- **Homebrew**
- **GitHub releases:** a plain download per platform

## Platforms

| OS      | CPU                  | npm package                                                        | Rust target                                               |
| ------- | -------------------- | ------------------------------------------------------------------ | --------------------------------------------------------- |
| macOS   | Apple silicon, Intel | `@blotch/omni-build-darwin-arm64`, `@blotch/omni-build-darwin-x64` | `aarch64-apple-darwin`, `x86_64-apple-darwin`             |
| Linux   | ARM64, x64           | `@blotch/omni-build-linux-arm64`, `@blotch/omni-build-linux-x64`   | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` |
| Windows | ARM64, x64           | `@blotch/omni-build-win32-arm64`, `@blotch/omni-build-win32-x64`   | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc`       |

The Linux binaries are linked statically against musl, so one package per CPU runs on glibc and musl distributions alike. CI builds and runs each of the six on a runner of that OS and CPU for every pull request.
