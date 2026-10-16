package dev.omnibuild.shell

import android.content.Context
import android.content.pm.PackageInfo
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.os.Build
import org.json.JSONObject
import java.util.Locale

/**
 * What the app found in its own APK: every value OmniBuild may rewrite. The
 * keys match the expected values in packages/e2e/src/shell.js.
 */
object ShellReport {
    const val LOG_TAG = "OmniBuildE2E"

    fun read(context: Context): JSONObject {
        val packageManager = context.packageManager
        @Suppress("DEPRECATION") // The flags overload is API 33 and up.
        val packageInfo = packageManager.getPackageInfo(context.packageName, 0)
        val payload = runCatching {
            context.assets.open("payload.txt").bufferedReader().use { it.readText() }
        }.getOrNull()
        return JSONObject()
            .put("id", context.packageName)
            // The name under the icon in the launcher, from resources.arsc.
            .put("name", context.applicationInfo.loadLabel(packageManager).toString())
            .put("version", packageInfo.versionName ?: JSONObject.NULL)
            .put("build", versionCode(packageInfo).toString())
            .put("payload", payload ?: JSONObject.NULL)
            // The icon the launcher shows, found the way the launcher finds
            // it: through the manifest's android:icon resource.
            .put("icon", centerColor(packageManager.getApplicationIcon(context.applicationInfo)))
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
