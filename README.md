# OmniBuild

Build, sign, install and ship iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK. One standalone binary that repackages a prebuilt app shell with your app's config and assets, then signs it.

**Status:** Phase 0 of [the plan](https://github.com/Blotch-Smart-Frames/omni-build/issues/1), with Phase 1 begun. `omni repackage` gives an Android APK or app bundle (AAB) shell, or an iOS shell built for the Simulator or for devices, an app's ID, name, version, build number, icon, payload and permissions, and the prebuilt native modules among its npm dependencies, and signs it, frameworks included, with nothing else installed. `omni key create` makes an Android upload key, `omni key request` and `key import` an Apple signing key without a Mac, `omni provision` its certificate and provisioning profiles through the App Store Connect API, with an API key, `omni upload` uploads an app bundle to a Google Play track, as a Google Cloud service account, or an iOS app to App Store Connect for TestFlight, with the API key, and `omni install` installs an APK on an Android phone plugged in by USB, an emulator or a device on the network, over ADB, or an iOS app on an iPhone or iPad, through the system's usbmux daemon, where `omni tcpip` has a phone plugged in listen, to take apps without its cable, and `omni pair` pairs with one for Android 11's wireless debugging, encrypted. Build tools read what it reports as JSON lines, through the npm package's API or `--json`. It's on npm as `@blotch/omni-build`, and from the release after 0.0.1 on crates.io as `omni-build`, in Homebrew as `blotch-smart-frames/tap/omni-build`, and as a download per platform from GitHub releases, beside OmniBuild's WebView shell, which makes a web app an iOS and Android app. Frameworks build and publish their own shells with the same reusable workflow, `.github/workflows/shell.yml`.

## Repository

An npm workspaces and [Turborepo](https://turborepo.com) monorepo, with the Rust inside it:

| Package                   | What it is                                                                                                                                                                                                   |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `packages/cli`            | The `omni` binary, a Rust crate. Its `package.json` is private and only lets turbo run Cargo                                                                                                                 |
| `packages/omni-build`     | The `@blotch/omni-build` npm package: a launcher that runs the binary built for the user's OS and CPU, and an API for build tools                                                                            |
| `packages/shells`         | The app shells OmniBuild ships, Swift and Kotlin projects: the WebView shell, for web apps. And the tool the Shell workflow runs, which builds any shell with Xcode and Gradle and checks omni repackages it |
| `packages/e2e`            | End-to-end tests: the shells and a fixture, repackaged on every OS, run on the iOS Simulator and an Android emulator                                                                                         |
| `packages/platform-watch` | Opens an issue when the app shells need a newer Xcode or Android API level, or can't read a vendor's page. Runs daily in CI                                                                                  |
| `packages/docs`           | The documentation site and landing page, built with Docusaurus and published to GitHub Pages                                                                                                                 |
| `packages/coverage-guard` | Fails the coverage gate wherever 100% would pass over untested code. `npm run test:coverage` runs it after the tests                                                                                         |
| `packages/release`        | Packs the npm packages a release publishes, checks `npm install @blotch/omni-build` gets each machine its own binary, and publishes them                                                                     |

The npm packages are TypeScript. Node runs the private ones' `.ts` files as they are, by stripping the types, so their imports name the `.ts` file and they use only syntax that strips away (no enums or namespaces). The launcher is compiled to `dist/` for npm, because Node won't strip types in `node_modules`.

The per-platform npm packages that hold the binaries (`@blotch/omni-build-darwin-arm64`, `@blotch/omni-build-linux-x64`, ...) aren't in the repository. npm refuses to install a workspace whose `os` or `cpu` doesn't match the machine, so `packages/release` makes them from the list in `packages/omni-build/src/platforms.ts`, and CI installs them on each OS and CPU from a registry on the runner. A `v<version>` tag publishes what CI packed for that commit to npm (the steps are in `packages/release/README.md`).

## Development

You need Node 24 and [rustup](https://rustup.rs), which installs the Rust version pinned in `rust-toolchain.toml` the first time Cargo runs.

```sh
npm install
npm run build          # cargo build --release, and the docs site
npm test               # cargo test, and every package's vitest tests
npm run test:coverage  # the same, failing below 100% coverage, then coverage-guard (needs cargo-llvm-cov)
npm run lint           # clippy and oxlint
npm run cspell         # spelling, with the words in cspell.json and each package's own
npm run typecheck      # tsc over the TypeScript packages
npm run format         # rustfmt and oxfmt; CI runs format:check
npm run e2e -- --platform ios  # build, repackage and run a real app on the iOS Simulator, or android (see packages/e2e)
```

The USB tests (`packages/cli/src/adb/usb.rs`) plug made-up phones into vhci-hcd, Linux's virtual USB host controller, so they run only on Linux, and elsewhere `npm run test:coverage` finds that file short of 100%. CI runs them. Docker Desktop's Linux has vhci-hcd built in, so on a Mac or a PC they run in a privileged container:

```sh
docker run --rm --privileged -v /dev:/dev -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/tmp/target rust:1.99.0 cargo test adb::usb
```

Pull requests must pass CI: formatting, lint (warnings are errors), spelling, type checks, 100% test coverage, tests on Linux, macOS and Windows, a release build on each of the six targets OmniBuild ships for, `npm install @blotch/omni-build` from those builds on each of the six, the end-to-end tests on the iOS Simulator and an Android emulator, and a build of the docs site.

Cargo also works on its own from the repository root, for example `cargo run -- --help`.
