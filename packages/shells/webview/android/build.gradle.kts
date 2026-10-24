// AGP 9 compiles Kotlin itself (built-in Kotlin), so there is no separate
// org.jetbrains.kotlin.android plugin to apply. The same AGP as the E2E
// fixture's, so the two move to a new one together.
plugins {
    id("com.android.application") version "9.4.1" apply false
}
