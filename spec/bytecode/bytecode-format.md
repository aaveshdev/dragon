# Dragon Language Bytecode Format

## Status: Implementation-defined (based on current compiler/VM implementation)

This document describes the binary format of Dragon bytecode files.

## Bytecode File Structure

Dragon bytecode files are currently not saved to disk as a standard practice - the compilation process generates bytecode in memory that is immediately executed by the VM. However, the in-memory structure follows a specific format.

## In-Memory Bytecode Structure

The bytecode is structured as a `Chunk` (defined in `src/engine/bytecode.rs`):

```rust
pub struct Chunk {
    pub instructions: Vec<Instruction>,  // Vector of bytecode instructions
    pub source_lines: Vec<usize>,        // Source line number for each instruction
}
```

### Instructions Vector
- Contains all the bytecode instructions in execution order
- Each instruction is a variant of the `Instruction` enum
- Instructions are executed sequentially unless modified by jump instructions

### Source Lines Vector
- Parallel vector to `instructions`
- `source_lines[i]` gives the source code line number for `instructions[i]`
- Used for error reporting and debugging
- If no line information is available, may contain 0 or line number of surrounding construct

## Instruction Encoding

Instructions are represented as Rust enum variants in memory. When serialized (if ever implemented to disk), they would need a binary encoding. Currently, no disk serialization format is defined.

## Constants Pool

Unlike many bytecode formats, Dragon does not use a separate constants pool. Instead:
- Constants are embedded directly in `LoadConst` instructions as `DgnValue` values
- This means each `LoadConst` instruction carries its constant value directly
- String and object constants are duplicated wherever they appear in bytecode

## Example Bytecode Structure

For a simple program like:
```dragon
func main() {
    return 42;
}
```

The bytecode chunk might contain:
- `LoadConst(Number(42.0))` - Push the number 42 onto stack
- `Return` - Return from function (top of stack is return value)

With corresponding source lines:
- `[2, 2]` - Both instructions from line 2

## Implementation Notes

1. **In-memory only**: Currently, Dragon bytecode exists only in memory during program execution - it is not saved to .dgb files or similar.

2. **No separate constants pool**: Constants are embedded in instructions, which can lead to duplication but simplifies the format.

3. **Line number tracking**: Each instruction tracks its source line for error reporting.

4. **Variable storage**: Variable names are stored as strings in instructions that reference them (`LoadVar`, `StoreVar`, etc.).

5. **Function inlining**: Functions are not inlined - each function gets its own chunk of bytecode.

6. **Closure handling**: Closures capture outer variables by reference, not by value.

## Related Files
- `src/engine/bytecode.rs` - Chunk and Instruction definitions
- `src/engine/compiler.rs` - Generates bytecode chunks from AST
- `src/engine/vm.rs` - Executes bytecode chunks
- `spec/bytecode/instruction-set.md` - Individual instruction specifications