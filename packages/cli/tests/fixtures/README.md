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

The Rust tests find files in both Android shells by their place in the archive, so a remade fixture needs the same entries in the same order.

## `shell.app.zip`

The iOS E2E shell (`packages/e2e/fixtures/ios`), as Xcode builds it for the Simulator, zipped by `ditto` as the E2E tests zip it, so each file keeps its mode. Its executable is a tiny program instead of the shell's Swift, linked for the Simulator's two architectures and signed by the linker: the Rust tests read and rewrite its `Info.plist`, a binary property list with Xcode's `DT*` keys, its icon, in the asset catalog (`Assets.car`) and the PNGs beside it, and its signature, but never run it. `--norsrc` leaves out the `._` files that would otherwise carry each file's extended attributes.

```sh
node packages/e2e/src/cli.ts build --platform ios
ditto -x -k packages/e2e/build/shells/ios/OmniBuildShell.zip trimmed
printf 'int main(void) { return 0; }\n' > main.c
xcrun -sdk iphonesimulator clang -arch x86_64 -arch arm64 -mios-simulator-version-min=17.0 \
  -Wl,-adhoc_codesign main.c -o trimmed/OmniBuildShell.app/OmniBuildShell
ditto -c -k --norsrc --keepParent trimmed/OmniBuildShell.app packages/cli/tests/fixtures/shell.app.zip
```

## `shell.ipa`

The same shell built for devices, as the E2E tests build it (unsigned, with room for a signature), in an `.ipa`, with a tiny program for its executable, linked for arm64 and signed by the linker. Its PNGs are Apple's variant, as Xcode writes them for devices.

```sh
ditto -x -k packages/e2e/build/shells/ios/OmniBuildShell.ipa trimmed-ipa
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=17.0 -Wl,-adhoc_codesign main.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/OmniBuildShell
cd trimmed-ipa && ditto -c -k --norsrc --keepParent Payload ../packages/cli/tests/fixtures/shell.ipa
```

The tests compare what OmniBuild signs with what `codesign` signs from the same files: ad hoc, with `codesign --sign - --force` on each unzipped shell, and with the identity below, from a keychain, with `codesign --sign "OmniBuild Test" --timestamp=none --entitlements` the entitlements the profile gives `dev.omnibuild.shell`, and the profile copied in as `embedded.mobileprovision`. A remade shell needs those values in the tests made again.

## `identity.pem` and `profile.mobileprovision`

A key with a self-signed certificate, "OmniBuild Test", for a team that doesn't exist (`ABCDE12345`), and a provisioning profile signed with the same key, which allows it and any bundle ID (`ABCDE12345.*`). Made as the E2E tests' are (see `packages/e2e/README.md`), with `OmniBuild Test`, `ABCDE12345` and `ABCDE12345.*`, and written to `identity.pem` and `profile.mobileprovision`.

## `keystores/`

PKCS #12 keystores, as each tool writes one, with the password `omnibuild`: the key tests read each, and check what they find. From the identity above (`key.pem` and `cert.pem`, before `cat`):

| File              | Made by                                                               | How                                                                                                                  |
| ----------------- | --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `openssl.p12`     | OpenSSL 3 and later: PBES2 (AES-256), an HMAC-SHA256 MAC              | `openssl pkcs12 -export -inkey key.pem -in cert.pem -name "OmniBuild Test" -out openssl.p12 -passout pass:omnibuild` |
| `keychain.p12`    | Keychain Access: 3DES and 40-bit RC2, an HMAC-SHA1 MAC of 1 iteration | `security import openssl.p12` into a new keychain, then `security export -t identities -f pkcs12 -P omnibuild`       |
| `no-password.p12` | OpenSSL, with an empty password                                       | as `openssl.p12`, with `-passout pass:`                                                                              |
| `unencrypted.p12` | OpenSSL, with the key and certificate unencrypted                     | as `openssl.p12`, with `-keypbe NONE -certpbe NONE`                                                                  |
| `no-key.p12`      | OpenSSL, with the certificate alone                                   | `openssl pkcs12 -export -nokeys -in cert.pem -out no-key.p12 -passout pass:omnibuild`                                |

And with keys of their own, from a JDK's keytool: `keytool.p12` (`keytool -genkeypair -keystore keytool.p12 -storetype PKCS12 -storepass omnibuild -alias upload -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Upload, O=OmniBuild"`), `two-keys.p12` (the same twice, with the aliases `first` and `second`), and `ec.p12` (one key, `-keyalg EC -groupname secp256r1`).
