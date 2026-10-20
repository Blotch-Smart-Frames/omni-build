plugins {
    id("com.android.application")
}

android {
    // The package of the shell's own Kotlin code and R class. It stays fixed
    // when OmniBuild changes the applicationId, as in a real shell.
    namespace = "dev.omnibuild.shell"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.omnibuild.shell"
        // Android 5.0, as low as the AndroidX libraries go, rather than the
        // template's 24 (Android 7.0): an APK for earlier versions than 7.0
        // needs a v1 signature, and installs another way.
        minSdk = 21
        targetSdk = 37
        versionCode = 1
        versionName = "1.0.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // The debug key (AGP creates ~/.android/debug.keystore), so the
            // E2E tests can install the release build without a keystore of
            // their own. OmniBuild re-signs whatever it repackages anyway.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    packaging {
        jniLibs {
            // The native libraries are stand-ins, there for their ABIs
            // (src/main/jniLibs), so there's nothing to strip, and no NDK to
            // strip them with.
            keepDebugSymbols += "**/*.so"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
