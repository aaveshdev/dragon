# Windows desktop

Dragon apps can run as native Windows desktop apps. The pipeline is the same as on phones: your `.dgn` code is
transpiled to C and compiled to a native `dragon_lib.dll`. A thin WPF (.NET 8) host loads that DLL, asks it for JSON
frames and draws them with native WPF controls. No `.dgn` source and no interpreter ship with the app.

## Status, exactly

| Part | State |
| --- | --- |
| `dragon_lib.dll` (x64) cross-compiled from Linux with `zig cc` | built and its 11 `dgn_app_*` exports checked (tests/run.sh) |
| WPF host, renderer, flex layout, controls (`templates/windows/cs/*.cs`) | built and run on a real Windows machine (.NET SDK 10, clang): the `counter` and `components` examples work. Other examples, plugins and edge cases are untested |
| `dragon_web` (WebView2) and `dragon_audio` (Windows media player + TagLibSharp) | written, syntax-checked only; **not yet run**. The browser's content-blocking and privacy features that rely on Android-only assets are not wired on Windows |

Expect visual differences on screens other than the two examples above. `DragonRenderer.kt` (Android) is the
reference for behaviour when something looks different.

## Using it

```
dragon create myapp                    # android + ios + windows by default
dragon create myapp --platforms windows
cd myapp
dragon build --platform windows        # DLL on any OS; the .exe needs the .NET 8 SDK (Windows)
dragon run --platform windows          # on Windows: build and launch
dragon windows init                    # only regenerate windows/ (runtime sources, plugins registry, csproj)
```

On Linux or macOS `dragon build --platform windows` stops after producing `windows/Native/dragon_lib.dll`. Copy the
project to Windows and run `dragon build --platform windows` there. Cross-compiling the DLL needs `zig` (`pip install ziglang`) or mingw-w64.

## Project layout

```
windows/
  <name>.csproj          WinExe, net8.0-windows, UseWPF
  app.manifest           per-monitor DPI
  Sources/DragonRuntime/ host, renderer, controls (rewritten by `dragon windows init`)
  Sources/DragonPlugins.cs   generated: window title and plugin registry
  Sources/Plugins/<pkg>/     C# from packages (native/windows/cs)
  Resources/               icon font, assets/ (copied next to the exe)
  Native/dragon_lib.dll    generated
```

## package.json

```json
{
  "platforms": ["android", "ios", "windows"],
  "windows": { "title": "My App", "dir": "windows" }
}
```

## Plugins for Windows

A package adds a `windows` key and C# under `native/windows/cs/`:

```json
{ "windows": { "plugin": "MyPlugin", "nuget": ["Microsoft.Web.WebView2:1.0.2592.51"] } }
```

```csharp
public sealed class MyPlugin : DragonPluginBase
{
    public MyPlugin(PluginContext ctx) { }
    public override string Namespace => "my";
    public override bool Handle(string method, JsonElement args, Action<object> reply) { reply("ok"); return true; }
}
```

The class needs a constructor taking `PluginContext`. Native views (`nativeview`) come from `CreateView`.

## Behaviour notes

- Back: Escape, Alt+Left or the mouse back button. Window resize updates the media metrics.
- Dark mode follows the Windows app theme unless the app sets one.
- Web view needs the WebView2 runtime (preinstalled on Windows 11). Audio plays files from your Music folder (mp3, m4a, aac, wav, wma, flac, ogg, opus); the equalizer is not available.
- Not available yet: camera and location (`takePhoto`, `location` reply with an error).

## Example: desktop browser

`examples/desktop_browser` is a tabbed desktop browser (address bar with suggestions, bookmarks bar, find, zoom, history, downloads, settings, private tabs, keyboard shortcuts). The `dragon_web` Windows plugin reports browser shortcuts (Ctrl+T, Ctrl+W, ...) as `shortcut` events, because WebView2 swallows key presses that happen inside a page.

## Native views (WebView2) and layering

WPF cannot draw Dragon widgets on top of a native HWND-hosted control (the "airspace" problem). The Windows host therefore
keeps native views in their own persistent layer above the page: `NativeView` leaves an empty placeholder in the tree, and after each layout
the real control is moved to the placeholder's rectangle. Because the control is never re-parented, rebuilding the
frame does not reload or flicker a web page. While a dialog or sheet is open the native views are hidden, and a snackbar leaves a
strip at the bottom free. Layout rule for apps: do not overlay Dragon widgets on a `WebView`; put them in a row or column next to it
(see `examples/desktop_browser`).
