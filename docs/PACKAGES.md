# Packages and plugins

Dragon's core stays small on purpose. Anything beyond it (audio, web views, maps, an ad blocker, a UI kit) is a
**package**: a folder you import. A package can contain Dragon code, native Android code (Kotlin), native iOS code
(Swift), permissions and build settings. An app only gets the native code of the packages it imports.

## Using a package

```dragon
import "package:dragon_audio" as *          // main file of the package
import "package:dragon_web/extra" as web    // lib/extra.dgn inside the package
```

Lookup order, first match wins:

1. `<app>/packages/<name>/`
2. `dependencies` in the app's `package.json`: `{"dependencies": {"my_kit": {"path": "../my_kit"}}}`
3. every folder listed in the `DRAGON_PACKAGES` environment variable
4. packages that ship with the toolchain (`dragon_audio`, `dragon_web`)

## Writing a package

```
my_package/
  package.json
  lib/my_package.dgn          Dragon code (the file `package:my_package` loads)
  lib/more.dgn                `package:my_package/more`
  native/android/kotlin/      Kotlin files copied into the Android project
  native/android/assets/      files copied into the APK assets
  native/android/manifest.xml <service>/<receiver> XML added inside <application>
  native/ios/swift/           Swift files copied into the Xcode project (UNTESTED, see IOS.md)
```

```json
{
  "name": "my_package", "version": "0.1.0", "main": "lib/my_package.dgn",
  "dependencies": { "other": { "path": "../other" } },
  "android": {
    "plugin": "org.example.MyPlugin",
    "permissions": ["android.permission.INTERNET"],
    "gradle": ["implementation(\"androidx.webkit:webkit:1.12.1\")"],
    "manifest": "native/android/manifest.xml"
  },
  "ios": { "plugin": "MyPlugin", "infoPlist": { "NSCameraUsageDescription": "..." } }
}
```

A permission is a string, or `{ "name": "...", "maxSdkVersion": 32 }`.

## Plugins: Dragon <-> native

A plugin is one class per platform that answers calls from Dragon and can provide native views.

```dragon
Platform.invoke("greeter.hello", {name: "Ada"}, func(result) { print(result.text) })
func build(ctx, s) { return Col([ NativeView(kind: "map", id: "m1", params: {lat: 52.5, lng: 13.4}) ]) }
```

```kotlin
class GreeterPlugin(ctx: PluginContext) : DragonPlugin {
    override val namespace = "greeter"                         // answers "greeter.*"
    override fun handle(method: String, args: JSONObject, reply: (Any?) -> Unit): Boolean {
        reply(JSONObject().put("text", "Hello " + args.optString("name"))); return true
    }
    override fun createView(kind: String, id: String, params: JSONObject, context: Context): View? = null
}
```

```swift
final class GreeterPlugin: DragonPlugin {
    let namespace = "greeter"
    init(context: PluginContext) {}
    func handle(_ method: String, args: [String: Any], reply: @escaping (Any?) -> Void) -> Bool {
        reply(["text": "Hello \(args["name"] ?? "")"]); return true
    }
}
```

`reply` must be called once for calls that pass a callback. A call nobody answers calls back with `null`.
`dragon build` / `dragon ios init` generate `DragonPlugins.kt` / `DragonPlugins.swift` from the manifests, so there is
nothing to register by hand. The manifest, permission and Gradle blocks in the generated Android project are rewritten
between `dragon:` marker comments on every build, and removed again when you stop importing the package.

App-level permissions and Gradle lines go in the app's own `package.json` under `android` (same keys); iOS Info.plist
entries under `ios.infoPlist`.

## What ships

| Package | Dragon API | Android | iOS (untested) |
|---|---|---|---|
| `dragon_audio` | `MusicLibrary`, `AudioPlayer` | Media3 foreground service, notification, lock screen, media library | AVPlayer queue only: no lock-screen controls, no media library, no shuffle/sleep/equalizer |
| `dragon_sketch` | `SketchPad`, `sketchParse`, `sketchWithImage`, `sketchPngBase64` | pure Dragon: nothing native, same on every platform | same |
| `dragon_web` | `Web`, `WebView` | WebView tabs, content-blocking hooks, find, snapshots, downloads | WKWebView tabs: open/load/back/forward/reload/stop/eval; no blocking hooks, find, snapshots, downloads |

## Package ideas that need no change to the core

maps, video, camera, biometrics, SQLite, charts, a Cupertino-style look, an ad blocker. If a feature needs a new
node type that the renderers do not know, add a `NativeView` kind in your plugin instead of changing the framework.

## Windows

Add `"windows": { "plugin": "ClassName", "nuget": ["Id:Version"] }` to the manifest and put C# files in `native/windows/cs/`.
`dragon windows init` copies them into the project and generates the registry. See `docs/WINDOWS.md`.
