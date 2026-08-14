# Changelog

All notable changes to the Dragon programming language will be documented in this file.

## [Unreleased]
### Added
- Repository architecture modernization with comprehensive documentation structure
- Language specification in `spec/` directory
- Proposal process for language evolution
- Architectural Decision Records (ADR) framework
- Comprehensive test structure
- Contributor documentation
- Deployment guides
- Governance documentation
### Contributed by
- Smit Shetye (repository architecture)

## [v0.1.3] - 2026-08-12
### Added
- Examples directory with sample Dragon programs
- Initial TurboDragon IDE implementation
- Basic standard library modules (math, json, file, net, crypto)

### Fixed
- Minor parser edge cases
- Memory leak in garbage collector

## [v0.1.2] - 2026-06-15
### Added
- Async/await support
- Threading module for concurrency
- Executable bundling feature
- Shared library builds

### Changed
- Improved error reporting with line numbers
- Enhanced REPL functionality

## [v0.1.1] - 2026-03-22
### Added
- Class inheritance with `extends` keyword
- `super` keyword for parent class access
- Destructuring assignment for arrays and objects
- Optional chaining (`?.`) operator

### Fixed
- Closure variable capture issues
- Module import resolution

## [v0.1.0] - 2026-01-10
### Added
- Initial Dragon language implementation
- Lexer, parser, bytecode compiler, and virtual machine
- Basic standard library
- Command-line interface with run, repl, and bundle commands
- TurboDragon terminal IDE
- MIT license
