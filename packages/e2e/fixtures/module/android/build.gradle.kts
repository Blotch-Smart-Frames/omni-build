// AGP 9 compiles Kotlin itself (built-in Kotlin), so there is no separate
// org.jetbrains.kotlin.android plugin to apply.
plugins {
    id("com.android.library") version "9.4.1" apply false
}
