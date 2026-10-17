---
title: AngularLynx
description: How AngularLynx's builder will use OmniBuild for ng build --platform.
---

# AngularLynx

[AngularLynx](https://github.com/Blotch-Smart-Frames/AngularLynx) is OmniBuild's first user. Its builder, `@blotch/angular-lynx-build`, depends on the `omni-build` npm package and runs the binary, reading progress and errors as JSON lines so they show up like any other `ng build` output.

## Choosing the path

Repackaging becomes one path inside the builder, chosen automatically:

```text
ng build --platform android
   │
   ├─ ejected (android/ exists)?          → Gradle, as today
   ├─ native modules that need compiling? → generated project + Gradle, as today
   └─ otherwise                           → download the prebuilt shell (cached by checksum)
                                            → OmniBuild: rewrite config, add bundle, sign
                                            → dist/android/*.apk, *.aab
```

- **The options don't change.** The builder maps the `angular.json` options users already set (`appId`, `appName`, `icon`, `permissions`, signing environment variables) to OmniBuild's app config.
- **Shells are downloaded, not bundled** in the npm package: a GitHub release asset pinned by SHA-256 and cached per user. There are release and debug variants per platform, iOS device and Simulator shells, and one universal Android shell from which only the requested ABIs are kept.
- **The generated-project path stays**, for ejected apps and native source code, and as a fallback if a store starts rejecting repackaged binaries.
- **`ng deploy`** gets a target that uploads to TestFlight and a Google Play track through OmniBuild.

## Native modules from npm

A prebuilt shell can't compile a module's source, so the plan is for module packages to ship binaries built against a pinned Lynx version, and declare them in `package.json`:

```jsonc
"angularLynx": {
  "lynx": "4.1",
  "ios": { "frameworks": ["ios/Camera.xcframework"], "modules": ["CameraModule"] },
  "android": { "dex": "android/classes.dex", "jniLibs": "android/jni", "modules": ["dev.example.CameraModule"], "manifest": "android/AndroidManifest.xml" }
}
```

The builder collects these and passes them to OmniBuild, which embeds the binaries and writes the module list into the app's configuration. The shell registers each module by name at startup, so nothing is generated and nothing is compiled. This is [Phase 5](./roadmap.md#phases).
