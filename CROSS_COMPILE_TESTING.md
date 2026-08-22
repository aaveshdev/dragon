# Using `--native --lib --target ...`

## 1. Build the CLI

```bash
cargo build --release
# binary at target/release/dragon (or dragon.exe on Windows)
```

## 2. A minimal test script

```dragon
// mathlib.dgn
func add(a, b) {
    return a + b;
}

func greet_len(name) {
    return len(name);
}
```

Top-level `func`s become the exported C-ABI symbols.

## 3. Android

Requires the Android NDK, with `ANDROID_NDK_HOME` set:

```bash
sdkmanager --install "ndk;26.1.10909125"
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/26.1.10909125
```

Build:

```bash
dragon -c mathlib.dgn -o mathlib --native --lib --target android-all
```

Output:

```
out_dir/
  jniLibs/
    arm64-v8a/libmathlib.so
    armeabi-v7a/libmathlib.so
    x86_64/libmathlib.so
    x86/libmathlib.so
  mathlib.h
```

Copy `jniLibs/` into `app/src/main/jniLibs/` in your Android Studio
project (Gradle picks it up automatically), then load + call it from
Kotlin via `System.loadLibrary("mathlib")` + `external fun` JNI
declarations matching `mathlib.h`.

## 4. iOS (macOS + Xcode required)

```bash
xcode-select --install     # if not already done
dragon -c mathlib.dgn -o mathlib --native --lib --target ios-all --xcframework
```

Output:

```
out_dir/
  ios/
    ios-arm64/libmathlib.a
    ios-arm64-sim/libmathlib.a
    ios-x86_64-sim/libmathlib.a
    include/mathlib.h
  mathlib.xcframework/
  mathlib.h
```

Drag `mathlib.xcframework` into your Xcode project (Frameworks pane, "Do
Not Embed" for a static xcframework), then `#import "mathlib.h"` from a
bridging header to call it from Swift.
