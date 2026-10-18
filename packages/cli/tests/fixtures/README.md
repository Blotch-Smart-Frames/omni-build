# Test fixtures

## `shell.apk`

The Android E2E shell (`packages/e2e/fixtures/android`), as Gradle builds it, without its code: the Rust tests read and rewrite its manifest, resource table, icon and payload, and never run it. Without `classes.dex` and the Kotlin metadata it's 8 KB instead of 650 KB. It keeps a v2 signature by the Android SDK's debug key, as the real shell has, which repackaging replaces.

To make it again after changing the E2E shell (with the Android SDK's build tools on the `PATH`):

```sh
node packages/e2e/src/run.js build --platform android
cp packages/e2e/build/shells/android/OmniBuildShell.apk trimmed.apk
zip -d trimmed.apk classes.dex 'kotlin/*' META-INF/version-control-info.textproto
zipalign -f -P 16 4 trimmed.apk aligned.apk
apksigner sign --ks ~/.android/debug.keystore --ks-pass pass:android \
  --v1-signing-enabled false --v3-signing-enabled false \
  --out packages/cli/tests/fixtures/shell.apk aligned.apk
```
