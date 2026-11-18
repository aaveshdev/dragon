# Component coverage

Components marked *(extras)* need `import "dragon/extras" as *` in addition to `dragon/ui`.

What the framework ships in `import "dragon/ui" as *`, and what it does not (yet). "Verified" means rendered and
compared in the browser preview and in the natively compiled test driver (identical frames); nothing here has been
checked on a physical device by the author.

| Area | Components |
|---|---|
| App & screens | `App`, `Screen`, `TopBar`, `SideMenu`, `AccountHeader`, `BottomBar`, `NavBar`, `NavItem`, `NavRail` *(extras)*, `Tabs`, `TabPages`, `TabHost`, `ActionButton` |
| Navigation | `Nav.push`, `pop`, `pushNamed`, `popUntil`, `pushReplacement`, `Route`, `CustomRoute`, `Alert`, `ChoiceDialog`, `showSheet`, `Notices` / `notify` (`Notice`), `Menu` |
| Layout | `Box`, `Pad`, `Place`/`Center`, `Space`, `Limit`, `Portion`, `Ratio`, `Scaled`, `Row`, `Col`, `Grow`, `Weighted`, `Spring`, `Layers`, `Pin`, `Pick`, `Flow`, `Rule`, `Safe`, `Show`, `Fade`, `Round`, `Oval`, `Cut`, `Transform`, `SizeBuilder`, `OrientationBuilder` |
| Scrolling | `Scroll`, `Feed` (+ `.builder`, virtual lists), `Grid`, `Pager`, `PullRefresh`, `ScrollController`, `Carousel` *(extras)* |
| Text & input | `Text`, `RichText`, `SelectableText`, `Input`, `FormInput`, `Form`, `TextController`, `SearchField`, `Suggest` *(extras)* |
| Controls | `Button` (+ `SolidButton`, `LineButton`, `FlatButton`, `IconButton`), `Check`, `Toggle`, `Radio`, `Slider`, `Dropdown`, `Segments`, `Rating`, `Steps`, `Calendar` *(extras)* |
| Content | `Item`, `SwitchItem`, `CheckItem`, `RadioItem`, `Expander`, `Card`, `Surface`, `Avatar`, `Badge`, `Tag` family, `Banner`, `EmptyState`, `TableView` *(extras)*, `Picture`, `Icon` (full Material set), `Spinner`, `ProgressBar` |
| Gestures | `Touch` (tap, double tap, long press, pan, h/v drag), `Tap`, `Swipe` (swipe to dismiss) |
| Animation | `AnimBox`, `AnimFade`, `AnimPad`, `AnimPlace`, `AnimSwap`, `AnimCross`, `Animator`, tweens, curves |
| Drawing | `Canvas` / `Paint` / `Path` / `Drawing` |
| State | `View`, `LiveView` + `Live`, `Component` (`state()` / `build(ctx, s)` / `ctx.set`), `Provider`, `Signal`, `Notifier`, builders |
| Services | `Http`, `Prefs`, `Clipboard`, `Haptics`, `Toast`, `Links`, `Share`, `DeviceInfo`, `Permissions`, `Notifications`, `Pickers` (date, time, file, photo), `Location`, `SystemBars`, `DateTime`, `Time` |
| Packages | `dragon_audio`, `dragon_web`; write your own (PACKAGES.md) |

## Not there

* Drag-to-reorder and drag and drop: `ReorderList` moves items with controls, there is no `Draggable` / `DragTarget`.
* Slivers and collapsing app bars, `InteractiveViewer` (pinch zoom / pan), `RangeSlider`, `ClipPath`, `BackdropFilter`
  (blur), text-selection toolbars, `Hero` transitions (the widget exists but only wraps its child).
* Video, maps, camera preview, charts, SQLite, biometrics: intended as packages with a plugin; none written.
* Accessibility: `Label` / `NoLabel` exist but the renderers do not pass them to TalkBack / VoiceOver yet.
* Right-to-left layout, locale-aware formatting.
* Gradle, the NDK cross build and everything on a real device or iOS: see IOS.md and the "verified" list in the changelog.
