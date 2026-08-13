# Dragon Language Runtime

This directory contains the specification of the Dragon runtime behavior and memory model.

## Files

- `values.md` - Dragon values and data types
- `scopes.md` - Variable scoping and execution contexts
- `memory-model.md` - Memory allocation and garbage collection
- `runtime-errors.md` - Runtime errors and exception handling
- `execution.md` - Runtime execution details

## Source of Truth

The runtime specification describes how Dragon programs behave when executed. Where the specification is incomplete or marked as implementation-defined, refer to the reference implementation in `src/engine/` for current behavior.