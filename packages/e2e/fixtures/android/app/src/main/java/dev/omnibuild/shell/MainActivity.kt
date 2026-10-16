package dev.omnibuild.shell

import android.app.Activity
import android.os.Bundle
import android.util.Log
import android.widget.ScrollView
import android.widget.TextView

/**
 * A stand-in for the app shells OmniBuild repackages. As in a real shell,
 * nothing that differs between apps is in the Kotlin code: the application
 * ID, name, version, icon and payload are all data in the APK (the binary
 * manifest, resources.arsc and assets), which OmniBuild can rewrite without
 * a compiler. On launch the app reports what it finds there, and the E2E
 * harness checks that against what it built.
 *
 * A plain Activity with no AndroidX, so the APK holds nothing but the shell.
 */
class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val report = ShellReport.read(this)
        // The harness reads this line from logcat. The log rather than a
        // file, because a release build isn't debuggable, so `adb shell
        // run-as` can't read the app's private files.
        Log.i(ShellReport.LOG_TAG, report.toString())

        // The same report on screen, for the harness's screenshot and for
        // anyone running the app by hand.
        setContentView(
            ScrollView(this).apply {
                fitsSystemWindows = true
                addView(
                    TextView(this@MainActivity).apply {
                        text = report.toString(2)
                        textSize = 16f
                        setPadding(48, 48, 48, 48)
                    },
                )
            },
        )
    }
}
