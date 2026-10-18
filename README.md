# OmniBuild

Build, sign, install and ship iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK. One standalone binary that repackages a prebuilt app shell with your app's config and assets, then signs it.

**Status:** Phase 0 of [the plan](https://github.com/Blotch-Smart-Frames/omni-build/issues/1). `omni-build repackage` gives an Android APK or app bundle (AAB) shell an app's ID, name, version, build number, icon and payload, and signs it, with nothing else installed. Nothing is published yet.

OmniBuild started as an investigation in [AngularLynx](https://github.com/Blotch-Smart-Frames/AngularLynx), which will be its first user.

## Repository

An npm workspaces and [Turborepo](https://turborepo.com) monorepo, with the Rust inside it:

| Package                   | What it is                                                                                                                   |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `packages/cli`            | The `omni-build` binary, a Rust crate. Its `package.json` is private and only lets turbo run Cargo                           |
| `packages/omni-build`     | The `omni-build` npm package: a launcher that runs the binary built for the user's OS and CPU                                |
| `packages/e2e`            | End-to-end tests: real Swift and Kotlin app shells, repackaged on every OS, run on the iOS Simulator and an Android emulator |
| `packages/platform-watch` | Opens an issue when the app shells need a newer Xcode or Android API level, or can't read a vendor's page. Runs daily in CI  |
| `packages/docs`           | The documentation site and landing page, built with Docusaurus and published to GitHub Pages                                 |

The npm packages are TypeScript. Node runs the private ones' `.ts` files as they are, by stripping the types, so their imports name the `.ts` file and they use only syntax that strips away (no enums or namespaces). The `omni-build` launcher is compiled to `dist/` for npm, because Node won't strip types in `node_modules`.

The per-platform npm packages that will hold the binaries (`@omni-build/darwin-arm64`, `@omni-build/linux-x64`, ...) aren't in the repository. npm refuses to install a workspace whose `os` or `cpu` doesn't match the machine, so the release workflow will generate them from the list in `packages/omni-build/src/platforms.ts`.

## Development

You need Node 24 and [rustup](https://rustup.rs), which installs the Rust version pinned in `rust-toolchain.toml` the first time Cargo runs.

```sh
npm install
npm run build          # cargo build --release, and the docs site
npm test               # cargo test, and every package's vitest tests
npm run test:coverage  # the same, failing below 100% coverage (needs cargo-llvm-cov)
npm run lint           # clippy and oxlint
npm run typecheck      # tsc over the TypeScript packages
npm run format         # rustfmt and oxfmt; CI runs format:check
npm run e2e -- --platform ios  # build, repackage and run a real app on the iOS Simulator, or android (see packages/e2e)
```

Pull requests must pass CI: formatting, lint (warnings are errors), type checks, 100% test coverage, tests on Linux, macOS and Windows, a release build on each of the six targets OmniBuild ships for, the end-to-end tests on the iOS Simulator and an Android emulator, and a build of the docs site.

Cargo also works on its own from the repository root, for example `cargo run -- --help`.
