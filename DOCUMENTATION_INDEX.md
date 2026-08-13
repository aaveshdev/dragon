# Dragon Documentation Index

This document serves as a master index to all major documentation in the Dragon project.

## Language

- [Language Specification](./spec/language/README.md) - Overview of language specification structure
- [Lexical Structure](./spec/language/lexical-structure.md) - Tokens, literals, identifiers
- [Syntax](./spec/language/syntax.md) - Grammar and syntactic structure
- [Identifiers](./spec/language/identifiers.md) - Naming rules and conventions
- [Literals](./spec/language/literals.md) - String, number, boolean, and null literals
- [Expressions](./spec/language/expressions.md) - Expression syntax and evaluation
- [Statements](./spec/language/statements.md) - Statement syntax and execution
- [Variables](./spec/language/variables.md) - Variable declaration and scoping
- [Functions](./spec/language/functions.md) - Function declaration and invocation
- [Closures](./spec/language/closures.md) - Closure behavior and scope
- [Classes](./spec/language/classes.md) - Class declaration and instantiation
- [Inheritance](./spec/language/inheritance.md) - Inheritance mechanisms and `super`
- [Modules](./spec/language/modules.md) - Module system and organization
- [Imports](./spec/language/imports.md) - Import statement and resolution
- [Exceptions](./spec/language/exceptions.md) - Exception handling with try-catch-finally
- [Concurrency](./spec/language/concurrency.md) - Thread-based concurrency primitives
- [Async](./spec/language/async.md) - Asynchronous programming with async/await
- [Execution Model](./spec/language/execution-model.md) - How Dragon code is executed

## Specification

- [Language Specification](./spec/language/README.md) - Core language definition
- [Runtime Specification](./spec/runtime/README.md) - Runtime behavior and memory model
- [Bytecode Specification](./spec/bytecode/README.md) - Bytecode instruction set and format
- [Standard Library](./spec/standard-library/README.md) - Standard library modules
- [Compatibility](./spec/compatibility/README.md) - Versioning and compatibility policies

## Compiler

- [Compiler Overview](./spec/bytecode/execution.md) - AST to bytecode compilation process
- [Instruction Set](./spec/bytecode/instruction-set.md) - Bytecode instructions
- [Execution](./spec/bytecode/execution.md) - How bytecode is executed by VM

## Runtime

- [Values](./spec/runtime/values.md) - Dragon value types and representation
- [Scopes](./spec/runtime/scopes.md) - Variable scoping and execution contexts (to be created)
- [Memory Model](./spec/runtime/memory-model.md) - Memory allocation and garbage collection (to be created)
- [Errors](./spec/runtime/runtime-errors.md) - Runtime errors and exception handling (to be created)
- [Execution Details](./spec/runtime/execution.md) - Runtime execution details (to be created)

## Standard Library

See [spec/standard-library/README.md](./spec/standard-library/README.md) for overview
Individual modules:
- [Math](./spec/standard-library/math.md)
- [JSON](./spec/standard-library/json.md)
- [File](./spec/standard-library/file.md)
- [Path](./spec/standard-library/path.md)
- [OS](./spec/standard-library/os.md)
- [Time](./spec/standard-library/time.md)
- [Net](./spec/standard-library/net.md)
- [Crypto](./spec/standard-library/crypto.md)
- [Zip](./spec/standard-library/zip.md)
- [XML](./spec/standard-library/xml.md)
- [UUID](./spec/standard-library/uuid.md)
- [PDF](./spec/standard-library/pdf.md)
- [Font](./spec/standard-library/font.md)
- [FFI](./spec/standard-library/ffi.md)
- [Struct](./spec/standard-library/struct.md)
- [Mem](./spec/standard-library/mem.md)

## CLI

- [Command Line Interface](./README.md#command-line-interface) - CLI usage and options
- [Build Commands](./README.md#building) - Building Dragon from source
- [Running Commands](./README.md#running) - Running Dragon programs
- [Native Compilation](./README.md#native-compilation) - Experimental native compilation
- [Bundling](./README.md#running-a-script) - Executable bundling
- [Shared Libraries](./README.md#shared-library-builds) - Shared library builds
- [IDE](./README.md#turbodragon-ide) - TurboDragon terminal IDE

## REPL

- [Interactive REPL](./README.md#interactive-repl) - Using the Dragon REPL
- [REPL Features](./README.md#interactive-repl) - REPL capabilities and commands

## IDE

- [TurboDragon IDE](./README.md#turbodragon-ide) - Terminal-based IDE features
- [IDE Usage](./README.md#turbodragon-ide) - How to launch and use the IDE

## Packages

- [Package System](./README.md#packages) - Package-based projects with package.json
- [Creating Packages](./README.md#packages) - How to create Dragon packages
- [Dependencies](./README.md#packages) - Managing package dependencies

## Native Compilation

- [Native Compilation](./README.md#native-compilation) - Transpiling to C and native builds
- [Options](./README.md#native-compilation) - Available native compilation flags
- [Limitations](./README.md#native-compilation) - Current limitations of native compilation

## FFI

- [Foreign Function Interface](./spec/standard-library/ffi.md) - Calling C functions from Dragon
- [Struct](./spec/standard-library/struct.md) - Working with C structs
- [Mem](./spec/standard-library/mem.md) - Manual memory management

## Embedding

- [Shared Library Builds](./README.md#shared-library-builds) - Creating `.dll`/`.so`/`.dylib` for embedding
- [Library Entry Point](./README.md#shared-library-builds) - Using `src/lib.rs` as library target

## Security

- [Security Overview](./README.md#security) - Security considerations and best practices
- [Capabilities](./README.md#security) - System capabilities provided by Dragon
- [Best Practices](./README.md#security) - Secure usage guidelines
- [Security Policy](./SECURITY.md) - Project security policy
- [Security Spec](./spec/security/README.md) - Security considerations in language design

## Contribution

- [Contributing](./README.md#contributing) - How to contribute to Dragon
- [Development Setup](./README.md#development) - Building and testing Dragon
- [Code of Conduct](./CODE_OF_CONDUCT.md) - Community standards
- [Pull Request Process](./README.md#contributing) - How to submit changes

## Governance

- [Governance](./governance/README.md) - Project governance structure
- [Language Change Process](./governance/LANGUAGE_CHANGE_PROCESS.md) - How language evolves
- [Maintainers](./governance/MAINTAINERS.md) - Project maintainers
- [Release Governance](./governance/RELEASE_GOVERNANCE.md) - Release process and authority

## Deployment

- [Deployment](./deployment/README.md) - Deployment and distribution guide
- [Platform Support](./deployment/README.md) - Supported platforms (Linux, Windows, macOS, etc.)
- [Installation](./README.md#installation) - Installing Dragon from source
- [Distribution](./deployment/binary-distribution.md) - Binary distribution guidelines

## Compatibility

- [Versioning](./spec/compatibility/versioning.md) - Semantic versioning guidelines
- [Compatibility Policy](./spec/compatibility/compatibility-policy.md) - Backward compatibility guarantees
- [Breaking Changes](./spec/compatibility/breaking-changes.md) - Documented breaking changes
- [Deprecations](./spec/compatibility/deprecations.md) - Deprecated features

## Related Resources

- [Changelog](./CHANGELOG.md) - Notable changes for each version
- [License](./LICENSE) - MIT license terms
- [Examples](./examples/) - Example Dragon programs
- [Benchmarks](./benchmarks/) - Performance benchmarks (to be created)