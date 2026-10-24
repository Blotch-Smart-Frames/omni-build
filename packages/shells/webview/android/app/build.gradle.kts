plugins {
    id("com.android.application")
}

android {
    // The package of the shell's own Kotlin code and R class. It stays fixed
    // when OmniBuild changes the applicationId, the app's ID.
    namespace = "dev.omnibuild.webview"
    // The E2E fixture's SDK levels (packages/e2e/fixtures/android), which the
    // platform watch reads. packages/e2e's tests check the two match, so a
    // move to a new API level moves both shells.
    compileSdk = 37

    defaultConfig {
        // Placeholders: OmniBuild gives each app its own ID, version and
        // build number when it repackages the shell.
        applicationId = "dev.omnibuild.webview"
        // Android 5.0, as the fixture. Its own WebView is Chrome 37 until the
        // device updates it, so a page meant to run there has to be written
        // for that. With no AndroidX, nothing holds the shell to a later one:
        // androidx.webkit, whose WebViewAssetLoader the shell would otherwise
        // use, needs Android 7.0.
        minSdk = 21
        targetSdk = 37
        versionCode = 1
        versionName = "1.0.0"
    }

    buildTypes {
        release {
            // Nothing to shrink: the shell is one activity and no libraries.
            isMinifyEnabled = false
            // The debug key (AGP creates ~/.android/debug.keystore), so the
            // release build installs without a keystore of its own. OmniBuild
            // signs every app it makes from the shell with the app's key.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
