package ai.phantommesh.app

import android.content.Intent
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge

/**
 * SPEC-34 §10D — system back → React Router bridge.
 *
 * Tauri-generated MainActivity (copied over the generated one by
 * app/src-tauri/android/inject.sh). `TauriActivity` (← `WryActivity`) exposes
 * `open fun onWebViewCreate(webView: WebView)`; WryActivity's own back handling
 * is disabled (`handleBackNavigation = false`), so we own the back gesture.
 *
 * Native → JS: every system back press dispatches a `phantom://system-back`
 * DOM event into the WebView via `evaluateJavascript` (no Rust/plugin deps,
 * no internal Tauri event handles needed from the Activity).
 *
 * JS → native (root passthrough): when React Router is already at root it calls
 * `window.PhantomAndroidBack.passthroughSystemBack()`, which disables our
 * callback and re-runs Android's default back handling (exit app).
 */
class MainActivity : TauriActivity() {
    private var webView: WebView? = null

    // SPEC-34 §17-F: a route the focus tile (or any deep link) asked us to open.
    // Stored until the WebView exists, then dispatched as a `phantom://deep-link`
    // DOM event the SPA's MobileShell listens for to navigate React Router.
    private var pendingRoute: String? = null

    private val systemBackCallback = object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
            val wv = webView
            if (wv == null) {
                passthroughSystemBack()
                return
            }
            wv.post {
                wv.evaluateJavascript(
                    "window.dispatchEvent(new Event('phantom://system-back'))",
                    null
                )
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        onBackPressedDispatcher.addCallback(this, systemBackCallback)
        intent?.getStringExtra("focus_route")?.let { pendingRoute = it }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        intent.getStringExtra("focus_route")?.let { route ->
            // Already running (SINGLE_TOP) → dispatch immediately if the WebView
            // is up, else stash for onWebViewCreate.
            if (webView != null) dispatchDeepLink(route) else pendingRoute = route
        }
    }

    override fun onWebViewCreate(webView: WebView) {
        super.onWebViewCreate(webView)
        this.webView = webView
        webView.addJavascriptInterface(BackPassthroughBridge(), "PhantomAndroidBack")
        pendingRoute?.let { dispatchDeepLink(it); pendingRoute = null }
    }

    /** Fire a `phantom://deep-link` DOM event carrying the route to the SPA. */
    private fun dispatchDeepLink(route: String) {
        val wv = webView ?: return
        // route is an app-controlled literal ("/focus"); JSON-encode defensively.
        val safe = route.replace("\\", "\\\\").replace("'", "\\'")
        wv.post {
            wv.evaluateJavascript(
                "window.dispatchEvent(new CustomEvent('phantom://deep-link',{detail:{route:'$safe'}}))",
                null,
            )
        }
    }

    private fun passthroughSystemBack() {
        runOnUiThread {
            systemBackCallback.isEnabled = false
            try {
                onBackPressedDispatcher.onBackPressed()
            } finally {
                systemBackCallback.isEnabled = true
            }
        }
    }

    private inner class BackPassthroughBridge {
        @JavascriptInterface
        fun passthroughSystemBack() {
            this@MainActivity.passthroughSystemBack()
        }
    }
}
