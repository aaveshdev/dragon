# Dragon Language Execution Model

## Status: Implementation-defined (based on current parser/implementation and VM)

This document describes the execution model of the Dragon programming language.

## Execution Model Overview

Dragon uses a hybrid execution model that combines elements of interpreted and compiled approaches:

1. **Parsing**: Source code is parsed into an Abstract Syntax Tree (AST)
2. **Bytecode compilation**: The AST is compiled into Dragon bytecode
3. **Virtual machine execution**: The bytecode is executed by the Dragon Virtual Machine (VM)

## Compilation Process

### Source to AST
- Source code (.dg files) is tokenized by the lexer
- Tokens are parsed into an Abstract Syntax Tree (AST) by the parser
- The AST represents the syntactic structure of the program

### AST to Bytecode
- The AST is traversed and converted to Dragon bytecode instructions
- Each language construct maps to one or more bytecode instructions
- Function calls, loops, and control flow generate appropriate jump instructions

### Bytecode Format
Dragon bytecode consists of:
- **OpCodes**: Operation codes that specify what action to perform
- **Operands**: Data or references needed by the operation
- **Constants pool**: Storage for string, number, and other constant values
- **Debug information**: Line numbers and source mappings (when available)

## Virtual Machine

The Dragon VM is a stack-based virtual machine that executes bytecode:

### Stack Architecture
- **Operand stack**: Used for temporary values during expression evaluation
- **Call stack**: Tracks function calls and returns
- **Variable stack/storage**: Stores local variables and function parameters

### Instruction Set
The VM executes instructions such as:
- **Load/Store**: Load constants, load/store variables
- **Arithmetic**: Add, subtract, multiply, divide, modulo
- **Comparison**: Equal, not equal, less than, greater than, etc.
- **Branch**: Jump, conditional jump, jump if false/true
- **Function calls**: Call function, return from function
- **Object/Array**: Create objects, create arrays, access properties
- **Control flow**: Loop constructs, exception handling

## Execution Flow

### Program Start
1. The main module is loaded and compiled to bytecode
2. The VM begins execution at the entry point (top-level statements)
3. Statements are executed sequentially unless altered by control flow

### Function Calls
1. When a function is called:
   - Current execution point is saved on the call stack
   - Arguments are evaluated and placed on the operand stack
   - A new stack frame is created for the function
   - Function parameters are initialized from the argument values
   - Execution jumps to the function's bytecode
2. When a function returns:
   - The return value (if any) is placed on the operand stack
   - The function's stack frame is destroyed
   - Execution resumes from the saved point on the call stack

### Variable Access
- Variables are accessed via their offset in the current stack frame
- Global variables are accessed via a global variable table
- Variable lookup follows lexical scoping rules

### Object and Array Handling
- Objects are implemented as hash maps (string keys to values)
- Arrays are implemented as resizable vectors
- Both are allocated on the heap and garbage collected

## Memory Management

### Stack Memory
- Used for function call frames, local variables, and temporary expression values
- Automatically managed - allocated on function entry, deallocated on exit

### Heap Memory
- Used for objects, arrays, strings, and other dynamically allocated data
- Managed by a garbage collector

### Garbage Collection
Dragon uses a garbage collector to reclaim unused heap memory:
- **Mark-and-sweep** or similar algorithm
- Runs periodically when memory allocation reaches a threshold
- Traverses reachable objects from roots (stack, globals)
- Reclaims memory from unreachable objects

## Concurrency Model

### Single-threaded Execution
- The primary execution model is single-threaded
- Async/await provides non-blocking I/O within a single thread

### Thread-based Concurrency
- Optional thread support available through the standard library
- Uses native OS threads (pthreads/Windows threads)
- Requires explicit synchronization (mutexes, condition variables)

## Execution Characteristics

### Deterministic
- Given the same input, Dragon programs produce the same output
- Except for intentional non-determinism (e.g., time-based functions)

### Performance
- Bytecode interpretation provides reasonable performance
- JIT compilation is not currently implemented
- Critical sections can be optimized via native extensions

### Portability
- Bytecode is platform-independent
- VM has been ported to multiple platforms (Windows, Linux, macOS)
- Same bytecode runs on all platforms with a compatible VM

## Implementation Details

### Current Implementation
The current Dragon implementation (`src/engine/`) includes:
- `lexer.rs` - Tokenizes source code
- `parser.rs` - Parses tokens into AST
- `compiler.rs` - Compiles AST to bytecode
- `vm.rs` - Executes bytecode on the virtual machine
- `bytecode.rs` - Defines bytecode format and instructions
- `value.rs` - Represents Dragon values in the VM

### Bytecode Instructions
Key bytecode instructions include:
- `LOAD_CONST` - Load constant from constants pool
- `LOAD_VAR` - Load variable from stack frame
- `STORE_VAR` - Store variable to stack frame
- `ADD`, `SUB`, `MUL`, `DIV`, `MOD` - Arithmetic operations
- `EQ`, `NE`, `LT`, `GT`, `LE`, `GE` - Comparison operations
- `JUMP`, `JUMP_IF_FALSE`, `JUMP_IF_TRUE` - Control flow
- `CALL` - Call a function
- `RETURN` - Return from function
- `NEW_OBJECT`, `NEW_ARRAY` - Create objects and arrays
- `GET_PROP`, `SET_PROP` - Get/set object properties
- `GET_ELEM`, `SET_ELEM` - Get/set array elements

## Related Files
- `src/engine/` - Complete implementation of compiler and VM
- `spec/language/bytecode/` - Bytecode specification
- `spec/language/runtime/` - Runtime behavior specification