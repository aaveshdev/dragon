# dragon_sketch

A pen pad for sketches and signatures, written in plain Dragon (no native code, so it behaves the same on Android,
iOS and Windows).

```dragon
import "package:dragon_sketch" as *

SketchPad(value: saved, onChange: func(v) { saved = v }, height: 220, hint: "Sign here")
```

* `value` / `onChange` carry one JSON string: `{"data": {"width", "height", "strokes"}, "image": null, "path": null}`.
  An empty pad reports `""`. Strokes are lists of `[x, y]` points in the pad's own pixels.
* `sketchWithImage(value, outW = 640, lineW = 3)` returns the same value with `image` filled in as a
  `data:image/png;base64,...` string (a 1-bit black-on-white PNG), or `""` for an empty pad. Do this when you upload,
  not on every stroke.
* `sketchPngBase64(strokes, srcW, srcH, outW, lineW)` and `sketchPngBytes(...)` render strokes without the widget.
* The PNG uses uncompressed zlib blocks, so size is about `outW * outH / 8` bytes (640 x 451 is roughly 36 KB).
  Lower `outW` for smaller uploads.

The shape matches the WorkflowManager `InkData` object (`data`, `image`, `path`).
