# Dragon Language Bytecode

This directory contains the specification of the Dragon bytecode instruction set and format.

## Files

- `instruction-set.md` - Detailed specification of each bytecode instruction
- `bytecode-format.md` - Binary format of bytecode files
- `execution.md` - How bytecode is executed by the VM

## Source of Truth

The bytecode specification describes the low-level representation of Dragon programs. Where the specification is incomplete or marked as implementation-defined, refer to the reference implementation in `src/engine/` for current behavior, particularly:
- `src/engine/compiler.rs` - AST to bytecode compilation
- `src/engine/bytecode.rs` - Bytecode definitions
- `src/engine/vm.rs` - Bytecode execution