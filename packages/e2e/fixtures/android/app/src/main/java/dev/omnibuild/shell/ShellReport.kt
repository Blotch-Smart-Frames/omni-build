package dev.omnibuild.shell

import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.content.pm.ResolveInfo
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.os.Build
import org.json.JSONObject
import java.util.Locale
import java.util.zip.ZipFile

/**
 * What the app found in its own APK: every value OmniBuild may rewrite. The
 * keys match the expected values in packages/e2e/src/shell.js.
 */
object ShellReport {
    const val LOG_TAG = "OmniBuildE2E"

    /** `openedWith` is the link the app was opened with, if it was. */
    fun read(context: Context, openedWith: String?): JSONObject {
        val packageManager = context.packageManager
        @Suppress("DEPRECATION") // The flags overload is API 33 and up.
        val packageInfo = packageManager.getPackageInfo(
            context.packageName,
            PackageManager.GET_PERMISSIONS or PackageManager.GET_PROVIDERS,
        )
        val payload = runCatching {
            context.assets.open("payload.txt").bufferedReader().use { it.readText() }
        }.getOrNull()
        return JSONObject()
            .put("id", context.packageName)
            // The name under the icon in the launcher, from resources.arsc:
            // the launcher activity's label, which is the application's
            // unless the activity has one of its own.
            .put("name", launcherLabel(context))
            .put("version", packageInfo.versionName ?: JSONObject.NULL)
            .put("build", versionCode(packageInfo).toString())
            .put("payload", payload ?: JSONObject.NULL)
            // The icon the launcher shows, found the way the launcher finds
            // it: through the manifest's android:icon resource.
            .put("icon", centerColor(packageManager.getApplicationIcon(context.applicationInfo)))
            // Values in the manifest that Gradle derived from the application
            // ID, which have to change with it.
            .put("provider", packageInfo.providers?.joinToString { it.authority } ?: JSONObject.NULL)
            .put("permissions", packageInfo.requestedPermissions?.sorted()?.joinToString() ?: "")
            // The ABIs the APK has native libraries for: its lib/ folders.
            .put("abis", abis(context))
            // Only an intent filter for the app's URL scheme leads its link
            // here.
            .put("openedWith", openedWith ?: JSONObject.NULL)
            // Which way up the launcher activity's screen is, as Android
            // read it from the manifest.
            .put("orientation", orientation(launcherActivity(context).activityInfo))
    }

    private fun launcherActivity(context: Context): ResolveInfo {
        val launcher = Intent(Intent.ACTION_MAIN)
            .addCategory(Intent.CATEGORY_LAUNCHER)
            .setPackage(context.packageName)
        @Suppress("DEPRECATION") // The flags overload is API 33 and up.
        return context.packageManager.queryIntentActivities(launcher, 0).first()
    }

    private fun launcherLabel(context: Context): String =
        launcherActivity(context).loadLabel(context.packageManager).toString()

    /** Its `android:screenOrientation`, as the manifest names it. */
    private fun orientation(activity: ActivityInfo): String =
        when (activity.screenOrientation) {
            ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED -> "unspecified"
            ActivityInfo.SCREEN_ORIENTATION_PORTRAIT -> "portrait"
            ActivityInfo.SCREEN_ORIENTATION_USER_LANDSCAPE -> "userLandscape"
            else -> activity.screenOrientation.toString()
        }

    private fun abis(context: Context): String =
        ZipFile(context.applicationInfo.sourceDir).use { apk ->
            apk.entries().asSequence()
                .map { it.name }
                .filter { it.startsWith("lib/") }
                .map { it.split('/')[1] }
                .distinct()
                .sorted()
                .joinToString()
        }

    private fun versionCode(info: PackageInfo): Long =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.longVersionCode
        } else {
            @Suppress("DEPRECATION")
            info.versionCode.toLong()
        }

    /**
     * The colour of the drawable's centre pixel as `#RRGGBB`. The fixture
     * icons are solid colours, so this tells the harness which icon the app
     * got without comparing images.
     */
    private fun centerColor(drawable: Drawable): String {
        val width = drawable.intrinsicWidth.coerceAtLeast(1)
        val height = drawable.intrinsicHeight.coerceAtLeast(1)
        val pixel = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        // Offset the drawable so that its centre pixel is the bitmap's only one.
        drawable.setBounds(-width / 2, -height / 2, width - width / 2, height - height / 2)
        drawable.draw(Canvas(pixel))
        return String.format(Locale.ROOT, "#%06X", pixel.getPixel(0, 0) and 0xFFFFFF)
    }
}
