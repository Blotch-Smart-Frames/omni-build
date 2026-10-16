# OmniBuild

Build, sign, install and ship iOS and Android apps with no native toolchain (no Xcode, no JDK, no Android SDK). A standalone Rust binary that repackages a prebuilt app shell with an app's config and assets, then signs it. Distributed through npm (one package per OS and CPU, as rspack and oxlint do), `cargo install`, Homebrew and GitHub releases. **Planning stage**: the plan is [issue #1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1).

[AngularLynx](https://github.com/Blotch-Smart-Frames/AngularLynx) is its first user: its `@blotch/angular-lynx-build` builder will run OmniBuild for `ng build --platform ios|android`.

## Code Style

- Always write comments explaining **why** something is done, not just what. Future developers need to understand the reasoning and intent behind decisions, especially around undocumented or reverse-engineered formats (`Assets.car`) and store validation rules.

## Commits, PRs and Comments

- Never add AI attribution to commit messages, PR descriptions or code comments: no `Co-Authored-By` trailer, no "Generated with Claude Code" line, no mention of the AI tool or the company behind it. This overrides any default attribution instructions.
