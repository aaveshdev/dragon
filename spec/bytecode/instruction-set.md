# Dragon Language Bytecode Instruction Set

## Status: Implementation-defined (based on current bytecode.rs implementation)

This document describes the Dragon bytecode instruction set based on the current implementation in `src/engine/bytecode.rs`.

## Instruction Set Overview

Dragon bytecode is a stack-based instruction set where most operations pop their operands from the stack and push results back onto the stack.

## Instructions

### Stack Manipulation
- `LoadConst(value)` - Push a constant value onto the stack
- `LoadVar(name)` - Push the value of a variable onto the stack
- `StoreVar(name)` - Pop a value from the stack and store it in a variable
- `DefineConst(name)` - Pop a value and define it as a constant (cannot be reassigned)
- `Pop` - Discard the top value from the stack
- `Dup` - Duplicate the top value on the stack

### Arithmetic and Binary Operations
- `BinaryOp(op)` - Perform a binary operation:
  - Arithmetic: `Add`, `Sub`, `Mul`, `Div`, `Mod`
  - Comparison: `Eq`, `Ne`, `Lt`, `Gt`, `Le`, `Ge`
  - Logical: `And`, `Or`
  - Null-coalescing: `NullCoalesce` (`??`)
  - Bitwise: `BitAnd`, `BitOr`, `BitXor`, `Shl` (`<<`), `Shr` (`>>`)
- `UnaryOp(op)` - Perform a unary operation:
  - `-` (negation)
  - `not` (logical NOT)

### Collections
- `MakeList(count)` - Pop `count` values and push them as a new array/list
- `MakeMap(count)` - Pop `count` key-value pairs and push them as a new object/map

### Control Flow
- `Jump(target)` - Unconditionally jump to the instruction at index `target`
- `JumpIfFalse(target)` - Pop a value; if falsy, jump to `target`
- `JumpIfTrue(target)` - Pop a value; if truthy, jump to `target` (used for `or` short-circuit)
- `JumpIfNotNull(target)` - Pop a value; if not null, jump to `target` (used for `??` operator)

### Functions and Closures
- `MakeClosure(name, params, instruction_count, is_async, chunk)` - Create a function closure:
  - `name`: function name (optional for anonymous functions)
  - `params`: list of parameter names and optional type annotations
  - `instruction_count`: number of instructions in the function body
  - `is_async`: whether the function is async
  - `chunk`: the compiled instructions for the function body
- `MakeClosureRef(name, owner)` - Create a closure reference
- `MakeClassRef(name, owner, superclass)` - Create a class reference
- `Call(arg_count)` - Pop a function and `arg_count` arguments, then call the function
- `Return` - Return from the current function (top of stack is return value)
- `MakeClass(name, superclass, methods, method_lines)` - Create a class:
  - `name`: class name
  - `superclass`: optional parent class name
  - `methods`: vector of method instructions
  - `method_lines`: vector of source line numbers for methods
- `NewObject(arg_count)` - Pop a class and `arg_count` arguments, create a new instance
- `SuperCall(method_name, arg_count)` - Call a method on the parent class (`super.method()`)

### Object Access
- `GetAttr(name)` - Get property `name` from object (top of stack)
- `GetOptionalAttr(name)` - Get property with optional chaining (`obj?.name`)
- `SetAttr(name)` - Set property `name` on object (top of stack) to value (second from top)
- `GetIndex` - Get array/object element at index (top of stack) from object (second from top)
- `SetIndex` - Set array/object element at index (top of stack) to value (third from top)

### Loops
- `ForIter(var_name, exit_ip, array_obj)` - Initialize for-loop iteration:
  - `var_name`: variable to receive each element
  - `exit_ip`: instruction to jump to when iteration is complete
  - `array_obj`: the array/object to iterate over
- `Break` - Exit the current loop
- `Continue` - Skip to the next iteration of the current loop
- `DoWhileStart` - Mark the start of a do-while loop body (patch target set later)

### Exception Handling
- `PushTry(catch_ip)` - Push a try frame onto the exception stack; `catch_ip` is where to jump on exception
- `PopTry` - Pop the try frame (when no exception occurs)
- `Raise` - Pop a value from the stack and raise it as an exception
- `EnterCatch(var_name)` - Bind the caught exception to variable `var_name`

### Async/Await
- `Await` - Pop a promise-like object and await its resolution

### Module System
- `Import(path, alias)` - Import a module:
  - `path`: module path to import
  - `alias`: optional alias for the imported module

### Increment/Decrement
- `IncVar(name)` - Pre-increment variable (`++var`), pushes new value
- `DecVar(name)` - Pre-decrement variable (`--var`), pushes new value
- `PostIncVar(name)` - Post-increment variable (`var++`), pushes old value
- `PostDecVar(name)` - Post-decrement variable (`var--`), pushes old value
- `ReadModWriteAttr(is_pre, is_dec)` - Read-modify-write object attribute:
  - Pops object, pushes result
- `ReadModWriteIndex(is_pre, is_dec)` - Read-modify-write array/object element:
  - Pops object+index, pushes result

### Miscellaneous
- `This` - Push the `this` value (current object context)
- `Nop` - No operation
- `Halt` - Stop execution

## Operand Types

### DgnValue Constants
Values pushed by `LoadConst` are represented as `DgnValue` enum variants:
- `Null`
- `Bool(bool)`
- `Number(f64)`
- `String(String)`
- `Object(DgnObject)` - though constants are typically primitive values
- `Array(DgnArray)` - though constants are typically primitive values
- `Function(DgnFunction)` - function constants

## Chunk Structure

Bytecode is organized into chunks:
```rust
pub struct Chunk {
    pub instructions: Vec<Instruction>,
    pub source_lines: Vec<usize>,  // Maps instruction index to source line
}
```

Each instruction has an associated source line number for debugging and error reporting.

## Implementation Notes

1. **Stack-based**: Most instructions operate on a value stack - popping inputs and pushing outputs.

2. **Line number tracking**: Each instruction retains its source line number for error reporting.

3. **Jump targets**: Jump instructions use absolute instruction indices as targets.

4. **Try/catch frames**: Exception handling uses a separate stack for try frames.

5. **Closure creation**: `MakeClosure` captures the function's bytecode and creates a closure that can access outer scope variables.

6. **Object representation**: Objects and arrays are represented as `DgnObject` and `DgnArray` structs in the runtime.

## Related Files
- `src/engine/bytecode.rs` - Bytecode instruction definitions
- `src/engine/compiler.rs` - AST to bytecode compilation (generates these instructions)
- `src/engine/vm.rs` - Bytecode execution (interprets these instructions)
- `spec/runtime/values.md` - Runtime value representation