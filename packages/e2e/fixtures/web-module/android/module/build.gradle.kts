plugins {
    id("com.android.library")
}

android {
    // The package of the module's Kotlin code, whose class the WebView shell
    // finds by name.
    namespace = "dev.omnibuild.webmodule"
    // The WebView shell's SDK levels (packages/shells/webview/android), which
    // packages/e2e's tests check every shell and module share.
    compileSdk = 37

    defaultConfig {
        minSdk = 21
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
