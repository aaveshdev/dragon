# Setting up a Development Environment

This guide explains how to set up a development environment for contributing to the Dragon programming language.

## Prerequisites

### Rust Toolchain
Dragon is written in Rust, so you need a recent stable Rust toolchain:

```bash
# Install Rust using rustup (recommended)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Or update your existing installation
rustup update
```

Verify installation:
```bash
rustc --version
cargo --version
```

### C Compiler (for Native Compilation Features)
Some Dragon features require a C compiler:

- **Linux**: `gcc` or `clang` (usually installed via `build-essential` on Debian/Ubuntu)
- **macOS**: Xcode Command Line Tools (`xcode-select --install`)
- **Windows**: Visual Studio Build Tools or Windows SDK

### Git
You'll need Git for version control:
```bash
git --version
```

## Getting the Source Code

1. Fork the Dragon repository on GitHub
2. Clone your fork:
```bash
git clone https://github.com/YOUR_USERNAME/dragon.git
cd dragon
```

3. Add the upstream repository to stay updated:
```bash
git remote add upstream https://github.com/aaveshdev/dragon
git fetch upstream
```

## Building Dragon

### Debug Build (for development)
```bash
cargo build
```

### Release Build (optimized)
```bash
cargo build --release
```

### Running Tests
```bash
# Run all tests
cargo test

# Run specific tests
cargo test test_function_name

# Run tests with output
cargo test -- --nocapture
```

## Common Development Tasks

### Running the REPL
```bash
cargo run -- --repl
```

### Running a Dragon Script
```bash
cargo run -- path/to/script.dragon
```

### Launching the IDE
```bash
cargo run -- --ide
```

### Building an Executable
```bash
cargo run -- -c path/to/script.dragon -o output_name
```

## Repository Structure

See [contributor-docs/repository-structure.md](./repository-structure.md) for detailed information about the repository organization.

## Code Style

Dragon follows standard Rust conventions:

- Use `rustfmt` for code formatting:
```bash
cargo fmt
```

- Use `clippy` for linting:
```bash
cargo clippy
```

- Write clear, descriptive commit messages
- Add comments for non-obvious logic
- Keep functions focused and reasonably sized

## Submitting Changes

1. Create a feature branch:
```bash
git checkout -b feature/your-feature-name
```

2. Make your changes
3. Commit your changes:
```bash
git commit -m "Add feature: description of what you did"
```

4. Push to your fork:
```bash
git push origin feature/your-feature-name
```

5. Open a Pull Request against the upstream repository's main branch

## Getting Help

- Check the [existing documentation](../docs/)
- Look at [existing tests](../tests/) for examples
- Ask questions in the project's communication channels
- Refer to the [language specification](../spec/) when implementing language features

## Thank You!

Thank you for contributing to Dragon! Your efforts help make the language better for everyone.