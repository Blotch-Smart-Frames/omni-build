plugins {
    id("com.android.library")
}

android {
    // The package of the module's Kotlin code, whose class the fixture shell
    // finds by name.
    namespace = "dev.omnibuild.module"
    // The fixture shell's SDK levels (packages/e2e/fixtures/android), which
    // packages/e2e's tests check every shell and module share.
    compileSdk = 37

    defaultConfig {
        minSdk = 21
    }

    packaging {
        jniLibs {
            // The native libraries are stand-ins, there for their ABIs
            // (src/main/jniLibs), as the fixture shell's are, so there's
            // nothing to strip, and no NDK to strip them with.
            keepDebugSymbols += "**/*.so"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
