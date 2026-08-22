# Android app development with Dragon

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
    return Center(Column([
        StyledText("Welcome to MyApp", "", 22, ""),
        Text("Made with Dragon."),
        ElevatedButton("Tap me", "tap"),
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

## What's available right now

**Layout**: `Row`, `Column`, `Stack`, `Wrap`, `ListView`, `GridView`.

**Sizing & positioning**: `Container`, `SizedBox`, `Padding`, `Center`,
`Align`, `AspectRatio`, `Positioned`, `Expanded`, `Flexible`, `Spacer`.

**Display**: `Text`, `StyledText`, `Icon`, `StyledIcon`, `Image`,
`SizedImage`.

**Input**: `ElevatedButton`, `ColoredButton`, `LongPressButton`,
`TextField`, `FocusTextField`, `Checkbox`, `Switch`, `Slider`,
`RadioGroup`, `DropdownButton`.

**Interaction**: `GestureDetector`, `LongPressDetector` - make any widget
tappable.

**Feedback**: `CircularProgress`, `LinearProgress`, `AlertDialog`,
`Snackbar`.

**Structure & navigation**: `Card`, `AppBar`, `FloatingActionButton`,
`Scaffold` (navigation drawer, paired with `MenuButton()`),
`BottomNavigationBar`, `Chip`, `Badge`, `WebView`.

**File storage**: `read_doc` / `write_doc` / `append_doc` / `doc_exists`
/ `delete_doc` for your app's own storage, `read_asset` / `asset_exists`
for bundled read-only files.

**Not yet available**: network access, and iOS support.

`lib/ui.dgn` has the full reference - every function includes a comment
explaining its parameters.

## Example apps

Ship in `examples/`:

- `todo_app.dgn` - a to-do list.
- `calculator.dgn` - a styled calculator.
- `nav_app.dgn` - multi-page navigation with a sidebar drawer and a
  floating action button.

## Further reading

- [CROSS_COMPILE_TESTING.md](CROSS_COMPILE_TESTING.md) - NDK and
  cross-compilation setup.
