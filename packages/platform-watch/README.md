# @blotch/omni-build-platform-watch

Opens a GitHub issue when the app shells need a newer Xcode or Android API level. Apple raises the Xcode that App Store uploads must be built with every April, and Google Play raises the API level apps must target every August 31. The shells are where that work happens: OmniBuild keeps the SDK a shell was built with rather than rewriting it, so meeting a new minimum means rebuilding the shells.

[`.github/workflows/platform-watch.yml`](../../.github/workflows/platform-watch.yml) runs it daily, when the shells' toolchain or the watch changes on master, and from the Actions tab.

## What it reads

| Source                                                                                                   | For                                                           |
| -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `DEVELOPER_DIR` in `.github/workflows/ci.yml`                                                            | The Xcode the iOS shells are built with                       |
| `compileSdk` and `targetSdk` in `packages/e2e/fixtures/android/app/build.gradle.kts`                     | The API levels the Android shells compile against and target  |
| [Apple's upcoming requirements](https://developer.apple.com/news/upcoming-requirements/)                 | The Xcode App Store uploads must be built with, and from when |
| [Apple's releases feed](https://developer.apple.com/news/releases/rss/releases.rss)                      | New Xcode releases, betas and release candidates left out     |
| [Google Play's target API level page](https://developer.android.com/google/play/requirements/target-sdk) | The API level Play requires apps to target, and from when     |
| [The Android SDK repository](https://dl.google.com/android/repository/repository2-3.xml)                 | New stable API levels, previews left out                      |

The shells' two files are the E2E fixtures for now, the only shells there are. When the release shells arrive (Phase 1 of [#1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1)), point those sources in `src/sources.ts` at theirs.

## The issues it opens

All have the `platform-watch` label, and are written so a person or a coding agent can pick them up: what changed, with links, where the value is set, what to do, and how to tell it's done (CI passes).

- **One per update**, such as "Build the iOS shells with Xcode 27". It opens when the version ships, and gets a comment when the store makes it a requirement, so Apple's deadline lands on the issue opened when Xcode shipped. It closes itself once the shells' files reach that version.
- **One per source it can't read**, such as "Platform watch can't read Apple's SDK minimum requirements", whether the download failed after three tries or the parser didn't find what it looks for. The vendors' pages aren't APIs, so a reworded page shows up here, not as silence. It closes itself once the source reads again.

Closing one yourself:

- **as completed** says the work is done. If the watch finds the same thing again, it opens a new issue.
- **as not planned** (or as a duplicate) leaves it closed. Only a new reason, such as the store now requiring the update, reopens it.

The watch finds its issues by the label and a hidden marker in the body, so taking the label off an issue makes it open another.

## Run it

```sh
GITHUB_REPOSITORY=Blotch-Smart-Frames/omni-build npm run check -w @blotch/omni-build-platform-watch -- --dry-run
```

`--dry-run` reads the sources and the repository's issues, and prints what it would open, comment on, reopen or close, without a token. Without it, `GITHUB_TOKEN` must be able to write issues.

## Things to know

- **The tests hold excerpts of the real pages** (`src/sources.test.ts`). When a page changes and the watch opens an issue about it, replace the excerpt with the new markup and fix the parser to match.
- **GitHub turns off scheduled workflows** in a public repository after 60 days without activity. The Actions tab has the button to turn it back on.
- **Issues opened with the workflow's `GITHUB_TOKEN` don't trigger other workflows**, as GitHub stops tokens from chaining runs. For an agent workflow to start on them (on `issues: opened` or the label), the watch has to open them with a GitHub App's token or a personal access token instead.
- Like the launcher, `cli.ts` only wires `run.ts` to the process, and the tests reach everything else in-process.
- **Node runs the TypeScript as it is**, stripping the types (Node 22.18 or later), so CI runs the watch without `npm ci` or a build. `npm run typecheck` is what checks the types.
