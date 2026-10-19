# OmniBuild

Build, sign, install and ship iOS and Android apps with no native toolchain (no Xcode, no JDK, no Android SDK). A standalone Rust binary that repackages a prebuilt app shell with an app's config and assets, then signs it. Distributed through npm (one package per OS and CPU, as rspack and oxlint do), `cargo install`, Homebrew and GitHub releases. **Phase 0** (the spikes) of the plan in [issue #1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1): `omni-build repackage` handles Android APK and AAB shells, and iOS shells built for the Simulator (no signature yet).

**OmniBuild is framework-agnostic.** The frameworks that build apps with it are its users, so the code, docs, issues and PRs name none of them, their packages or their commands. Lynx and WebView apps appear only as examples of apps whose per-app part is data.

## Repository

An npm workspaces + turbo monorepo with the Rust inside `packages/` (see the README for the package table). The npm names are `omni-build` for the launcher and `@omni-build/<platform>-<arch>` for the platform packages. Things that aren't obvious from the files:

- **Each Rust crate is listed by path** in the root `Cargo.toml`'s `members`. A `packages/*` glob would also match the npm packages, which have no `Cargo.toml`, and Cargo would fail.
- **A Rust crate that turbo should run has a private `package.json`** whose scripts call Cargo (see `packages/cli`). Its turbo `build` and `test` aren't cached: Cargo caches them in the shared `target/`.
- **Platform packages aren't in the repository**, and the launcher's `package.json` doesn't list them. As workspaces, npm fails `npm install` with EBADPLATFORM on every machine but one. As `optionalDependencies` of an unpublished version, npm can't lock them and `npm ci` fails. `packages/omni-build/src/platforms.ts` is the list. The release tooling (Phase 1) will generate the packages from it and write the `optionalDependencies` when it publishes.
- **Versions move in lockstep:** `[workspace.package] version` in `Cargo.toml` and every `package.json`. The launcher's tests fail if `omni-build --version` differs from its `package.json`.

- **The npm packages are TypeScript, run without a build** wherever Node can: it strips the types from the private packages' `.ts` files (CI runs the e2e harness and the platform watch with no `npm ci`). So imports name the `.ts` file and only erasable syntax is allowed (`erasableSyntaxOnly` in `tsconfig.base.json`: no enums, namespaces or parameter properties). The launcher is compiled to `dist/`, because Node won't strip types under `node_modules`, and its tests run the compiled `dist/cli.js`. Node, vitest and Docusaurus all strip types without checking them, so `npm run typecheck` (CI's `typecheck` job) is the only type check.
- **Every `.ts` and `.tsx` module has a `.test.ts` or `.test.tsx` beside it** (tool configs such as `vitest.config.ts` aside), the e2e harness and the docs site's included: the harness's tests stand in for Xcode, Gradle, adb and simctl, and the docs' run in jsdom, the components with React Testing Library. A new module gets its test in the same change.
- **CI requires 100% test coverage** of lines, functions and regions or branches, in Rust (cargo-llvm-cov) and TypeScript (vitest thresholds). Code that can only run in a child process gets an in-process seam rather than an exclusion: in each npm package with a command, `cli.ts` is a few lines wiring `run.ts` to the process, and a `tests/` integration test running the binary covers `main()`.
- **100% must come from each module's own test.** `npm run test:coverage` ends with `packages/coverage-guard`, which runs every module's test on its own, measuring only that module, and fails if it doesn't cover all of it: a component a page renders is covered by the page's test, but only its own test checks it. It also fails on a module with no test beside it, a coverage-ignore comment (`v8`, `c8`, `istanbul`, `#[coverage(off)]`, `cfg(coverage)`), and a Rust file missing from cargo-llvm-cov's report. So never exclude code from coverage; test it, or give it a seam.
- **The PR gate is the "CI passed" job** in `.github/workflows/ci.yml`, which needs every other job. Add new jobs to its `needs`.
- **E2E tests run in three stages** (`packages/e2e`): Xcode and Gradle build real Swift and Kotlin shells (only in our CI, never for users), omni-build repackages them on Linux, Windows and macOS with no `PATH` or toolchain variables, and the iOS Simulator and an Android emulator run every result. The fixture apps report their own ID, name, version, payload and icon on launch, so no UI driver is needed. A value OmniBuild learns to rewrite gets a field in both fixtures' reports.
- **The platform watch** (`packages/platform-watch`) runs daily and opens an issue when the shells need a newer Xcode or Android API level, or when it can't download or parse one of Apple's or Google's pages. It reads the shells' toolchain from `DEVELOPER_DIR` in `ci.yml` and the Android fixture's `build.gradle.kts`, so moving either means updating `src/sources.ts`.

```sh
npm run build | test | lint | cspell | typecheck | format | format:check   # all packages, through turbo
npm run test:coverage                                             # then coverage-guard; needs cargo-llvm-cov installed
cargo run -- --help                                               # the binary, straight from Cargo
```

## Code Style

- Always write comments explaining **why** something is done, not just what. Future developers need to understand the reasoning and intent behind decisions, especially around undocumented or reverse-engineered formats (`Assets.car`) and store validation rules.

## Commits, PRs and Comments

- Never add AI attribution to commit messages, PR descriptions or code comments: no `Co-Authored-By` trailer, no "Generated with Claude Code" line, no mention of the AI tool or the company behind it. This overrides any default attribution instructions.
