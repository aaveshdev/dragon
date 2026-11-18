# Dragon

Dragon is a general-purpose programming language with a readable syntax. It supports imperative, object-oriented and
functional styles, and runs on Windows, Linux and macOS.

This repository is the Rust implementation: lexer, parser, bytecode compiler and VM, a C transpiler for native builds, a
standard library, a terminal IDE (TurboDragon), a UI framework for Android, iOS and Windows apps, and a bare-metal target
that can host an operating system written in Dragon. Vaelis is one, and lives in its own project.

Website and docs: [dragon-lang.org](https://www.dragon-lang.org/). Build steps are in [BUILD.md](BUILD.md).

## Features

- **Readable syntax** that is quick to pick up.
- **Dynamic typing** with first-class functions, closures, and classes (`func`, `class`, `extends`, `this`, `super`, `new`).
- **Familiar control flow**: `if` / `elif` / `else`, `while`, `for ... in`, `switch` / `case`, `try` / `catch` / `finally` / `raise`.
- **Async/await & threading**: built-in `async` / `await` support plus a `thread` module for spawning concurrent work.
- **Standard library** available via `import`, including:
  - `math`: numeric and math utilities
  - `json`: encode/decode JSON
  - `file` / `path`: filesystem access
  - `os`: environment and system info
  - `time`: dates, timers, sleeping
  - `net`: HTTP client/server utilities
  - `crypto`: hashing (SHA-2, MD5), HMAC, PBKDF2, AES-GCM
  - `zip`: archive creation/extraction
  - `xml`: XML parsing
  - `uuid`: UUID generation
  - `pdf` / `font`: PDF and font/text rendering
  - `ffi` / `struct` / `mem`: native interop with C libraries
- **Bytecode VM** for running scripts directly.
- **Native compilation (experimental)**: translate Dragon source to C and build it with your system's C compiler (gcc/clang/MSVC).
- **Self-contained executables**: bundle a script (and its dependencies) into a single encrypted, distributable binary.
- **Shared library builds**: compile Dragon scripts into a `.dll` / `.so` / `.dylib` for embedding in other applications.
- **TurboDragon IDE**: a built-in terminal UI editor/runner, launched with `--ide`.
- **Bare metal**: build a freestanding image for x86-64, RISC-V 64 or AArch64 (`--baremetal`) and run it in QEMU, with no operating system underneath. See [baremetal/README.md](baremetal/README.md).
- **Package-based projects**: run a project directly from a `package.json` manifest with `dragon run`.

## Platforms

One project can target several platforms. `package.json` lists them:

```json
{ "platforms": ["android", "ios", "windows"] }
```

`dragon create <name>` generates all three (`--platforms android,windows` to choose). `dragon run [--platform p]` and
`dragon build [--platform p|all]` act on the platforms in that list; with no flag, `run` picks the one that matches your
OS and `build` builds all. The older single `"platform": "android"` key still works.

| Platform | Renderer | Status |
| --- | --- | --- |
| Android | Jetpack Compose | Kotlin type-checks; not yet run on a device |
| iOS | SwiftUI (`docs/IOS.md`) | untested |
| Windows | WPF (`docs/WINDOWS.md`) | `counter` and `components` run on Windows; other screens untested |

## App development

Apps are written with a class-based widget framework (`import "dragon/ui" as *`: `App`, `Screen`, `Col`, `Button`,
`Component` and so on) and compiled to a native library, so no interpreter or source ships with the app. `dragon preview`
shows the UI in a browser while you work. Apps can be split over many files with `import`, and extras such as audio or web
views come as packages that may carry Kotlin or Swift plugins.

Docs: `docs/FRAMEWORK.md` (start here), `docs/COMPONENTS.md` (what exists and what doesn't), `docs/PACKAGES.md`,
`docs/RENDERER_PROTOCOL.md`, `docs/IOS.md`, `docs/WINDOWS.md`, `ANDROID_DEVELOPMENT.md`. Samples are in `examples/`:
`counter`, `components`, `music_player`, `browser`, `desktop_browser`. If you have code from before the 2.0 vocabulary change,
`dragon migrate` renames it for you.

`dragon create <name> [--package <id>]` sets up a project: a `package.json`, `lib/main.dgn`, and the Android, iOS and
Windows project files. `dragon build` and `dragon run` compile the app, build the platform project and, for `run`, install
and launch it.

Still young. What has and hasn't been checked:

- **Tested:** the C transpiler, JNI wrapper generation, string and list memory handling and the file-storage builtins were
  run under AddressSanitizer on several apps (a to-do list, a calculator, a multi-page app with drawer and FAB), and the
  leaks and use-after-free bugs that turned up were fixed. The files `dragon create` writes were checked, and the generated
  `lib/main.dgn` compiles and runs.
- **Not tested yet:** Gradle and ADB steps of `dragon build` / `dragon run` have never run end to end, and the Kotlin
  renderer has only been type-checked, not run on a device. `dragon create` doesn't generate a Gradle wrapper; open the
  project in Android Studio once, or run `gradle wrapper`, before `dragon build` will work. The iOS renderer is a Swift
  skeleton that has not been compiled.
- **Limits of `--native --lib`:** scalar types and one collection type (`strlist`, a list of strings). No structs, closures,
  generics or threads. Something like a to-do item has to be packed into a delimited string. A Dragon function named after a
  C keyword (`goto`, say) produces invalid C instead of an error, so rename it.
- **No networking in native libraries.** The host app makes the request and calls a Dragon function with the result, the
  same way `on_event()` works.

`CROSS_COMPILE_TESTING.md` covers cross-compiling (NDK, and the 16KB page-size linking Android requires).

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

Dragon builds with Cargo and needs a recent stable [Rust toolchain](https://www.rust-lang.org/tools/install) (edition 2021).

```bash
# Clone the repository
git clone https://github.com/aaveshdev/dragon.git
cd dragon

# Build the CLI in debug mode
cargo build

# Release build (recommended)
cargo build --release
```

The binary ends up at:

```
target/release/dragon
```

`cargo build` also produces a shared library (`dragon_lib`, from `src/lib.rs`) next to the CLI.

### Optional: install the CLI globally

```bash
cargo install --path .
```

### Feature/target notes

- **Native codegen** (`--native`) shells out to a system C compiler (`gcc`, `clang`, or `cl` on Windows), so one of them has to be on your `PATH`.
- **Android** targets add the `jni` / `ndk` dependencies through a `cfg(target_os = "android")` block in `Cargo.toml`.
- **WASM** targets add `wasm-bindgen`, `web-sys` and `getrandom` (`js` feature).
- **Windows apps** (`dragon build --platform windows`) need a Windows-capable C compiler to build the DLL. On Linux or macOS that means Zig (`pip install ziglang` works) or mingw-w64; on Windows, MinGW-w64 or LLVM clang. Native builds for other targets use the system `gcc` or `clang`.
- On `aarch64-unknown-linux-gnu`, OpenSSL is vendored and built from source, so you need a C toolchain there too.

## Running

Run the binary directly, or through `cargo run --`:

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

Extra arguments are passed to the script as `argv`.

### Interactive REPL

```bash
dragon --repl
```

Variables and functions you define stay available on later lines. Type `exit` to quit.

### Run a project via `package.json`

```bash
dragon run
```

Reads `package.json` in the current directory and runs the file named by `"main"` (or `"entry"`):

```json
{
  "name": "my-dragon-app",
  "main": "src/main.dgn"
}
```

### Compile a script

```bash
# Bundle a script and its files into one executable
dragon -c input.dgn -o output

# Same, without a console window on Windows
dragon -c input.dgn -o output --window

# Compile into a shared library (.dll / .so / .dylib)
dragon -c input.dgn -o output.dll --lib

# (Experimental) Translate to C and build a native binary
dragon -c input.dgn -o output --native

# (Experimental) Native shared library
dragon -c input.dgn -o output --native --lib

# Keep the generated C source
dragon -c input.dgn -o output --native --keep-c
```

### Bare-metal images

```bash
# Bootable image for QEMU (x86_64, riscv64 or aarch64); --qemu runs it straight away
dragon -c kernel.dgn -o kernel.elf --native --baremetal x86_64 --qemu

# Ring-3 program for an OS that loads .dxe files
dragon -c prog.dgn -o prog.dxe --native --baremetal dxe
```

Needs `clang`, `ld.lld` and the matching `qemu-system-*`. What works and what doesn't is in [baremetal/README.md](baremetal/README.md).

Bare-metal programs get extra built-in modules that don't exist in the interpreter or other builds:

| Module | What it does |
| --- | --- |
| `hw` | `peek`/`poke` (8 to 64 bit), port I/O, `alloc`, clock and tick counters, heap stats, `halt` |
| `con` | text console |
| `gfx` | framebuffer drawing, back buffer, blit, rounded rectangles, blur |
| `input` | raw keyboard and pointer events, `poll`, `cursor`, `inject` for drivers written in Dragon |
| `disk` | the built-in ATA / virtio block device |
| `task` | preemptive multitasking: `spawn`, `sleep`, `pause`, `wait`, `atomic` |
| `proc` | ring-3 processes (x86-64 only) |
| `blob` | files embedded in the image with `--embed FILE` |

`--sdk DIR` overlays your own copy of the embedded kernel sources. This is enough to write an operating system in Dragon; Vaelis
is one, in a separate project. Limits (64 tasks, single core, no files or sockets from the language itself) are in
[baremetal/README.md](baremetal/README.md).

### TurboDragon IDE

```bash
dragon --ide
```

A keyboard-driven terminal editor and runner, built on `crossterm`.

## Project Structure

```
src/
├── main.rs           # CLI (run, repl, compile, bundle, ide)
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

MIT. See [LICENSE](LICENSE).
