# Ember (Windows desktop)

A Chrome/Edge-style desktop browser written in Dragon. Pages are rendered by WebView2 through the `dragon_web` package;
everything else (tab strip, toolbar, address bar with suggestions, bookmarks bar, find bar, new tab page, history,
bookmarks, downloads, settings) is Dragon UI code compiled to native code.

```
cd examples/desktop_browser
dragon run --platform windows      # needs the .NET SDK and the WebView2 runtime (preinstalled on Windows 11)
dragon preview                     # layout preview in a normal browser (no real web pages)
```

Files: `lib/main.dgn` (window shell, shortcuts), `lib/store.dgn` (settings, tabs, bookmarks, history),
`lib/widgets.dgn` (tab strip, bookmarks bar, suggestions, find bar), `lib/pages.dgn` (new tab, history, bookmarks,
downloads, settings), `lib/util.dgn`.

Shortcuts: Ctrl+T / Ctrl+W / Ctrl+Shift+T tabs, Ctrl+Tab / Ctrl+Shift+Tab / Ctrl+1..9 switch, Ctrl+L or F6 address bar,
Ctrl+F find, Ctrl+D bookmark, Ctrl+H history, Ctrl+J downloads, Ctrl+R / F5 reload, Alt+Left / Alt+Right back / forward,
Ctrl+Plus / Minus / 0 zoom, Ctrl+Shift+N private tab, Ctrl+Shift+Delete clear data, F12 developer tools.

Status: runs natively in the test driver and matches the interpreter frame for frame. The Windows renderer and the WebView2
plugin have not been run by the author on a real machine with this app; see docs/WINDOWS.md.
