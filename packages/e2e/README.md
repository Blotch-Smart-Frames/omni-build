# @omni-build/e2e

End-to-end tests on real platforms: real Swift and Kotlin app shells, repackaged by omni-build on Linux, Windows and macOS, then installed and launched on the iOS Simulator and an Android emulator. They run on every pull request.

Each fixture is a stand-in for the app shells OmniBuild repackages. As in a real shell, nothing that differs between apps is in the Swift or Kotlin code: the ID, name, version, icon and payload are data in the bundle. On launch, the app reads those values from its own bundle and reports them, and the harness checks the report.

## The three stages

They follow how OmniBuild is meant to be used ([#1](https://github.com/Blotch-Smart-Frames/omni-build/issues/1)), and CI runs each on different machines:

| Stage       | Where it runs in CI                 | What it does                                                                                                                                    |
| ----------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `build`     | macOS (Xcode), Linux (Gradle)       | Compiles the fixture shells. The only stage with a native toolchain: OmniBuild's own CI makes the shells, never its users                       |
| `repackage` | Linux, Windows and macOS            | Runs each shell through the omni-build release binary, which gets no `PATH` and no toolchain variables (below)                                  |
| `test`      | macOS (Simulator), Linux (emulator) | Installs and launches every repackaged app, one per OS it was repackaged on, and checks what each reports. omni-build installs the Android apps |

**Every shell goes through `omni-build repackage`**, which tells an iOS app from an APK or an AAB by its contents. Gradle builds an APK and an app bundle (AAB), and omni-build gives each the ID, name, version, build number, icon, payload, permissions and ABIs in `app/`. The shell is as Android Studio's template makes one: WebP icons, an adaptive icon whose layers are vectors, a launcher activity with a label of its own, and a provider and a permission named after the application ID. It runs on Android 5.0 and later, where the template starts at 7.0, so its APK gets a v1 signature as well as v2 and v3, and on Android before 7.0 installs the way `adb` installs there. CI runs every app on an emulator of each, Android 5.0 and 14. It signs the APK with the debug key it creates in the home folder, and the app bundle as Google Play takes one, with an upload key that `omni-build key create` makes first, as a developer without a JDK would. Each value differs from the shell's, so an app that reports it got it from omni-build. The `test` stage checks the results with Google's own tools first. For an APK, that's `zipalign` for the alignment Android needs to map stored entries, and `apksigner` for every signature scheme. An app bundle gets what Google Play does with an upload: `jarsigner` checks its signature and `bundletool validate` its contents, then `bundletool build-apks` makes the universal APK that's installed. bundletool is downloaded once, pinned by its SHA-256, and signs that APK with a key of its own, as Play signs with the app's. `omni-build install` installs each APK on the emulator, over ADB, with the same environment as `repackage`, and adb does the rest: opening the app's link, which only the URL scheme omni-build gave it leads to, and reading its report. A phone on USB, which omni-build can't reach yet, gets the APK from adb instead. Xcode builds the iOS shell for the Simulator, and for devices as an `.ipa`, unsigned but with room for a signature, and omni-build gives both the same values, the icon in its asset catalog (`Assets.car`) among them, and the reason it uses the camera. The shell embeds a framework, `ShellKit`, two extensions, an App Clip and a watch app as Xcode's templates make them, which omni-build signs with the app, giving each an ID under the app's: `ShellNotifications`, a notification service extension, in `PlugIns/`, `ShellIntents`, an App Intents extension, which ExtensionKit loads from `Extensions/`, `ShellClip`, an App Clip, in `AppClips/`, whose signature and the app's name each other, and `ShellWatch`, a watch app, in `Watch/`, which pairs with the app. Code is nested in those in turn, as Xcode nests it: the watch app has widgets, `ShellWatchWidgets`, an extension in its own `PlugIns/`, whose ID is under the watch app's, and the App Clip embeds a copy of `ShellKit` in its own `Frameworks/`, since it downloads without the app. omni-build signs each before the bundle it's in. The watch app runs on watchOS 11, so omni-build signs its code for devices with SHA-1 hashes as well as SHA-256 ones, as `codesign` does, and its widgets, for watchOS 26, with SHA-256 alone. The shell also translates its name and the camera's reason into English (`InfoPlist.strings`), which omni-build takes out so iOS shows the app's. It signs the Simulator build ad hoc, and the `.ipa` with a key and provisioning profiles made for these tests (`app/signing-key.p12`, a keystore in Keychain Access's format, `app/OmniBuildE2E.mobileprovision` for the app, `app/ShellNotifications.mobileprovision` and `app/ShellIntents.mobileprovision` for its extensions, `app/ShellClip.mobileprovision` for its App Clip, `app/ShellWatch.mobileprovision` for its watch app, and `app/ShellWatchWidgets.mobileprovision` for the watch app's widgets, for a team that doesn't exist, made as below). Both keystores' password is `omnibuild-e2e`. The `test` stage checks each signature with `codesign --verify --strict --deep`. The Simulator can't run a build for devices, so for the `.ipa` it checks what the app's files say instead: `Info.plist`, the payload, and the entitlements in its signature. `expected()` in `src/repackage.ts` says what each platform's apps must report: the app's values where omni-build rewrites them, and the shell's elsewhere.

**No toolchain in reach.** GitHub's Linux and Windows runners come with a JDK and the Android SDK, and its macOS runners with Xcode. So the harness runs omni-build, to repackage and to install, with only `HOME`, `USERPROFILE`, the temporary folder variables and `SystemRoot`: no `PATH` (it can't start a program by name) and no `JAVA_HOME`, `ANDROID_HOME` or `DEVELOPER_DIR`. A step that quietly relied on a vendor tool fails in CI, not on a user's machine without one.

**As a build tool runs it.** The harness runs omni-build with `--json`, and checks the event each command ends with: the keystore `key create` wrote, the app `repackage` wrote, and the device `install` installed it on (`omniBuild()` in `src/repackage.ts`). So what omni-build reports is checked on all three OSes, as well as what it makes.

## Run it

```sh
npm run e2e -- --platform ios                    # All three stages. Needs Xcode with an iOS Simulator runtime
npm run e2e -- --platform android --avd <name>   # Boots that emulator (`emulator -list-avds`) and shuts it down after
npm run e2e -- --platform android                # Uses the one device adb sees, or $ANDROID_SERIAL
npm run e2e -- build --platform ios              # One stage; then `repackage`, then `test`
npm run e2e -- --help                            # Every option
```

`npm run e2e` from the repository root builds the omni-build binary first. The harness runs `target/release/omni-build`, or the binary `OMNI_BUILD_BINARY` names.

Android needs a JDK (17 or later, for Gradle, apksigner, jarsigner and bundletool) and the Android SDK with API 37 and build tools 35 or later (for `zipalign -P`), found through `ANDROID_HOME` or Android Studio's default location. The iOS tests create and reuse their own simulator, "OmniBuild E2E", so your own simulators stay untouched.

## What it checks

| Value                                   | iOS: where it comes from                                                                                                    | Android: where it comes from                                                                                 |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `id`                                    | `CFBundleIdentifier`                                                                                                        | the package name in the binary manifest                                                                      |
| `name`                                  | `CFBundleDisplayName`, translated in `en.lproj/InfoPlist.strings`, as the home screen shows it                              | the launcher activity's own label, in `resources.arsc`, as the launcher shows it                             |
| `version`                               | `CFBundleShortVersionString`                                                                                                | `versionName`                                                                                                |
| `build`                                 | `CFBundleVersion`                                                                                                           | `versionCode`                                                                                                |
| `payload`                               | `payload.txt` in the `.app`                                                                                                 | `assets/payload.txt` in the APK                                                                              |
| `icon`                                  | the centre pixel of the icon in `Assets.car` and of the PNGs beside it, one colour when they agree                          | the centre pixel of the manifest's `android:icon`: an adaptive icon, whose layers are vectors in the shell   |
| `signer`                                | `ad hoc` for the Simulator, or the certificate's name for devices, from `codesign`                                          | the signing certificate's name, from `apksigner` or, for an app bundle, `jarsigner`                          |
| `applicationIdentifier`                 | for devices only: the App ID in the signature's entitlements                                                                | not checked                                                                                                  |
| `cameraUsage`                           | `NSCameraUsageDescription`, translated in `en.lproj/InfoPlist.strings`                                                      | not checked                                                                                                  |
| `framework`                             | for the Simulator: a name from the embedded framework, `ShellKit`, which only loads signed                                  | not checked                                                                                                  |
| `extensionId`                           | the app extension's `CFBundleIdentifier`, under the app's, or the Simulator wouldn't install it                             | not checked                                                                                                  |
| `extensionApplicationIdentifier`        | for devices only: the App ID in the extension's signature's entitlements, from its own profile                              | not checked                                                                                                  |
| `intentsExtensionId`                    | the same for the ExtensionKit extension, in `Extensions/`                                                                   | not checked                                                                                                  |
| `intentsExtensionApplicationIdentifier` | the same for the ExtensionKit extension                                                                                     | not checked                                                                                                  |
| `appClipId`                             | the App Clip's `CFBundleIdentifier`, in `AppClips/`, under the app's, as Apple requires                                     | not checked                                                                                                  |
| `appClipApplicationIdentifier`          | for devices only: the App ID in the App Clip's signature's entitlements, from its own profile                               | not checked                                                                                                  |
| `appClipParent`                         | for devices only: the app the App Clip's signature names as its own (`com.apple.developer.parent-application-identifiers`)  | not checked                                                                                                  |
| `associatedAppClip`                     | for devices only: the App Clip the app's signature names in turn (`com.apple.developer.associated-appclip-app-identifiers`) | not checked                                                                                                  |
| `watchAppId`                            | the watch app's `CFBundleIdentifier`, in `Watch/`, under the app's                                                          | not checked                                                                                                  |
| `watchCompanion`                        | the app the watch app pairs with (`WKCompanionAppBundleIdentifier`): this one                                               | not checked                                                                                                  |
| `watchAppApplicationIdentifier`         | for devices only: the App ID in the watch app's signature's entitlements, from its own profile                              | not checked                                                                                                  |
| `watchWidgetsId`                        | the watch app's widgets' `CFBundleIdentifier`, in the watch app's `PlugIns/`, under the watch app's                         | not checked                                                                                                  |
| `watchWidgetsApplicationIdentifier`     | for devices only: the App ID in the widgets' signature's entitlements, from their own profile                               | not checked                                                                                                  |
| `provider`                              | not checked                                                                                                                 | the provider's authority, under the application ID                                                           |
| `permissions`                           | not checked                                                                                                                 | the permissions the manifest asks for, the shell's own among them                                            |
| `abis`                                  | not checked                                                                                                                 | the ABIs the APK has native libraries for (stand-ins, which the app never loads)                             |
| `urlSchemes`                            | the URL schemes in `Info.plist`'s URL types, which iOS opens the app's links with                                           | not checked                                                                                                  |
| `openedWith`                            | not checked: iOS asks before it opens an app's link, and no one is there to answer                                          | the link the harness opened the app with, `omnibuild-e2e://report`, which only the app's URL scheme leads to |

The app also has to install and launch at all, which checks the package's structure and signature. The shells' own values are in `src/shell.ts`, and must match the fixtures.

## Debug a failure

Each stage writes to `artifacts/<platform>/<stage>/` (in CI, the `e2e-logs-*` and `e2e-results-*` workflow artifacts). `commands.log` has every command and its output. The `test` stage adds a folder per app, named for its file, with `report.json`, what the app reported, and `screenshot.png`, the app on screen. When an app never reports, iOS adds `app.log` and any crash report, and Android's `logcat.log` has the crash, which the error also quotes.

## Things to know

- **No Maestro, on purpose.** The app reports what it found itself (a file in its data container on iOS, a logcat line on Android), so nothing has to drive the UI. That's faster and steadier, and needs nothing installed beyond the platforms' own tools. A test that has to interact with an app (tapping through a Lynx shell, accepting a permission) is where Maestro would come in.
- **The iOS shell travels as a zip of the `.app`**, made with `ditto`, which keeps each file's mode. A `.app` is a folder, and its executable's mode wouldn't survive a workflow artifact or a Windows file system.
- **The iOS icon check reads both copies of the icon:** the one in `Assets.car`, which the home screen draws, and the PNGs Xcode writes beside it for earlier versions of iOS. No public API reads the first (`actool` stores app icons as a kind of asset `UIImage(named:)` doesn't load), so the fixture calls CoreUI's `CUICatalog` by name, as the home screen does. The app reports one colour when the copies agree, and both when they don't.
- **The iOS signing key and profile are made for these tests.** CI has no certificate Apple issued, so the key's certificate is self-signed, and the profile is signed with the same key instead of by Apple, for a team that doesn't exist (`OMNIBUILD0`). No device would install the `.ipa`, but `codesign` checks its signature all the same, and the profile has what OmniBuild reads of a real one. To make them again:

```sh
cd packages/e2e/app
openssl req -x509 -newkey rsa:2048 -nodes -days 36500 -keyout key.pem -out cert.pem \
  -subj "/CN=OmniBuild E2E/OU=OMNIBUILD0/O=OmniBuild" \
  -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
  -addext "basicConstraints=critical,CA:FALSE"
openssl pkcs12 -export -legacy -inkey key.pem -in cert.pem -name "OmniBuild E2E" \
  -out signing-key.p12 -passout pass:omnibuild-e2e
python3 - <<'PLIST'
import plistlib, datetime, subprocess
cert = subprocess.run(['openssl', 'x509', '-in', 'cert.pem', '-outform', 'DER'], capture_output=True).stdout
plistlib.dump({
    'AppIDName': 'OmniBuild E2E', 'ApplicationIdentifierPrefix': ['OMNIBUILD0'], 'Platform': ['iOS'],
    'CreationDate': datetime.datetime(2026, 10, 19), 'ExpirationDate': datetime.datetime(2126, 10, 19),
    'DeveloperCertificates': [cert],
    'Entitlements': {'application-identifier': 'OMNIBUILD0.dev.omnibuild.e2e.app', 'com.apple.developer.team-identifier': 'OMNIBUILD0',
                     'get-task-allow': True, 'keychain-access-groups': ['OMNIBUILD0.*']},
    'Name': 'OmniBuild E2E', 'ProvisionedDevices': [], 'TeamIdentifier': ['OMNIBUILD0'], 'TeamName': 'OmniBuild',
    'TimeToLive': 36500, 'UUID': '0A1B2C3D-4E5F-4061-8273-94A5B6C7D8E9', 'Version': 1,
}, open('profile.plist', 'wb'), sort_keys=False)
PLIST
openssl cms -sign -nodetach -binary -outform DER -md sha256 -signer cert.pem -inkey key.pem \
  -in profile.plist -out OmniBuildE2E.mobileprovision
rm key.pem cert.pem profile.plist
```

The extensions' profiles are made the same way, with the same key and certificate (`openssl pkcs12 -legacy -in signing-key.p12 -nodes` gives both back): for the App ID `OMNIBUILD0.dev.omnibuild.e2e.app.ShellNotifications`, named `OmniBuild E2E Notifications`, with the UUID `1B2C3D4E-5F60-4172-8384-A5B6C7D8E9FA`, and written to `ShellNotifications.mobileprovision`, and for `OMNIBUILD0.dev.omnibuild.e2e.app.ShellIntents`, named `OmniBuild E2E Intents`, with the UUID `2C3D4E5F-6071-4283-9495-B6C7D8E9FA0B`, and written to `ShellIntents.mobileprovision`. The App Clip's is too, for `OMNIBUILD0.dev.omnibuild.e2e.app.Clip`, named `OmniBuild E2E Clip`, with the UUID `3D4E5F60-7182-4394-A5A6-C7D8E9FA0B1C`, and written to `ShellClip.mobileprovision`, with what Apple's profile for an App Clip's App ID has besides: `'com.apple.developer.on-demand-install-capable': True` and `'com.apple.developer.parent-application-identifiers': ['OMNIBUILD0.dev.omnibuild.e2e.app']` in its `Entitlements`. The watch app's is made as the extensions' are, for `OMNIBUILD0.dev.omnibuild.e2e.app.watchkitapp`, named `OmniBuild E2E Watch`, with the UUID `4E5F6071-8293-44A5-B6B7-D8E9FA0B1C2D`, and written to `ShellWatch.mobileprovision`, and so is its widgets', for `OMNIBUILD0.dev.omnibuild.e2e.app.watchkitapp.ShellWatchWidgets`, named `OmniBuild E2E Watch Widgets`, with the UUID `5F607182-93A4-45B6-C7C8-E9FA0B1C2D3E`, and written to `ShellWatchWidgets.mobileprovision`.

- **The fixtures are native projects**, so they keep Xcode's and Android Studio's formatting: oxfmt skips `fixtures/`.
- **The release APK is signed with the Android SDK's debug key**, so it installs without a keystore of its own. omni-build re-signs it with its own.
- **The harness uses only Node's built-in modules**, and Node runs its TypeScript as it is, stripping the types (Node 22.18 or later), so CI runs it without `npm ci` or a build.
- **The harness has unit tests too**, a `.test.ts` beside each module, which CI's coverage job holds to 100%. They stand in for Xcode, Gradle, the JDK, adb, simctl and the emulator, so they run on any machine in a fraction of a second and catch a broken harness before the slow jobs do. Run them with `npm test -w @omni-build/e2e`. As in the other packages, `cli.ts` only wires `run.ts` to the process, so the tests reach everything else in-process.
