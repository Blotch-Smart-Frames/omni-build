---
title: Contributing
description: Building, testing and linting OmniBuild's monorepo.
---

# Contributing

OmniBuild is an npm workspaces and [Turborepo](https://turborepo.com) monorepo with the Rust inside it.

| Package                   | What it is                                                                                                                   |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `packages/cli`            | The `omni-build` binary, a Rust crate                                                                                        |
| `packages/omni-build`     | The `omni-build` npm package: a launcher that runs the binary built for the user's OS and CPU                                |
| `packages/e2e`            | End-to-end tests: real Swift and Kotlin app shells, repackaged on every OS, run on the iOS Simulator and an Android emulator |
| `packages/platform-watch` | Opens an issue when the app shells need a newer Xcode or Android API level, or can't read a vendor's page. Runs daily in CI  |
| `packages/docs`           | This site                                                                                                                    |

## Development

You need Node 24 and [rustup](https://rustup.rs), which installs the Rust version pinned in `rust-toolchain.toml` the first time Cargo runs.

```sh
npm install
npm run build          # cargo build --release, and this site
npm test               # cargo test, and the launcher's tests with vitest
npm run test:coverage  # the same, failing below 100% coverage (needs cargo-llvm-cov)
npm run lint           # clippy and oxlint
npm run format         # rustfmt and oxfmt; CI runs format:check
npm run e2e -- --platform ios  # build, repackage and run a real app on the iOS Simulator, or android
```

To work on this site:

```sh
npm start -w @omni-build/docs
```

Pull requests must pass CI: formatting, lint (warnings are errors), 100% test coverage, tests on Linux, macOS and Windows, a release build on each of the six targets, the end-to-end tests on the iOS Simulator and an Android emulator, and a build of this site.
