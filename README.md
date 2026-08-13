# Dragon Programming Language

**Dragon** is an innovative, practical, general-purpose programming language. It's designed to be simple, small, flexible, and fast, with an expressive, readable syntax that's easy to pick up. Dragon supports multiple programming paradigms (imperative, object-oriented, functional, and declarative) and is portable across Windows, Linux, and macOS, and can be used to build console applications.

> Official website & documentation: [dragon-lang.org](https://www.dragon-lang.org/)
> Language specification: [spec/](./spec/)

## What is Dragon?

Dragon is a dynamically-typed programming language that combines simplicity with powerful features. It features a clean syntax, first-class functions, closures, classes, async/await capabilities, and a rich standard library. Dragon code is compiled to bytecode and executed by a fast virtual machine, with options for native compilation and executable bundling.

## Key Features

- **Simple, readable syntax** — spend less time fighting the language, more time solving problems.
- **Dynamic typing** with first-class functions, closures, and classes (`func`, `class`, `extends`, `this`, `super`, `new`).
- **Familiar control flow** — `if` / `elif` / `else`, `while`, `for ... in`, `switch` / `case`, `try` / `catch` / `finally` / `raise`.
- **Async/await & threading** — built-in `async` / `await` support plus a `thread` module for spawning concurrent work.
- **Rich standard library** available via `import`, including math, JSON, file/I/O, networking, cryptography, and more.
- **Bytecode VM** for fast interpreted execution.
- **Native compilation (experimental)** — transpile Dragon source to C and build it with your system's C compiler (gcc/clang/MSVC).
- **Self-contained executables** — bundle a script (and its dependencies) into a single encrypted, distributable binary.
- **Shared library builds** — compile Dragon scripts into a `.dll` / `.so` / `.dylib` for embedding in other applications.
- **TurboDragon IDE** — a built-in terminal UI editor/runner, launched with `--ide`.
- **Package-based projects** — run a project directly from a `package.json` manifest with `dragon run`.

## Small Example

```dragon
import "std/thread";
import "std/math";

class Animal {
    func init(name) {
        this.name = name;
    }
    func speak() {
        println(this.name + " speaks");
    }
}

func worker(id) {
    sleep(0.5);
    println("Task " + id + " done");
}

animal = new Animal("Dragon");
animal.speak();

for i in math.range(5) {
    thread.spawn(worker, [i]);
}

println("All tasks started");
```

## Installation

Dragon is built with Cargo and requires a recent stable [Rust toolchain](https://www.rust-lang.org/tools/install) (edition 2021 or later).

```bash
# Clone the repository
git clone https://github.com/smitshetye/dragon.git
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

Optional: install the CLI globally
```bash
cargo install --path .
```

This installs the `dragon` binary onto your `PATH`.

## Quick Start

Once built, invoke the binary directly:

```bash
# Run a Dragon script
dragon hello.dragon

# Start the interactive REPL
dragon --repl

# Run the project defined by ./package.json
dragon run

# Launch the TurboDragon terminal IDE
dragon --ide

# Print version info
dragon --version
```

## Command Line Interface (CLI)

The Dragon CLI provides several commands:

- `dragon <file.dragon> [args...]` - Run a Dragon script
- `dragon --repl` - Start the interactive REPL
- `dragon run` - Run project from package.json
- `dragon --ide` - Launch TurboDragon IDE
- `dragon -v | --version` - Print version information
- `dragon -c input.dragon -o output` - Bundle script into executable
- `dragon -c input.dragon -o output.dll --lib` - Build shared library
- `dragon -c input.dragon -o output --native` - Experimental native compilation

## Read-Eval-Print Loop (REPL)

Start an interactive Dragon session with `dragon --repl`:

```bash
$ dragon --repl
Dragon REPL v0.1.0
Type '.help' for help, '.exit' to quit
> let x = 42
> x * 2
84
> func greet(name) { return "Hello, " + name; }
> greet("World")
"Hello, World"
> .exit
```

## TurboDragon IDE

Dragon includes a built-in terminal-based IDE launched with `dragon --ide`:

- Keyboard-driven interface
- Syntax highlighting
- File browser
- Integrated debugger
- One-click run/build
- Works over SSH (terminal-only)

## Packages

Dragon supports package-based projects using a `package.json` manifest:

```json
{
  "name": "my-dragon-app",
  "version": "1.0.0",
  "main": "src/main.dragon",
  "dependencies": {
    "std/math": "^1.0.0",
    "std/io": "^1.0.0"
  }
}
```

Run with: `dragon run`

## Native Compilation

Experimental feature to transpile Dragon to C and compile natively:

```bash
dragon -c script.dragon -o output --native
# For Windows GUI application (no console):
dragon -c script.dragon -o output --native --window
# Keep intermediate C source:
dragon -c script.dragon -o output --native --keep-c
```

## Standard Library

Dragon's standard library provides batteries-included functionality:

- **std/math** — Mathematical functions and constants
- **std/json** — JSON parsing and serialization
- **std/file** / **std/path** — File system access and path manipulation
- **std/os** — Environment variables, process info, system commands
- **std/time** — Date/time functions, timers, sleep
- **std/net** — HTTP client/server, DNS, sockets
- **std/crypto** — Cryptographic hashing (SHA-2, MD5), HMAC, PBKDF2, AES
- **std/zip** — ZIP archive creation and extraction
- **std/xml** — XML parsing and manipulation
- **std/uuid** — UUID generation and parsing
- **std/pdf** / **std/font** — PDF generation and font/text rendering
- **std/ffi** / **std/struct** / **std/mem** — Foreign Function Interface and manual memory management

## Documentation

Comprehensive documentation is available in the `docs/` directory:

- **Getting Started** — Installation, quick start, tutorials
- **Language Guide** — Complete guide to Dragon syntax and features
- **API Reference** — Detailed documentation of all standard library modules
- **CLI Reference** — Complete reference of command-line options
- **IDE Guide** — Using the TurboDragon terminal IDE
- **Native Compilation** — Guide to experimental native compilation features
- **Packaging** — Creating and distributing Dragon packages
- **Debugging** — Debugging Dragon applications
- **Performance** — Performance optimization tips

## Language Specification

The formal language specification is maintained in the `spec/` directory:

- **spec/language/** — Dragon language syntax and semantics
- **spec/runtime/** — Runtime behavior and memory model
- **spec/bytecode/** — Bytecode instruction set and execution
- **spec/standard-library/** — Standard library module specifications
- **spec/compatibility/** — Versioning and compatibility policies

See [SPECIFICATION.md](./SPECIFICATION.md) for details on how the specification relates to the implementation.

## Contributing

We welcome contributions to the Dragon project! Please see our contributing guidelines:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Commit your changes (`git commit -m 'Add amazing feature'`)
5. Push to the branch (`git push origin feature/amazing-feature`)
6. Open a Pull Request

Please read [CONTRIBUTING.md](./CONTRIBUTING.md) for detailed information on:
- Code of conduct
- Development setup
- Pull request process
- Coding standards
- Testing guidelines

## Development

For developers interested in contributing to the Dragon implementation:

### Building from Source
```bash
# Install Rust if not already installed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone https://github.com/smitshetye/dragon.git
cd dragon
cargo build --release
```

### Running Tests
```bash
# Run the test suite
cargo test

# Run specific tests
cargo test test_name
```

### Documentation
- See [CONTRIBUTING.md](./CONTRIBUTING.md) for development setup
- See [docs/development/](./docs/development/) for contributor documentation
- API documentation is generated with `cargo doc --open`

## Security

Dragon takes security seriously. While the language provides powerful features, it's important to use them responsibly:

### Capabilities
Dragon provides access to system capabilities including:
- File system read/write
- Network communication
- Process spawning
- Foreign function interface (FFI)
- Native code execution

### Best Practices
- Validate all external input
- Use least privilege principles when possible
- Be cautious when using FFI or native compilation
- Keep dependencies updated
- Report security vulnerabilities responsibly

See [SECURITY.md](./SECURITY.md) for our security policy and [spec/security/](./spec/security/) for security considerations in the language design.

## License

Dragon is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

## Related Resources

- [Language Specification](./spec/) — Formal definition of the Dragon language
- [Documentation Index](./DOCUMENTATION_INDEX.md) — Guide to all documentation
- [Changelog](./CHANGELOG.md) — Notable changes for each version
- [Code of Conduct](./CODE_OF_CONDUCT.md) — Our community standards