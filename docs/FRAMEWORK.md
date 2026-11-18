# The Dragon UI framework

```dragon
import "dragon/ui" as *

// One class per screen. `state()` gives the starting values, `build(ctx, s)` draws them,
// and `ctx.set({...})` changes them and redraws.
class Counter extends Component {
    func state() { return {count: 0} }

    func build(ctx, s) {
        add = func() { ctx.set({count: s.count + 1}) }
        return Screen(
            topBar: TopBar(title: Text("Counter")),
            body: Col([
                Text("You have pushed the button this many times:"),
                Text(str(s.count), size: 48, bold: True),
                Button("Add", onTap: add, icon: Icons.add)
            ], gap: 16, main: "center").centered(),
            actionButton: ActionButton(onTap: add, child: Icon(Icons.add))
        )
    }
}

func main() { runApp(App(title: "Counter", theme: ThemeSpec(primarySwatch: Colors.teal), home: Counter())) }
```

## The model

* Things on screen are **widgets**: plain classes you call with named arguments. Every argument is optional.
* A screen is a `Component` (state in a map, `ctx.set` to change it) or, when you want classic control,
  a `View` (stateless: `build(ctx)`) or `LiveView` + `Live` (`onInit`, `update`, `onExit`).
* `Ctx` is what a widget receives while it builds (theme, navigation, media size).
* Callbacks are `onTap`, `onChange`, `onSubmit`; layout arguments are `main`, `cross`, `mainSize`, `gap`.

## Modifiers

Any widget can be wrapped with a chain instead of nesting:

```dragon
Text("Hello").pad(16).bg(Colors.amber).round(12).centered()
Item(title: Text("Row")).tap(open).faded(0.5)
```

`.pad .inset .bg .sized .round .outline .centered .aligned .grow .tap .longPress .faded .when .scrolls .safe .tip`.
`Text` takes `size`, `bold`, `color`, `weight`, `italic`, `align` directly. `Button("Save", onTap: f, kind: "raised"|"solid"|"tonal"|"line"|"flat", icon: ...)`.
`Input(hint:, label:, icon:, onChange:, onSubmit:)`.

## Many files

Real apps are split up. `lib/main.dgn` imports screens and widgets from other files:

```dragon
import "pages/home" as *            // names stay global, like the same file
import "widgets/cards" as cards     // names live under `cards.`
import "package:dragon_audio" as *  // a package (see PACKAGES.md)
```

* A name starting with `_` is private to its file.
* Two files defining the same name, or an import clashing with your own name, is an error that names both places.
* Import cycles are errors. Mixing `as *` and an alias for the same module is an error.
* `dragon create MyApp` writes `lib/main.dgn` + `lib/pages/home.dgn`. `examples/components` splits a gallery over files.

## Modules

`dragon/core` (Color, Insets, Font, Look, Icons), `dragon/widgets` (layout, scrolling, gestures, animation, Canvas),
`dragon/material` (App, Screen, buttons, inputs, dialogs, navigation, theme), `dragon/services` (Http, Prefs, Clipboard, Toast,
DateTime, ...). `import "dragon/ui" as *` brings in those. `dragon/extras` is opt-in (`import "dragon/extras" as *`) so its
generic names (`Banner`, `Calendar`, `Rating`, ...) never clash with yours. The core stays small: audio, web views, maps and the
like are packages ([PACKAGES.md](PACKAGES.md)). Full list: [COMPONENTS.md](COMPONENTS.md).

## Coming from Dragon 1 / Flutter-style names

`dragon migrate [paths] [--dry-run] [--strings]` rewrites old names (`Scaffold`, `setState`, `ElevatedButton`, ...) to the
new vocabulary in `.dgn` files and in the code blocks of `.md` files. The table is `src/migrate/renames.txt`
(`--table FILE` to use your own). It refuses to run if your code already declares a name that is now built in.

## Tools

* `dragon preview` – browser preview with hot reload.
* `dragon ui-dump --pretty` – headless tree dump; `--tap h12` simulates a tap.
* `dragon create MyApp --package com.example.myapp` – Android project (Compose renderer + natively compiled app).
* `dragon -c lib/main.dgn -o app --native --app [--test-main]` – compile the app to native code (what `dragon run` does per ABI).
* `preview`/`ui-dump` use the interpreter (development only); the Android app itself is native.

## Long lists: `ListView.builder(virtual: True)`

A normal `ListView.builder` builds every row each frame. For thousands of rows pass a fixed
`itemExtent`, a `ScrollController` and `virtual: True`: only a window of ~48 rows is built, the
renderer reports scrolling and the framework moves the window. Rows must have the same height.

```
ctl = ScrollController()            # keep it in your Live
Feed.builder(itemCount: songs.length, itemExtent: 64, virtual: True, controller: ctl,
                 itemBuilder: func(context, i) { return SongRow(songs[i]) })
```

## Music: library and background playback

```
MusicLibrary.requestAccess(func(granted) { ... })   # READ_MEDIA_AUDIO / READ_EXTERNAL_STORAGE
MusicLibrary.scan(func(tracks, error) { ... })      # tracks: [{id, uri, title, artist, album, albumId,
                                                    #           artistId, durationMs, track, year, added, size}]
MusicLibrary.artwork(albumId, 192, func(path) { })  # cached cover file path (Picture.file) or null

AudioPlayer.setQueue(tracks, index)    AudioPlayer.toggle()    AudioPlayer.next() / previous()
AudioPlayer.seekTo(ms)                 AudioPlayer.skipTo(i)   AudioPlayer.setShuffle(on)
AudioPlayer.setRepeat("off"|"all"|"one")    AudioPlayer.setSpeed(1.25)    AudioPlayer.setSleepTimer(minutes)
AudioPlayer.addToQueue(track, next)    AudioPlayer.removeFromQueue(i)    AudioPlayer.moveInQueue(from, to)
AudioPlayer.equalizer(cb) / setEqualizer({enabled, preset, band, level, bass})
AudioPlayer.state                      # Signal with {playing, positionMs, durationMs, index, count,
                                       #   mediaId, shuffle, repeat, speed, sleepLeftMs, ...}
```

On Android playback runs in a foreground `MediaSessionService` (Media3/ExoPlayer): notification,
lock-screen and Bluetooth/headset controls, audio focus and background play come with it.
`examples/music_player` is a complete player built on these APIs; `dragon preview` runs it against a
generated demo library with a simulated player. The service, permissions and Media3 dependency are
added to the Android project automatically, and only when your code uses `AudioPlayer`/`MusicLibrary`.

## Web: `WebView` and `Web`

`WebView(tab: id)` shows the native WebView of a browser tab; the `Web` service opens, navigates, configures
and queries tabs (navigation, cookies, https upgrade, find in page, snapshots, data clearing; ad blocking and the like are added by an app through a `WebExtension` in its `native/` folder), and
`WidgetsBinding.addBackHandler(fn)` lets an app handle the system Back button. Full reference, limits and
verification status: [BROWSER.md](BROWSER.md). Everything is added to the Android project only when your code uses it.
