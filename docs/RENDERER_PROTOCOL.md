# Renderer protocol

What a host (Android Compose today, SwiftUI skeleton for iOS, the browser preview) has to implement to run a
Dragon app. The app itself is native code with a plain C ABI; a renderer only pulls JSON frames out and pushes events in.

## C ABI (every compiled app exports this; `dragon_app.h` is generated next to the iOS library)

| Function | Purpose |
|---|---|
| `dgn_app_init(docsDir, mediaJson)` | boot; returns `""` or an error message |
| `dgn_app_render()` | current frame (JSON string, valid until the next call: copy it) |
| `dgn_app_event("hN" or "hN:payload")` | deliver a callback; returns 1 if state changed |
| `dgn_app_is_dirty()` | a redraw is pending (timers, animations) |
| `dgn_app_tick(nowMs)` | run due timers; returns ms until the next one, or -1 |
| `dgn_app_platform_calls()` | JSON array of pending `Platform.invoke` calls `[{id, method, args}]` |
| `dgn_app_platform_result(id, json)` | answer one (`json` empty = null) |
| `dgn_app_back()` | system Back; 1 if the app handled it |
| `dgn_app_set_media(json)` | screen metrics changed |
| `dgn_app_lifecycle("resumed"\|"inactive"\|"paused")` | app lifecycle |
| `dgn_app_last_error()` | last uncaught Dragon error |

Call everything on one thread (the main thread): the runtime records the thread that booted it.
Loop: `tick(now)` every `min(next, 100)` ms (at least 8), re-render when `is_dirty()`, drain `platform_calls()`
after every event, tick and result.

`media` = `{width, height, pixelRatio, top, bottom, left, right, keyboard, brightness: "light"|"dark", textScale, locale}`
(logical pixels).

## Frame

```json
{ "v": 1, "root": <page>, "overlays": [<overlay>], "theme": {...}, "title": "App",
  "canPop": false, "hasBack": false, "commands": [], "error": null }
```

* `root` = `{type:"page", key, depth, content:<node>}`. A new `key` is a route change: animate forward when `depth`
  grew, otherwise back.
* `overlays` = `{id, kind:"dialog"|"sheet"|"snackbar", content, dismissible, onDismiss, barrierColor, bg,
  fullscreen, dragHandle}` drawn above the page in order.
* `theme` = `{dark?, background?, scheme:{primary, onPrimary, primaryContainer, secondary, tertiary, error, surface,
  onSurface, surfaceVariant, outline, ...}}` as `#AARRGGBB` strings.
* `commands`: `{cmd:"drawer", side:"start"|"end", op:"open"|"close"}`, `{cmd:"scroll", id, index|offset|end, animate}`.
* `canPop` / `hasBack`: route system Back to `dgn_app_back()` when either is true or an overlay is open.
* `error`: set when the app threw; draw it over the UI.

## Events

Every callback property holds an id string such as `"h12"`. Send `"h12"` for plain callbacks, `"h12:<payload>"` when
the callback takes a value: strings as they are, `true`/`false`, numbers, `x,y` positions, an index for lists/menus.

## Nodes

All nodes are `{type, ...props}`; `child` / `children` hold nodes. Sizes are logical pixels; `1e9` means "fill".
`alignment` is `{x,y}` in -1..1. Colors are `#AARRGGBB`/`#RRGGBB`. Insets are `{l,t,r,b}`; radii `{tl,tr,br,bl}`.
A renderer must draw an unknown `type` as its children (or nothing) and never crash.

| type | main props |
|---|---|
| `text` | `text`, `style{size,color,weight,italic,spacing,lh,decoration,bg}`, `align`, `maxLines`, `overflow`, `softWrap`, `scale` |
| `richText` | `spans[{text,style,onTap}]`, `style`, `align`, `maxLines` |
| `icon` | `name` (Material Icons ligature), `size`, `color` |
| `image` | `src`, `kind: asset\|file\|network`, `width`, `height`, `fit` |
| `nativeview` | `kind`, `id`, `params` (answered by plugins; see PACKAGES.md) |
| `container` | `width,height,minWidth..maxHeight,color,gradient,radius,shape,border,shadows,elevation,padding,margin,alignment,clip,child,duration,bgImage` |
| `padding` `align` `sizedBox` `constrained` `fractional` `aspectRatio` `opacity` `ignorePointer` `clip` `transform` `safeArea` | single `child` plus the obvious props |
| `flex` | `dir: vertical\|horizontal`, `main`, `cross`, `mainSize`, `spacing`, `children` |
| `flexible` | `flex`, `fit: tight\|loose`, `child` |
| `wrap` | `dir`, `spacing`, `runSpacing`, `alignment`, `children` |
| `stack` / `positioned` | `alignment`, `fit` / `left,top,right,bottom,width,height` |
| `divider` | `dir`, `thickness` |
| `scroll` | `dir`, `padding`, `reverse`, `noScroll`, `controller` (id for scroll commands), `child` |
| `list` | `dir`, `children`, `padding`, `itemExtent`, `shrinkWrap`, `noScroll`, `reverse`, `controller`; virtualised: `total`, `offset`, `onWindow` |
| `grid` | `children`, `columns` or `maxExtent`, `mainSpacing`, `crossSpacing`, `ratio`, `padding`, `dir` |
| `pageView` | `children`, `index`, `onChanged`, `dir`, `noScroll` |
| `refresh` | `onRefresh`, `child` |
| `gesture` | `onTap onDoubleTap onLongPress onTapDown onPanStart onPanUpdate onPanEnd onHDrag onHDragEnd onVDrag onVDragEnd`, `ripple`, `radius`, `child` |
| `dismissible` | `onDismissed` (`"startToEnd"`/`"endToStart"`), `background`, `child` |
| `canvas` | `width`, `height`, `ops[{op: line\|rect\|rrect\|circle\|oval\|arc\|path\|text, paint{color,style,width,cap}, ...}]` |
| `animatedSwitcher` | `child` |
| `button` | `variant: text\|filled\|elevated\|tonal\|outlined\|icon*\|fab*`, `child`, `label`, `onPressed`, `enabled`, `bg`, `fg`, `shape`, `padding`, `width`, `height`, `borderColor` |
| `checkbox` `switch` `radio` | `value`/`selected`, `onChanged`, `enabled` (payload `true`/`false`) |
| `slider` | `value,min,max,divisions,onChanged,onChangeEnd` (payload number) |
| `progress` | `kind: linear\|circular`, `value` (null = indeterminate), `color`, `track` |
| `dropdown` | `items[{label,enabled}]`, `selected`, `hint`, `onChanged` (payload index), `expanded` |
| `popupMenu` | `child`, `items[{label,divider,enabled}]`, `onSelected` (payload index) |
| `textField` | `value`, `onChanged`, `onSubmitted`, `onTap`, `label`, `hint`, `error`, `helper`, `prefix`, `suffix`, `keyboard`, `action`, `capitalization`, `obscure`, `maxLines`, `minLines`, `maxLength`, `autofocus`, `readOnly`, `filled`, `style` |
| `scaffold` | `appBar`, `body`, `bottomNav`, `fab`, `fabLocation`, `drawer`, `endDrawer`, `bottomSheet`, `bg`, `resizeInset` |
| `appBar` | `title`, `leading`, `autoLeading: menu\|back`, `onBack`, `actions`, `centerTitle`, `bottom`, `bg`, `fg`, `elevation`, `height` |
| `drawer` `page` `errorBox` | single child / content / `message` |
| `navBar` | `items[{icon,activeIcon,label}]`, `selected`, `onChanged` (payload index), `bg` |
| `tabBar` | `labels[{text,icon}]`, `selected`, `onChanged`, `scrollable` |

Properties with a given name are not available on every node; the Android renderer
(`templates/android_vm/kotlin/DragonRenderer.kt`) is the reference for exact behaviour.

## Platform methods

`Platform.invoke(method, args, cb)` -> `{id, method, args}` in `dgn_app_platform_calls()`. Built in: `http`,
`clipboard.get/set`, `toast`, `openUrl`, `share`, `haptic`, `vibrate`, `deviceInfo`, `permission.check/request`,
`notify`, `notify.cancel`, `datePicker`, `timePicker`, `pickFile`, `takePhoto`, `location`, `statusBar`, `orientation`,
`exit`. Everything else is offered to plugins by name prefix; if nobody answers, reply with null.
Replies are JSON (`""` = null).

## Writing a renderer for another platform

1. Port the C ABI loop above.
2. Port `Node` as a switch on `type` with a children fallback.
3. Start with: text, container, flex, stack, scroll, list, button, textField, scaffold, appBar, gesture, image, icon.
4. Implement the platform methods your app uses; send null for the rest.
5. Run the same app on the browser preview and your renderer and compare screens.

The iOS renderer in `templates/ios/swift/` follows these steps; see [IOS.md](IOS.md) for its coverage.

## Renderers

Android (Compose), iOS (SwiftUI, untested) and Windows (WPF, untested) all implement this protocol. The Windows host
is in `templates/windows/cs/`.
