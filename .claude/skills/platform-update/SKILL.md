---
name: platform-update
description: Do the update a platform-watch issue asks for. Move the app shells to a new Xcode (the DEVELOPER_DIR pin in CI) or a new Android API level (compileSdk and targetSdk), prove it with the E2E jobs, and open a PR that closes the issue. Or fix the platform watch when it can't download or parse Apple's or Google's pages. Use whenever the user mentions a platform-watch issue or label, asks to bump, upgrade or move to a new Xcode, iOS SDK, compileSdk, targetSdk or Android API level, brings up an App Store or Google Play SDK or target API deadline, or says the watch is broken or can't read a page, even if they don't name this skill.
---

# Platform update

Apple raises the Xcode that App Store uploads must be built with every April. Google Play raises the API level apps must target every August 31. The daily platform watch (`packages/platform-watch`, see its README) notices and opens an issue with the `platform-watch` label. This skill does the work that issue asks for, or the same work when the user asks for it directly.

| Issue title starts with                                     | Section                                              |
| ----------------------------------------------------------- | ---------------------------------------------------- |
| "Build the iOS shells with Xcode N"                         | [iOS: a new Xcode](#ios-a-new-xcode)                 |
| "Compile and target the Android shells against API level N" | [Android: a new API level](#android-a-new-api-level) |
| "Platform watch can't read ..."                             | [Fix the watch](#fix-the-watch)                      |

**Why the shells get rebuilt instead of patched:** OmniBuild keeps the `DT*` keys Xcode writes into a shell's `Info.plist`, because App Store validation reads them. Raising `targetSdk` opts the app into that Android release's behaviour changes. So never meet a new minimum by editing those values in a built shell. Rebuild the shells with the new toolchain and let the E2E jobs prove they still work.

## 1. Start

```bash
gh issue list --label platform-watch --state open
GITHUB_REPOSITORY=Blotch-Smart-Frames/omni-build npm run check -w @omni-build/platform-watch -- --dry-run
```

- **With an issue:** read all of it, comments included (`gh issue view <number> --comments`).
- **Without one** (the user asked directly, or the watch hasn't run yet): the dry run's `open an issue: ...` line is the issue you're doing. Its readings say what's out and what's required.
- **Urgency:** a **Required** reason with a date is a store deadline, so do it first. **Released** on its own means there's time.
- **Read `packages/platform-watch/src/sources.js`.** Its `SOURCES` list says which files hold the shells' toolchain today. Use those paths rather than any you remember: they move when the release shells arrive.
- **Branch from the branch the watch is on** (`master` once it's merged), e.g. `git switch -c platform/xcode-27`.

When looking for other mentions of the old version, leave out `packages/platform-watch` and `.claude`. Their mentions are test data, examples and quotes of the vendors' wording, not the toolchain.

## iOS: a new Xcode

1. **Find a runner image that has it.** CI can only use an Xcode installed on GitHub's runner image. GitHub keeps one Xcode major per macOS image, so a new major usually arrives on a new image, not on `macos-latest`.

   ```bash
   gh api repos/actions/runner-images/contents/README.md -q .content | base64 -d | grep -iE 'Xcode <N>|xcode-<N>|macOS <N>|macos-latest'
   gh api repos/actions/runner-images/contents/images/macos/<image>-Readme.md -q .content | base64 -d | grep 'Xcode_[0-9]'
   ```

   The first command shows each image's `runs-on` labels and any **preview** or **beta** badge. The second lists that image's Xcodes, each with its folder and its aliases.

   - **Generally available** (no preview or beta badge)? Use it.
   - **Preview or beta only?** Stop. `e2e-shell-ios` is part of the "CI passed" gate, so a queued or unstable runner would block every PR. See [Stopping](#stopping).

2. **Pin it.** In `.github/workflows/ci.yml`, job `e2e-shell-ios`:
   - Set `runs-on` to the image's label if it isn't `macos-latest`.
   - Set `DEVELOPER_DIR: /Applications/Xcode_<major.minor>.app/Contents/Developer`, using the `Xcode_<major.minor>.app` alias from the aliases column. Don't copy the folder name: it can say `_Release_Candidate` or `_beta_2`, and the watch only reads the plain form. The alias also stays put when GitHub swaps in a patch release.
   - Keep one `DEVELOPER_DIR` version in the file, or the watch can't read it.

   Pick a release, not a beta or RC. Judge that by the build number against Apple's releases feed (an item titled "Xcode 27.1 (27A9275)"), not by the folder name.

3. **Decide about the Simulator.** `e2e-ios` runs the shells on its own runner's Simulator. If it stays on `macos-latest`, the shells still run (their deployment target is far below N), but iOS N itself goes untested in CI. Move its `runs-on` too, or say in the PR that it didn't move.

4. **Build it locally** if there's a Mac with Xcode N. Check which one with `xcodebuild -version`. It's usually just `/Applications/Xcode.app`, and may be a different minor from the one CI pins: say so in the PR or comment.

   ```bash
   DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer npm run e2e -- --platform ios
   ```

   Fix whatever the new SDK breaks in the Swift shells (`packages/e2e/fixtures/ios`). Without a Mac, CI does this step.

5. **Update other mentions** of the old Xcode that describe the toolchain:

   ```bash
   git grep -n -e 'Xcode_<old>' -e 'Xcode <old>' -- ':!packages/platform-watch' ':!.claude'
   ```

## Android: a new API level

1. **Raise both levels.** In the shells' Gradle file (today `packages/e2e/fixtures/android/app/build.gradle.kts`), set `compileSdk = N` and `targetSdk = N`. Keep the plain `name = N` form, which is what the watch reads. Since API 36, Google names the platform package `android-N.0` (then `N.1`, `N.2`), and `compileSdk = N` resolves to `N.0`.

2. **Check that the Android Gradle plugin supports N.** The AGP release notes (developer.android.com/build/releases/gradle-plugin) give the highest API level each version supports, and the Gradle version it needs. If the version in `packages/e2e/fixtures/android/build.gradle.kts` is too old, raise it. If the new AGP needs a newer Gradle, update the wrapper with `./gradlew wrapper --gradle-version <version> --gradle-distribution-sha256-sum <sum>` (checksums at gradle.org/release-checksums). Don't edit `gradle-wrapper.properties` by hand: CI checks the wrapper.

3. **Check the CI runner has the platform** (AGP downloads it if not, so a miss only costs time):

   ```bash
   gh api repos/actions/runner-images/contents/images/ubuntu/Ubuntu2404-Readme.md -q .content | base64 -d | grep 'Android SDK Platforms'
   ```

   Use the readme of the image `ubuntu-latest` points at (the runner-images README says which).

4. **Go through the behaviour changes for apps targeting N:** `developer.android.com/about/versions/<A>/behavior-changes-<A>`, where `A` is the Android version (so far API − 20: API 37 is Android 17). Fix what applies to the Kotlin shells.

5. **Run it on an emulator of N locally.** Behaviour changes only take effect on a device running N or newer. CI's `e2e-android` emulator level is shared with AngularLynx's choice, so keep it unless the user asks. Locally, the shells' behaviour at N gets checked:

   ```bash
   sdkmanager --install 'system-images;android-<N>.0;google_apis;arm64-v8a'
   avdmanager create avd -n api-<N> -k 'system-images;android-<N>.0;google_apis;arm64-v8a' -d pixel_6
   npm run e2e -- --platform android --avd api-<N>
   ```

   The tools are in `~/Library/Android/sdk/cmdline-tools/latest/bin` on a Mac when `ANDROID_HOME` isn't set. Also run it on the existing AVD, which matches CI. If you do raise CI's level, use the package's name (`api-level: 37.0`): the emulator action installs `system-images;android-<api-level>;...`. If CI stays lower, say in the PR that N's behaviour changes were tested only locally.

6. **Update other mentions** of the old level, such as the E2E README and the comments in `ci.yml`:

   ```bash
   git grep -n -e 'API <old>' -e 'API level <old>' -e 'android-<old>' -- ':!packages/platform-watch' ':!.claude'
   ```

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

The dry run is the real test of the change. It reads the live pages and your edited files, then prints what it would do. It should print no `couldn't` lines. With an issue, it should list `close #<number>` for it. Without one, its `open an issue: ...` line should be gone. If neither happens, the pin or the levels didn't reach the version, or the watch can't read your edit.

## 3. Hand it over

- Commit in the repository's style (see `git log`): `ci: Build the iOS shells with Xcode 27` for a change in the workflow, `test: Compile and target the Android shells against API level 37` for one in the E2E fixtures.
- Open a PR (ask first if the user hasn't said to). Its body says what changed and why, and what CI doesn't test (a Simulator or emulator left on an older version, a local Xcode that differs from the pin). With an issue, end it with `Fixes #<number>`.
- **CI passing is what "done" means.** The E2E jobs build the shells with the new toolchain, repackage them on Linux, Windows and macOS, and run them on the Simulator and the emulator.
- Don't close the issue by hand. The PR closes it on merge. If it doesn't, the watch runs on that push to master and closes it once the files reach the version.

## Stopping

Stop instead of changing anything when there's no generally available runner image with the toolchain, when the new toolchain needs a change to OmniBuild itself (a new `Info.plist` key or manifest attribute it must handle), when a dependency doesn't support the new version yet, or when a behaviour change needs a product decision (a permission, a UI change).

Before stopping, still build and run the shells locally if you can (iOS step 4, Android step 5). Then comment on the issue, or tell the user if there's none, with:

- why it's waiting, with a link (the runner-images announcement, a dependency's issue),
- the local result: whether the shells build and pass with the new toolchain, and what broke if not,
- the exact change to make once it's unblocked, so whoever picks it up doesn't start over.

Leave the issue open, and skip "2. Check it" and "3. Hand it over".
