# Dragon Language Specification

This directory contains the authoritative conceptual definition of the Dragon programming language.

## Structure

- `language/` - Defines the Dragon language syntax and semantics.
- `runtime/` - Defines the Dragon runtime behavior and memory model.
- `bytecode/` - Defines the Dragon bytecode instruction set and format.
- `standard-library/` - Defines the Dragon standard library.
- `compatibility/` - Defines versioning, compatibility policies, and breaking changes.

## Source of Truth

As defined in the project governance:

1. `spec/` defines language semantics.
2. `proposals/` defines proposed language evolution.
3. `adr/` records architectural decisions.
4. `src/` contains the reference implementation.
5. `tests/` verifies behavior.
6. `docs/` explains Dragon to users and developers.

When there is a discrepancy between the specification and the reference implementation, the specification takes precedence and the implementation should be updated to match.

## Status

Where something is not yet formally specified, it is marked as:

- `Status: Unspecified` - No decision has been made.
- `Status: Implementation-defined` - The behavior is defined by the current implementation but may change in the future.

Do not invent undocumented language features. Where the specification is incomplete, refer to the reference implementation for current behavior.