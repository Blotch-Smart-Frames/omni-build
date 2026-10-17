---
name: platform-update
description: Do the update a platform-watch issue asks for. Move the app shells to a new Xcode (the DEVELOPER_DIR pin in CI) or a new Android API level (compileSdk and targetSdk), prove it with the E2E jobs, and open a PR that closes the issue. Or fix the platform watch when it can't download or parse Apple's or Google's pages. Use whenever the user mentions a platform-watch issue or label, asks to bump, upgrade or move to a new Xcode, iOS SDK, compileSdk, targetSdk or Android API level, brings up an App Store or Google Play SDK or target API deadline, or says the watch is broken or can't read a page, even if they don't name this skill.
---

# Platform update

Apple raises the Xcode that App Store uploads must be built with every April. Google Play raises the API level apps must target every August 31. The daily platform watch (`packages/platform-watch`, see its README) notices and opens an issue with the `platform-watch` label. This skill does the work that issue asks for.

There are three kinds of issue:

| Title starts with                                     | What to do                                     |
| ----------------------------------------------------- | ---------------------------------------------- |
| "Build the iOS shells with Xcode N"                   | [iOS: a new Xcode](#ios-a-new-xcode)           |
| "Compile and target the Android shells against API N" | [Android: a new API level](#android-a-new-api-level) |
| "Platform watch can't read ..."                       | [Fix the watch](#fix-the-watch)                |

**Why the shells get rebuilt instead of patched:** OmniBuild keeps the `DT*` keys Xcode writes into a shell's `Info.plist`, because App Store validation reads them. Raising `targetSdk` opts the app into that Android release's behaviour changes. So never meet a new minimum by editing those values in a built shell. Rebuild the shells with the new toolchain and let the E2E jobs prove they still work.

## 1. Start

```bash
gh issue list --label platform-watch --state open
gh issue view <number>
```

- Read the whole issue, comments included. A **Required** line with a date is a store deadline, so work on that one first. A **Released** line on its own means there's time.
- Read `packages/platform-watch/src/sources.js`. Its `SOURCES` list says which files hold the shells' toolchain today. Use those paths rather than any you remember: they move when the release shells arrive.
- Work on a branch, e.g. `git switch -c platform/xcode-27` or `platform/android-api-37`.

## iOS: a new Xcode

1. **Check that GitHub's macOS runner has it.** CI can only use an Xcode that's installed on the runner image.

   ```bash
   gh api repos/actions/runner-images/contents/README.md -q .content | base64 -d | grep macos-latest
   gh api repos/actions/runner-images/contents/images/macos/<image>-Readme.md -q .content | base64 -d | grep 'Xcode_[0-9]'
   ```

   The second command lists each Xcode with its path, such as `/Applications/Xcode_27.1.app`. Pick the newest release (not a beta or RC) of the major the issue asks for.

   - **Not on `macos-latest` yet,** but on a newer image that's generally available (not a beta label)? You can also move `runs-on` for `e2e-shell-ios` to that image.
   - **On no generally available image?** Stop. Comment on the issue that it's waiting for GitHub's runner images, and leave it open.

2. **Point the pin at it.** In `.github/workflows/ci.yml`, job `e2e-shell-ios`, set `DEVELOPER_DIR: /Applications/Xcode_<version>.app/Contents/Developer`. Use the exact path from the table. There must be only one `DEVELOPER_DIR` version in the file, or the watch can't read it.

3. **Find the other mentions** of the old version and update the ones that describe the toolchain:

   ```bash
   git grep -n -e 'Xcode_<old>' -e 'Xcode <old>'
   ```

4. **Build locally if you can** (on a Mac with that Xcode installed):

   ```bash
   DEVELOPER_DIR=/Applications/<that Xcode>.app/Contents/Developer npm run e2e -- --platform ios
   ```

   Fix whatever the new SDK breaks in the Swift shells (`packages/e2e/fixtures/ios`). Without a Mac, CI does this step.

## Android: a new API level

1. **Raise both levels.** In the shells' Gradle file (today `packages/e2e/fixtures/android/app/build.gradle.kts`), set `compileSdk = N` and `targetSdk = N`. Keep the plain `name = N` form, which is what the watch reads.

2. **Check that the Android Gradle plugin supports N.** Each AGP release's notes (developer.android.com/build/releases/gradle-plugin) give the highest API level it supports. If the version in `packages/e2e/fixtures/android/build.gradle.kts` is too old, raise it. If the new AGP needs a newer Gradle, update the wrapper with `./gradlew wrapper --gradle-version <version> --gradle-distribution-sha256-sum <sum>` (checksums at gradle.org/release-checksums). Don't edit `gradle-wrapper.properties` by hand: CI checks the wrapper.

3. **Go through the behaviour changes for apps targeting N** (developer.android.com/about/versions, then the "behavior changes: apps targeting" page for that release). Fix what applies to the Kotlin shells.

4. **Think about the emulator.** Behaviour changes only take effect on a device running N or newer, and the `e2e-android` job's `api-level` may be lower. If a system image for N runs reliably on GitHub's runners, raise it. If not, keep it and say so in the PR, so nobody assumes N was tested.

5. **Find the other mentions** of the old level, such as the E2E README and the comments in `ci.yml`:

   ```bash
   git grep -n -e 'API <old>' -e 'API level <old>' -e 'android-<old>'
   ```

6. **Build locally if you can:** `npm run e2e -- --platform android --avd <name>`. Without the Android SDK, CI does this step.

## Fix the watch

The issue quotes the error and names the source by its `id` in `packages/platform-watch/src/sources.js`.

1. **See what changed.** Download the source into the scratchpad and look for the value the parser wants. The source's `looksFor` describes it. The page may have moved (a new URL), been reworded (the regex no longer matches), or changed shape. A file source in this repository may have had its line moved or rewritten.
2. **Fix the parser or the `location`.** Keep it strict: it should find the number in the sentence that states it, or throw. A loose match that quietly returns an old number is worse than an issue.
3. **Replace that source's excerpt** in `src/sources.test.js` with the new real markup, trimmed to the relevant part. Keep the case that throws when the sentence is missing.
4. **If the vendor stopped publishing the value** at all, find another official source and add it. Don't scrape an unofficial mirror.

## 2. Check it

```bash
npm run test:coverage -w @omni-build/platform-watch   # only if the watch changed; 100% coverage is required
npm run lint
npm run format:check
GITHUB_REPOSITORY=Blotch-Smart-Frames/omni-build npm run check -w @omni-build/platform-watch -- --dry-run
```

The dry run is the real test of the change. It reads the live pages and your edited files, then prints what it would do. It should print no `couldn't` lines, and it should list `close #<number>` for the issue you're fixing. If it would still open or keep that issue, the pin or the levels didn't reach the version, or the watch can't read your edit.

## 3. Hand it over

- Commit in the repository's style (see `git log`), e.g. `ci: Build the iOS shells with Xcode 27`.
- Open a PR (ask first if the user hasn't said to) whose body says what changed and why, notes anything left untested (such as the emulator level), and ends with `Fixes #<number>`.
- **CI passing is what "done" means.** The E2E jobs build the shells with the new toolchain, repackage them on Linux, Windows and macOS, and run them on the Simulator and the emulator.
- Don't close the issue by hand. The PR closes it on merge. If it doesn't, the watch runs on that push to master and closes it once the files reach the version.

**When to stop and ask instead:** the new toolchain needs a change to OmniBuild itself (a new `Info.plist` key or manifest attribute it must handle), a dependency doesn't support the new version yet, or a behaviour change needs a product decision (a permission, a UI change). Comment on the issue with what you found, and leave it open.
