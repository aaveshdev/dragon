package org.dragonlang.runtime

import android.content.Context
import android.content.res.Configuration
import android.os.Handler
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File
import kotlin.math.max

class DragonHost(private val activity: ComponentActivity) {
    var frame by mutableStateOf<JSONObject?>(null)
        private set
    var fatalError by mutableStateOf<String?>(null)
        private set

    val drawers = mutableStateMapOf<String, Boolean>()

    val listStates = HashMap<Int, LazyListState>()
    val scrollStates = HashMap<Int, ScrollState>()
    var uiScope: CoroutineScope? = null

    private val main = Handler(Looper.getMainLooper())
    private var started = false
    private var mediaJson = ""
    val plugins: List<DragonPlugin> get() = platform.plugins

    private val platform = DragonPlatform(activity) { id, value ->
        main.post {
            val json = when (value) {
                null -> ""
                is JSONObject -> value.toString()
                is Boolean -> value.toString()
                else -> JSONObject.quote(value.toString())
            }
            DragonNative.platformResult(id, json)
            refresh()
            drainPlatform()
        }
    }

    private val loop = object : Runnable {
        override fun run() {
            if (fatalError != null) return
            val next = DragonNative.tick(System.currentTimeMillis())
            if (DragonNative.isDirty()) refresh()
            drainPlatform()
            val delay = if (next in 0..99) max(next, 8L) else 100L
            main.postDelayed(this, delay)
        }
    }

    fun start(media: JSONObject) {
        if (started) return
        started = true
        mediaJson = media.toString()
        val appDir = installApp(activity)
        val docs = File(activity.filesDir, "dragon_docs").apply { mkdirs() }
        val err = try {
            DragonNative.init(appDir.absolutePath, docs.absolutePath, mediaJson)
        } catch (e: Throwable) {
            e.toString()
        }
        if (err.isNotEmpty()) {
            fatalError = err
            return
        }
        activity.lifecycle.addObserver(LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> { DragonNative.lifecycle("resumed"); refresh() }
                Lifecycle.Event.ON_PAUSE -> DragonNative.lifecycle("inactive")
                Lifecycle.Event.ON_STOP -> DragonNative.lifecycle("paused")
                else -> {}
            }
        })
        refresh()
        drainPlatform()
        main.post(loop)
    }

    fun stop() {
        main.removeCallbacks(loop)
    }

    fun updateMedia(media: JSONObject) {
        val s = media.toString()
        if (s == mediaJson || !started || fatalError != null) return
        mediaJson = s
        DragonNative.setMedia(s)
        refresh()
    }

    fun dispatch(evt: String?) {
        if (evt == null || evt.isEmpty()) return
        DragonNative.event(evt)
        refresh()
        drainPlatform()
    }

    fun back(): Boolean {
        val open = drawers.filterValues { it }.keys
        if (open.isNotEmpty()) {
            for (k in open) drawers[k] = false
            return true
        }
        val handled = DragonNative.back()
        refresh()
        return handled
    }

    fun canHandleBack(): Boolean {
        val f = frame ?: return false
        val ov = f.optJSONArray("overlays")
        return drawers.values.any { it } || f.optBoolean("canPop") || f.optBoolean("hasBack") || (ov != null && ov.length() > 0)
    }

    private fun refresh() {
        try {
            val f = JSONObject(DragonNative.render())
            frame = f
            val cmds = f.optJSONArray("commands")
            if (cmds != null) {
                for (i in 0 until cmds.length()) handleCommand(cmds.getJSONObject(i))
            }
        } catch (e: Exception) {
            fatalError = "Bad frame from the Dragon runtime: $e"
        }
    }

    private fun handleCommand(c: JSONObject) {
        when (c.optString("cmd")) {
            "drawer" -> drawers[c.optString("side", "start")] = c.optString("op") == "open"
            "scroll" -> {
                val id = c.optInt("id", -1)
                val scope = uiScope ?: return
                val animate = c.optBoolean("animate")
                val ls = listStates[id]
                val ss = scrollStates[id]
                scope.launch {
                    if (ls != null) {
                        val idx = when {
                            c.has("index") -> c.optInt("index")
                            c.optBoolean("end") -> Int.MAX_VALUE
                            else -> -1
                        }
                        if (idx >= 0) {
                            val target = if (idx == Int.MAX_VALUE) max(0, ls.layoutInfo.totalItemsCount - 1) else idx
                            if (animate) ls.animateScrollToItem(target) else ls.scrollToItem(target)
                        } else if (c.has("offset")) {
                            val off = c.optDouble("offset", 0.0).toFloat()
                            if (animate) ls.animateScrollBy(off - ls.firstVisibleItemScrollOffset) else ls.scrollBy(off - ls.firstVisibleItemScrollOffset)
                        }
                    } else if (ss != null) {
                        val target = if (c.optBoolean("end")) ss.maxValue else c.optDouble("offset", 0.0).toInt()
                        if (animate) ss.animateScrollTo(target) else ss.scrollTo(target)
                    }
                }
            }
        }
    }

    private fun drainPlatform() {
        val txt = DragonNative.platformCalls()
        if (txt.length <= 2) return
        try {
            val arr = org.json.JSONArray(txt)
            for (i in 0 until arr.length()) platform.handle(arr.getJSONObject(i))
        } catch (e: Exception) {
        }
    }

    companion object {
        fun installApp(ctx: Context): File {
            val dest = File(ctx.filesDir, "dragon_app")
            val stamp = try {
                ctx.packageManager.getPackageInfo(ctx.packageName, 0).lastUpdateTime.toString()
            } catch (e: Exception) {
                "0"
            }
            val marker = File(dest, ".installed")
            if (marker.exists() && marker.readText() == stamp) return dest
            dest.deleteRecursively()
            dest.mkdirs()
            copyAssetDir(ctx, "dragon", dest)
            marker.writeText(stamp)
            return dest
        }

        private fun copyAssetDir(ctx: Context, assetPath: String, out: File) {
            val names = ctx.assets.list(assetPath) ?: return
            out.mkdirs()
            for (n in names) {
                val child = "$assetPath/$n"
                val sub = ctx.assets.list(child)
                if (sub != null && sub.isNotEmpty()) {
                    copyAssetDir(ctx, child, File(out, n))
                } else {
                    ctx.assets.open(child).use { input -> File(out, n).outputStream().use { input.copyTo(it) } }
                }
            }
        }
    }
}

@Composable
fun DragonApp(host: DragonHost) {
    val density = LocalDensity.current
    val cfg = LocalConfiguration.current
    val ctx = LocalContext.current
    val sys = WindowInsets.systemBars
    val ime = WindowInsets.ime
    val scope = rememberCoroutineScope()
    host.uiScope = scope

    val dark = (cfg.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
    val media = JSONObject()
        .put("width", cfg.screenWidthDp)
        .put("height", cfg.screenHeightDp)
        .put("pixelRatio", density.density.toDouble())
        .put("top", with(density) { sys.getTop(density).toDp().value.toDouble() })
        .put("bottom", with(density) { sys.getBottom(density).toDp().value.toDouble() })
        .put("left", 0)
        .put("right", 0)
        .put("keyboard", with(density) { ime.getBottom(density).toDp().value.toDouble() })
        .put("brightness", if (dark) "dark" else "light")
        .put("textScale", cfg.fontScale.toDouble())
        .put("locale", cfg.locales.get(0).toString())

    LaunchedEffect(Unit) { host.start(media) }
    LaunchedEffect(media.toString()) { host.updateMedia(media) }

    BackHandler(enabled = host.canHandleBack()) {
        if (!host.back()) (ctx as? android.app.Activity)?.finish()
    }

    val f = host.frame
    val err = host.fatalError
    if (err != null) {
        FatalErrorScreen(err)
        return
    }
    if (f == null) {
        Box(Modifier.fillMaxSize())
        return
    }
    val scheme = dragonColorScheme(f.optJSONObject("theme"), dark)
    MaterialTheme(colorScheme = scheme) {
        Surface(Modifier.fillMaxSize(), color = scheme.background) {
            DragonFrame(host, f)
        }
    }
}
