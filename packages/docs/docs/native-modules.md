---
title: Native modules
description: Add native code to an app with npm install, as prebuilt binaries OmniBuild puts in the app with no compiler, and what a module and a shell need for it.
---

# Native modules

:::warning[Phase 4]

`omni repackage` adds native modules to apps, but no shell OmniBuild ships registers them yet: the WebView shell will, and a workflow that builds modules for their authors is to come. Until then, a shell has to register them itself, as [below](#shells-that-take-native-modules).

:::

A prebuilt shell can't compile a module's Swift or Kotlin. So a native module is an npm package with its native code already built, for the shell's runtime, and OmniBuild adds it to the app as it repackages the shell:

```sh
npm install @example/haptics
omni repackage shell.apk --config app.json --out app.apk
```

```json
{
  "id": "com.example.tides",
  "payload": "dist",
  "packageJson": "package.json"
}
```

`packageJson` is the app's `package.json`. OmniBuild looks through its dependencies, its devDependencies and optional dependencies, and theirs in turn, where npm installed them in `node_modules`, as Node finds a package, pnpm's links included. A package with an `omniBuild` field is a native module. npm installs a package once for each version something asks for, and two versions of a module would define the same classes, so OmniBuild refuses two and asks for one, which an `overrides` entry in `package.json` gives. A dependency in `dependencies` that isn't installed is an error, since the app would be built without it: run `npm install` first. With `--json`, each module the app gets is a [`module-found` event](./build-tools.md).

A module has its binaries built against one runtime, the version of the shell's native code it was compiled for, which the shell names. OmniBuild refuses a module built for another, which would crash the app, rather than add it, and a shell that names no runtime takes no modules.

## Writing a module

A module says what it has for each platform in its `package.json`. The paths are in the package:

```json
{
  "name": "@example/haptics",
  "version": "1.0.0",
  "omniBuild": {
    "runtime": "example@1",
    "ios": {
      "frameworks": ["ios/Haptics.xcframework"],
      "modules": ["Haptics"]
    },
    "android": {
      "dex": ["android/classes.dex"],
      "jniLibs": "android/jni",
      "modules": ["com.example.haptics.Haptics"],
      "permissions": ["VIBRATE"]
    }
  }
}
```

| Field                 | What it is                                                                                                                                                                                           |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `runtime`             | The runtime the binaries were built against, as the shell names its own, such as `example@1`                                                                                                         |
| `ios.frameworks`      | `.xcframework` bundles, each with a dynamic framework for iOS devices and one for the Simulator. The one for the app's platform goes in the app's `Frameworks/`, and is signed with the app          |
| `ios.modules`         | What the shell registers, by name: for OmniBuild's shells, Objective-C class names, as `NSClassFromString` takes them                                                                                |
| `android.dex`         | DEX files, as d8 compiles them, which go beside the shell's: `classes2.dex` and on                                                                                                                   |
| `android.jniLibs`     | A folder of native libraries, one folder for each ABI, as Gradle's `jniLibs` are: `arm64-v8a/libhaptics.so`                                                                                          |
| `android.modules`     | What the shell registers, by name: for OmniBuild's shells, class names, as `Class.forName` takes them                                                                                                |
| `android.permissions` | The permissions the module asks for, as the app config's `android.permissions` names them (`CAMERA`, or `android.permission.CAMERA`), which the app asks for too, with the app's own and the shell's |

A module may have something for one platform only, and the other's apps don't get it.

### What OmniBuild checks

Each of these would otherwise fail on a device, at the latest when the app starts:

- **The runtime.** The module's has to be the one the shell names.
- **iOS frameworks are dynamic.** A static library needs a linker. Build the framework with `MACH_O_TYPE = mh_dylib`.
- **iOS frameworks have the app's architectures**, each the app's executable has: `arm64` for devices, and `arm64` and `x86_64` for the Simulator, if the shell has both.
- **iOS frameworks run where the app does.** A framework's `MinimumOSVersion` can't be later than the app's.
- **iOS frameworks are signed, or have room for a signature**, which OmniBuild replaces, as it does the shell's own: build them signed, if only ad hoc, or with `-Wl,-adhoc_codesign`. A framework with frameworks nested in it isn't signed yet.
- **DEX files run on the app's oldest Android.** The format's version says which Android reads a file: build it with `d8 --min-api` and the shell's `minSdk`. Android before 5.0 reads only an app's first DEX file, so OmniBuild adds modules with code to apps for Android 5.0 and later.
- **No class twice.** Android loads the first definition of a class it finds, so a module's copy of a library the shell has, such as the Kotlin standard library, would run against the shell's. Compile a module's DEX from its own classes alone, with the shell's libraries on d8's classpath, as a library module's are.
- **Native libraries for every ABI the app has.** A device of an ABI the app has libraries for, but a module hasn't, couldn't load the module's. Every module with native libraries, and the shell if it has any, needs them for each of the app's ABIs: those in `android.abis`, or else every ABI any of them has libraries for.

Not yet: Android resources and manifest entries other than permissions, such as a service or a receiver, and AndroidX libraries the shell doesn't have, which would need merging as Gradle merges them.

### Building one

A module's iOS code is a framework target, built as a dynamic framework for devices and for the Simulator, then put together with `xcodebuild -create-xcframework`. Its Android code is a library module's classes, compiled by Gradle, then by d8 into DEX without the libraries it uses, for the shell's oldest Android (`--min-api`). A workflow that builds both, as the [Shell workflow](./shells.md) builds shells, is to come.

A module's classes have to be ones the shell finds by name: on iOS, give a Swift class an Objective-C name, `@objc(Haptics)`, which `NSClassFromString("Haptics")` finds, and on Android keep its name from R8. What a class has to do to be registered is the shell's runtime's business.

## Shells that take native modules

A shell that registers native modules names its runtime, the version of its native code a module is built against, such as `example@1`. Give a new version a new name when modules built for the old one would break with it. OmniBuild lists the modules where the shell reads them as it starts:

| Platform | The runtime                                                                                       | The modules                                                                                                                                                                                       |
| -------- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| iOS      | `OmniBuildRuntime` in `Info.plist`                                                                | `OmniBuildModuleFrameworks` in `Info.plist`, the frameworks to load, by their folders' names in `Frameworks/`, then `OmniBuildModules`, the modules' names, each after any the shell lists itself |
| Android  | `<meta-data android:name="dev.omnibuild.runtime" android:value="example@1" />` in `<application>` | `<meta-data android:name="dev.omnibuild.modules">`, the modules' names, between commas, after any the shell lists itself                                                                          |

Nothing compiled the shell against the modules, so on iOS nothing loads their frameworks for it: the shell loads each, then finds each module's class by name.

```swift
let info = Bundle.main.infoDictionary ?? [:]
for name in info["OmniBuildModuleFrameworks"] as? [String] ?? [] {
    Bundle(url: Bundle.main.privateFrameworksURL!.appendingPathComponent(name))?.load()
}
for name in info["OmniBuildModules"] as? [String] ?? [] {
    if let module = NSClassFromString(name) as? NSObject.Type {
        register(module)
    }
}
```

Android loads every DEX file in the app with the shell's own, so the classes are there to find:

```kotlin
val metaData = packageManager.getApplicationInfo(packageName, PackageManager.GET_META_DATA).metaData
for (name in metaData?.getString("dev.omnibuild.modules").orEmpty().split(',').filter { it.isNotEmpty() }) {
    register(Class.forName(name))
}
```

A shell's own code that modules call has to keep its names: leave it out of R8's renaming (`-keep`), as a library's public API is.
