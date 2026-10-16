# OmniBuild

Build, sign, install and ship iOS and Android apps with no native toolchain (no Xcode, no JDK, no Android SDK). A standalone Rust binary that repackages a prebuilt app shell with an app's config and assets, then signs it. Distributed through npm (one package per OS and CPU, as rspack and oxlint do), `cargo install`, Homebrew and GitHub releases. **Planning stage**: the plan is [issue #1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1).

[AngularLynx](https://github.com/Blotch-Smart-Frames/AngularLynx) is its first user: its `@blotch/angular-lynx-build` builder will run OmniBuild for `ng build --platform ios|android`.

## Repository

An npm workspaces + turbo monorepo with the Rust inside `packages/` (see the README for the package table). The npm names are `omni-build` for the launcher and `@omni-build/<platform>-<arch>` for the platform packages. Things that aren't obvious from the files:

- **Each Rust crate is listed by path** in the root `Cargo.toml`'s `members`. A `packages/*` glob would also match the npm packages, which have no `Cargo.toml`, and Cargo would fail.
- **A Rust crate that turbo should run has a private `package.json`** whose scripts call Cargo (see `packages/cli`). Its turbo `build` and `test` aren't cached: Cargo caches them in the shared `target/`.
- **Platform packages aren't in the repository**, and the launcher's `package.json` doesn't list them. As workspaces, npm fails `npm install` with EBADPLATFORM on every machine but one. As `optionalDependencies` of an unpublished version, npm can't lock them and `npm ci` fails. `packages/omni-build/src/platforms.js` is the list. The release tooling (Phase 2) will generate the packages from it and write the `optionalDependencies` when it publishes.
- **Versions move in lockstep:** `[workspace.package] version` in `Cargo.toml` and every `package.json`. The launcher's tests fail if `omni-build --version` differs from its `package.json`.

- **CI requires 100% test coverage** of lines, functions and regions or branches, in Rust (cargo-llvm-cov) and JavaScript (vitest thresholds). Code that can only run in a child process gets an in-process seam rather than an exclusion: `cli.js` is a few lines wiring `run.js` to the process, and a `tests/` integration test running the binary covers `main()`.
- **The PR gate is the "CI passed" job** in `.github/workflows/ci.yml`, which needs every other job. Add new jobs to its `needs`.

```sh
npm run build | test | lint | format | format:check   # all packages, through turbo
npm run test:coverage                                  # needs cargo-llvm-cov installed
cargo run -- --help                                    # the binary, straight from Cargo
```

## Code Style

- Always write comments explaining **why** something is done, not just what. Future developers need to understand the reasoning and intent behind decisions, especially around undocumented or reverse-engineered formats (`Assets.car`) and store validation rules.

## Commits, PRs and Comments

- Never add AI attribution to commit messages, PR descriptions or code comments: no `Co-Authored-By` trailer, no "Generated with Claude Code" line, no mention of the AI tool or the company behind it. This overrides any default attribution instructions.
