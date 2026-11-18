# Dragon UI frame schema

Every frame the app produces (natively compiled on Android; interpreted in preview) (`DragonNative.render()` on Android, `/api/frame` in
`dragon preview`) is a JSON object:

```json
{ "v": 1, "root": <node>, "overlays": [<node>], "theme": {...},
  "title": "App", "canPop": false, "hasBack": false, "commands": [], "error": null }
```

- `root` – the widget tree after build, flattened to renderer nodes (`{"type": "...", ...props}`). The exact node types and
  their properties are listed in [RENDERER_PROTOCOL.md](RENDERER_PROTOCOL.md). Renderers must render unknown nodes as their
  children (or nothing) and never crash.
- `overlays` – dialogs, bottom sheets, snackbars and popup menus, drawn above `root` in order.
- `theme` – resolved Material colour scheme (`#AARRGGBB` strings), `dark`, typography.
- `commands` – one-shot host commands (`drawer` open/close, `scroll` to index/offset/end).
- `canPop` – whether back should be routed to the app (`DragonNative.back()`).
- `hasBack` – the app registered a back handler (`AppEvents.addBackHandler`); route Back to the app.
- `error` – set when the app threw; the renderer shows an error screen.

## Events

Callbacks are registered by id (`h12`). The renderer sends `"h12"` for void callbacks
or `"h12:payload"` for ones that carry a value (string, bool, number, int, or `x,y` position).
Call `DragonNative.event(str)` and then re-render.

## Platform channel

Dart-style: `Platform.invoke(method, args, callback)` in Dragon. The host drains pending calls
(`DragonNative.platformCalls()`) and answers with `DragonNative.platformResult(id, json)`.
Methods: `http`, `clipboard`, `toast`, `openUrl`, `share`, `haptic`, `deviceInfo`, `permissions`,
`notify`, `datePicker`, `timePicker`, `pickFile`, `takePhoto`, `location`, `orientation`, `exit`,
`media.query`, `media.art` (music library) and `audio.*` (state, setQueue, play, pause, toggle, next, previous,
seek, skipTo, shuffle, repeat, speed, stop, sleep, queue.add/remove/move, eq.get/set) for background playback, and
`web.*` (config, state, open, load, back, forward, reload, stop, close, desktop, find, findNext, findClear, snapshot, favicon,
clear, print, eval, download) for browser tabs (see docs/BROWSER.md). `Web.state` tabs carry a generic `counts` map and `totals` for counters supplied by the app's `WebExtension`.

## Virtualised lists

A `list` node with `total`, `offset` and `onWindow` is virtualised: `children` holds only rows
`[offset, offset + children.length)` of `total` rows (all `itemExtent` high). The renderer scrolls
a list of `total` rows and fires `"<onWindow>:<firstVisibleRow>"` when the visible rows get close to
the edge of the built window; the next frame then carries the new window.
