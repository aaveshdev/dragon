package org.dragonlang.runtime

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.app.DownloadManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.net.Uri
import android.net.http.SslError
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Message
import android.print.PrintManager
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.GeolocationPermissions
import android.webkit.JavascriptInterface
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.SslErrorHandler
import android.webkit.URLUtil
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebStorage
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.webkit.ScriptHandler
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.io.File
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger


interface WebExtension {
    fun attach(env: WebExtensionEnv) {}
    fun configure(cfg: JSONObject) {}
    fun rewriteUrl(url: String): String = url
    fun intercept(tab: String, url: String, pageHost: String?, accept: String?): Boolean = false
    fun documentScripts(tab: String, privateTab: Boolean): List<String> = emptyList()
    fun bridges(tab: String): Map<String, Any> = emptyMap()
    fun state(): JSONObject = JSONObject()
}

class WebExtensionEnv(
    val activity: ComponentActivity,
    val main: Handler,
    val count: (tab: String, name: String) -> Unit,
    val eval: (tab: String, js: String) -> Unit,
    val userAgent: (tab: String) -> String,
    val pageUrl: (tab: String) -> String,
    val emit: (JSONObject) -> Unit,
    val refresh: () -> Unit
)

object WebHosts {
    fun of(url: String): String? {
        val s = url.indexOf("://")
        if (s < 0) return null
        var e = url.length
        for (i in s + 3 until url.length) {
            val c = url[i]
            if (c == '/' || c == '?' || c == '#') { e = i; break }
        }
        var host = url.substring(s + 3, e)
        val at = host.lastIndexOf('@')
        if (at >= 0) host = host.substring(at + 1)
        if (host.startsWith("[")) return host.lowercase()
        val colon = host.lastIndexOf(':')
        if (colon >= 0) host = host.substring(0, colon)
        return host.lowercase().trimEnd('.')
    }
}

class WebConfig {
    @Volatile var thirdPartyCookies = false
    @Volatile var acceptCookies = true
    @Volatile var httpsUpgrade = true
    @Volatile var httpsOnly = false
    @Volatile var gpc = true
    @Volatile var javascript = true
    @Volatile var blockPopups = true
    @Volatile var safeBrowsing = true
    @Volatile var geolocation = "block"
    @Volatile var mediaPermission = "ask"
    @Volatile var autoplay = false
    @Volatile var drm = false
    @Volatile var reducedUserAgent = true
    @Volatile var textZoom = 100
    @Volatile var forceDark = false
    @Volatile var desktop = false

    fun update(o: JSONObject) {
        if (o.has("thirdPartyCookies")) thirdPartyCookies = o.optBoolean("thirdPartyCookies")
        if (o.has("acceptCookies")) acceptCookies = o.optBoolean("acceptCookies")
        if (o.has("httpsUpgrade")) httpsUpgrade = o.optBoolean("httpsUpgrade")
        if (o.has("httpsOnly")) httpsOnly = o.optBoolean("httpsOnly")
        if (o.has("gpc")) gpc = o.optBoolean("gpc")
        if (o.has("javascript")) javascript = o.optBoolean("javascript")
        if (o.has("blockPopups")) blockPopups = o.optBoolean("blockPopups")
        if (o.has("safeBrowsing")) safeBrowsing = o.optBoolean("safeBrowsing")
        if (o.has("geolocation")) geolocation = o.optString("geolocation", "block")
        if (o.has("mediaPermission")) mediaPermission = o.optString("mediaPermission", "ask")
        if (o.has("autoplay")) autoplay = o.optBoolean("autoplay")
        if (o.has("drm")) drm = o.optBoolean("drm")
        if (o.has("reducedUserAgent")) reducedUserAgent = o.optBoolean("reducedUserAgent")
        if (o.has("textZoom")) textZoom = o.optInt("textZoom", 100).coerceIn(50, 200)
        if (o.has("forceDark")) forceDark = o.optBoolean("forceDark")
        if (o.has("desktop")) desktop = o.optBoolean("desktop")
    }
}

class Tab(val id: String, val priv: Boolean) {
    lateinit var wv: WebView
    @Volatile var url = ""
    @Volatile var title = ""
    @Volatile var pageHost: String? = null
    var progress = 0
    var loading = false
    var canBack = false
    var canForward = false
    var secure = false
    var desktop = false
    var errorKind: String? = null
    var favicon: String? = null
    var findCount = 0
    var findIdx = 0
    val counts = java.util.concurrent.ConcurrentHashMap<String, AtomicInteger>()
    val pendingLoads = HashSet<String>()
    val upgraded = HashSet<String>()
    val httpOk = HashSet<String>()
    var scripts = ArrayList<ScriptHandler>()
    var lastInjected = ""
    var pendingScripts: List<String> = emptyList()
}

class DragonWeb(private val activity: ComponentActivity, private val main: Handler, private val docsDir: File) {
    private val ctx: Context = activity.applicationContext
    private val io = Executors.newCachedThreadPool()
    private val cfg = WebConfig()
    private val tabs = java.util.concurrent.ConcurrentHashMap<String, Tab>()
    private val events = ConcurrentLinkedQueue<JSONObject>()
    private var counter = 0
    private var seq = 0
    private val ext: WebExtension? = try {
        Class.forName("org.dragonlang.runtime.DragonWebExtension").getDeclaredConstructor().newInstance() as WebExtension
    } catch (_: Throwable) { null }
    private val totals = java.util.concurrent.ConcurrentHashMap<String, AtomicInteger>()
    private var privateIsolated = false
    private var privateReady = false
    private val totalUpgrades = AtomicInteger(0)
    private val faviconDir = File(docsDir, "favicons").apply { mkdirs() }
    private val thumbDir = File(ctx.cacheDir, "dragon_web").apply { mkdirs() }
    private var fullscreenView: View? = null
    private var fullscreenCb: WebChromeClient.CustomViewCallback? = null
    private var fullscreenBack: OnBackPressedCallback? = null

    private val defaultUa: String by lazy { WebSettings.getDefaultUserAgent(ctx) }
    private val chromeMajor: String by lazy { Regex("Chrome/(\\d+)").find(defaultUa)?.groupValues?.get(1) ?: "120" }

    init {
        ext?.attach(WebExtensionEnv(
            activity, main,
            count = { tab, name ->
                totals.getOrPut(name) { AtomicInteger(0) }.incrementAndGet()
                if (tab.isNotEmpty()) tabs[tab]?.counts?.getOrPut(name) { AtomicInteger(0) }?.incrementAndGet()
            },
            eval = { tab, js -> main.post { tabs[tab]?.wv?.evaluateJavascript(js, null) } },
            userAgent = { tab -> userAgent(tabs[tab]?.desktop == true) },
            pageUrl = { tab -> tabs[tab]?.url ?: "" },
            emit = { events.add(it) },
            refresh = { main.post { for (t in tabs.values) applyConfig(t) } }
        ))
    }


    fun viewFor(tabId: String): View? {
        val t = tabs[tabId] ?: return null
        (t.wv.parent as? ViewGroup)?.removeView(t.wv)
        t.wv.onResume()
        return t.wv
    }

    fun released(tabId: String) {
        val t = tabs[tabId] ?: return
        (t.wv.parent as? ViewGroup)?.removeView(t.wv)
    }


    fun handle(method: String, a: JSONObject, reply: (Any?) -> Unit): Boolean {
        if (!method.startsWith("web.")) return false
        val tab = tabs[a.optString("tab")]
        main.post {
            try {
                dispatch(method, a, tab, reply)
            } catch (e: Throwable) {
                reply(JSONObject().put("error", e.toString()))
            }
        }
        return true
    }

    private fun dispatch(method: String, a: JSONObject, tab: Tab?, reply: (Any?) -> Unit) {
        when (method) {
            "web.config" -> { configure(a); reply(true) }
            "web.state" -> reply(state())
            "web.open" -> {
                val id = a.optString("tab")
                val t = tabs[id] ?: createTab(id, a.optBoolean("private"), a.optBoolean("desktop"))
                val u = a.optString("url")
                if (u.isNotEmpty()) navigate(t, u)
                reply(true)
            }
            "web.load" -> { if (tab != null) navigate(tab, a.optString("url")); reply(true) }
            "web.back" -> { if (tab != null && tab.wv.canGoBack()) tab.wv.goBack(); reply(true) }
            "web.forward" -> { if (tab != null && tab.wv.canGoForward()) tab.wv.goForward(); reply(true) }
            "web.reload" -> { tab?.wv?.reload(); reply(true) }
            "web.stop" -> { tab?.wv?.stopLoading(); reply(true) }
            "web.close" -> { if (tab != null) closeTab(tab); reply(true) }
            "web.desktop" -> {
                if (tab != null) {
                    tab.desktop = a.optBoolean("on")
                    tab.wv.settings.userAgentString = userAgent(tab.desktop)
                    tab.wv.settings.useWideViewPort = true
                    tab.wv.settings.loadWithOverviewMode = true
                    tab.wv.reload()
                }
                reply(true)
            }
            "web.find" -> {
                if (tab != null) {
                    val t = a.optString("text")
                    if (t.isEmpty()) tab.wv.clearMatches() else tab.wv.findAllAsync(t)
                }
                reply(true)
            }
            "web.findNext" -> { tab?.wv?.findNext(a.optBoolean("forward", true)); reply(true) }
            "web.findClear" -> { tab?.wv?.clearMatches(); tab?.findCount = 0; reply(true) }
            "web.snapshot" -> reply(if (tab == null) null else snapshot(tab))
            "web.favicon" -> reply(if (tab?.favicon == null) null else JSONObject().put("path", tab.favicon))
            "web.clear" -> { clearData(a); reply(true) }
            "web.print" -> { if (tab != null) print(tab); reply(true) }
            "web.eval" -> {
                if (tab == null) reply(null)
                else tab.wv.evaluateJavascript(a.optString("code")) { v -> reply(JSONObject().put("value", v ?: "null")) }
            }
            "web.download" -> { download(a.optString("url"), null, null, null, null); reply(true) }
            else -> reply(null)
        }
    }


    private fun configure(o: JSONObject) {
        cfg.update(o)
        CookieManager.getInstance().setAcceptCookie(cfg.acceptCookies)
        ext?.configure(o)
        for (t in tabs.values) applyConfig(t)
    }

    private fun userAgent(desktop: Boolean): String = when {
        desktop -> "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/$chromeMajor.0.0.0 Safari/537.36"
        cfg.reducedUserAgent -> "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/$chromeMajor.0.0.0 Mobile Safari/537.36"
        else -> defaultUa
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun applyConfig(t: Tab) {
        val wv = t.wv
        val s = wv.settings
        s.javaScriptEnabled = cfg.javascript
        s.domStorageEnabled = true
        s.javaScriptCanOpenWindowsAutomatically = !cfg.blockPopups
        s.setSupportMultipleWindows(true)
        s.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
        s.allowFileAccess = false
        s.allowContentAccess = false
        @Suppress("DEPRECATION") run {
            s.allowFileAccessFromFileURLs = false
            s.allowUniversalAccessFromFileURLs = false
            s.saveFormData = false
            s.setGeolocationEnabled(cfg.geolocation != "block")
        }
        s.setSupportZoom(true)
        s.builtInZoomControls = true
        s.displayZoomControls = false
        s.useWideViewPort = true
        s.loadWithOverviewMode = true
        s.textZoom = cfg.textZoom
        s.mediaPlaybackRequiresUserGesture = !cfg.autoplay
        s.userAgentString = userAgent(t.desktop || cfg.desktop)
        if (Build.VERSION.SDK_INT >= 26) s.safeBrowsingEnabled = cfg.safeBrowsing
        CookieManager.getInstance().setAcceptThirdPartyCookies(wv, cfg.thirdPartyCookies && cfg.acceptCookies)
        try {
            if (WebViewFeature.isFeatureSupported(WebViewFeature.ALGORITHMIC_DARKENING))
                WebSettingsCompat.setAlgorithmicDarkeningAllowed(s, cfg.forceDark)
            if (WebViewFeature.isFeatureSupported(WebViewFeature.REQUESTED_WITH_HEADER_ALLOW_LIST))
                WebSettingsCompat.setRequestedWithHeaderOriginAllowList(s, emptySet())
        } catch (_: Throwable) {}
        applyScripts(t)
    }

    private fun applyScripts(t: Tab) {
        val list = ArrayList<String>(ext?.documentScripts(t.id, t.priv) ?: emptyList())
        val key = list.joinToString("\u0000").hashCode().toString() + ":" + list.size
        if (key == t.lastInjected) return
        t.lastInjected = key
        for (h in t.scripts) try { h.remove() } catch (_: Throwable) {}
        t.scripts.clear()
        if (list.isEmpty()) return
        if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
            for (js in list) try { t.scripts.add(WebViewCompat.addDocumentStartJavaScript(t.wv, js, setOf("*"))) } catch (_: Throwable) {}
        } else {
            t.pendingScripts = list
        }
    }


    @SuppressLint("SetJavaScriptEnabled", "AddJavascriptInterface")
    private fun createTab(id0: String?, priv: Boolean, desktop: Boolean): Tab {
        val id = id0 ?: ("x" + (++counter))
        val t = Tab(id, priv)
        t.desktop = desktop
        val wv = WebView(activity)
        t.wv = wv
        if (priv) {
            try {
                if (WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
                    androidx.webkit.ProfileStore.getInstance().getOrCreateProfile(PRIVATE_PROFILE)
                    WebViewCompat.setProfile(wv, PRIVATE_PROFILE)
                    privateIsolated = true
                }
            } catch (_: Throwable) {}
        }
        wv.setBackgroundColor(Color.TRANSPARENT)
        wv.isVerticalScrollBarEnabled = false
        wv.webViewClient = Client(t)
        wv.webChromeClient = Chrome(t)
        ext?.bridges(id)?.forEach { (name, obj) -> wv.addJavascriptInterface(obj, name) }
        wv.setDownloadListener { url, ua, cd, mime, _ -> download(url, ua, cd, mime, t) }
        wv.setFindListener { active, count, done -> t.findIdx = active + 1; t.findCount = count }
        wv.setOnLongClickListener { longPress(t) }
        tabs[id] = t
        applyConfig(t)
        return t
    }

    private fun closeTab(t: Tab) {
        tabs.remove(t.id)
        (t.wv.parent as? ViewGroup)?.removeView(t.wv)
        for (h in t.scripts) try { h.remove() } catch (_: Throwable) {}
        t.wv.stopLoading()
        t.wv.destroy()
        if (t.priv && tabs.values.none { it.priv }) wipePrivate()
    }

    private fun wipePrivate() {
        if (privateIsolated) {
            try {
                androidx.webkit.ProfileStore.getInstance().deleteProfile(PRIVATE_PROFILE)
            } catch (_: Throwable) {
                try {
                    val p = androidx.webkit.ProfileStore.getInstance().getProfile(PRIVATE_PROFILE)
                    p?.cookieManager?.removeAllCookies(null)
                    p?.webStorage?.deleteAllData()
                } catch (_: Throwable) {}
            }
        }
    }


    private fun navHeaders(): Map<String, String> {
        val h = HashMap<String, String>()
        if (cfg.gpc) { h["Sec-GPC"] = "1"; h["DNT"] = "1" }
        return h
    }

    private fun hostIsLocal(host: String): Boolean =
        host == "localhost" || host.endsWith(".local") || host.endsWith(".lan") || host.endsWith(".internal") ||
            Regex("^[0-9.]+$").matches(host) || host.startsWith("[")

    private fun rewrite(t: Tab, url: String): String {
        var u = url
        if (ext != null) u = ext.rewriteUrl(u)
        if (cfg.httpsUpgrade && u.startsWith("http://")) {
            val host = WebHosts.of(u)
            val portless = !u.substring(7).substringBefore('/').contains(':')
            if (host != null && !hostIsLocal(host) && portless && !t.httpOk.contains(host)) {
                u = "https://" + u.substring(7)
                t.upgraded.add(u)
                totalUpgrades.incrementAndGet()
            }
        }
        return u
    }

    private fun navigate(t: Tab, url0: String) {
        var url = url0.trim()
        if (url.isEmpty()) return
        if (!url.contains("://") && !url.startsWith("about:") && !url.startsWith("data:") && !url.startsWith("javascript:")) url = "https://$url"
        if (url.startsWith("javascript:")) return
        t.errorKind = null
        if (url.startsWith("http")) {
            url = rewrite(t, url)
            t.pendingLoads.add(url)
            t.wv.loadUrl(url, navHeaders())
        } else {
            t.wv.loadUrl(url)
        }
    }


    private inner class Client(val t: Tab) : WebViewClient() {

        override fun shouldInterceptRequest(view: WebView, req: WebResourceRequest): WebResourceResponse? {
            val url = req.url.toString()
            if (!url.startsWith("http")) return null
            if (req.isForMainFrame) { t.pageHost = WebHosts.of(url); return null }
            val e = ext ?: return null
            return if (e.intercept(t.id, url, t.pageHost, req.requestHeaders?.get("Accept"))) blocked() else null
        }

        private fun blocked() = WebResourceResponse("text/plain", "utf-8", 403, "Blocked", HashMap<String, String>(), ByteArrayInputStream(ByteArray(0)))

        override fun shouldOverrideUrlLoading(view: WebView, req: WebResourceRequest): Boolean {
            val url = req.url.toString()
            val scheme = req.url.scheme ?: ""
            if (scheme == "dragon-continue") {
                val target = url.removePrefix("dragon-continue:")
                WebHosts.of(target)?.let { t.httpOk.add(it) }
                navigate(t, target)
                return true
            }
            if (scheme != "http" && scheme != "https") {
                if (req.hasGesture()) openExternal(url)
                return true
            }
            if (!req.isForMainFrame) return false
            if (t.pendingLoads.remove(url)) return false
            val target = rewrite(t, url)
            if (target == url && !cfg.gpc) return false
            t.pendingLoads.add(target)
            view.loadUrl(target, navHeaders())
            return true
        }

        override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
            t.url = url
            t.pageHost = WebHosts.of(url)
            t.loading = true
            t.progress = 5
            t.secure = url.startsWith("https://")
            t.errorKind = null
            t.counts.clear()
            for (js in t.pendingScripts) view.evaluateJavascript(js, null)
        }

        override fun onPageFinished(view: WebView, url: String) {
            t.pendingLoads.clear()
            t.loading = false
            t.progress = 100
            t.canBack = view.canGoBack()
            t.canForward = view.canGoForward()
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) {
            if (!url.startsWith("data:")) t.url = url
            t.canBack = view.canGoBack()
            t.canForward = view.canGoForward()
        }

        override fun onReceivedError(view: WebView, req: WebResourceRequest, err: WebResourceError) {
            if (!req.isForMainFrame) return
            val url = req.url.toString()
            if (t.upgraded.remove(url) && !cfg.httpsOnly) {
                val http = "http://" + url.removePrefix("https://")
                WebHosts.of(url)?.let { t.httpOk.add(it) }
                t.pendingLoads.add(http)
                view.loadUrl(http, navHeaders())
                return
            }
            showError(view, url, err.description?.toString() ?: "The page could not be loaded", "error")
        }

        override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) {
            handler.cancel()
            val url = error.url ?: ""
            if (url == t.url || url == view.originalUrl || t.pendingLoads.contains(url) || t.upgraded.contains(url))
                showError(view, url, "The site's security certificate is not trusted, so the connection was stopped.", "ssl")
        }

        override fun onRenderProcessGone(view: WebView, detail: android.webkit.RenderProcessGoneDetail): Boolean {
            val u = t.url
            (view.parent as? ViewGroup)?.removeView(view)
            events.add(JSONObject().put("type", "crashed").put("tab", t.id))
            tabs.remove(t.id)
            view.destroy()
            val nt = createTab(t.id, t.priv, t.desktop)
            if (u.isNotEmpty() && !u.startsWith("data:")) navigate(nt, u)
            return true
        }

        override fun onReceivedHttpAuthRequest(view: WebView, handler: android.webkit.HttpAuthHandler, host: String, realm: String) {
            val box = LinearLayout(activity)
            box.orientation = LinearLayout.VERTICAL
            box.setPadding(48, 16, 48, 0)
            val u = EditText(activity); u.hint = "User name"
            val p = EditText(activity); p.hint = "Password"
            p.inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
            box.addView(u); box.addView(p)
            AlertDialog.Builder(activity).setTitle("Sign in to $host").setMessage(realm).setView(box)
                .setPositiveButton("Sign in") { _, _ -> handler.proceed(u.text.toString(), p.text.toString()) }
                .setNegativeButton("Cancel") { _, _ -> handler.cancel() }
                .setOnCancelListener { handler.cancel() }.show()
        }
    }

    private fun showError(view: WebView, url: String, message: String, kind: String) {
        val t = tabs.values.firstOrNull { it.wv === view } ?: return
        t.errorKind = kind
        t.loading = false
        t.progress = 100
        val host = WebHosts.of(url) ?: url
        val esc = { s: String -> s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;") }
        val http = if (url.startsWith("https://") && !cfg.httpsOnly && kind != "ssl")
            "<p><a class=b2 href=\"dragon-continue:http://${esc(url.removePrefix("https://"))}\">Continue over http (not secure)</a></p>" else ""
        val html = """<!doctype html><meta name=viewport content="width=device-width,initial-scale=1"><title>Can't open page</title>
<style>:root{color-scheme:light dark}body{font-family:system-ui,sans-serif;margin:0;padding:48px 24px;max-width:520px;margin:auto;line-height:1.5}
h1{font-size:22px;margin:0 0 8px}p{opacity:.8}.b{display:inline-block;padding:10px 18px;border-radius:20px;background:#6750a4;color:#fff;text-decoration:none;margin-top:8px}
.b2{color:#b3261e}code{word-break:break-all}</style>
<h1>${if (kind == "ssl") "Connection is not private" else "This page can't be opened"}</h1>
<p><code>${esc(host)}</code></p><p>${esc(message)}</p>
<a class=b href="${esc(url)}">Try again</a>$http"""
        view.loadDataWithBaseURL(url, html, "text/html", "UTF-8", url)
    }

    private fun openExternal(url: String) {
        try {
            val i = if (url.startsWith("intent:")) Intent.parseUri(url, Intent.URI_INTENT_SCHEME).apply {
                addCategory(Intent.CATEGORY_BROWSABLE); component = null; selector = null
            } else Intent(Intent.ACTION_VIEW, Uri.parse(url))
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(i)
        } catch (_: Exception) {}
    }


    private inner class Chrome(val t: Tab) : WebChromeClient() {
        override fun onProgressChanged(view: WebView, p: Int) { t.progress = p; if (p >= 100) t.loading = false }

        override fun onReceivedTitle(view: WebView, title: String?) { if (title != null && !title.startsWith("data:")) t.title = title }

        override fun onReceivedIcon(view: WebView, icon: Bitmap?) {
            if (icon == null) return
            val host = t.pageHost ?: return
            io.execute {
                try {
                    val f = File(faviconDir, host.replace(Regex("[^a-z0-9.\\-]"), "_") + ".png")
                    f.outputStream().use { icon.compress(Bitmap.CompressFormat.PNG, 100, it) }
                    t.favicon = f.absolutePath
                } catch (_: Exception) {}
            }
        }

        override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
            if (cfg.blockPopups && !isUserGesture) {
                events.add(JSONObject().put("type", "popupBlocked").put("tab", t.id))
                return false
            }
            val nt = createTab(null, t.priv, t.desktop)
            val transport = resultMsg.obj as WebView.WebViewTransport
            transport.webView = nt.wv
            resultMsg.sendToTarget()
            events.add(JSONObject().put("type", "newtab").put("tab", nt.id).put("private", t.priv).put("from", t.id))
            return true
        }

        override fun onCloseWindow(window: WebView) {
            val tab = tabs.values.firstOrNull { it.wv === window } ?: return
            events.add(JSONObject().put("type", "closed").put("tab", tab.id))
        }

        override fun onPermissionRequest(request: PermissionRequest) {
            val wants = request.resources.toList()
            val needed = ArrayList<String>()
            val granted = ArrayList<String>()
            for (r in wants) when (r) {
                PermissionRequest.RESOURCE_VIDEO_CAPTURE -> { if (cfg.mediaPermission == "block") continue; needed.add(android.Manifest.permission.CAMERA); granted.add(r) }
                PermissionRequest.RESOURCE_AUDIO_CAPTURE -> { if (cfg.mediaPermission == "block") continue; needed.add(android.Manifest.permission.RECORD_AUDIO); granted.add(r) }
                PermissionRequest.RESOURCE_PROTECTED_MEDIA_ID -> if (cfg.drm) granted.add(r)
            }
            if (granted.isEmpty()) { request.deny(); return }
            val host = request.origin.host ?: "this site"
            val what = if (needed.isEmpty()) "play protected content" else
                wants.filter { it in granted }.joinToString(" and ") { if (it == PermissionRequest.RESOURCE_VIDEO_CAPTURE) "use your camera" else "use your microphone" }
            val go = {
                ensureAndroidPermissions(needed) { ok ->
                    if (ok) request.grant(granted.toTypedArray()) else request.deny()
                }
            }
            if (needed.isEmpty()) go() else
                AlertDialog.Builder(activity).setTitle(host).setMessage("$host wants to $what.")
                    .setPositiveButton("Allow") { _, _ -> go() }.setNegativeButton("Block") { _, _ -> request.deny() }
                    .setOnCancelListener { request.deny() }.show()
        }

        override fun onGeolocationPermissionsShowPrompt(origin: String, callback: GeolocationPermissions.Callback) {
            if (cfg.geolocation == "block") { callback.invoke(origin, false, false); return }
            AlertDialog.Builder(activity).setTitle(WebHosts.of(origin) ?: origin).setMessage("Allow this site to see your location?")
                .setPositiveButton("Allow") { _, _ ->
                    ensureAndroidPermissions(listOf(android.Manifest.permission.ACCESS_FINE_LOCATION)) { ok -> callback.invoke(origin, ok, false) }
                }
                .setNegativeButton("Block") { _, _ -> callback.invoke(origin, false, false) }
                .setOnCancelListener { callback.invoke(origin, false, false) }.show()
        }

        override fun onShowFileChooser(view: WebView, cb: ValueCallback<Array<Uri>>, params: FileChooserParams): Boolean {
            val launcher = activity.activityResultRegistry.register("dragon_web_files_" + (seq++), ActivityResultContracts.StartActivityForResult()) { r ->
                cb.onReceiveValue(FileChooserParams.parseResult(r.resultCode, r.data))
            }
            try { launcher.launch(params.createIntent()) } catch (e: Exception) { cb.onReceiveValue(null); return false }
            return true
        }

        override fun onShowCustomView(view: View, callback: CustomViewCallback) {
            if (fullscreenView != null) { callback.onCustomViewHidden(); return }
            fullscreenView = view
            fullscreenCb = callback
            val decor = activity.window.decorView as FrameLayout
            view.setBackgroundColor(Color.BLACK)
            decor.addView(view, FrameLayout.LayoutParams(-1, -1))
            val c = WindowCompat.getInsetsController(activity.window, decor)
            c.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            c.hide(WindowInsetsCompat.Type.systemBars())
            fullscreenBack = object : OnBackPressedCallback(true) { override fun handleOnBackPressed() { onHideCustomView() } }
            activity.onBackPressedDispatcher.addCallback(activity, fullscreenBack!!)
        }

        override fun onHideCustomView() {
            val v = fullscreenView ?: return
            (v.parent as? ViewGroup)?.removeView(v)
            fullscreenView = null
            fullscreenCb?.onCustomViewHidden()
            fullscreenCb = null
            fullscreenBack?.remove()
            fullscreenBack = null
            val decor = activity.window.decorView
            WindowCompat.getInsetsController(activity.window, decor).show(WindowInsetsCompat.Type.systemBars())
        }

        override fun onJsAlert(view: WebView, url: String, message: String, result: JsResult): Boolean {
            AlertDialog.Builder(activity).setTitle(WebHosts.of(url) ?: "").setMessage(message)
                .setPositiveButton("OK") { _, _ -> result.confirm() }.setOnCancelListener { result.cancel() }.show()
            return true
        }

        override fun onJsConfirm(view: WebView, url: String, message: String, result: JsResult): Boolean {
            AlertDialog.Builder(activity).setTitle(WebHosts.of(url) ?: "").setMessage(message)
                .setPositiveButton("OK") { _, _ -> result.confirm() }.setNegativeButton("Cancel") { _, _ -> result.cancel() }
                .setOnCancelListener { result.cancel() }.show()
            return true
        }

        override fun onJsPrompt(view: WebView, url: String, message: String, def: String?, result: JsPromptResult): Boolean {
            val input = EditText(activity); input.setText(def ?: "")
            AlertDialog.Builder(activity).setTitle(WebHosts.of(url) ?: "").setMessage(message).setView(input)
                .setPositiveButton("OK") { _, _ -> result.confirm(input.text.toString()) }
                .setNegativeButton("Cancel") { _, _ -> result.cancel() }.setOnCancelListener { result.cancel() }.show()
            return true
        }
    }

    private fun ensureAndroidPermissions(perms: List<String>, done: (Boolean) -> Unit) {
        val missing = perms.filter { ContextCompat.checkSelfPermission(activity, it) != PackageManager.PERMISSION_GRANTED }
        if (missing.isEmpty()) { done(true); return }
        val launcher = activity.activityResultRegistry.register("dragon_web_perm_" + (seq++), ActivityResultContracts.RequestMultiplePermissions()) { r ->
            done(r.values.all { it })
        }
        launcher.launch(missing.toTypedArray())
    }


    private fun longPress(t: Tab): Boolean {
        val h = t.wv.hitTestResult
        val ev = JSONObject().put("type", "longpress").put("tab", t.id)
        when (h.type) {
            WebView.HitTestResult.SRC_ANCHOR_TYPE -> ev.put("kind", "link").put("url", h.extra ?: return false)
            WebView.HitTestResult.IMAGE_TYPE -> ev.put("kind", "image").put("url", h.extra ?: return false)
            WebView.HitTestResult.SRC_IMAGE_ANCHOR_TYPE -> {
                val msg = main.obtainMessage()
                t.wv.requestFocusNodeHref(msg)
                val link = msg.data?.getString("url")
                ev.put("kind", "imagelink").put("src", h.extra ?: "").put("url", link ?: (h.extra ?: ""))
            }
            else -> return false
        }
        events.add(ev)
        return true
    }

    private fun download(url: String, ua: String?, cd: String?, mime: String?, t: Tab?) {
        try {
            if (!url.startsWith("http")) { events.add(JSONObject().put("type", "download").put("ok", false).put("name", "Unsupported link")); return }
            val name = URLUtil.guessFileName(url, cd, mime)
            val req = DownloadManager.Request(Uri.parse(url))
            req.setMimeType(mime)
            CookieManager.getInstance().getCookie(url)?.let { req.addRequestHeader("Cookie", it) }
            req.addRequestHeader("User-Agent", ua ?: userAgent(false))
            req.setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
            req.setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
            (activity.getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager).enqueue(req)
            events.add(JSONObject().put("type", "download").put("ok", true).put("name", name).put("url", url).put("tab", t?.id ?: ""))
        } catch (e: Exception) {
            events.add(JSONObject().put("type", "download").put("ok", false).put("name", e.message ?: "Download failed"))
        }
    }

    private fun print(t: Tab) {
        val pm = activity.getSystemService(Context.PRINT_SERVICE) as PrintManager
        pm.print(t.title.ifEmpty { "page" }, t.wv.createPrintDocumentAdapter(t.title.ifEmpty { "page" }), null)
    }

    private fun snapshot(t: Tab): JSONObject? {
        val wv = t.wv
        val w = wv.width
        val h = wv.height
        if (w <= 0 || h <= 0) return null
        return try {
            val scale = 360f / w
            val bmp = Bitmap.createBitmap(360, maxOf(1, (h * scale).toInt()), Bitmap.Config.ARGB_8888)
            val c = Canvas(bmp)
            c.scale(scale, scale)
            wv.draw(c)
            val f = File(thumbDir, "thumb_" + t.id + ".jpg")
            f.outputStream().use { bmp.compress(Bitmap.CompressFormat.JPEG, 70, it) }
            JSONObject().put("path", f.absolutePath).put("t", System.currentTimeMillis())
        } catch (e: Throwable) { null }
    }

    private fun clearData(a: JSONObject) {
        if (a.optBoolean("history")) for (t in tabs.values) t.wv.clearHistory()
        if (a.optBoolean("cache")) {
            val any = tabs.values.firstOrNull()
            if (any != null) any.wv.clearCache(true) else WebView(activity).apply { clearCache(true); destroy() }
        }
        if (a.optBoolean("cookies")) { CookieManager.getInstance().removeAllCookies(null); CookieManager.getInstance().flush() }
        if (a.optBoolean("storage")) WebStorage.getInstance().deleteAllData()
        if (a.optBoolean("passwords")) android.webkit.WebViewDatabase.getInstance(ctx).clearHttpAuthUsernamePassword()
        if (a.optBoolean("private")) wipePrivate()
        if (a.optBoolean("cache") || a.optBoolean("storage")) thumbDir.listFiles()?.forEach { it.delete() }
        if (a.optBoolean("history") || a.optBoolean("cookies")) faviconDir.listFiles()?.forEach { it.delete() }
    }


    private fun state(): JSONObject {
        val o = JSONObject()
        val ts = JSONObject()
        for (t in tabs.values) {
            val wv = t.wv
            ts.put(
                t.id, JSONObject()
                    .put("url", t.url).put("title", t.title).put("progress", t.progress).put("loading", t.loading)
                    .put("canBack", wv.canGoBack()).put("canForward", wv.canGoForward()).put("secure", t.secure && t.errorKind == null)
                    .put("counts", JSONObject().also { c -> for ((k, v) in t.counts) c.put(k, v.get()) })
                    .put("favicon", t.favicon ?: JSONObject.NULL).put("private", t.priv).put("desktop", t.desktop || cfg.desktop)
                    .put("findCount", t.findCount).put("findIdx", t.findIdx).put("error", t.errorKind ?: JSONObject.NULL)
            )
        }
        o.put("tabs", ts)
        val tot = JSONObject().put("upgrades", totalUpgrades.get())
        for ((k, v) in totals) tot.put(k, v.get())
        o.put("totals", tot)
        o.put("privateIsolated", privateIsolated)
        ext?.state()?.let { x -> val it = x.keys(); while (it.hasNext()) { val k = it.next(); o.put(k, x.get(k)) } }
        val ev = JSONArray()
        while (true) { val x = events.poll() ?: break; ev.put(x) }
        o.put("events", ev)
        return o
    }

    companion object {
        const val PRIVATE_PROFILE = "dragon_private"
    }
}
