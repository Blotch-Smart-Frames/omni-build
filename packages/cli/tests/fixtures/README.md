# Test fixtures

## `shell.apk`

The Android E2E shell (`packages/e2e/fixtures/android`), as Gradle builds it, without its code: the Rust tests read and rewrite its manifest, resource table, icons (a WebP per density, and an adaptive icon whose layers are vectors), payload and native libraries (stand-ins, one per ABI), and never run it. Without `classes.dex` and the Kotlin metadata it's 78 KB instead of 710 KB, most of it the 16 KB each native library is aligned to. It keeps a v2 signature by the Android SDK's debug key, as the real shell has, which repackaging replaces.

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

The Rust tests find files in both Android shells by name, and resources by their IDs, which aapt2 gives in the order of their types and names: a remade fixture keeps them while the shell's resources stay as they are.

## `shell.app.zip`

The iOS E2E shell (`packages/e2e/fixtures/ios`), as Xcode builds it for the Simulator, zipped by `ditto` as the E2E tests zip it, so each file keeps its mode. Its executable, its framework's (`Frameworks/ShellKit.framework`), its extensions' (`PlugIns/ShellNotifications.appex`, an app extension, and `Extensions/ShellIntents.appex`, an ExtensionKit extension), its watch app's (`Watch/ShellWatch.app`, linked for the watchOS Simulator, for watchOS 11) and its widgets' (`PlugIns/ShellWatchWidgets.appex` in the watch app, for watchOS 26), and its App Clip's (`AppClips/ShellClip.app`) and the App Clip's copy of the framework (`Frameworks/ShellKit.framework` in the App Clip) are tiny programs instead of the shell's Swift, linked for the Simulator's two architectures and signed by the linker: the Rust tests read and rewrite its `Info.plist`, a binary property list with Xcode's `DT*` keys, its English translation (`en.lproj/InfoPlist.strings`), its icon, in the asset catalog (`Assets.car`) and the PNGs beside it, and its signatures, but never run it. `--norsrc` leaves out the `._` files that would otherwise carry each file's extended attributes.

```sh
node packages/e2e/src/cli.ts build --platform ios
ditto -x -k packages/e2e/build/shells/ios/OmniBuildShell.zip trimmed
printf 'int main(void) { return 0; }\n' > main.c
printf 'int shell_kit(void) { return 0; }\n' > lib.c
xcrun -sdk iphonesimulator clang -arch x86_64 -arch arm64 -mios-simulator-version-min=17.0 \
  -Wl,-adhoc_codesign main.c -o trimmed/OmniBuildShell.app/OmniBuildShell
xcrun -sdk iphonesimulator clang -arch x86_64 -arch arm64 -mios-simulator-version-min=17.0 \
  -dynamiclib -install_name @rpath/ShellKit.framework/ShellKit -Wl,-adhoc_codesign lib.c \
  -o trimmed/OmniBuildShell.app/Frameworks/ShellKit.framework/ShellKit
xcrun -sdk iphonesimulator clang -arch x86_64 -arch arm64 -mios-simulator-version-min=17.0 \
  -Wl,-adhoc_codesign main.c -o trimmed/OmniBuildShell.app/PlugIns/ShellNotifications.appex/ShellNotifications
xcrun -sdk iphonesimulator clang -arch x86_64 -arch arm64 -mios-simulator-version-min=17.0 \
  -Wl,-adhoc_codesign main.c -o trimmed/OmniBuildShell.app/Extensions/ShellIntents.appex/ShellIntents
xcrun -sdk iphonesimulator clang -arch x86_64 -arch arm64 -mios-simulator-version-min=17.0 \
  -Wl,-adhoc_codesign main.c -o trimmed/OmniBuildShell.app/AppClips/ShellClip.app/ShellClip
xcrun -sdk iphonesimulator clang -arch x86_64 -arch arm64 -mios-simulator-version-min=17.0 \
  -dynamiclib -install_name @rpath/ShellKit.framework/ShellKit -Wl,-adhoc_codesign lib.c \
  -o trimmed/OmniBuildShell.app/AppClips/ShellClip.app/Frameworks/ShellKit.framework/ShellKit
xcrun -sdk watchsimulator clang -arch x86_64 -arch arm64 -mwatchos-simulator-version-min=11.0 \
  -Wl,-adhoc_codesign main.c -o trimmed/OmniBuildShell.app/Watch/ShellWatch.app/ShellWatch
xcrun -sdk watchsimulator clang -arch x86_64 -arch arm64 -mwatchos-simulator-version-min=26.0 \
  -Wl,-adhoc_codesign main.c \
  -o trimmed/OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets
ditto -c -k --norsrc --keepParent trimmed/OmniBuildShell.app packages/cli/tests/fixtures/shell.app.zip
```

## `shell.ipa`

The same shell built for devices, as the E2E tests build it (unsigned, with room for a signature), in an `.ipa`, with tiny programs for its executables, linked for arm64 and signed by the linker. The watch app's and its widgets' are linked for `arm64_32` too: the watch app's for watchOS 11, which `codesign` signs with SHA-1 hashes as well as SHA-256 ones, and so the `arm64` code beside it, which clang raises to watchOS 26, and the widgets' for watchOS 26, which it signs with SHA-256 alone. Its PNGs are Apple's variant, as Xcode writes them for devices.

```sh
ditto -x -k packages/e2e/build/shells/ios/OmniBuildShell.ipa trimmed-ipa
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=17.0 -Wl,-adhoc_codesign main.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/OmniBuildShell
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=17.0 -dynamiclib \
  -install_name @rpath/ShellKit.framework/ShellKit -Wl,-adhoc_codesign lib.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/Frameworks/ShellKit.framework/ShellKit
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=17.0 -Wl,-adhoc_codesign main.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/PlugIns/ShellNotifications.appex/ShellNotifications
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=17.0 -Wl,-adhoc_codesign main.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/Extensions/ShellIntents.appex/ShellIntents
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=17.0 -Wl,-adhoc_codesign main.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/AppClips/ShellClip.app/ShellClip
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=17.0 -dynamiclib \
  -install_name @rpath/ShellKit.framework/ShellKit -Wl,-adhoc_codesign lib.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/AppClips/ShellClip.app/Frameworks/ShellKit.framework/ShellKit
xcrun -sdk watchos clang -arch arm64 -arch arm64_32 -mwatchos-version-min=11.0 -Wl,-adhoc_codesign main.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/Watch/ShellWatch.app/ShellWatch
xcrun -sdk watchos clang -arch arm64 -arch arm64_32 -mwatchos-version-min=26.0 -Wl,-adhoc_codesign main.c \
  -o trimmed-ipa/Payload/OmniBuildShell.app/Watch/ShellWatch.app/PlugIns/ShellWatchWidgets.appex/ShellWatchWidgets
cd trimmed-ipa && ditto -c -k --norsrc --keepParent Payload ../packages/cli/tests/fixtures/shell.ipa
```

The tests compare what OmniBuild signs with what `codesign` signs from the same files, from the inside out, as Xcode signs them: the framework, the extensions, the watch app's widgets and then the watch app, the App Clip's framework and then the App Clip, and then the app. Ad hoc, with `codesign --sign - --force` on each bundle of each unzipped shell, and with the identity below, from a keychain, with `codesign --sign "OmniBuild Test" --timestamp=none`. That adds, for each program, `--entitlements` with the entitlements OmniBuild signs it with (`dev.omnibuild.shell.ShellNotifications`, `dev.omnibuild.shell.ShellIntents`, `dev.omnibuild.shell.watchkitapp.ShellWatchWidgets`, `dev.omnibuild.shell.watchkitapp`, `dev.omnibuild.shell.Clip` and `dev.omnibuild.shell`), byte for byte as OmniBuild's signature has them: what the profile gives each ID, and for the App Clip and the app, the entitlements that name each other. Each program gets its profile copied into it as `embedded.mobileprovision`: `clip.mobileprovision` for the App Clip, and `profile.mobileprovision` for the rest. In the device shell, `codesign` pads each signature with what the linker's left there, and OmniBuild with zeros. So the tests compare each executable up to the end of its signature, the watch app's and its widgets', which have two architectures (`arm64` and `arm64_32`), whole with each one's padding zeroed, and each bundle with code nested in it, the watch app, the App Clip and the app, is signed around that code with its padding zeroed. With the identity, each CMS signature has the time it was made in it, so the seals of those three bundles, and their code directories' hashes of them, differ each time: the tests check the seals have every file in them as codesign would seal it instead. A remade shell needs those values in the tests made again.

## `packed-icons.car`

An asset catalog `actool` compiled from an icon set with every size, as Xcode made them before Xcode 14, so that it packs the smaller sizes into larger images. Each size is a square of its own colour, `#XX6FEB` with its size in pixels (mod 256) as the red, so the tests can tell where `actool` put each one. Made with a script writing those PNGs and the set's `Contents.json` for 20, 29, 38, 40, 60, 64, 68, 76 and 83.5 points at 2x and 3x, with 1024×1024 for the App Store, then:

```sh
xcrun actool --compile out --platform iphoneos --minimum-deployment-target 17.0 --app-icon AppIcon \
  --target-device iphone --output-partial-info-plist partial.plist Assets.xcassets
cp out/Assets.car packages/cli/tests/fixtures/packed-icons.car
```

## `identity.pem`, `profile.mobileprovision` and `clip.mobileprovision`

A key with a self-signed certificate, "OmniBuild Test", for a team that doesn't exist (`ABCDE12345`), and a provisioning profile signed with the same key, which allows it and any bundle ID (`ABCDE12345.*`). Made as the E2E tests' are (see `packages/e2e/README.md`), with `OmniBuild Test`, `ABCDE12345` and `ABCDE12345.*`, and written to `identity.pem` and `profile.mobileprovision`. `clip.mobileprovision` is the shell's App Clip's, made the same way as the E2E tests' App Clip profile, for the App ID `ABCDE12345.dev.omnibuild.shell.Clip`, of `ABCDE12345.dev.omnibuild.shell`, named `OmniBuild Test Clip`, with the UUID `7A2B3C4D-5E6F-4071-8283-A4B5C6D7E8F9`.

## `AuthKey_TESTKEY123.p8`

An App Store Connect API key of no account, as App Store Connect gives one: a P-256 private key in PKCS #8 PEM, in a file named by the key's ID. The App Store Connect tests sign tokens with it, and check them with its public key. `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out AuthKey_TESTKEY123.p8`.

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

## `adb/`

A key pair as adb makes one, `adbkey` (PKCS #8) and `adbkey.pub` (Android's own format, in base64, then who the key belongs to): `adb keygen adbkey`, with the name of the user and computer that made it at the end of `adbkey.pub` replaced by `omnibuild@test`. The ADB tests check that OmniBuild writes the public key as adb did, byte for byte, and connect with the private key.

`devicekey` is a key of a made-up device's own, which the made-up devices in the TLS and pairing tests make their certificates with, as a device makes its own: `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out devicekey`.

`peer-info` is what an Android 14 emulator sent OmniBuild when the two paired: its peer info, encrypted with the key pairing gave them, which the pairing tests decrypt to the emulator's GUID, as a check of OmniBuild's SPAKE2 against BoringSSL's. The test has the rest of that pairing: the code, the TLS connection's keying material, and the two SPAKE2 messages, from OmniBuild's random bytes fixed as 64 sevens (`Spake2::with_random`). To record another, have `pair` print those, and the device's peer info, and pair with an emulator through a port `adb forward` forwards to the one its **Pair device with pairing code** shows.
