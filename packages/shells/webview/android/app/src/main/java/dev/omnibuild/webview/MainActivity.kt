package dev.omnibuild.webview

import android.annotation.SuppressLint
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.View
import android.view.WindowInsets
import android.webkit.ConsoleMessage
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher

/**
 * OmniBuild's WebView shell: a web app, the payload's `index.html` and the
 * files beside it, in a WebView that fills the screen. Nothing that differs
 * between apps is in this code. The application ID, name, version, icon and
 * the web app itself are data in the APK (the binary manifest,
 * resources.arsc and assets/), which OmniBuild rewrites without a compiler.
 *
 * A plain Activity with no AndroidX, so the APK holds nothing but the shell,
 * and it runs on Android 5.0 as the platform's own APIs do.
 */
class MainActivity : Activity() {
    private lateinit var webView: WebView

    /** Goes back in the page's history on Android 13 and later. */
    private var back: BackNavigation? = null

    /** The page's file input waiting for the files the user picks. */
    private var filePicked: ValueCallback<Array<Uri>>? = null

    // The page is the app, so it runs JavaScript.
    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // chrome://inspect only for a debuggable build, as Gradle's debug
        // build type makes, never for one in a store.
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) {
            WebView.setWebContentsDebuggingEnabled(true)
        }
        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            // The payload comes over https from PayloadLoader, so the page has
            // no need to read files or content providers directly.
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            webChromeClient = ShellChromeClient()
        }
        // The native modules OmniBuild added, which the payload's pages call
        // through window.omniBuild, the script PayloadLoader puts in each.
        val modules = NativeModules(this, webView)
        webView.addJavascriptInterface(modules.bridge, NativeModules.INTERFACE)
        webView.webViewClient = ShellClient(PayloadLoader(assets, HOST, modules.script))
        keepClearOfSystemBars(webView)
        setContentView(webView)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            back = BackNavigation(webView)
        }
        if (savedInstanceState == null || webView.restoreState(savedInstanceState) == null) {
            webView.loadUrl(START_URL)
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        webView.saveState(outState)
    }

    override fun onResume() {
        super.onResume()
        webView.onResume()
    }

    override fun onPause() {
        webView.onPause()
        super.onPause()
    }

    override fun onDestroy() {
        back?.unregister()
        webView.destroy()
        super.onDestroy()
    }

    /**
     * Before Android 13, the back button goes back in the page's history,
     * and leaves the app when there's none. From 13, BackNavigation does.
     */
    @Deprecated("Android 13 and later go back through OnBackInvokedCallback")
    override fun onBackPressed() {
        if (webView.canGoBack()) {
            webView.goBack()
        } else {
            @Suppress("DEPRECATION")
            super.onBackPressed()
        }
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode == PICK_FILES) {
            filePicked?.onReceiveValue(WebChromeClient.FileChooserParams.parseResult(resultCode, data))
            filePicked = null
        } else {
            super.onActivityResult(requestCode, resultCode, data)
        }
    }

    /**
     * Pads the page clear of the status and navigation bars, a display
     * cutout and the keyboard. From Android 15 the app draws behind the bars
     * (edge to edge), and the page would otherwise sit under them. Before,
     * the window keeps clear of them itself, and there's nothing to pad.
     */
    private fun keepClearOfSystemBars(view: View) {
        view.setOnApplyWindowInsetsListener { padded, insets ->
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val clear = insets.getInsets(
                    WindowInsets.Type.systemBars() or
                        WindowInsets.Type.displayCutout() or
                        WindowInsets.Type.ime(),
                )
                padded.setPadding(clear.left, clear.top, clear.right, clear.bottom)
                WindowInsets.CONSUMED
            } else {
                @Suppress("DEPRECATION")
                padded.setPadding(
                    insets.systemWindowInsetLeft,
                    insets.systemWindowInsetTop,
                    insets.systemWindowInsetRight,
                    insets.systemWindowInsetBottom,
                )
                @Suppress("DEPRECATION")
                insets.consumeSystemWindowInsets()
            }
        }
    }

    /**
     * Opens a link outside the app, in the browser or the app that handles
     * it, and says whether it did: links to other sites, `mailto:` and
     * `tel:` links and the like. The payload's own pages, and URLs that
     * only exist inside the page (`about:`, `data:`, `blob:`, `javascript:`),
     * stay in the WebView.
     */
    private fun openOutside(uri: Uri): Boolean {
        if ((uri.scheme == "https" && uri.host == HOST) || uri.scheme in IN_PAGE_SCHEMES) {
            return false
        }
        try {
            startActivity(Intent(Intent.ACTION_VIEW, uri))
        } catch (_: ActivityNotFoundException) {
            Log.w(LOG_TAG, "Nothing on the device opens $uri")
        }
        return true
    }

    private inner class ShellClient(private val payload: PayloadLoader) : WebViewClient() {
        override fun shouldInterceptRequest(
            view: WebView,
            request: WebResourceRequest,
        ): WebResourceResponse? = payload.load(request.url)

        // A frame inside the page (an embedded video, say) loads where it
        // points; only the page itself leaving goes outside the app.
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
            request.isForMainFrame && openOutside(request.url)

        @Deprecated("Android before 7.0 calls only this one")
        override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean =
            openOutside(Uri.parse(url))

        override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) {
            back?.update()
        }
    }

    private inner class ShellChromeClient : WebChromeClient() {
        /**
         * Logs what the page logs to its console, under the shell's own tag,
         * so `adb logcat -s OmniBuildWebView` shows a web app's console
         * alongside the system's, as Android's WebView does without a
         * WebChromeClient. Warnings and errors say where they came from.
         */
        override fun onConsoleMessage(message: ConsoleMessage): Boolean {
            val priority = when (message.messageLevel()) {
                ConsoleMessage.MessageLevel.ERROR -> Log.ERROR
                ConsoleMessage.MessageLevel.WARNING -> Log.WARN
                ConsoleMessage.MessageLevel.DEBUG -> Log.DEBUG
                else -> Log.INFO
            }
            val text = if (priority >= Log.WARN) {
                "${message.message()} (${message.sourceId()}:${message.lineNumber()})"
            } else {
                message.message()
            }
            Log.println(priority, LOG_TAG, text)
            return true
        }

        /**
         * An `<input type="file">`: the system's picker, for the types and
         * as many files as the input takes. A WebView has none of its own.
         */
        override fun onShowFileChooser(
            view: WebView,
            callback: ValueCallback<Array<Uri>>,
            params: FileChooserParams,
        ): Boolean {
            filePicked?.onReceiveValue(null)
            filePicked = callback
            return try {
                startActivityForResult(params.createIntent(), PICK_FILES)
                true
            } catch (_: ActivityNotFoundException) {
                filePicked = null
                false
            }
        }
    }

    /**
     * Android 13's back navigation: a callback that takes the back gesture
     * only while the page has history to go back to, so that the system
     * leaves the app, with its predictive animation, when it hasn't.
     */
    private class BackNavigation(private val webView: WebView) {
        private var registered = false
        private val callback = OnBackInvokedCallback { webView.goBack() }

        fun update() {
            val dispatcher = webView.findOnBackInvokedDispatcher() ?: return
            if (webView.canGoBack() && !registered) {
                dispatcher.registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                    callback,
                )
                registered = true
            } else if (!webView.canGoBack() && registered) {
                dispatcher.unregisterOnBackInvokedCallback(callback)
                registered = false
            }
        }

        fun unregister() {
            if (registered) {
                webView.findOnBackInvokedDispatcher()?.unregisterOnBackInvokedCallback(callback)
                registered = false
            }
        }
    }

    private companion object {
        /**
         * The host the payload is served from: the one androidx.webkit's
         * WebViewAssetLoader uses, which Google keeps for it, so no real site
         * is there. The page's origin is
         * `https://appassets.androidplatform.net`.
         */
        const val HOST = "appassets.androidplatform.net"

        /** The payload's `index.html`, in the APK's assets/. */
        const val START_URL = "https://$HOST/assets/index.html"

        /** The tag the page's console goes to logcat under. */
        const val LOG_TAG = "OmniBuildWebView"

        /** URLs that only mean something inside the page. */
        val IN_PAGE_SCHEMES = setOf("about", "blob", "data", "javascript")

        /** The request code for the file picker. */
        const val PICK_FILES = 1
    }
}
