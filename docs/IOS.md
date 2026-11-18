# iOS (UNTESTED skeleton)

**Read this first.** iOS support was written on a Linux machine with no Xcode, no Swift compiler and no Mac. What
that means:

| Part | Status |
|---|---|
| App -> C -> static library with the plain `dgn_app_*` ABI (same code Android uses) | the C compiles with the host compiler, exports all 11 functions, and boots and renders through the generated header. **Not** cross-compiled for iOS. |
| `dragon ios init` (project inputs, plugin sync, `project.yml`) | runs and produces the files; the YAML/JSON is not checked by XcodeGen |
| `dragon ios build` (iOS slices, xcframework, XcodeGen, xcodebuild) | **never run**: needs macOS |
| Swift host, platform layer, plugin protocol, SwiftUI renderer, web and audio plugins | **only syntax-checked** (tree-sitter), never type-checked or run. Expect compile errors on first contact. |

So: the architecture and the glue exist; a first pass of fixes on a Mac is expected before it runs.

## Using it

On a Mac with Xcode and [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`):

```
dragon ios init            # writes ios/: Swift runtime, plugins, project.yml (safe to re-run)
dragon ios build           # compiles the app for device + simulator, makes ios/Native/dragon_lib.xcframework,
                           # runs xcodegen and xcodebuild (simulator). --simulator skips the device slice.
open ios/<Name>.xcodeproj  # run on a device from Xcode (set your signing team)
```

`package.json` keys (all optional): `ios.bundleId` (defaults to `package`), `ios.minVersion` (16.0),
`ios.dir` ("ios"), `ios.infoPlist` (extra Info.plist entries, e.g. usage descriptions).

Like Android, the app ships as compiled native code: no `.dgn` source and no interpreter in the app bundle.

## What the renderer covers

Implemented (first pass): text, richText (no per-span taps), icon (bundled Material Icons font), image
(asset/file/network), container (size, color, gradient, radius, border, shadow, padding, alignment), padding, align,
sizedBox, constrained, fractional, aspectRatio, opacity, clip, transform, safeArea, flex, flexible, wrap, stack,
positioned, divider, scroll (with controller commands), list, grid, pageView, refresh, gesture (tap, double tap, long
press, pan, h/v drag), dismissible, canvas (line, rect, rrect, circle, oval, text), button variants, checkbox, switch,
radio, slider, progress, dropdown, popupMenu, textField, scaffold (appBar, bottom nav, fab, drawers, bottom sheet),
appBar, navBar, tabBar, dialogs / sheets / snackbars, `nativeview` through plugins, unknown nodes fall back to children.

Known gaps against Android: `flexible` ignores the flex ratio (SwiftUI shares space equally), canvas `arc`/`path` ops, rich
text span taps, virtualised lists (`total/offset/onWindow`) are drawn as plain lists, keyboard inset is not reported
(SwiftUI handles it itself), the theme scheme is only used for backgrounds and primary colour, animated container
transitions are approximate, text selection toolbars. Platform methods not implemented: `datePicker`, `timePicker`,
`pickFile`, `takePhoto`, `location`, `orientation`, `statusBar` (they reply with an error or nothing).

## Making it real

1. Run `dragon ios build` on a Mac, fix compile errors (expect a handful in `DragonRenderer.swift`).
2. Run the counter example, then `examples/components`; compare with `dragon preview`.
3. Fill the platform gaps above, one at a time, as plugins where possible.
