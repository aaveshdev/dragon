# Dragon Programming Language

**Dragon** is an innovative, practical, general-purpose programming language. It's designed to be simple, small, flexible, and fast, with an expressive, readable syntax that's easy to pick up. Dragon supports multiple programming paradigms (imperative, object-oriented, functional, and declarative) and is portable across Windows, Linux, and macOS, and can be used to build console applications.

This repository contains the official Rust implementation of Dragon: a lexer, parser, bytecode compiler, and virtual machine, plus a C transpiler for native/AOT-style builds, a batteries-included standard library, and a terminal-based IDE ("TurboDragon IDE").

> Official website & documentation: [dragon-lang.org](https://www.dragon-lang.org/)

## Features

- **Simple, readable syntax** — spend less time fighting the language, more time solving problems.
- **Dynamic typing** with first-class functions, closures, and classes (`func`, `class`, `extends`, `this`, `super`, `new`).
- **Familiar control flow** — `if` / `elif` / `else`, `while`, `for ... in`, `switch` / `case`, `try` / `catch` / `finally` / `raise`.
- **Async/await & threading** — built-in `async` / `await` support plus a `thread` module for spawning concurrent work.
- **Rich standard library** available via `import`, including:
  - `math` — numeric and math utilities
  - `json` — encode/decode JSON
  - `file` / `path` — filesystem access
  - `os` — environment and system info
  - `time` — dates, timers, sleeping
  - `net` — HTTP client/server utilities
  - `crypto` — hashing (SHA-2, MD5), HMAC, PBKDF2, AES-GCM
  - `zip` — archive creation/extraction
  - `xml` — XML parsing
  - `uuid` — UUID generation
  - `pdf` / `font` — PDF and font/text rendering
  - `ffi` / `struct` / `mem` — native interop with C libraries
- **Bytecode VM** for fast interpreted execution.
- **Native compilation (experimental)** — transpile Dragon source to C and build it with your system's C compiler (gcc/clang/MSVC).
- **Self-contained executables** — bundle a script (and its dependencies) into a single encrypted, distributable binary.
- **Shared library builds** — compile Dragon scripts into a `.dll` / `.so` / `.dylib` for embedding in other applications.
- **TurboDragon IDE** — a built-in terminal UI editor/runner, launched with `--ide`.
- **Package-based projects** — run a project directly from a `package.json` manifest with `dragon run`.

## Android app development (experimental)

Dragon can compile to a real Android `.so` via `dragon --native --lib
--target android-all`, callable directly from Kotlin via generated JNI
bindings (`--jni-package` / `--jni-class`) — no hand-written JNI bridge
required. On top of that, `dragon`, `ui.dgn`, and a small generic Jetpack
Compose renderer together let you build an app's entire UI and logic in
`.dgn` files: layouts (row, column, stack, grid, list_view), components
(text, button, textfield, checkbox, switch, slider, icon, image, card,
app_bar, fab, webview), navigation (`drawer_scaffold`), per-widget
styling, file storage (documents and assets), and app state — all driven
from Dragon.

`dragon create <name> [--package <id>]` scaffolds a full project — a
`package.json` manifest, `lib/main.dgn` + `lib/ui.dgn` (the only files you
should need to touch day to day), and a complete Android Gradle project.
`dragon build` and `dragon run` (from inside a generated project)
compile your `.dgn` app to native `.so`s, build the Android project, and
— for `run` — install and launch on a connected device or emulator.

This is genuinely experimental and young, not a finished framework:

- **Verified rigorously, with real confidence**: the native compilation
  pipeline itself — the C transpiler, the JNI wrapper generation, the
  string/list memory-ownership model, and the file-storage builtins —
  stress-tested with AddressSanitizer across several real apps (a stateful
  to-do list, a calculator, a multi-page app with a sidebar drawer and a
  floating action button) through many rounds of finding and fixing
  genuine use-after-free and leak bugs. `dragon create`'s project
  scaffolding is real, tested output too — every generated file was
  inspected, and the generated `lib/main.dgn` was compiled and run.
- **Not yet verified on real hardware/toolchains by us**: the Android/
  Kotlin side of the example integration (`examples/android/*.kt`) is
  hand-written and reviewed, not compiled here — you are the first real
  test of any given change to it. `dragon build`/`dragon run`'s Gradle and
  ADB steps are similarly written carefully but never executed end to end
  (no Gradle/ADB/emulator available in the environment this was built in)
  — treat your first real run as a genuine first test. `dragon create`
  does not generate a Gradle wrapper (a binary jar with no way to vendor
  it here); open the generated project in Android Studio once, or run
  `gradle wrapper` yourself, before `dragon build`/`dragon run` will work.
  iOS is not supported at all yet: the `--target ios-*` cross-compilation
  flags work, but no SwiftUI renderer exists, so there's currently no way
  to actually display a Dragon UI on iOS.
- **Language limits that matter for real apps**: `--native --lib` has
  scalar types plus a single collection type (`strlist`, a list of
  strings) — no structs/records, no closures, no generics, single-
  threaded only. State that would naturally be a struct (e.g. a to-do
  item) has to be encoded as a delimited string and parsed by hand today.
  Identifiers that collide with C keywords (e.g. a Dragon function named
  `goto`) currently produce invalid C rather than a clear error — rename
  around it for now.
- **No network access**: implementing HTTPS/TLS safely from scratch in
  plain C is a large, security-sensitive undertaking not worth rushing.
  The documented pattern instead: the host app performs the request and
  calls a Dragon function with the result, the same way `on_event()`
  already works.

See `examples/android/UI_BRIDGE_README.md` for the full integration guide,
and `CROSS_COMPILE_TESTING.md` for cross-compilation setup (NDK, and the
Android-specific 16KB page-size linking requirement).

## Example

```dragon
import "thread";
import "math";

class Animal {
    func init(name) {
        this.name = name;
    }
    func speak() {
        showln(this.name, "speaks");
    }
}

func worker(id) {
    sleep(0.5);
    showln("Task", id, "done");
}

animal = new Animal("Dragon");
animal.speak();

for i in math.range(5) {
    thread.spawn(worker, [i]);
}

showln("All tasks started");
```

## Building

Dragon is built with Cargo and requires a recent stable [Rust toolchain](https://www.rust-lang.org/tools/install) (edition 2021).

```bash
# Clone the repository
git clone https://github.com/aaveshdev/dragon.git
cd dragon

# Build the CLI in debug mode
cargo build

# Build an optimized release binary (recommended)
cargo build --release
```

The compiled CLI binary is produced as `dragon` (or `dragon.exe` on Windows) at:

```
target/release/dragon
```

Dragon also builds a shared library (`dragon_lib`, i.e. `dragon_lib.dll` / `libdragon_lib.so` / `libdragon_lib.dylib`) from `src/lib.rs`, since the `[lib]` target uses `crate-type = ["cdylib", "rlib"]`. This is produced automatically alongside the CLI whenever you run `cargo build`.

### Optional: install the CLI globally

```bash
cargo install --path .
```

This installs the `dragon` binary onto your `PATH`.

### Feature/target notes

- **Native codegen** (`--native`) shells out to a system C compiler (`gcc`, `clang`, or `cl` on Windows), so one of these must be available on your `PATH` for that feature to work.
- **Android** targets pull in `jni` / `ndk` dependencies automatically via the `cfg(target_os = "android")` block in `Cargo.toml`.
- **WASM** targets pull in `wasm-bindgen` / `web-sys` / `getrandom` (`js` feature) automatically.
- On `aarch64-unknown-linux-gnu`, OpenSSL is vendored and built from source, so a C toolchain is required there as well.

## Running

Once built, invoke the binary directly (or via `cargo run --`) followed by a command:

```bash
dragon <file.dgn> [args...]        # Run a Dragon script
dragon --repl                      # Start the interactive REPL
dragon run                         # Run the project defined by ./package.json
dragon --ide                       # Launch the TurboDragon terminal IDE
dragon -v | --version              # Print version info
```

### Run a script

```bash
dragon hello.dgn
```

Any extra arguments after the filename are passed through to the script and are accessible as `argv` inside Dragon code.

### Interactive REPL

```bash
dragon --repl
```

Starts a persistent session — variables and functions defined on one line remain available on the next. Type `exit` to quit.

### Run a project via `package.json`

```bash
dragon run
```

Looks for a `package.json` in the current directory and runs the file referenced by its `"main"` (or `"entry"`) field, e.g.:

```json
{
  "name": "my-dragon-app",
  "main": "src/main.dgn"
}
```

### Compile a script

```bash
# Bundle a script (and any included files) into a single distributable executable
dragon -c input.dgn -o output

# Same, but hide the console window on Windows (background/GUI mode)
dragon -c input.dgn -o output --window

# Compile into a shared library (.dll / .so / .dylib)
dragon -c input.dgn -o output.dll --lib

# (Experimental) Transpile to C and build a native binary via gcc/clang/MSVC
dragon -c input.dgn -o output --native

# (Experimental) Native shared library
dragon -c input.dgn -o output --native --lib

# Keep the intermediate generated C source when using --native
dragon -c input.dgn -o output --native --keep-c
```

### TurboDragon IDE

```bash
dragon --ide
```

Launches a lightweight, keyboard-driven terminal IDE (built with `crossterm`) for editing and running Dragon scripts without leaving the terminal.

## Project Structure

```
src/
├── main.rs           # CLI entry point (run, repl, compile, bundle, ide)
├── lib.rs             # Library entry point (cdylib/rlib target)
├── tui.rs             # TurboDragon terminal IDE
├── engine/            # Core language implementation
│   ├── lexer.rs        # Tokenizer
│   ├── parser.rs       # Parser -> AST
│   ├── ast.rs           # AST node definitions
│   ├── compiler.rs     # AST -> bytecode compiler
│   ├── bytecode.rs     # Bytecode instruction set
│   ├── vm.rs             # Bytecode virtual machine
│   ├── value.rs         # Runtime value types
│   ├── environment.rs  # Variable/scope environment
│   ├── builtins.rs      # Standard library (math, json, file, net, crypto, ...)
│   ├── linter.rs         # Static AST linting
│   ├── crypto.rs         # Cryptography helpers
│   ├── bundle.rs         # Executable bundling/packaging
│   ├── gc.rs               # Memory/garbage-collection helpers
│   └── error.rs           # Error types and pretty-printing
└── codegen_c/          # Experimental Dragon -> C transpiler ("--native")
    ├── transpiler.rs
    ├── runtime.rs
    ├── native.rs
    └── mangle.rs
```

## License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.
