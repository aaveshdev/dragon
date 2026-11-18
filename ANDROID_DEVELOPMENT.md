# Android app development with Dragon

## Widget framework, compiled natively (current default)

`dragon create` scaffolds a Flutter-style, class-based app. **Your app is compiled to native code**:
the widget framework (`framework/*.dgn`) and your `lib/**.dgn` files are flattened into one
program, transpiled to C and compiled with the Android NDK clang into `libdragon_lib.so` for every
ABI. **No `.dgn` source and no interpreter ship in the APK** - only the native library, the thin
Kotlin/Jetpack Compose renderer (which draws the JSON frames the native code produces) and your
`assets/`. See [docs/FRAMEWORK.md](docs/FRAMEWORK.md) and [docs/UI_SCHEMA.md](docs/UI_SCHEMA.md).

Workflow: `dragon create MyApp` -> `cd MyApp` -> `dragon preview` (browser, development only)
-> `dragon run` (NDK build + Gradle + adb install + launch).

How `dragon build`/`run` produce the library: `dragon -c lib/main.dgn --native --app --target
android-<abi>` (done for you for each ABI in `package.json`). The library exports the JNI
functions of `org.dragonlang.runtime.DragonNative` (init, render, event, isDirty, tick,
platformCalls, platformResult, back, setMedia, lifecycle, lastError).

Environment: `ANDROID_NDK_HOME` (required, the NDK), `DRAGON_ANDROID_API` (min API, default 24).
`package.json`: `"runtime":"app"`, `abis`, `main`, `androidDir`. The Material icon font is bundled
automatically; `assets/` is copied into the APK.

`dragon preview` and `dragon ui-dump` run the app in Dragon's interpreter purely as *development
tooling* (hot reload in the browser, headless tests). The shipped Android app is always native.

**Packages:** audio and web views are packages, not part of the core: `import "package:dragon_audio" as *` gives
`MusicLibrary` / `AudioPlayer`, `import "package:dragon_web" as *` gives `Web` / `WebView`. The build copies a package's Kotlin,
permissions, manifest entries and Gradle dependencies into the Android project only when the app imports it, and removes
them again when it stops. Write your own the same way: [docs/PACKAGES.md](docs/PACKAGES.md). Samples: `examples/music_player`,
`examples/browser` ([docs/BROWSER.md](docs/BROWSER.md)).

**App permissions / Gradle:** `package.json` -> `"android": {"permissions": ["android.permission.CAMERA"], "gradle": ["implementation(...)"]}`.
The default manifest only declares what every app needs (internet, notifications); packages and `package.json` add the rest.

**iOS:** an untested SwiftUI renderer and `dragon ios init|build` exist, see [docs/IOS.md](docs/IOS.md).

**App-specific native code (`native/`):** Kotlin files in `<app>/native/kotlin/` and APK assets in `<app>/native/assets/`
are copied into the Android project on each build (and removed again when deleted); an app without `native/` gets
none. The browser's ad blocker, NSFW detector, filter lists and model live there, not in the framework.

**Verification status:** the native pipeline (framework + example apps transpiled to C and run,
frame-for-frame identical to the interpreter, including taps/timers/navigation and under GC
stress), the JNI entry points (exercised through a real JVM) and the project scaffolding were
tested in the authoring sandbox. The Kotlin/Compose code, Gradle build and the real NDK/bionic
cross-compilation could not be run there - expect to fix small compile errors on first build.
Language quirk: assignment updates an existing outer-scope name rather than shadowing it.

Known limitations of the native backend: classes must be defined before use at top level;
named functions nested inside functions do not capture enclosing locals (use `func` expressions);
`finally` blocks do not run on early `return` paths.

---


> **Status: experimental, still in active development.** Core app logic
> and the native build pipeline are well tested; the Android/Kotlin side
> is newer and less battle-tested. Expect rough edges.

Dragon can build Android apps - UI, app logic, and file storage - written
entirely in `.dgn` files, compiled to a native library and rendered by a
small Jetpack Compose interpreter. A CLI (`dragon create`/`run`/`build`)
handles project setup and building.

## Requirements

- The Android NDK, with `ANDROID_NDK_HOME` set (see
  [CROSS_COMPILE_TESTING.md](CROSS_COMPILE_TESTING.md)).
- Gradle, available either via a system install (e.g. `sdk install
  gradle` through [sdkman](https://sdkman.io/), or `brew install
  gradle`) or through Android Studio.

## Create a new project

```bash
dragon create MyApp --package com.example.myapp
```

This generates:

```
MyApp/
├── package.json      # project manifest (name, package, entry point)
├── lib/
│   ├── main.dgn        # your app
│   └── ui.dgn            # the widget library main.dgn imports
└── android/               # generated Android/Gradle project
```

**`lib/main.dgn` is where you build your app.** `lib/ui.dgn` and the
`android/` folder are generated for you and shouldn't need editing for
ordinary app development.

## Run it

```bash
cd MyApp
dragon run
```

Builds the app, installs it on a connected device or running emulator,
and launches it.

```bash
dragon run --watch
```

Same as `dragon run`, but keeps running: it watches every `.dgn` file
under `lib/`, and on any change automatically rebuilds, reinstalls, and
relaunches on the connected device. This is a full restart on each
change (app state resets), not an in-place stateful hot reload -- Dragon
compiles your app to native code per build, so true stateful hot reload
would need an interpreted runtime, which isn't how the Android backend
works today. `--watch` gets you most of the iteration-speed benefit
without that: save a file, the app is back on screen in a few seconds.

Compiled native libraries are written to `android/app/src/main/jniLibs/`
-- the standard location Android Gradle Plugin scans for prebuilt
`.so` files. That means one binary artifact does live under `src/`
(unlike Flutter, which pulls its engine in as a separate prebuilt
dependency); a generated `android/.gitignore` keeps it, along with
`build/` and Gradle's own caches, out of version control, so it never
gets committed even though it's technically inside the source tree.

## Build an APK without installing

```bash
dragon build            # debug build
dragon build --release  # release build
```

## Writing an app

`render()` returns your current UI as a tree of widgets; `on_event(evt)`
is called whenever something is tapped, changed, or otherwise
interacted with, and should update your app's state. Dragon calls
`render()` again after every event to get the updated UI.

```dragon
import ui;

tap_count = 0;

func render() {
    return Center(Col([
        StyledText("Welcome to MyApp", "", 22, ""),
        Text("Made with Dragon."),
        Button("Tap me", "tap"),
        StyledText("Tapped " + tap_count + " times", "#8E8E93", 0, "")
    ]));
}

func on_event(evt: str) {
    if (evt == "tap") {
        tap_count = tap_count + 1;
    }
    return 0;
}
```

## Legacy function-based widgets (`dragon create --legacy-native`)

The list below describes the older function-based `ui` module. For the current class-based framework see
[docs/COMPONENTS.md](docs/COMPONENTS.md).

**Layout**: `Row`, `SpacedRow`, `Column`, `ColoredColumn`,
`CenteredColumn`, `Stack`, `Wrap`, `ListView`, `GridView`,
`RefreshableListView`, `TabView`.

**Sizing & positioning**: `Container`, `Box`, `Circle`, `GradientBox`,
`GradientContainer`, `SizedBox`, `Padding`, `EdgePadding`,
`SymmetricPadding`, `Center`, `Align`, `AspectRatio`, `Positioned`,
`Expanded`, `Flexible`, `Spacer`, `Empty`.

`Container` takes optional trailing attributes beyond the basic
`(child, width_dp, height_dp, bg)` you may already know --
`radius_dp`, `border_color`, `border_width_dp`, `elevation_dp` -- so a
single call can describe a rounded, bordered, drop-shadowed box, no
extra wrapping needed. `Box`/`Circle`/`GradientBox` are the fully
childless versions of the same thing (a colored/gradient/rounded
rectangle or circle with attributes only, exactly like using Flutter's
`Container()` on its own with no child).

**Display**: `Text`, `StyledText`, `Heading`, `Subheading`, `Caption`,
`Icon`, `StyledIcon`, `SizedIcon`, `Image`, `SizedImage`,
`RoundedImage`, `Avatar`, `AvatarImage`.

**Input**: `ElevatedButton`, `ColoredButton`, `LongPressButton`,
`TextButton`, `OutlinedButton`, `IconButton`, `WideButton`,
`DisabledButton`, `LoadingButton`, `TextField`, `FocusTextField`,
`PasswordField`, `MultilineTextField`, `IconTextField`,
`ClearableTextField`, `NumberField`, `DecimalField`, `EmailField`,
`PhoneField`, `SubmitTextField`, `ValidatedTextField`,
`LimitedTextField`, `DisabledTextField`, `Checkbox`, `Switch`, `Slider`,
`RadioGroup`, `DropdownButton`.

**Interaction**: `GestureDetector`, `LongPressDetector` - make any widget
tappable.

**Feedback**: `CircularProgress`, `LinearProgress`, `AlertDialog`,
`Snackbar`.

**Structure & navigation**: `Card`, `ColoredCard`, `FlatCard`, `AppBar`,
`BackAppBar`, `FloatingActionButton`, `IconFloatingActionButton`,
`Scaffold` (navigation drawer, paired with `MenuButton()`),
`BottomNavigationBar`, `Chip`, `SelectableChip`, `Badge`, `WebView`.

**Multi-page navigation**: `Page(content)` marks a screen as part of a
navigable stack -- wrap whatever `render()` returns in `Page(...)` and
Dragon gives you a real back stack with slide/fade transitions between
screens, and the Android system back button wired up automatically.
Manage the stack with `nav_push(route)`, `nav_pop()`,
`nav_replace(route)`, `nav_reset(route)`, `nav_route()` (the current
route string), `nav_can_pop()`, and `nav_depth()`. See
`examples/nav_app.dgn` for a full multi-screen app built this way --
`on_event` just needs to handle a `"__nav_back__"` event (fired by the
system back button and by `BackAppBar`'s back arrow) by calling
`nav_pop()`.

**File storage**: `read_doc` / `write_doc` / `append_doc` / `doc_exists`
/ `delete_doc` for your app's own storage, `read_asset` / `asset_exists`
for bundled read-only files.

**Networking**: `http_get(url, event)` / `http_post(url, event, body)`
queue a single in-flight HTTP request; Dragon can't do async I/O itself,
so `DragonApp` polls for a pending request and performs it on a
background thread, then delivers the result back through the normal
`on_event` pipeline as `"<event>:<3-digit-status>:<body>"`. Parse it with
`http_status_of(evt, "<event>:")`, `http_body_of(evt, "<event>:")`, or
`http_ok(evt, "<event>:")`. Only one request is tracked at a time --
firing a second `http_get`/`http_post` before the first resolves
replaces it, rather than queuing both.

```dragon
func on_event(evt: str) {
    if (evt == "load") {
        http_get("https://api.example.com/todos/1", "got_todo");
    } else if (str_starts_with(evt, "got_todo:")) {
        if (http_ok(evt, "got_todo:")) {
            todo_json = http_body_of(evt, "got_todo:");
        }
    }
    return 0;
}
```

**Theming**: reassign `theme_primary` / `theme_secondary` /
`theme_background` / `theme_surface` (hex strings) and `theme_dark`
(bool) -- either directly at the top of `lib/main.dgn`, or at runtime via
`set_theme(primary, secondary, background, surface, dark)` /
`toggle_dark_mode()`. Applies to every themed widget on the next
`render()`, no restart needed. This covers the framework's own palette
(buttons, cards, text, backgrounds); it doesn't yet let you swap fonts or
shapes.

**Timers**: there's no `Future.delayed`/`Timer` builtin (Dragon has no
clock access), but `DragonApp` dispatches a synthetic `"__tick__"` event
once a second automatically. Handle it in your own `on_event` like any
other event, and keep your own counter for anything duration-based (e.g.
an auto-dismissing `Snackbar`):

```dragon
if (evt == "__tick__") {
    if (snackbar_visible) {
        snackbar_ticks = snackbar_ticks + 1;
        if (snackbar_ticks >= 3) { snackbar_visible = False; }
    }
}
```

**Lists**: `strlist` values now support `list.push(item)` (mutates in
place, appends `item`) and `list.pop()` (removes and returns the last
item, or `""` if empty), alongside the existing `list[i]` indexing and
`len(list)`. This required a real compiler change (`--native --lib` now
recognizes `.push()`/`.pop()` method syntax and lowers it to the C list
runtime that already existed but wasn't reachable from Dragon source) --
rebuild `dragon` itself (`cargo build --release`) to pick it up, not
just regenerate your project. A top-level `tabs = [];` global is valid
(literal empty-list initializers already worked); non-empty list
literals still can't be a top-level global's initial value, only `[]`.

**Browser-style WebView navigation**: `BrowserWebView(url, load_seq,
nav_command, nav_seq, state_event)` replaces plain `WebView(url)` when
you need back/forward/reload. Bump `load_seq` to force a fresh
`loadUrl()`; set `nav_command` to `"back"`/`"forward"`/`"reload"` and
bump `nav_seq` to fire it once. `state_event` fires on every completed
page load as `"<event>:<canGoBack 0/1>:<canGoForward 0/1>:<url>"`, so
you can keep your address bar and back/forward button states in sync
with what the WebView is actually showing (including navigation the
WebView does on its own, like following a link).

**Haptics**: automatic on long-press actions (`LongPressButton`,
`LongPressDetector`) and `Switch` toggles -- nothing to opt into.

**Not yet available**: push notifications, deep linking, and iOS
support.

`lib/ui.dgn` has the full reference.

## Example apps

Ships in `examples/`:

- `nav_app.dgn` - a multi-page app (welcome -> login -> tabbed home ->
  profile) exercising the navigation stack, `AlertDialog`/`Snackbar`,
  `BottomNavigationBar`, `RadioGroup`, `DropdownButton`, `Chip`,
  `Badge`, `Avatar`, and the richer `Container`/`Box` attributes. Copy
  it over `lib/main.dgn` in a project made with `dragon create` to try
  it on a device.

## Further reading

- [CROSS_COMPILE_TESTING.md](CROSS_COMPILE_TESTING.md) - NDK and
  cross-compilation setup.
