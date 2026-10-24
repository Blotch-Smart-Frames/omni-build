package dev.omnibuild.webview

import android.content.res.AssetManager
import android.net.Uri
import android.webkit.MimeTypeMap
import android.webkit.WebResourceResponse
import java.io.ByteArrayInputStream
import java.io.FileNotFoundException

/**
 * Serves the payload, the APK's assets/, at `https://<host>/assets/`, as
 * androidx.webkit's WebViewAssetLoader does, without the library, which
 * needs Android 7.0. A page served over https from a host of its own has a
 * secure origin, `https://appassets.androidplatform.net`, where file:// URLs
 * have none: fetch, ES modules, localStorage and the APIs browsers keep for
 * secure contexts work as on the web.
 */
class PayloadLoader(private val assets: AssetManager, private val host: String) {
    /**
     * The response for a request the page makes, or null for one that isn't
     * for the payload, which the WebView loads from the network as usual.
     */
    fun load(uri: Uri): WebResourceResponse? {
        val path = uri.path
        if (uri.scheme != "https" || uri.host != host || path == null || !path.startsWith(PREFIX)) {
            return null
        }
        // A folder's index.html, as a web server serves it.
        val file = path.removePrefix(PREFIX).let { if (it.isEmpty() || it.endsWith("/")) "${it}index.html" else it }
        // AssetManager reads a path as it's given, so `..` could reach the
        // APK's other files.
        if (file.split('/').any { it == ".." }) {
            return notFound()
        }
        return try {
            val type = mimeType(file)
            WebResourceResponse(
                type,
                if (type.startsWith("text/") || type in UTF8_TYPES) "utf-8" else null,
                200,
                "OK",
                mapOf("Cache-Control" to "no-cache"),
                assets.open(file),
            )
        } catch (_: FileNotFoundException) {
            notFound()
        }
    }

    /**
     * A 404, so the page's fetch sees a missing file as a web server's would,
     * rather than the WebView going to a network the host isn't on.
     */
    private fun notFound() = WebResourceResponse(
        "text/plain",
        "utf-8",
        404,
        "Not Found",
        emptyMap(),
        ByteArrayInputStream(ByteArray(0)),
    )

    private companion object {
        const val PREFIX = "/assets/"

        /**
         * The web's own types for the files a web app is made of. Browsers
         * refuse a module script that isn't served as JavaScript, and
         * Android's own table, MimeTypeMap, lacks some of these before
         * later versions.
         */
        val WEB_TYPES = mapOf(
            "css" to "text/css",
            "htm" to "text/html",
            "html" to "text/html",
            "js" to "text/javascript",
            "json" to "application/json",
            "map" to "application/json",
            "mjs" to "text/javascript",
            "svg" to "image/svg+xml",
            "txt" to "text/plain",
            "wasm" to "application/wasm",
            "webmanifest" to "application/manifest+json",
            "woff" to "font/woff",
            "woff2" to "font/woff2",
            "xml" to "application/xml",
        )

        /** Types that are text, though not text/. */
        val UTF8_TYPES = setOf("application/json", "application/manifest+json", "application/xml", "image/svg+xml")

        fun mimeType(file: String): String {
            val extension = file.substringAfterLast('/').substringAfterLast('.', "").lowercase()
            return WEB_TYPES[extension]
                ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension)
                ?: "application/octet-stream"
        }
    }
}
