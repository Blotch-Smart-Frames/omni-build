# Test fixtures

## `shell.apk`

The Android E2E shell (`packages/e2e/fixtures/android`), as Gradle builds it, without its code: the Rust tests read and rewrite its manifest, resource table, icon and payload, and never run it. Without `classes.dex` and the Kotlin metadata it's 8 KB instead of 650 KB. It keeps a v2 signature by the Android SDK's debug key, as the real shell has, which repackaging replaces.

To make it again after changing the E2E shell (with the Android SDK's build tools on the `PATH`):

```sh
node packages/e2e/src/cli.ts build --platform android
cp packages/e2e/build/shells/android/OmniBuildShell.apk trimmed.apk
zip -d trimmed.apk classes.dex 'kotlin/*' META-INF/version-control-info.textproto
zipalign -f -P 16 4 trimmed.apk aligned.apk
apksigner sign --ks ~/.android/debug.keystore --ks-pass pass:android \
  --v1-signing-enabled false --v3-signing-enabled false \
  --out packages/cli/tests/fixtures/shell.apk aligned.apk
```

## `shell.aab`

The same shell as an Android App Bundle, as `bundleRelease` builds it, without its code. It keeps a JAR signature by the Android SDK's debug key, which repackaging replaces.

```sh
cd packages/e2e/fixtures/android && ./gradlew bundleRelease && cd -
cp packages/e2e/fixtures/android/app/build/outputs/bundle/release/app-release.aab trimmed.aab
zip -d trimmed.aab base/dex/classes.dex 'base/root/kotlin/*' \
  base/root/META-INF/version-control-info.textproto 'META-INF/*'
jarsigner -keystore ~/.android/debug.keystore -storepass android \
  -digestalg SHA-256 -sigalg SHA256withRSA \
  -signedjar packages/cli/tests/fixtures/shell.aab trimmed.aab androiddebugkey
```

The Rust tests find files in both shells by their place in the archive, so a remade fixture needs the same entries in the same order.
