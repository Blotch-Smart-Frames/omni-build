package dev.omnibuild.webview

import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.util.Log
import android.webkit.JavascriptInterface
import android.webkit.ValueCallback
import android.webkit.WebView
import org.json.JSONArray
import org.json.JSONObject
import java.lang.reflect.Method
import java.util.UUID

/**
 * The native modules OmniBuild added to the app, which the page calls from
 * JavaScript, as the WebView shell's runtime, `webview@1` (the manifest's
 * dev.omnibuild.runtime), has them:
 *
 * ```js
 * omniBuild.modules; // ['Haptics']
 * const result = await omniBuild.call('Haptics', 'impact', { style: 'heavy' });
 * ```
 *
 * Nothing compiled the shell against them, so it finds each class the
 * manifest lists (dev.omnibuild.modules) by name, in the DEX files OmniBuild
 * added beside the shell's, and makes it with the activity, if it takes a
 * Context, or with nothing. The page calls a module by its class's own name,
 * without its package (`Haptics` for `dev.example.haptics.Haptics`), and the
 * module answers through a method of its own, which its class declares as:
 *
 * ```kotlin
 * class Haptics(context: Context) {
 *     fun call(method: String, argument: String, reply: ValueCallback<String>, fail: ValueCallback<String>)
 * }
 * ```
 *
 * The argument is JSON, and the module replies once, with JSON to `reply`
 * or with why it failed to `fail`, from any thread. It's called on the main
 * thread. The page's call resolves with the JSON parsed, or rejects with an
 * Error of the module's message.
 *
 * Android gives a JavascriptInterface to every frame, so only the payload's
 * own pages reach native code: PayloadLoader puts this class's script in
 * each page it serves, with a token made for this launch, which a frame from
 * elsewhere, such as an embedded video, never sees.
 */
class NativeModules(private val activity: Activity, private val webView: WebView) {
    private val token = UUID.randomUUID().toString()

    /** Each module, by the name the page calls it by, and its call method. */
    private val modules: Map<String, Pair<Any, Method?>> = load()

    /** The interface the page's script calls, as `omniBuildNative`. */
    val bridge = Bridge()

    /**
     * `window.omniBuild`, for PayloadLoader to put in every page before the
     * page's own scripts. ES5, for Android 5.0's own WebView.
     */
    val script: String
        get() {
            val names = JSONArray(modules.keys.sorted()).toString()
            return """
                (function () {
                  var native = window.$INTERFACE, token = ${JSONObject.quote(token)}, calls = {}, next = 0;
                  window.omniBuild = {
                    modules: $names,
                    call: function (module, method, argument) {
                      return new Promise(function (resolve, reject) {
                        var id = ++next;
                        calls[id] = { resolve: resolve, reject: reject };
                        native.call(token, id, String(module), String(method),
                          JSON.stringify(argument === undefined ? null : argument));
                      });
                    },
                    _settle: function (id, result, error) {
                      var call = calls[id];
                      delete calls[id];
                      if (error !== null) {
                        call.reject(new Error(error));
                      } else {
                        call.resolve(JSON.parse(result));
                      }
                    },
                  };
                })();
            """.trimIndent()
        }

    private fun load(): Map<String, Pair<Any, Method?>> {
        @Suppress("DEPRECATION") // The flags overload is API 33 and up.
        val metaData = activity.packageManager
            .getApplicationInfo(activity.packageName, PackageManager.GET_META_DATA)
            .metaData
        val names = metaData?.getString(MODULES).orEmpty().split(',').filter { it.isNotEmpty() }
        val modules = mutableMapOf<String, Pair<Any, Method?>>()
        for (name in names) {
            val module = runCatching {
                val type = Class.forName(name)
                runCatching { type.getConstructor(Context::class.java).newInstance(activity) }
                    .getOrElse { type.getConstructor().newInstance() }
            }.onFailure {
                Log.e(LOG_TAG, "The app has no class $name for a native module", it)
            }.getOrNull() ?: continue
            val call = runCatching {
                module.javaClass.getMethod(
                    "call",
                    String::class.java,
                    String::class.java,
                    ValueCallback::class.java,
                    ValueCallback::class.java,
                )
            }.getOrNull()
            modules[name.substringAfterLast('.')] = module to call
        }
        return modules
    }

    /** Settles the page's call `id`, on the main thread, where the WebView is. */
    private fun settle(id: Int, result: String?, error: String?) {
        // The result is JSON, which the page parses: as a string here, so
        // that what a module replies can't run as script.
        val settled = "omniBuild._settle($id, ${JSONObject.quote(result ?: "null")}, " +
            "${error?.let(JSONObject::quote) ?: "null"})"
        webView.post { webView.evaluateJavascript(settled, null) }
    }

    inner class Bridge {
        /**
         * The page's call: Android calls this on a thread of its own, so the
         * module is called on the main thread, as a page's code expects.
         */
        @JavascriptInterface
        fun call(token: String, id: Int, name: String, method: String, argument: String) {
            if (token != this@NativeModules.token) {
                return
            }
            val (module, call) = modules[name] ?: return settle(id, null, "The app has no native module $name")
            if (call == null) {
                return settle(id, null, "$name has no call(method, argument, reply, fail) for the page to call")
            }
            // Once: a module that replies twice, or replies and fails, is
            // heard the first time.
            var settled = false
            val once = { result: String?, error: String? ->
                synchronized(this) {
                    if (!settled) {
                        settled = true
                        settle(id, result, error)
                    }
                }
            }
            webView.post {
                try {
                    call.invoke(
                        module,
                        method,
                        argument,
                        ValueCallback<String> { once(it, null) },
                        ValueCallback<String> { once(null, it ?: "$name.$method failed") },
                    )
                } catch (error: Exception) {
                    once(null, (error.cause ?: error).toString())
                }
            }
        }
    }

    companion object {
        /** The name the page reaches the bridge by. */
        const val INTERFACE = "omniBuildNative"
        private const val MODULES = "dev.omnibuild.modules"
        private const val LOG_TAG = "OmniBuildWebView"
    }
}
