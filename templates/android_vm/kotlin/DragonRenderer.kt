@file:OptIn(
    androidx.compose.foundation.ExperimentalFoundationApi::class,
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.material.ExperimentalMaterialApi::class,
    androidx.compose.foundation.layout.ExperimentalLayoutApi::class,
    androidx.compose.ui.text.ExperimentalTextApi::class
)

package org.dragonlang.runtime

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.FlowColumn
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyHorizontalGrid
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.VerticalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.pullrefresh.PullRefreshIndicator
import androidx.compose.material.pullrefresh.pullRefresh
import androidx.compose.material.pullrefresh.rememberPullRefreshState
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ElevatedButton
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LargeFloatingActionButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedIconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.ScrollableTabRow
import androidx.compose.material3.Slider
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import kotlinx.coroutines.flow.distinctUntilChanged
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.roundToInt

private const val INF = 1.0e9f

private fun JSONObject.str(k: String): String? = if (has(k) && !isNull(k)) optString(k) else null
private fun JSONObject.f(k: String): Float? = if (has(k) && !isNull(k)) optDouble(k).toFloat() else null
private fun JSONObject.i(k: String): Int? = if (has(k) && !isNull(k)) optInt(k) else null
private fun JSONObject.b(k: String): Boolean? = if (has(k) && !isNull(k)) optBoolean(k) else null
private fun JSONObject.o(k: String): JSONObject? = if (has(k) && !isNull(k)) optJSONObject(k) else null
private fun JSONObject.arr(k: String): JSONArray? = if (has(k) && !isNull(k)) optJSONArray(k) else null

private inline fun JSONArray.forEachObject(block: (Int, JSONObject) -> Unit) {
    for (i in 0 until length()) {
        val o = optJSONObject(i) ?: continue
        block(i, o)
    }
}

private fun color(s: String?): Color? {
    if (s == null) return null
    return try {
        Color(android.graphics.Color.parseColor(s))
    } catch (e: Exception) {
        null
    }
}

private fun dp(v: Float?): Dp? = if (v == null) null else v.dp

private fun isFill(v: Float?): Boolean = v != null && v >= INF

fun dragonColorScheme(theme: JSONObject?, systemDark: Boolean): ColorScheme {
    val s = theme?.o("scheme")
    val dark = theme?.b("dark") ?: systemDark
    if (s == null) return if (dark) darkColorScheme() else lightColorScheme()
    fun c(k: String, d: Color): Color = color(s.str(k)) ?: d
    val base = if (dark) darkColorScheme() else lightColorScheme()
    val surface = c("surface", base.surface)
    return base.copy(
        primary = c("primary", base.primary),
        onPrimary = c("onPrimary", base.onPrimary),
        primaryContainer = c("primaryContainer", base.primaryContainer),
        onPrimaryContainer = c("onPrimaryContainer", base.onPrimaryContainer),
        secondary = c("secondary", base.secondary),
        onSecondary = c("onSecondary", base.onSecondary),
        secondaryContainer = c("secondaryContainer", base.secondaryContainer),
        onSecondaryContainer = c("onSecondaryContainer", base.onSecondaryContainer),
        tertiary = c("tertiary", base.tertiary),
        error = c("error", base.error),
        onError = c("onError", base.onError),
        background = color(theme?.str("background")) ?: c("background", surface),
        onBackground = c("onBackground", base.onBackground),
        surface = surface,
        onSurface = c("onSurface", base.onSurface),
        surfaceVariant = c("surfaceVariant", base.surfaceVariant),
        onSurfaceVariant = c("onSurfaceVariant", base.onSurfaceVariant),
        outline = c("outline", base.outline),
        outlineVariant = c("outlineVariant", base.outlineVariant),
        inverseSurface = c("inverseSurface", base.inverseSurface),
        inverseOnSurface = c("onInverseSurface", base.inverseOnSurface)
    )
}

@Composable
fun FatalErrorScreen(message: String) {
    Box(Modifier.fillMaxSize().background(Color(0xFF1B1B1F)).padding(24.dp)) {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            Text("Dragon error", color = Color(0xFFFF8A80), fontSize = 20.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(12.dp))
            Text(message, color = Color.White, fontSize = 13.sp)
        }
    }
}

enum class Parent { COL, ROW, STACK, OTHER }

class Ctx(
    val host: DragonHost,
    val parent: Parent,
    val row: RowScope?,
    val col: ColumnScope?,
    val box: BoxScope?,
    val topInset: Float,
    val bottomInset: Float,
    val consumedTop: Float,
    val consumedBottom: Float,
    val closeSide: String? = null,
    val unbH: Boolean = false
) {
    fun with(
        parent: Parent = this.parent,
        row: RowScope? = null,
        col: ColumnScope? = null,
        box: BoxScope? = null,
        consumedTop: Float = this.consumedTop,
        consumedBottom: Float = this.consumedBottom,
        closeSide: String? = this.closeSide,
        unbH: Boolean = this.unbH
    ) = Ctx(host, parent, row, col, box, topInset, bottomInset, consumedTop, consumedBottom, closeSide, unbH)
}

private fun Ctx.fire(id: String?) {
    if (id == null) return
    if (closeSide != null) host.drawers[closeSide] = false
    host.dispatch(id)
}

private fun Ctx.fire(id: String?, payload: Any) {
    if (id == null) return
    host.dispatch("$id:$payload")
}

private class PageState(var key: String, var depth: Int) {
    var forward = false
    var first = true
}

@Composable
fun DragonFrame(host: DragonHost, f: JSONObject) {
    val density = LocalDensity.current
    val sys = WindowInsets.systemBars
    val top = with(density) { sys.getTop(density).toDp().value }
    val bottom = with(density) { sys.getBottom(density).toDp().value }
    val ctx = Ctx(host, Parent.OTHER, null, null, null, top, bottom, 0f, 0f)

    Box(Modifier.fillMaxSize()) {
        val root = f.o("root")
        if (root != null) {
            val pageKey = root.str("key") ?: "root"
            val depth = root.i("depth") ?: 1
            val state = remember { PageState(pageKey, depth) }
            if (state.key != pageKey) {
                state.forward = depth > state.depth
                state.first = false
                state.key = pageKey
                state.depth = depth
            }
            key(state.key) {
                AnimatedPage(forward = state.forward, animate = !state.first) {
                    val content = root.o("content") ?: root
                    Node(content, ctx)
                }
            }
        }
        val overlays = f.arr("overlays")
        if (overlays != null) {
            overlays.forEachObject { _, o -> key(o.optInt("id")) { OverlayHost(o, ctx) } }
        }
        val err = f.str("error")
        if (err != null) {
            Box(
                Modifier.align(Alignment.BottomCenter).fillMaxWidth().background(Color(0xE6B71C1C)).padding(10.dp)
            ) {
                Text(err, color = Color.White, fontSize = 11.sp, maxLines = 8)
            }
        }
    }
}

@Composable
private fun AnimatedPage(forward: Boolean, animate: Boolean, content: @Composable () -> Unit) {
    val anim = remember { Animatable(if (animate) 1f else 0f) }
    LaunchedEffect(Unit) { if (animate) anim.animateTo(0f, tween(260)) }
    Box(
        Modifier.fillMaxSize().graphicsLayer {
            if (forward) {
                translationX = anim.value * size.width
            } else {
                alpha = 1f - anim.value * 0.5f
            }
        }
    ) { content() }
}

@Composable
private fun OverlayHost(o: JSONObject, ctx: Ctx) {
    val kind = o.str("kind") ?: "dialog"
    val content = o.o("content")
    val dismissible = o.b("dismissible") ?: true
    val onDismiss = o.str("onDismiss")
    val c = ctx.with(parent = Parent.OTHER)
    when (kind) {
        "snackbar" -> {
            Box(Modifier.fillMaxSize().navigationBarsPadding(), contentAlignment = Alignment.BottomCenter) {
                Box(Modifier.padding(bottom = 80.dp)) { Node(content, c) }
            }
        }
        else -> {
            val scrim = color(o.str("barrierColor")) ?: Color(0x66000000)
            Box(Modifier.fillMaxSize()) {
                Box(
                    Modifier.fillMaxSize().background(scrim).pointerInput(Unit) {
                        detectTapGestures { if (dismissible) ctx.fire(onDismiss) }
                    }
                )
                if (kind == "dialog") {
                    Box(
                        Modifier.fillMaxSize().padding(24.dp).imePadding(),
                        contentAlignment = Alignment.Center
                    ) {
                        Box(Modifier.pointerInput(Unit) { detectTapGestures { } }) { Node(content, c) }
                    }
                } else {
                    val bg = color(o.str("bg")) ?: MaterialTheme.colorScheme.surface
                    val full = o.b("fullscreen") ?: false
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
                        Surface(
                            color = bg,
                            shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
                            modifier = Modifier.fillMaxWidth()
                                .then(if (full) Modifier.fillMaxHeight(0.92f) else Modifier.heightIn(max = 560.dp))
                                .pointerInput(Unit) { detectTapGestures { } }
                        ) {
                            Column(Modifier.navigationBarsPadding().imePadding()) {
                                if (o.b("dragHandle") == true) {
                                    Box(
                                        Modifier.padding(vertical = 12.dp).align(Alignment.CenterHorizontally)
                                            .size(32.dp, 4.dp).clip(RoundedCornerShape(2.dp))
                                            .background(MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f))
                                    )
                                }
                                Node(content, c.with(parent = Parent.COL))
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun Node(n: JSONObject?, c: Ctx) {
    if (n == null) return
    when (n.optString("type")) {
        "text" -> NText(n)
        "richText" -> NRichText(n, c)
        "icon" -> MIcon(n.str("name") ?: "help", n.f("size") ?: 24f, color(n.str("color")))
        "image" -> NImage(n)
        "nativeview" -> NNativeView(n, c)
        "container" -> NContainer(n, c)
        "padding" -> NPadding(n, c)
        "align" -> NAlign(n, c)
        "sizedBox" -> NSizedBox(n, c)
        "constrained" -> NConstrained(n, c)
        "fractional" -> NFractional(n, c)
        "aspectRatio" -> Box(Modifier.aspectRatio(n.f("ratio") ?: 1f)) { Node(n.o("child"), c.with(Parent.OTHER)) }
        "opacity" -> NOpacity(n, c)
        "visibility" -> {}
        "ignorePointer" -> NIgnorePointer(n, c)
        "clip" -> NClip(n, c)
        "transform" -> NTransform(n, c)
        "safeArea" -> NSafeArea(n, c)
        "flex" -> NFlex(n, c)
        "flexible" -> NFlexible(n, c)
        "wrap" -> NWrap(n, c)
        "stack" -> NStack(n, c)
        "positioned" -> NPositioned(n, c)
        "divider" -> NDivider(n)
        "scroll" -> NScroll(n, c)
        "list" -> NList(n, c)
        "grid" -> NGrid(n, c)
        "pageView" -> NPageView(n, c)
        "gesture" -> NGesture(n, c)
        "dismissible" -> NDismissible(n, c)
        "canvas" -> NCanvas(n, c)
        "animatedSwitcher" -> Box { Node(n.o("child"), c.with(Parent.OTHER)) }
        "button" -> NButton(n, c)
        "checkbox" -> NCheckbox(n, c)
        "switch" -> NSwitch(n, c)
        "radio" -> NRadio(n, c)
        "slider" -> NSlider(n, c)
        "progress" -> NProgress(n)
        "refresh" -> NRefresh(n, c)
        "dropdown" -> NDropdown(n, c)
        "popupMenu" -> NPopupMenu(n, c)
        "textField" -> NTextField(n, c)
        "scaffold" -> NScaffold(n, c)
        "appBar" -> NAppBar(n, c)
        "drawer" -> Box { Node(n.o("child"), c.with(Parent.COL)) }
        "navBar" -> NNavBar(n, c)
        "tabBar" -> NTabBar(n, c)
        "page" -> Node(n.o("content"), c)
        "errorBox" -> NErrorBox(n)
        else -> Text("⚠ unknown node: " + n.optString("type"), color = Color.Red, fontSize = 11.sp)
    }
}

@Composable
private fun NodeList(arr: JSONArray?, c: Ctx) {
    if (arr == null) return
    arr.forEachObject { i, o -> key(i) { Node(o, c) } }
}

private fun textAlign(s: String?): TextAlign? = when (s) {
    "left" -> TextAlign.Left
    "right" -> TextAlign.Right
    "center" -> TextAlign.Center
    "justify" -> TextAlign.Justify
    "start" -> TextAlign.Start
    "end" -> TextAlign.End
    else -> null
}

private fun decoration(st: JSONObject?): TextDecoration? {
    if (st == null) return null
    val u = st.b("underline") == true
    val l = st.b("lineThrough") == true
    return when {
        u && l -> TextDecoration.combine(listOf(TextDecoration.Underline, TextDecoration.LineThrough))
        u -> TextDecoration.Underline
        l -> TextDecoration.LineThrough
        else -> null
    }
}

@Composable
private fun NText(n: JSONObject) {
    val st = n.o("style")
    val scale = n.f("scale") ?: 1f
    val size = st?.f("size")
    val fontSize: TextUnit = if (size != null) (size * scale).sp else TextUnit.Unspecified
    var m: Modifier = Modifier
    val bg = color(st?.str("bg"))
    if (bg != null) m = m.background(bg)
    Text(
        text = n.optString("text"),
        modifier = m,
        color = color(st?.str("color")) ?: Color.Unspecified,
        fontSize = fontSize,
        fontStyle = if (st?.b("italic") == true) FontStyle.Italic else null,
        fontWeight = st?.i("weight")?.let { FontWeight(it.coerceIn(1, 1000)) },
        letterSpacing = st?.f("spacing")?.sp ?: TextUnit.Unspecified,
        textDecoration = decoration(st),
        textAlign = textAlign(n.str("align")),
        lineHeight = st?.f("lh")?.let { ((size ?: 14f) * it * scale).sp } ?: TextUnit.Unspecified,
        overflow = if (n.str("overflow") == "ellipsis") TextOverflow.Ellipsis else TextOverflow.Clip,
        softWrap = n.b("softWrap") ?: true,
        maxLines = n.i("maxLines") ?: Int.MAX_VALUE
    )
}

@Composable
private fun NRichText(n: JSONObject, c: Ctx) {
    val spans = n.arr("spans")
    val b = androidx.compose.ui.text.AnnotatedString.Builder()
    if (spans != null) {
        spans.forEachObject { _, s ->
            val st = s.o("style")
            val style = androidx.compose.ui.text.SpanStyle(
                color = color(st?.str("color")) ?: Color.Unspecified,
                fontSize = st?.f("size")?.sp ?: TextUnit.Unspecified,
                fontWeight = st?.i("weight")?.let { FontWeight(it.coerceIn(1, 1000)) },
                fontStyle = if (st?.b("italic") == true) FontStyle.Italic else null,
                textDecoration = decoration(st)
            )
            b.pushStyle(style)
            val tap = s.str("onTap")
            if (tap != null) b.pushStringAnnotation("tap", tap)
            b.append(s.optString("text"))
            if (tap != null) b.pop()
            b.pop()
        }
    }
    val text = b.toAnnotatedString()
    val st = n.o("style")
    androidx.compose.foundation.text.ClickableText(
        text = text,
        style = LocalTextStyle.current.copy(
            color = color(st?.str("color")) ?: LocalContentColor.current,
            fontSize = st?.f("size")?.sp ?: TextUnit.Unspecified,
            textAlign = textAlign(n.str("align")) ?: TextAlign.Unspecified
        ),
        maxLines = n.i("maxLines") ?: Int.MAX_VALUE,
        onClick = { offset ->
            text.getStringAnnotations("tap", offset, offset).firstOrNull()?.let { c.fire(it.item) }
        }
    )
}

@Composable
fun MIcon(name: String, size: Float, tint: Color?) {
    val ctx = LocalContext.current
    val family = remember { FontFamily(Font("dragon_fonts/MaterialIcons-Regular.ttf", ctx.assets)) }
    val col = tint ?: LocalContentColor.current
    Box(Modifier.size(size.dp), contentAlignment = Alignment.Center) {
        Text(
            text = name,
            fontFamily = family,
            fontSize = size.sp,
            color = col,
            maxLines = 1,
            softWrap = false,
            style = TextStyle(fontFeatureSettings = "liga", fontFamily = family, fontSize = size.sp, color = col)
        )
    }
}

@Composable
private fun NNativeView(n: JSONObject, c: Ctx) {
    val kind = n.optString("kind")
    val id = n.optString("id")
    val params = n.optJSONObject("params") ?: JSONObject()
    androidx.compose.runtime.key(kind, id) {
        androidx.compose.ui.viewinterop.AndroidView(
            factory = { androidCtx ->
                c.host.plugins.firstNotNullOfOrNull { it.createView(kind, id, params, androidCtx) }
                    ?: android.widget.TextView(androidCtx).apply { text = "No plugin provides the native view '$kind'." }
            },
            modifier = Modifier.fillMaxSize(),
            onRelease = { c.host.plugins.forEach { it.releaseView(kind, id) } }
        )
    }
}

@Composable
private fun NImage(n: JSONObject) {
    val src = n.optString("src")
    val model: Any = when (n.str("kind")) {
        "asset" -> "file:///android_asset/" + src.removePrefix("assets/")
        "file" -> java.io.File(src)
        else -> src
    }
    var m: Modifier = Modifier
    val w = n.f("width")
    val h = n.f("height")
    m = if (isFill(w)) m.fillMaxWidth() else if (w != null) m.width(w.dp) else m
    m = if (isFill(h)) m.fillMaxHeight() else if (h != null) m.height(h.dp) else m
    val fit = when (n.str("fit")) {
        "cover" -> ContentScale.Crop
        "fill" -> ContentScale.FillBounds
        "fitWidth" -> ContentScale.FillWidth
        "fitHeight" -> ContentScale.FillHeight
        "none" -> ContentScale.None
        "scaleDown" -> ContentScale.Inside
        else -> ContentScale.Fit
    }
    AsyncImage(model = model, contentDescription = null, modifier = m, contentScale = fit)
}

@Composable
private fun NErrorBox(n: JSONObject) {
    Box(Modifier.background(Color(0xFFFFEBEE)).border(1.dp, Color(0xFFE57373)).padding(10.dp)) {
        Text("⚠ " + n.optString("message"), color = Color(0xFFB71C1C), fontSize = 11.sp)
    }
}

private fun insets(o: JSONObject?): PaddingValues? {
    if (o == null) return null
    return PaddingValues(
        start = (o.f("l") ?: 0f).dp, top = (o.f("t") ?: 0f).dp,
        end = (o.f("r") ?: 0f).dp, bottom = (o.f("b") ?: 0f).dp
    )
}

private fun radiusShape(r: JSONObject?): Shape? {
    if (r == null) return null
    return RoundedCornerShape(
        topStart = (r.f("tl") ?: 0f).dp, topEnd = (r.f("tr") ?: 0f).dp,
        bottomEnd = (r.f("br") ?: 0f).dp, bottomStart = (r.f("bl") ?: 0f).dp
    )
}

private fun shapeOf(s: JSONObject?): Shape? {
    if (s == null) return null
    return when (s.str("kind")) {
        "circle" -> CircleShape
        "stadium" -> RoundedCornerShape(50)
        else -> radiusShape(s.o("radius"))
    }
}

private fun alignment(a: JSONObject?, default: Alignment = Alignment.TopStart): Alignment {
    if (a == null) return default
    val x = a.f("x") ?: 0f
    val y = a.f("y") ?: 0f
    return BiasAlignment(x, y)
}

private fun BiasAlignment(x: Float, y: Float): Alignment = androidx.compose.ui.BiasAlignment(x, y)

private fun brush(g: JSONObject?): Brush? {
    if (g == null) return null
    val cs = g.arr("colors") ?: return null
    val colors = ArrayList<Color>()
    for (i in 0 until cs.length()) colors.add(color(cs.optString(i)) ?: Color.Transparent)
    if (colors.size < 2) return null
    return when (g.str("type")) {
        "radial" -> Brush.radialGradient(colors)
        "sweep" -> Brush.sweepGradient(colors)
        else -> {
            val b = g.o("begin")
            val e = g.o("end")
            val bx = ((b?.f("x") ?: -1f) + 1f) / 2f
            val by = ((b?.f("y") ?: 0f) + 1f) / 2f
            val ex = ((e?.f("x") ?: 1f) + 1f) / 2f
            val ey = ((e?.f("y") ?: 0f) + 1f) / 2f
            object : androidx.compose.ui.graphics.ShaderBrush() {
                override fun createShader(size: Size): android.graphics.Shader =
                    android.graphics.LinearGradient(
                        bx * size.width, by * size.height, ex * size.width, ey * size.height,
                        colors.map { it.toArgb() }.toIntArray(), null, android.graphics.Shader.TileMode.CLAMP
                    )
            }
        }
    }
}

@Composable
private fun NContainer(n: JSONObject, c: Ctx) {
    val child = n.o("child")
    val dur = n.i("duration") ?: 0

    var w = n.f("width")
    var h = n.f("height")
    val shape: Shape = if (n.str("shape") == "circle") CircleShape else (radiusShape(n.o("radius")) ?: androidx.compose.ui.graphics.RectangleShape)
    val hasShape = n.str("shape") == "circle" || n.o("radius") != null

    var m: Modifier = Modifier
    insets(n.o("margin"))?.let { m = m.padding(it) }

    if (dur > 0) {
        if (w != null && !isFill(w)) w = animateDpAsState(w.dp, tween(dur), label = "w").value.value
        if (h != null && !isFill(h)) h = animateDpAsState(h.dp, tween(dur), label = "h").value.value
    }
    m = if (isFill(w)) m.fillMaxWidth() else if (w != null) m.width(w.dp) else m
    m = if (isFill(h)) m.fillMaxHeight() else if (h != null) m.height(h.dp) else m
    val minW = n.f("minWidth"); val maxW = n.f("maxWidth"); val minH = n.f("minHeight"); val maxH = n.f("maxHeight")
    if (minW != null || minH != null) m = m.defaultMinSize(minWidth = (minW ?: 0f).dp, minHeight = (minH ?: 0f).dp)
    if (maxW != null) m = m.widthIn(max = maxW.dp)
    if (maxH != null) m = m.heightIn(max = maxH.dp)

    val elevation = n.f("elevation") ?: 0f
    val shadows = n.arr("shadows")
    var shadowDp = elevation
    if (shadows != null && shadows.length() > 0) {
        shadowDp = (shadows.optJSONObject(0)?.f("blur") ?: 0f) / 2f
    }
    if (shadowDp > 0f) m = m.shadow(shadowDp.dp, shape, clip = false)

    if (hasShape || n.b("clip") == true) m = m.clip(shape)

    var bgColor = color(n.str("color"))
    if (dur > 0 && bgColor != null) bgColor = animateColorAsState(bgColor, tween(dur), label = "bg").value
    val grad = brush(n.o("gradient"))
    if (grad != null) m = m.background(grad, shape)
    else if (bgColor != null) m = m.background(bgColor, shape)

    val bgImage = n.o("bgImage")

    val border = n.o("border")
    if (border != null) {
        val sides = border.o("sides")
        if (sides == null) {
            val bc = color(border.str("color")) ?: Color.Black
            val bw = border.f("width") ?: 1f
            if (bw > 0f) m = m.border(bw.dp, bc, shape)
        } else {
            m = m.drawBehind {
                fun side(k: String, from: Offset, to: Offset) {
                    val s = sides.o(k) ?: return
                    drawLine(color(s.str("color")) ?: Color.Black, from, to, (s.f("width") ?: 1f).dp.toPx())
                }
                side("t", Offset(0f, 0f), Offset(size.width, 0f))
                side("b", Offset(0f, size.height), Offset(size.width, size.height))
                side("l", Offset(0f, 0f), Offset(0f, size.height))
                side("r", Offset(size.width, 0f), Offset(size.width, size.height))
            }
        }
    }

    var pad = insets(n.o("padding"))
    if (pad != null && dur > 0) {
        val l = animateDpAsState(pad.calculateLeftPadding(androidx.compose.ui.unit.LayoutDirection.Ltr), tween(dur), label = "pl").value
        val t = animateDpAsState(pad.calculateTopPadding(), tween(dur), label = "pt").value
        val r = animateDpAsState(pad.calculateRightPadding(androidx.compose.ui.unit.LayoutDirection.Ltr), tween(dur), label = "pr").value
        val b = animateDpAsState(pad.calculateBottomPadding(), tween(dur), label = "pb").value
        pad = PaddingValues(start = l, top = t, end = r, bottom = b)
    }

    val align = n.o("alignment")
    if (child == null) {
        val sized = w != null || h != null
        if (!sized) {
            m = if (c.parent == Parent.COL || c.parent == Parent.ROW) m.size(0.dp) else m.fillMaxSize()
        }
        Box(m) { if (bgImage != null) BgImage(bgImage) }
    } else {
        if (align != null) {
            if (w == null) m = m.fillMaxWidth()
            if (h == null && c.parent != Parent.COL) m = m.fillMaxHeight()
        }
        if (pad != null) m = m.padding(pad)
        Box(m, contentAlignment = if (align != null) alignment(align) else Alignment.TopStart) {
            if (bgImage != null) BgImage(bgImage)
            Node(child, c.with(Parent.OTHER))
        }
    }
}

@Composable
private fun BgImage(b: JSONObject) {
    val fit = when (b.str("fit")) {
        "contain" -> ContentScale.Fit
        "fill" -> ContentScale.FillBounds
        else -> ContentScale.Crop
    }
    val src = b.optString("src")
    val model: Any = if (b.str("kind") == "asset") "file:///android_asset/" + src.removePrefix("assets/") else src
    AsyncImage(model = model, contentDescription = null, modifier = Modifier.fillMaxSize(), contentScale = fit)
}

@Composable
private fun NPadding(n: JSONObject, c: Ctx) {
    var pv = insets(n.o("padding")) ?: PaddingValues(0.dp)
    val dur = n.i("duration") ?: 0
    if (dur > 0) {
        val l = animateDpAsState(pv.calculateLeftPadding(androidx.compose.ui.unit.LayoutDirection.Ltr), tween(dur), label = "pl").value
        val t = animateDpAsState(pv.calculateTopPadding(), tween(dur), label = "pt").value
        val r = animateDpAsState(pv.calculateRightPadding(androidx.compose.ui.unit.LayoutDirection.Ltr), tween(dur), label = "pr").value
        val b = animateDpAsState(pv.calculateBottomPadding(), tween(dur), label = "pb").value
        pv = PaddingValues(start = l, top = t, end = r, bottom = b)
    }
    Box(Modifier.padding(pv)) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NAlign(n: JSONObject, c: Ctx) {
    val a = alignment(n.o("alignment"), Alignment.Center)
    var m: Modifier = Modifier.fillMaxWidth()
    if (c.parent != Parent.COL) m = m.fillMaxHeight()
    Box(m, contentAlignment = a) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NSizedBox(n: JSONObject, c: Ctx) {
    val w = n.f("width")
    val h = n.f("height")
    var m: Modifier = Modifier
    m = if (isFill(w)) m.fillMaxWidth() else if (w != null) m.width(w.dp) else m
    m = if (isFill(h)) m.fillMaxHeight() else if (h != null) m.height(h.dp) else m
    val child = n.o("child")
    if (w == null && h == null && child == null) m = m.size(0.dp)
    Box(m) { Node(child, c.with(Parent.OTHER)) }
}

@Composable
private fun NConstrained(n: JSONObject, c: Ctx) {
    var m: Modifier = Modifier
    val minW = n.f("minWidth"); val maxW = n.f("maxWidth"); val minH = n.f("minHeight"); val maxH = n.f("maxHeight")
    if (minW != null || minH != null) m = m.defaultMinSize(minWidth = (minW ?: 0f).dp, minHeight = (minH ?: 0f).dp)
    if (maxW != null) m = m.widthIn(max = maxW.dp)
    if (maxH != null) m = m.heightIn(max = maxH.dp)
    Box(m) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NFractional(n: JSONObject, c: Ctx) {
    val a = alignment(n.o("alignment"), Alignment.Center)
    var m: Modifier = Modifier
    n.f("widthFactor")?.let { m = m.fillMaxWidth(it) }
    n.f("heightFactor")?.let { m = m.fillMaxHeight(it) }
    Box(Modifier.fillMaxSize(), contentAlignment = a) { Box(m) { Node(n.o("child"), c.with(Parent.OTHER)) } }
}

@Composable
private fun NOpacity(n: JSONObject, c: Ctx) {
    var a = n.f("opacity") ?: 1f
    val dur = n.i("duration") ?: 0
    if (dur > 0) a = animateFloatAsState(a, tween(dur), label = "alpha").value
    Box(Modifier.alpha(a)) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NIgnorePointer(n: JSONObject, c: Ctx) {
    var m: Modifier = Modifier
    if (n.b("ignoring") != false) {
        m = m.pointerInput(Unit) {
            awaitPointerEventScope {
                while (true) {
                    val e = awaitPointerEvent(androidx.compose.ui.input.pointer.PointerEventPass.Initial)
                    e.changes.forEach { it.consume() }
                }
            }
        }
    }
    Box(m) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NClip(n: JSONObject, c: Ctx) {
    val shape: Shape = when (n.str("shape")) {
        "oval" -> CircleShape
        "rrect" -> radiusShape(n.o("radius")) ?: androidx.compose.ui.graphics.RectangleShape
        else -> androidx.compose.ui.graphics.RectangleShape
    }
    Box(Modifier.clip(shape)) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NTransform(n: JSONObject, c: Ctx) {
    val rot = Math.toDegrees((n.f("rotate") ?: 0f).toDouble()).toFloat()
    val sx = n.f("scaleX") ?: n.f("scale") ?: 1f
    val sy = n.f("scaleY") ?: n.f("scale") ?: 1f
    val dx = n.f("dx") ?: 0f
    val dy = n.f("dy") ?: 0f
    val al = n.o("alignment")
    val ox = ((al?.f("x") ?: 0f) + 1f) / 2f
    val oy = ((al?.f("y") ?: 0f) + 1f) / 2f
    Box(
        Modifier.graphicsLayer {
            rotationZ = rot
            scaleX = sx
            scaleY = sy
            translationX = dx.dp.toPx()
            translationY = dy.dp.toPx()
            transformOrigin = androidx.compose.ui.graphics.TransformOrigin(ox, oy)
        }
    ) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NSafeArea(n: JSONObject, c: Ctx) {
    val top = if (n.b("top") == false) 0f else maxOf(0f, c.topInset - c.consumedTop)
    val bottom = if (n.b("bottom") == false) 0f else maxOf(0f, c.bottomInset - c.consumedBottom)
    Box(Modifier.fillMaxSize().padding(top = top.dp, bottom = bottom.dp)) {
        Node(n.o("child"), c.with(Parent.OTHER, consumedTop = c.topInset, consumedBottom = c.bottomInset))
    }
}

@Composable
private fun NDivider(n: JSONObject) {
    val th = (n.f("thickness") ?: 1f).dp
    if (n.str("dir") == "vertical") {
        VerticalDivider(Modifier.fillMaxHeight(), thickness = th)
    } else {
        HorizontalDivider(Modifier.fillMaxWidth(), thickness = th)
    }
}

private fun horizontalArrangement(main: String?, spacing: Float?): Arrangement.Horizontal = when (main) {
    "end" -> Arrangement.End
    "center" -> Arrangement.Center
    "spaceBetween" -> Arrangement.SpaceBetween
    "spaceAround" -> Arrangement.SpaceAround
    "spaceEvenly" -> Arrangement.SpaceEvenly
    else -> if (spacing != null && spacing > 0f) Arrangement.spacedBy(spacing.dp) else Arrangement.Start
}

private fun verticalArrangement(main: String?, spacing: Float?): Arrangement.Vertical = when (main) {
    "end" -> Arrangement.Bottom
    "center" -> Arrangement.Center
    "spaceBetween" -> Arrangement.SpaceBetween
    "spaceAround" -> Arrangement.SpaceAround
    "spaceEvenly" -> Arrangement.SpaceEvenly
    else -> if (spacing != null && spacing > 0f) Arrangement.spacedBy(spacing.dp) else Arrangement.Top
}

@Composable
private fun NFlex(n: JSONObject, c: Ctx) {
    val vertical = n.str("dir") == "vertical"
    val main = n.str("main")
    val cross = n.str("cross")
    val spacing = n.f("spacing")
    val kids = n.arr("children")
    val max = n.str("mainSize") == "max"
    if (vertical) {
        val h: Alignment.Horizontal = when (cross) {
            "start" -> Alignment.Start
            "end" -> Alignment.End
            else -> Alignment.CenterHorizontally
        }
        var m: Modifier = Modifier
        if (cross == "stretch") m = m.fillMaxWidth()
        if (max && !c.unbH) m = m.fillMaxHeight()
        Column(m, verticalArrangement(main, spacing), h) {
            val cc = c.with(Parent.COL, col = this, unbH = true)
            kids?.forEachObject { i, o -> key(i) { FlexChild(o, cc, cross == "stretch") } }
        }
    } else {
        val v: Alignment.Vertical = when (cross) {
            "start" -> Alignment.Top
            "end" -> Alignment.Bottom
            else -> Alignment.CenterVertically
        }
        var m: Modifier = Modifier
        if (max) m = m.fillMaxWidth()
        Row(m, horizontalArrangement(main, spacing), v) {
            val cc = c.with(Parent.ROW, row = this, unbH = true)
            kids?.forEachObject { i, o -> key(i) { FlexChild(o, cc, cross == "stretch") } }
        }
    }
}

@Composable
private fun FlexChild(o: JSONObject, c: Ctx, stretch: Boolean) {
    Node(o, c)
}

@Composable
private fun NFlexible(n: JSONObject, c: Ctx) {
    val flex = n.f("flex") ?: 1f
    val tight = n.str("fit") == "tight"
    val child = n.o("child")
    val inner = c.with(Parent.OTHER)
    val r = c.row
    val col = c.col
    when {
        r != null -> with(r) { Box(Modifier.weight(flex, tight)) { Node(child, inner) } }
        col != null -> with(col) { Box(Modifier.weight(flex, tight)) { Node(child, inner.with(unbH = false)) } }
        else -> Box { Node(child, inner) }
    }
}

@Composable
private fun NWrap(n: JSONObject, c: Ctx) {
    val sp = (n.f("spacing") ?: 0f).dp
    val rsp = (n.f("runSpacing") ?: 0f).dp
    val kids = n.arr("children")
    val inner = c.with(Parent.ROW)
    if (n.str("dir") == "vertical") {
        FlowColumn(verticalArrangement = Arrangement.spacedBy(sp), horizontalArrangement = Arrangement.spacedBy(rsp)) {
            kids?.forEachObject { i, o -> key(i) { Node(o, inner) } }
        }
    } else {
        val arr: Arrangement.Horizontal = when (n.str("alignment")) {
            "center" -> Arrangement.spacedBy(sp, Alignment.CenterHorizontally)
            "end" -> Arrangement.spacedBy(sp, Alignment.End)
            else -> Arrangement.spacedBy(sp)
        }
        FlowRow(horizontalArrangement = arr, verticalArrangement = Arrangement.spacedBy(rsp)) {
            kids?.forEachObject { i, o -> key(i) { Node(o, inner) } }
        }
    }
}

@Composable
private fun NStack(n: JSONObject, c: Ctx) {
    val a = alignment(n.o("alignment"), Alignment.TopStart)
    var m: Modifier = Modifier
    if (n.str("fit") == "expand") m = m.fillMaxSize()
    Box(m, contentAlignment = a) {
        val cc = c.with(Parent.STACK, box = this)
        n.arr("children")?.forEachObject { i, o -> key(i) { Node(o, cc) } }
    }
}

@Composable
private fun NPositioned(n: JSONObject, c: Ctx) {
    val l = n.f("left"); val t = n.f("top"); val r = n.f("right"); val b = n.f("bottom")
    val w = n.f("width"); val h = n.f("height")
    var m: Modifier = Modifier
    val horizAlign = if (l != null) 0 else if (r != null) 2 else 0
    val vertAlign = if (t != null) 0 else if (b != null) 2 else 0
    if (l != null && r != null && w == null) m = m.fillMaxWidth().padding(start = maxOf(l, 0f).dp, end = maxOf(r, 0f).dp)
    else {
        if (w != null) m = m.width(w.dp)
        if (l != null) m = if (l >= 0) m.padding(start = l.dp) else m.offset(x = l.dp)
        if (r != null) m = if (r >= 0) m.padding(end = r.dp) else m.offset(x = (-r).dp)
    }
    if (t != null && b != null && h == null) m = m.fillMaxHeight().padding(top = maxOf(t, 0f).dp, bottom = maxOf(b, 0f).dp)
    else {
        if (h != null) m = m.height(h.dp)
        if (t != null) m = if (t >= 0) m.padding(top = t.dp) else m.offset(y = t.dp)
        if (b != null) m = if (b >= 0) m.padding(bottom = b.dp) else m.offset(y = (-b).dp)
    }
    val al: Alignment = when {
        horizAlign == 2 && vertAlign == 2 -> Alignment.BottomEnd
        horizAlign == 2 -> Alignment.TopEnd
        vertAlign == 2 -> Alignment.BottomStart
        else -> Alignment.TopStart
    }
    val inner = c.with(Parent.OTHER)
    val bs = c.box
    if (bs != null) {
        with(bs) { Box(m.align(al)) { Node(n.o("child"), inner) } }
    } else {
        Box(m) { Node(n.o("child"), inner) }
    }
}

@Composable
private fun NScroll(n: JSONObject, c: Ctx) {
    val horizontal = n.str("dir") == "horizontal"
    val state = rememberScrollState()
    val id = n.i("controller")
    if (id != null) {
        androidx.compose.runtime.DisposableEffect(id) {
            c.host.scrollStates[id] = state
            onDispose { c.host.scrollStates.remove(id) }
        }
    }
    var m: Modifier = if (horizontal) Modifier.fillMaxWidth() else Modifier.fillMaxHeight()
    if (n.b("noScroll") != true) m = if (horizontal) m.horizontalScroll(state, reverseScrolling = n.b("reverse") == true) else m.verticalScroll(state, reverseScrolling = n.b("reverse") == true)
    insets(n.o("padding"))?.let { m = m.padding(it) }
    Box(m) { Node(n.o("child"), c.with(if (horizontal) Parent.ROW else Parent.COL)) }
}

@Composable
private fun NList(n: JSONObject, c: Ctx) {
    val horizontal = n.str("dir") == "horizontal"
    val kids = n.arr("children")
    val count = kids?.length() ?: 0
    val pad = insets(n.o("padding")) ?: PaddingValues(0.dp)
    val shrink = n.b("shrinkWrap") == true
    val noScroll = n.b("noScroll") == true
    val extent = n.f("itemExtent")
    val reverse = n.b("reverse") == true
    val inner = c.with(if (horizontal) Parent.ROW else Parent.COL)

    if (shrink || noScroll) {
        val st = rememberScrollState()
        var m: Modifier = Modifier
        if (!noScroll) m = if (horizontal) m.horizontalScroll(st) else m.verticalScroll(st)
        m = m.padding(pad)
        if (horizontal) Row(m) { for (i in 0 until count) key(i) { ListItem(kids!!.optJSONObject(i), inner, extent, true) } }
        else Column(m) { for (i in 0 until count) key(i) { ListItem(kids!!.optJSONObject(i), inner, extent, false) } }
        return
    }

    val state = rememberLazyListState()
    val total = n.i("total")
    val windowOffset = n.i("offset") ?: 0
    val onWindow = n.str("onWindow")
    if (total != null && onWindow != null && extent != null && !horizontal) {
        val id0 = n.i("controller")
        if (id0 != null) {
            androidx.compose.runtime.DisposableEffect(id0) {
                c.host.listStates[id0] = state
                onDispose { c.host.listStates.remove(id0) }
            }
        }
        LaunchedEffect(windowOffset, count, total) {
            snapshotFlow {
                val vis = state.layoutInfo.visibleItemsInfo
                val first = vis.firstOrNull()?.index ?: 0
                val last = vis.lastOrNull()?.index ?: 0
                val needs = (first < windowOffset + 4 && windowOffset > 0) ||
                    (last > windowOffset + count - 5 && windowOffset + count < total)
                if (needs) first else -1
            }.distinctUntilChanged().collect { first -> if (first >= 0) c.fire(onWindow + ":" + first) }
        }
        LazyColumn(Modifier.fillMaxSize(), state = state, contentPadding = pad, reverseLayout = reverse) {
            items(total) { i ->
                val j = i - windowOffset
                if (j in 0 until count) ListItem(kids!!.optJSONObject(j), inner, extent, false)
                else Spacer(Modifier.fillMaxWidth().height(extent.dp))
            }
        }
        return
    }
    val id = n.i("controller")
    if (id != null) {
        androidx.compose.runtime.DisposableEffect(id) {
            c.host.listStates[id] = state
            onDispose { c.host.listStates.remove(id) }
        }
    }
    val onEnd = n.str("onEnd")
    if (onEnd != null) {
        LaunchedEffect(count, onEnd) {
            snapshotFlow {
                val last = state.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1
                count > 0 && last >= count - 2
            }.distinctUntilChanged().collect { atEnd -> if (atEnd) c.fire(onEnd) }
        }
    }
    if (horizontal) {
        LazyRow(Modifier.fillMaxWidth(), state = state, contentPadding = pad, reverseLayout = reverse) {
            items(count) { i -> ListItem(kids!!.optJSONObject(i), inner, extent, true) }
        }
    } else {
        LazyColumn(Modifier.fillMaxSize(), state = state, contentPadding = pad, reverseLayout = reverse) {
            items(count) { i -> ListItem(kids!!.optJSONObject(i), inner, extent, false) }
        }
    }
}

@Composable
private fun ListItem(o: JSONObject?, c: Ctx, extent: Float?, horizontal: Boolean) {
    var m: Modifier = Modifier
    if (extent != null) m = if (horizontal) m.width(extent.dp) else m.height(extent.dp)
    Box(m) { Node(o, c) }
}

@Composable
private fun NGrid(n: JSONObject, c: Ctx) {
    val kids = n.arr("children")
    val count = kids?.length() ?: 0
    val columns = n.i("columns")
    val maxExtent = n.f("maxExtent")
    val cells = if (columns != null) GridCells.Fixed(columns) else GridCells.Adaptive(((maxExtent ?: 120f) * 0.75f).dp)
    val ms = (n.f("mainSpacing") ?: 0f).dp
    val cs = (n.f("crossSpacing") ?: 0f).dp
    val ratio = n.f("ratio") ?: 1f
    val pad = insets(n.o("padding")) ?: PaddingValues(0.dp)
    val inner = c.with(Parent.OTHER)
    val indices = remember(count) { (0 until count).toList() }
    if (n.str("dir") == "horizontal") {
        LazyHorizontalGrid(
            if (columns != null) androidx.compose.foundation.lazy.grid.GridCells.Fixed(columns) else androidx.compose.foundation.lazy.grid.GridCells.Adaptive(120.dp),
            Modifier.fillMaxWidth(), contentPadding = pad,
            horizontalArrangement = Arrangement.spacedBy(ms), verticalArrangement = Arrangement.spacedBy(cs)
        ) {
            items(indices) { i -> Box(Modifier.aspectRatio(1f / ratio)) { Node(kids!!.optJSONObject(i), inner) } }
        }
    } else {
        LazyVerticalGrid(
            cells, Modifier.fillMaxSize(), contentPadding = pad,
            verticalArrangement = Arrangement.spacedBy(ms), horizontalArrangement = Arrangement.spacedBy(cs)
        ) {
            items(indices) { i -> Box(Modifier.aspectRatio(ratio)) { Node(kids!!.optJSONObject(i), inner) } }
        }
    }
}

@Composable
private fun NPageView(n: JSONObject, c: Ctx) {
    val kids = n.arr("children")
    val count = kids?.length() ?: 0
    if (count == 0) return
    val index = (n.i("index") ?: 0).coerceIn(0, count - 1)
    val state = rememberPagerState(initialPage = index, pageCount = { count })
    val onChanged = n.str("onChanged")
    LaunchedEffect(index) { if (state.currentPage != index) state.animateScrollToPage(index) }
    LaunchedEffect(state, onChanged) {
        snapshotFlow { state.settledPage }.distinctUntilChanged().collect { p ->
            if (p != index) c.fire(onChanged, p)
        }
    }
    val inner = c.with(Parent.OTHER)
    val noScroll = n.b("noScroll") == true
    if (n.str("dir") == "vertical") {
        VerticalPager(state, Modifier.fillMaxSize(), userScrollEnabled = !noScroll) { p -> Node(kids!!.optJSONObject(p), inner) }
    } else {
        HorizontalPager(state, Modifier.fillMaxSize(), userScrollEnabled = !noScroll) { p -> Node(kids!!.optJSONObject(p), inner) }
    }
}

@Composable
private fun NRefresh(n: JSONObject, c: Ctx) {
    var refreshing by remember { mutableStateOf(false) }
    val onRefresh = n.str("onRefresh")
    val st = rememberPullRefreshState(refreshing, {
        refreshing = true
        c.fire(onRefresh)
    })
    LaunchedEffect(refreshing) {
        if (refreshing) {
            kotlinx.coroutines.delay(600)
            refreshing = false
        }
    }
    Box(Modifier.fillMaxSize().pullRefresh(st)) {
        Node(n.o("child"), c.with(Parent.OTHER))
        PullRefreshIndicator(refreshing, st, Modifier.align(Alignment.TopCenter))
    }
}

@Composable
private fun NGesture(n: JSONObject, c: Ctx) {
    val onTap = n.str("onTap")
    val onDouble = n.str("onDoubleTap")
    val onLong = n.str("onLongPress")
    val onTapDown = n.str("onTapDown")
    val onPanStart = n.str("onPanStart")
    val onPanUpdate = n.str("onPanUpdate")
    val onPanEnd = n.str("onPanEnd")
    val onH = n.str("onHDrag")
    val onV = n.str("onVDrag")
    val onHEnd = n.str("onHDragEnd")
    val onVEnd = n.str("onVDragEnd")
    val ripple = n.b("ripple") == true
    val shape = radiusShape(n.o("radius")) ?: androidx.compose.ui.graphics.RectangleShape

    var m: Modifier = Modifier
    if (ripple && (onTap != null || onLong != null || onDouble != null)) {
        m = m.clip(shape).combinedClickable(
            onClick = { c.fire(onTap) },
            onLongClick = if (onLong != null) ({ c.fire(onLong) }) else null,
            onDoubleClick = if (onDouble != null) ({ c.fire(onDouble) }) else null
        )
    } else if (onTap != null || onLong != null || onDouble != null || onTapDown != null) {
        m = m.pointerInput(onTap, onLong, onDouble, onTapDown) {
            detectTapGestures(
                onPress = { off -> if (onTapDown != null) c.fire(onTapDown, "${off.x / density},${off.y / density}") },
                onTap = { if (onTap != null) c.fire(onTap) },
                onDoubleTap = if (onDouble != null) ({ _: Offset -> c.fire(onDouble) }) else null,
                onLongPress = if (onLong != null) ({ _: Offset -> c.fire(onLong) }) else null
            )
        }
    }
    if (onPanStart != null || onPanUpdate != null || onPanEnd != null) {
        m = m.pointerInput(onPanStart, onPanUpdate, onPanEnd) {
            detectDragGestures(
                onDragStart = { off -> if (onPanStart != null) c.fire(onPanStart, "${off.x / density},${off.y / density}") },
                onDragEnd = { if (onPanEnd != null) c.fire(onPanEnd) },
                onDrag = { change, _ ->
                    if (onPanUpdate != null) c.fire(onPanUpdate, "${change.position.x / density},${change.position.y / density}")
                }
            )
        }
    }
    if (onH != null || onHEnd != null) {
        m = m.pointerInput(onH, onHEnd) {
            detectHorizontalDragGestures(
                onDragEnd = { if (onHEnd != null) c.fire(onHEnd) },
                onHorizontalDrag = { _, delta -> if (onH != null) c.fire(onH, delta / density) }
            )
        }
    }
    if (onV != null || onVEnd != null) {
        m = m.pointerInput(onV, onVEnd) {
            detectVerticalDragGestures(
                onDragEnd = { if (onVEnd != null) c.fire(onVEnd) },
                onVerticalDrag = { _, delta -> if (onV != null) c.fire(onV, delta / density) }
            )
        }
    }
    Box(m) { Node(n.o("child"), c.with(Parent.OTHER)) }
}

@Composable
private fun NDismissible(n: JSONObject, c: Ctx) {
    var dx by remember { mutableStateOf(0f) }
    var width by remember { mutableStateOf(1) }
    val onDismissed = n.str("onDismissed")
    val bg = n.o("background")
    Box(Modifier.fillMaxWidth().onSizeChanged { width = it.width }) {
        Box(Modifier.matchParentSizeCompat()) {
            if (bg != null) Node(bg, c.with(Parent.OTHER))
            else Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.error))
        }
        Box(
            Modifier.offset { IntOffset(dx.roundToInt(), 0) }
                .background(MaterialTheme.colorScheme.background)
                .draggable(
                    orientation = Orientation.Horizontal,
                    state = rememberDraggableState { dx += it },
                    onDragStopped = {
                        if (abs(dx) > width * 0.4f) {
                            c.fire(onDismissed, if (dx > 0) "startToEnd" else "endToStart")
                        }
                        dx = 0f
                    }
                )
        ) { Node(n.o("child"), c.with(Parent.OTHER)) }
    }
}

private fun Modifier.matchParentSizeCompat(): Modifier = this.fillMaxSize()

@Composable
private fun NCanvas(n: JSONObject, c: Ctx) {
    val w = n.f("width") ?: 100f
    val h = n.f("height") ?: 100f
    val ops = n.arr("ops")
    Box(Modifier.size(w.dp, h.dp)) {
        Canvas(Modifier.fillMaxSize()) {
            ops?.forEachObject { _, o -> drawOp(o) }
        }
        Node(n.o("child"), c.with(Parent.OTHER))
    }
}

private fun DrawScope.drawOp(o: JSONObject) {
    val d = density
    val p = o.o("paint")
    val col = color(p?.str("color")) ?: Color.Black
    val stroke = p?.str("style") == "stroke"
    val sw = (p?.f("width") ?: 1f) * d
    val cap = when (p?.str("cap")) {
        "round" -> StrokeCap.Round
        "square" -> StrokeCap.Square
        else -> StrokeCap.Butt
    }
    val style = if (stroke) Stroke(width = sw, cap = cap) else androidx.compose.ui.graphics.drawscope.Fill
    fun v(k: String): Float = (o.f(k) ?: 0f) * d
    when (o.optString("op")) {
        "line" -> drawLine(col, Offset(v("x1"), v("y1")), Offset(v("x2"), v("y2")), strokeWidth = sw, cap = cap)
        "rect" -> drawRect(col, Offset(v("x"), v("y")), Size(v("w"), v("h")), style = style)
        "rrect" -> drawRoundRect(col, Offset(v("x"), v("y")), Size(v("w"), v("h")),
            androidx.compose.ui.geometry.CornerRadius(v("r")), style = style)
        "circle" -> drawCircle(col, v("r"), Offset(v("x"), v("y")), style = style)
        "oval" -> drawOval(col, Offset(v("x"), v("y")), Size(v("w"), v("h")), style = style)
        "arc" -> drawArc(
            col, Math.toDegrees((o.f("start") ?: 0f).toDouble()).toFloat(),
            Math.toDegrees((o.f("sweep") ?: 0f).toDouble()).toFloat(), o.b("center") == true,
            Offset(v("x"), v("y")), Size(v("w"), v("h")), style = style
        )
        "path" -> {
            val path = Path()
            val cmds = o.arr("cmds")
            if (cmds != null) {
                for (i in 0 until cmds.length()) {
                    val cmd = cmds.optJSONArray(i) ?: continue
                    fun x(j: Int) = cmd.optDouble(j).toFloat() * d
                    when (cmd.optString(0)) {
                        "M" -> path.moveTo(x(1), x(2))
                        "L" -> path.lineTo(x(1), x(2))
                        "Q" -> path.quadraticBezierTo(x(1), x(2), x(3), x(4))
                        "C" -> path.cubicTo(x(1), x(2), x(3), x(4), x(5), x(6))
                        "Z" -> path.close()
                    }
                }
            }
            drawPath(path, col, style = style)
        }
        "text" -> {
            val st = o.o("style")
            val textColor = color(st?.str("color")) ?: col
            val paint = android.graphics.Paint().apply {
                isAntiAlias = true
                this.color = textColor.toArgb()
                textSize = (st?.f("size") ?: 14f) * d
            }
            drawContext.canvas.nativeCanvas.drawText(o.optString("text"), v("x"), v("y") + paint.textSize, paint)
        }
    }
}

@Composable
private fun NButton(n: JSONObject, c: Ctx) {
    val v = n.str("variant") ?: "text"
    val enabled = n.b("enabled") ?: true
    val onPressed = n.str("onPressed")
    val bg = color(n.str("bg"))
    val fg = color(n.str("fg"))
    val shape = shapeOf(n.o("shape"))
    val onClick: () -> Unit = { c.fire(onPressed) }
    val inner = c.with(Parent.ROW)
    var m: Modifier = Modifier
    val minW = n.f("minWidth"); val minH = n.f("minHeight")
    if (minW != null || minH != null) m = m.defaultMinSize(minWidth = (minW ?: 0f).dp, minHeight = (minH ?: 0f).dp)
    n.f("width")?.let { m = if (it >= INF) m.fillMaxWidth() else m.width(it.dp) }
    n.f("height")?.let { m = m.height(it.dp) }
    val pad = insets(n.o("padding")) ?: ButtonDefaults.ContentPadding
    val borderColor = color(n.str("borderColor"))
    val border: BorderStroke? = if (borderColor != null) BorderStroke((n.f("borderWidth") ?: 1f).dp, borderColor) else null
    val iconSize = n.f("iconSize")

    val content: @Composable RowScope.() -> Unit = {
        Node(n.o("child"), inner)
        val label = n.o("label")
        if (label != null) {
            Spacer(Modifier.width(8.dp))
            Node(label, inner)
        }
    }
    val iconContent: @Composable () -> Unit = {
        val child = n.o("child")
        if (child != null && child.optString("type") == "icon" && iconSize != null) {
            MIcon(child.str("name") ?: "help", iconSize, color(child.str("color")))
        } else {
            Node(child, inner)
        }
    }
    val unspec = Color.Unspecified
    when (v) {
        "elevated" -> ElevatedButton(
            onClick, m, enabled, shape ?: ButtonDefaults.elevatedShape,
            ButtonDefaults.elevatedButtonColors(containerColor = bg ?: unspec, contentColor = fg ?: unspec),
            contentPadding = pad, content = content
        )
        "filled" -> Button(
            onClick, m, enabled, shape ?: ButtonDefaults.shape,
            ButtonDefaults.buttonColors(containerColor = bg ?: unspec, contentColor = fg ?: unspec),
            border = border, contentPadding = pad, content = content
        )
        "tonal" -> FilledTonalButton(
            onClick, m, enabled, shape ?: ButtonDefaults.filledTonalShape,
            ButtonDefaults.filledTonalButtonColors(containerColor = bg ?: unspec, contentColor = fg ?: unspec),
            border = border, contentPadding = pad, content = content
        )
        "outlined" -> OutlinedButton(
            onClick, m, enabled, shape ?: ButtonDefaults.outlinedShape,
            ButtonDefaults.outlinedButtonColors(containerColor = bg ?: unspec, contentColor = fg ?: unspec),
            border = border ?: ButtonDefaults.outlinedButtonBorder, contentPadding = pad, content = content
        )
        "icon" -> IconButton(
            onClick, m, enabled,
            colors = IconButtonDefaults.iconButtonColors(containerColor = bg ?: unspec, contentColor = fg ?: LocalContentColor.current)
        ) { iconContent() }
        "iconFilled" -> FilledIconButton(
            onClick, m, enabled,
            colors = IconButtonDefaults.filledIconButtonColors(containerColor = bg ?: MaterialTheme.colorScheme.primary,
                contentColor = fg ?: MaterialTheme.colorScheme.onPrimary)
        ) { iconContent() }
        "iconTonal" -> FilledTonalIconButton(
            onClick, m, enabled,
            colors = IconButtonDefaults.filledTonalIconButtonColors(containerColor = bg ?: MaterialTheme.colorScheme.secondaryContainer,
                contentColor = fg ?: MaterialTheme.colorScheme.onSecondaryContainer)
        ) { iconContent() }
        "iconOutlined" -> OutlinedIconButton(
            onClick, m, enabled,
            colors = IconButtonDefaults.outlinedIconButtonColors(contentColor = fg ?: LocalContentColor.current)
        ) { iconContent() }
        "fab" -> FloatingActionButton(
            onClick, m, shape ?: FloatingActionButtonShapeDefault(),
            bg ?: MaterialTheme.colorScheme.primaryContainer, fg ?: MaterialTheme.colorScheme.onPrimaryContainer
        ) { iconContent() }
        "fabSmall" -> SmallFloatingActionButton(
            onClick, m, shape ?: RoundedCornerShape(12.dp),
            bg ?: MaterialTheme.colorScheme.primaryContainer, fg ?: MaterialTheme.colorScheme.onPrimaryContainer
        ) { iconContent() }
        "fabLarge" -> LargeFloatingActionButton(
            onClick, m, shape ?: RoundedCornerShape(28.dp),
            bg ?: MaterialTheme.colorScheme.primaryContainer, fg ?: MaterialTheme.colorScheme.onPrimaryContainer
        ) { iconContent() }
        "fabExtended" -> ExtendedFloatingActionButton(
            onClick, m, shape ?: RoundedCornerShape(16.dp),
            bg ?: MaterialTheme.colorScheme.primaryContainer, fg ?: MaterialTheme.colorScheme.onPrimaryContainer
        ) {
            iconContent()
            val label = n.o("label")
            if (label != null) {
                Spacer(Modifier.width(8.dp))
                Node(label, inner)
            }
        }
        else -> TextButton(
            onClick, m, enabled, shape ?: ButtonDefaults.textShape,
            ButtonDefaults.textButtonColors(containerColor = bg ?: unspec, contentColor = fg ?: unspec),
            contentPadding = pad, content = content
        )
    }
}

private fun FloatingActionButtonShapeDefault(): Shape = RoundedCornerShape(16.dp)

@Composable
private fun NCheckbox(n: JSONObject, c: Ctx) {
    val onChanged = n.str("onChanged")
    val value = n.b("value") ?: false
    val col = color(n.str("color"))
    Checkbox(
        checked = value,
        onCheckedChange = { c.fire(onChanged, it) },
        enabled = n.b("enabled") ?: true,
        colors = if (col != null) CheckboxDefaults.colors(checkedColor = col) else CheckboxDefaults.colors()
    )
}

@Composable
private fun NSwitch(n: JSONObject, c: Ctx) {
    val onChanged = n.str("onChanged")
    Switch(
        checked = n.b("value") ?: false,
        onCheckedChange = { c.fire(onChanged, it) },
        enabled = n.b("enabled") ?: true
    )
}

@Composable
private fun NRadio(n: JSONObject, c: Ctx) {
    val onChanged = n.str("onChanged")
    RadioButton(
        selected = n.b("selected") ?: false,
        onClick = { c.fire(onChanged) },
        enabled = n.b("enabled") ?: true
    )
}

@Composable
private fun NSlider(n: JSONObject, c: Ctx) {
    val min = n.f("min") ?: 0f
    val max = n.f("max") ?: 1f
    val div = n.i("divisions")
    val onChanged = n.str("onChanged")
    val onEnd = n.str("onChangeEnd")
    var local by remember { mutableStateOf(n.f("value") ?: min) }
    val server = n.f("value") ?: min
    LaunchedEffect(server) { local = server }
    Slider(
        value = local.coerceIn(min, max),
        onValueChange = { local = it; c.fire(onChanged, it) },
        modifier = Modifier.fillMaxWidth(),
        enabled = n.b("enabled") ?: true,
        valueRange = min..max,
        steps = if (div != null && div > 1) div - 1 else 0,
        onValueChangeFinished = { if (onEnd != null) c.fire(onEnd) }
    )
}

@Composable
private fun NProgress(n: JSONObject) {
    val value = n.f("value")
    val col = color(n.str("color")) ?: MaterialTheme.colorScheme.primary
    val track = color(n.str("track")) ?: MaterialTheme.colorScheme.surfaceVariant
    if (n.str("kind") == "linear") {
        val m = Modifier.fillMaxWidth()
        if (value == null) LinearProgressIndicator(m, color = col, trackColor = track)
        else LinearProgressIndicator(value.coerceIn(0f, 1f), m, color = col, trackColor = track)
    } else {
        val sw = (n.f("stroke") ?: 4f).dp
        if (value == null) CircularProgressIndicator(color = col, strokeWidth = sw)
        else CircularProgressIndicator(value.coerceIn(0f, 1f), color = col, strokeWidth = sw)
    }
}

@Composable
private fun NDropdown(n: JSONObject, c: Ctx) {
    var open by remember { mutableStateOf(false) }
    val items = n.arr("items")
    val selected = n.i("selected") ?: -1
    val enabled = n.b("enabled") ?: true
    val onChanged = n.str("onChanged")
    val inner = c.with(Parent.ROW)
    var m: Modifier = Modifier
    if (n.b("expanded") == true) m = m.fillMaxWidth()
    Box(m) {
        Row(
            Modifier.then(if (n.b("expanded") == true) Modifier.fillMaxWidth() else Modifier)
                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(4.dp))
                .clickable(enabled = enabled) { open = true }
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            val cur = if (selected >= 0) items?.optJSONObject(selected)?.o("label") else null
            if (cur != null) Node(cur, inner) else Node(n.o("hint"), inner)
            MIcon("arrow_drop_down", 24f, null)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            items?.forEachObject { i, it ->
                DropdownMenuItem(
                    text = { Node(it.o("label"), inner) },
                    enabled = it.b("enabled") ?: true,
                    onClick = {
                        open = false
                        c.fire(onChanged, i)
                    }
                )
            }
        }
    }
}

@Composable
private fun NPopupMenu(n: JSONObject, c: Ctx) {
    var open by remember { mutableStateOf(false) }
    val items = n.arr("items")
    val onSelected = n.str("onSelected")
    val inner = c.with(Parent.ROW)
    Box {
        Box(Modifier.clickable(enabled = n.b("enabled") ?: true) { open = true }) {
            Node(n.o("child"), inner)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            items?.forEachObject { i, it ->
                if (it.b("divider") == true) {
                    HorizontalDivider()
                } else {
                    DropdownMenuItem(
                        text = { Node(it.o("label"), inner) },
                        enabled = it.b("enabled") ?: true,
                        onClick = {
                            open = false
                            c.fire(onSelected, i)
                        }
                    )
                }
            }
        }
    }
}

@Composable
private fun NTextField(n: JSONObject, c: Ctx) {
    val serverValue = n.optString("value", "")
    var tfv by remember { mutableStateOf(TextFieldValue(serverValue, TextRange(serverValue.length))) }
    LaunchedEffect(serverValue) {
        if (tfv.text != serverValue) tfv = TextFieldValue(serverValue, TextRange(serverValue.length))
    }
    val onChanged = n.str("onChanged")
    val onSubmitted = n.str("onSubmitted")
    val onTap = n.str("onTap")
    val inner = c.with(Parent.ROW)
    val label = n.str("label")
    val hint = n.str("hint")
    val error = n.str("error")
    val helper = n.str("helper")
    val multi = (n.i("maxLines") ?: 1) != 1 || n.str("keyboard") == "multiline"
    val keyboard = when (n.str("keyboard")) {
        "number" -> KeyboardType.Number
        "email" -> KeyboardType.Email
        "phone" -> KeyboardType.Phone
        "url" -> KeyboardType.Uri
        "password" -> KeyboardType.Password
        else -> KeyboardType.Text
    }
    val ime = when (n.str("action")) {
        "done" -> ImeAction.Done
        "next" -> ImeAction.Next
        "search" -> ImeAction.Search
        "send" -> ImeAction.Send
        "go" -> ImeAction.Go
        else -> if (multi) ImeAction.Default else ImeAction.Done
    }
    val caps = when (n.str("capitalization")) {
        "words" -> KeyboardCapitalization.Words
        "sentences" -> KeyboardCapitalization.Sentences
        "characters" -> KeyboardCapitalization.Characters
        else -> KeyboardCapitalization.None
    }
    val focus = remember { FocusRequester() }
    if (n.b("autofocus") == true) LaunchedEffect(Unit) { focus.requestFocus() }

    val st = n.o("style")
    val textStyle = LocalTextStyle.current.copy(
        color = color(st?.str("color")) ?: Color.Unspecified,
        fontSize = st?.f("size")?.sp ?: TextUnit.Unspecified,
        textAlign = textAlign(n.str("textAlign")) ?: TextAlign.Unspecified
    )
    val onValue: (TextFieldValue) -> Unit = { nv ->
        val changed = nv.text != tfv.text
        tfv = nv
        if (changed) c.fire(onChanged, nv.text)
    }
    val m = Modifier.fillMaxWidth().focusRequester(focus)
        .then(if (onTap != null) Modifier.pointerInput(Unit) { detectTapGestures(onPress = { c.fire(onTap) }) } else Modifier)
    val labelC: (@Composable () -> Unit)? = if (label != null) ({ Text(label) }) else null
    val placeholder: (@Composable () -> Unit)? = if (hint != null) ({ Text(hint) }) else null
    val support: (@Composable () -> Unit)? = if (error != null) ({ Text(error) }) else if (helper != null) ({ Text(helper) }) else null
    val leading: (@Composable () -> Unit)? = if (n.o("prefix") != null) ({ Node(n.o("prefix"), inner) }) else null
    val trailing: (@Composable () -> Unit)? = if (n.o("suffix") != null) ({ Node(n.o("suffix"), inner) }) else null
    val prefixStr = n.str("prefixText")
    val suffixStr = n.str("suffixText")
    val prefixText: (@Composable () -> Unit)? = if (prefixStr != null) ({ Text(prefixStr) }) else null
    val suffixText: (@Composable () -> Unit)? = if (suffixStr != null) ({ Text(suffixStr) }) else null
    val vt = if (n.b("obscure") == true) PasswordVisualTransformation() else VisualTransformation.None
    val kOpts = KeyboardOptions(capitalization = caps, keyboardType = keyboard, imeAction = ime)
    val kActs = KeyboardActions(onAny = { if (onSubmitted != null) c.fire(onSubmitted) })
    val enabled = n.b("enabled") ?: true
    val readOnly = n.b("readOnly") ?: false
    val maxLines = if (multi) (n.i("maxLines") ?: 5).coerceAtLeast(1) else 1
    val minLines = n.i("minLines") ?: 1
    val ml = n.i("maxLength")
    val filled = n.b("filled") == true || n.str("border") == "underline" || n.str("border") == "UnderlineInputBorder"
    val shape = radiusShape(n.o("radius")) ?: RoundedCornerShape(4.dp)
    val valueC: (TextFieldValue) -> Unit = { nv -> if (ml == null || nv.text.length <= ml) onValue(nv) }

    if (filled) {
        TextField(
            value = tfv, onValueChange = valueC, modifier = m, enabled = enabled, readOnly = readOnly,
            textStyle = textStyle, label = labelC, placeholder = placeholder, leadingIcon = leading,
            trailingIcon = trailing, prefix = prefixText, suffix = suffixText, supportingText = support,
            isError = error != null, visualTransformation = vt, keyboardOptions = kOpts, keyboardActions = kActs,
            singleLine = !multi, maxLines = maxLines, minLines = minLines
        )
    } else {
        OutlinedTextField(
            value = tfv, onValueChange = valueC, modifier = m, enabled = enabled, readOnly = readOnly,
            textStyle = textStyle, label = labelC, placeholder = placeholder, leadingIcon = leading,
            trailingIcon = trailing, prefix = prefixText, suffix = suffixText, supportingText = support,
            isError = error != null, visualTransformation = vt, keyboardOptions = kOpts, keyboardActions = kActs,
            singleLine = !multi, maxLines = maxLines, minLines = minLines, shape = shape
        )
    }
}

@Composable
private fun NScaffold(n: JSONObject, c: Ctx) {
    val bg = color(n.str("bg")) ?: MaterialTheme.colorScheme.background
    val appBar = n.o("appBar")
    val bottomNav = n.o("bottomNav")
    val cc = c.with(Parent.OTHER, consumedTop = if (appBar != null) c.topInset else 0f,
        consumedBottom = if (bottomNav != null) c.bottomInset else 0f, unbH = false)
    val resize = n.b("resizeInset") ?: true

    Box(Modifier.fillMaxSize().background(bg)) {
        Column(Modifier.fillMaxSize().then(if (resize) Modifier.imePadding() else Modifier)) {
            if (appBar != null) Node(appBar, cc)
            Box(Modifier.weight(1f).fillMaxWidth()) {
                Node(n.o("body"), cc)
                val fab = n.o("fab")
                if (fab != null) {
                    val align = when (n.str("fabLocation")) {
                        "center" -> Alignment.BottomCenter
                        "start" -> Alignment.BottomStart
                        else -> Alignment.BottomEnd
                    }
                    Box(Modifier.align(align).padding(16.dp)) { Node(fab, cc) }
                }
                val sheet = n.o("bottomSheet")
                if (sheet != null) Box(Modifier.align(Alignment.BottomCenter)) { Node(sheet, cc) }
            }
            if (bottomNav != null) Node(bottomNav, cc)
        }
        for (side in listOf("start", "end")) {
            val d = (if (side == "start") n.o("drawer") else n.o("endDrawer")) ?: continue
            val open = c.host.drawers[side] == true
            AnimatedVisibility(open, enter = fadeIn(), exit = fadeOut()) {
                Box(
                    Modifier.fillMaxSize().background(Color(0x66000000))
                        .pointerInput(Unit) { detectTapGestures { c.host.drawers[side] = false } }
                )
            }
            AnimatedVisibility(
                open,
                enter = slideInHorizontally { if (side == "start") -it else it },
                exit = slideOutHorizontally { if (side == "start") -it else it }
            ) {
                Box(Modifier.fillMaxSize(), contentAlignment = if (side == "start") Alignment.CenterStart else Alignment.CenterEnd) {
                    Surface(
                        Modifier.fillMaxHeight().widthIn(max = 304.dp).fillMaxWidth(0.85f),
                        color = MaterialTheme.colorScheme.surface,
                        shape = if (side == "start") RoundedCornerShape(topEnd = 16.dp, bottomEnd = 16.dp)
                        else RoundedCornerShape(topStart = 16.dp, bottomStart = 16.dp)
                    ) {
                        val dctx = c.with(Parent.COL, consumedTop = 0f, consumedBottom = 0f, closeSide = side)
                        Column(Modifier.verticalScroll(rememberScrollState()).padding(top = c.topInset.dp)) {
                            Node(d, dctx)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun NAppBar(n: JSONObject, c: Ctx) {
    val bg = color(n.str("bg")) ?: MaterialTheme.colorScheme.surface
    val fg = color(n.str("fg")) ?: MaterialTheme.colorScheme.onSurface
    val elevation = (n.f("elevation") ?: 0f).dp
    val inner = c.with(Parent.ROW)
    Surface(color = bg, contentColor = fg, shadowElevation = elevation) {
        Column {
            Spacer(Modifier.height(c.topInset.dp))
            Row(
                Modifier.fillMaxWidth().height((n.f("height") ?: 64f).dp).padding(horizontal = 4.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                val leading = n.o("leading")
                when {
                    leading != null -> Box(Modifier.padding(horizontal = 4.dp)) { Node(leading, inner) }
                    n.str("autoLeading") == "menu" -> IconButton({ c.host.drawers["start"] = true }) { MIcon("menu", 24f, null) }
                    n.str("autoLeading") == "back" -> IconButton({ c.fire(n.str("onBack")) }) { MIcon("arrow_back", 24f, null) }
                    else -> Spacer(Modifier.width(12.dp))
                }
                CompositionLocalProvider(
                    LocalTextStyle provides LocalTextStyle.current.copy(fontSize = 22.sp)
                ) {
                    Box(
                        Modifier.weight(1f).padding(start = 4.dp),
                        contentAlignment = if (n.b("centerTitle") == true) Alignment.Center else Alignment.CenterStart
                    ) { Node(n.o("title"), inner) }
                }
                n.arr("actions")?.forEachObject { i, o -> key(i) { Node(o, inner) } }
            }
            val bottom = n.o("bottom")
            if (bottom != null) Node(bottom, c.with(Parent.COL))
        }
    }
}

@Composable
private fun NNavBar(n: JSONObject, c: Ctx) {
    val items = n.arr("items")
    val selected = n.i("selected") ?: 0
    val onChanged = n.str("onChanged")
    val bg = color(n.str("bg"))
    NavigationBar(
        containerColor = bg ?: MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.35f),
        modifier = Modifier
    ) {
        items?.forEachObject { i, it ->
            val sel = i == selected
            val icon = if (sel) (it.str("activeIcon") ?: it.str("icon")) else it.str("icon")
            NavigationBarItem(
                selected = sel,
                onClick = { c.fire(onChanged, i) },
                icon = { MIcon(icon ?: "circle", 24f, null) },
                label = { Text(it.str("label") ?: "") }
            )
        }
    }
}

@Composable
private fun NTabBar(n: JSONObject, c: Ctx) {
    val labels = n.arr("labels")
    val selected = (n.i("selected") ?: 0).coerceIn(0, maxOf(0, (labels?.length() ?: 1) - 1))
    val onChanged = n.str("onChanged")
    val inner = c.with(Parent.OTHER)
    val tabs: @Composable () -> Unit = {
        labels?.forEachObject { i, t ->
            Tab(
                selected = i == selected,
                onClick = { c.fire(onChanged, i) },
                text = if (t.str("text") != null) ({ Text(t.str("text") ?: "") }) else null,
                icon = if (t.str("icon") != null) ({ MIcon(t.str("icon") ?: "circle", 24f, null) }) else null
            )
        }
    }
    if (n.b("scrollable") == true) {
        ScrollableTabRow(selectedTabIndex = selected, containerColor = Color.Transparent, edgePadding = 8.dp, tabs = tabs)
    } else {
        TabRow(selectedTabIndex = selected, containerColor = Color.Transparent, tabs = tabs)
    }
}
