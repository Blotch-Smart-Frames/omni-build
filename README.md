# OmniBuild

Build, sign, install and ship iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK. One standalone binary that repackages a prebuilt app shell with your app's config and assets, then signs it.

**Status:** planning. See the plan in [#1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1).

OmniBuild started as an investigation in [AngularLynx](https://github.com/Blotch-Smart-Frames/AngularLynx), which will be its first user.

## Repository

An npm workspaces and [Turborepo](https://turborepo.com) monorepo, with the Rust inside it:

| Package               | What it is                                                                                         |
| --------------------- | -------------------------------------------------------------------------------------------------- |
| `packages/cli`        | The `omni-build` binary, a Rust crate. Its `package.json` is private and only lets turbo run Cargo |
| `packages/omni-build` | The `omni-build` npm package: a launcher that runs the binary built for the user's OS and CPU      |

The per-platform npm packages that will hold the binaries (`@omni-build/darwin-arm64`, `@omni-build/linux-x64`, ...) aren't in the repository. npm refuses to install a workspace whose `os` or `cpu` doesn't match the machine, so the release workflow will generate them from the list in `packages/omni-build/src/platforms.js`.

## Development

You need Node 24 and [rustup](https://rustup.rs), which installs the Rust version pinned in `rust-toolchain.toml` the first time Cargo runs.

```sh
npm install
npm run build          # cargo build --release
npm test               # cargo test, and the launcher's tests with vitest
npm run lint           # clippy and oxlint
npm run format         # rustfmt and oxfmt; CI runs format:check
```

Cargo also works on its own from the repository root, for example `cargo run -- --help`.
