# Repository Structure

This document explains the structure of the Dragon repository to help contributors navigate the codebase.

## Top-Level Directories

```
dragon/
├── .github/                 # GitHub workflows and templates
├── a11y/                    # Accessibility documentation and resources
├── adr/                     # Architectural Decision Records
├── compliance/              # Licensing, compliance, and legal documentation
├── contributor-docs/        # Documentation for contributors (this directory)
├── deployment/              # Deployment and distribution guides
├── docs/                    # End-user documentation
├── ecosystem/               # Information about Dragon ecosystem
├── fuzz/                    # Fuzz testing infrastructure
├── governance/              # Project governance documents
├── proposals/               # Language evolution proposals
├── regulatory/              # Regulatory and legal considerations
├── security/                # Security documentation and policies
├── spec/                    # Formal language specification
├── tests/                   # Test suite
├── tooling/                 # Development tooling documentation
├── tools/                   # Development and build tools
├── benchmarks/              # Performance benchmarks
├── www/                     # Website resources
├── examples/                # Example Dragon programs
├── src/                     # Source code implementation
├── Cargo.toml               # Rust package configuration
├── Cargo.lock               # Rust package lock file
├── build.rs                 # Rust build script
�└── README.md                # Project overview and getting started
```

## Source Code (`src/`)

The `src/` directory contains the Rust implementation of the Dragon language:

```
src/
├── main.rs                  # CLI entry point (handles commands: run, repl, bundle, ide)
├── lib.rs                   # Library entry point (for use as a shared library)
├── tui.rs                   # TurboDragon terminal IDE implementation
├── engine/                  # Core language implementation (lexer, parser, VM, etc.)
│   ├── lexer.rs             # Tokenizes source code into tokens
├── ├── parser.rs            # Parses tokens into Abstract Syntax Tree (AST)
├── ├── ast.rs               # AST node definitions
├── ├── compiler.rs          # Compiles AST to bytecode
├── ├── bytecode.rs          # Bytecode instruction set definitions
├── ├── vm.rs                # Virtual machine that executes bytecode
├── ├── value.rs             # Runtime value types and operations
├── ├── environment.rs       # Variable scoping and environment management
├── ├── builtins.rs          # Built-in standard library functions
├── ├── linter.rs            # Static analysis and linting
├── ├── crypto.rs            # Cryptographic helper functions
├── ├── bundle.rs            # Executable bundling and packaging
├── ├── gc.rs                # Garbage collection implementation
├── └── error.rs             # Error types and pretty-printing
�└── codegen_c/               # Experimental Dragon-to-C transpiler (--native feature)
    ├── transpiler.rs        # Main transpiler logic
    ├── runtime.rs           # C runtime for transpiled code
    ├── native.rs            # Native binding helpers
    └── mangle.rs            # Name mangling for C symbols
```

## Specification (`spec/`)

The `spec/` directory contains the formal specification of the Dragon language:

```
spec/
├── language/                # Language syntax and semantics
│   ├── lexical-structure.md
│   ├── syntax.md
│   ├── identifiers.md
│   ├── literals.md
│   ├── expressions.md
│   ├── statements.md
│   ├── variables.md
│   ├── functions.md
│   ├── closures.md
│   ├── classes.md
│   ├── inheritance.md
│   ├── modules.md
│   ├── imports.md
│   ├── exceptions.md
│   ├── concurrency.md
│   ├── async.md
│   └── execution-model.md
├── runtime/                 # Runtime behavior and memory model
├── bytecode/                # Bytecode instruction set and format
├── standard-library/        # Standard library module specifications
├── compatibility/           # Versioning and compatibility policies
�└── README.md                # Overview of specification structure
```

## Tests (`tests/`)

The `tests/` directory contains the test suite:

```
tests/
├── language/                # Language feature tests
├── lexer/                   # Lexer tests
├── parser/                  # Parser tests
├── compiler/                # Compiler tests
├── bytecode/                # Bytecode tests
├── vm/                      # Virtual machine tests
├── runtime/                 # Runtime tests
├── stdlib/                  # Standard library tests
├── cli/                     # CLI tests
├── native/                  # Native compilation tests
├── ffi/                     # FFI tests
├── packaging/               # Packaging/bundling tests
�└── compatibility/           # Compatibility tests
```

## Documentation (`docs/`)

The `docs/` directory contains end-user documentation:

```
docs/
├── getting-started/         # Installation and first steps
├── language-guide/          # Comprehensive language tutorial
├── reference/               # API reference
├── cli/                     # CLI reference
├── repl/                    # REPL documentation
├── ide/                     # TurboDragon IDE guide
├── packages/                # Packaging and distribution
├── standard-library/        # Standard library tutorials
├── native-compilation/      # Native compilation guide
├── embedding/               # Embedding Dragon in other applications
├── debugging/               # Debugging techniques
�└── performance/             # Performance optimization
```

## Proposals (`proposals/`)

The `proposals/` directory contains language evolution proposals:

```
proposals/
├── accepted/                # Accepted but not yet implemented
├── implemented/             # Implemented in released versions
├── rejected/                # Rejected proposals
├── deferred/                # Postponed for future consideration
�└── PROPOSAL_TEMPLATE.md     # Template for new proposals
```

## Architectural Decision Records (`adr/`)

The `adr/` directory contains documented architectural decisions:

```
adr/
├── README.md                # Overview of ADR process
├── ADR_TEMPLATE.md          # Template for new ADRs
�└── [number]-title.md        # Individual ADRs
```

## How to Navigate

When looking for specific functionality:

1. **Language features**: Check `spec/language/` for specification, `src/engine/` for implementation
2. **Standard library**: Check `spec/standard-library/` for specification, `src/engine/builtins.rs` for implementation
3. **Compiler**: Look at `src/engine/compiler.rs` and related files
4. **Virtual Machine**: Look at `src/engine/vm.rs` and `src/engine/bytecode.rs`
5. **CLI**: Look at `src/main.rs`
6. **IDE**: Look at `src/tui.rs`
7. **Tests**: Look in the corresponding subdirectory under `tests/`

## Finding Related Files

Many files have corresponding counterparts in different directories:
- `spec/language/functions.md` ↔ `src/engine/parser.rs` (function parsing) ↔ `src/engine/vm.rs` (function execution)
- `spec/bytecode/instruction-set.md` ↔ `src/engine/bytecode.rs` (instruction definitions) ↔ `src/engine/compiler.rs` (instruction generation)
- `spec/runtime/values.md` ↔ `src/engine/value.rs` (value representation) ↔ `src/engine/vm.rs` (value operations)

## Contribution Workflow

1. Identify the area you want to work on
2. Check the relevant specification in `spec/` (if it's a language feature)
3. Look at the implementation in `src/`
4. Look at existing tests in `tests/` to understand the testing patterns
5. Implement your changes
6. Add or update tests as needed
7. Update documentation if necessary
8. Submit a pull request

## Getting Help

If you're unsure where to find something:
- Check this document again
- Look at the README files in various directories (they often contain overview information)
- Use GitHub's file search functionality
- Ask in the project's communication channels