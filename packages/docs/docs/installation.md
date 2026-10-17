---
title: Installation
description: How the omni-build binary will reach you, and the platforms it's built for.
---

# Installation

:::warning[Not published yet]

The commands below are how OmniBuild will be installed once it's released. Today there's nothing to install: follow the [repository](https://github.com/Blotch-Smart-Frames/omni-build) to hear when the first phase lands.

:::

OmniBuild is one native binary, written in Rust, with no runtime and no dependencies. The same binary reaches every audience.

## npm

```sh
npm install --save-dev omni-build
npx omni-build --help
```

The `omni-build` package is a small launcher. npm installs the binary for your OS and CPU from one of the `@omni-build/*` packages, each of which declares its `os` and `cpu`, and `omni-build` runs it. esbuild, rspack and oxlint are distributed the same way.

To use a binary from somewhere else, such as one you built yourself, set `OMNI_BUILD_BINARY` to its path.

## Without Node

- **Cargo:** `cargo install omni-build`
- **Homebrew**
- **GitHub releases:** a plain download per platform

## Platforms

| OS      | CPU                  | npm package                                          | Rust target                                               |
| ------- | -------------------- | ---------------------------------------------------- | --------------------------------------------------------- |
| macOS   | Apple silicon, Intel | `@omni-build/darwin-arm64`, `@omni-build/darwin-x64` | `aarch64-apple-darwin`, `x86_64-apple-darwin`             |
| Linux   | ARM64, x64           | `@omni-build/linux-arm64`, `@omni-build/linux-x64`   | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` |
| Windows | ARM64, x64           | `@omni-build/win32-arm64`, `@omni-build/win32-x64`   | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc`       |

The Linux binaries are linked statically against musl, so one package per CPU runs on glibc and musl distributions alike. CI builds and runs each of the six on a runner of that OS and CPU for every pull request.
