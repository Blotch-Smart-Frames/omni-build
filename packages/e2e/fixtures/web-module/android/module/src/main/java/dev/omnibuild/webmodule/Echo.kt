package dev.omnibuild.webmodule

import android.content.Context
import android.webkit.ValueCallback
import org.json.JSONObject
import org.json.JSONTokener

/**
 * The E2E tests' native module for the WebView shell, as a module's author
 * writes one for its runtime, `webview@1`: a class the shell finds by its
 * name, and makes with the activity, which the page calls by its own name,
 * `Echo`, through `omniBuild.call`. Nothing compiled the shell against it.
 */
class Echo(private val context: Context) {
    /**
     * `echo` replies with what the page gave it, and the names of its class
     * and of the app it's in, which only the Context the shell made it with
     * knows. Any other method fails, as the page hears.
     */
    fun call(method: String, argument: String, reply: ValueCallback<String>, fail: ValueCallback<String>) {
        if (method != "echo") {
            fail.onReceiveValue("Echo has no method $method")
            return
        }
        val answer = JSONObject()
            .put("echo", JSONTokener(argument).nextValue())
            .put("from", "${javaClass.name} in ${context.packageName}")
        reply.onReceiveValue(answer.toString())
    }
}
