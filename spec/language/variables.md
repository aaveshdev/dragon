# Dragon Language Variables

## Status: Implementation-defined (based on current parser/implementation)

This document describes variable declaration, scoping, and behavior in the Dragon programming language.

## Variable Declaration

Dragon supports two kinds of variable declarations:

### Mutable Variables (`var`)
```dragon
var name = expr;        // Declaration with initializer
var name;               // Declaration without initializer (value: null)
```
- Variables declared with `var` can be reassigned
- If no initializer is provided, the variable is initialized to `null`
- Can be used in any scope where statements are allowed

### Immutable Constants (`const`)
```dragon
const name = expr;      // Must have initializer
```
- Variables declared with `const` cannot be reassigned after initialization
- Must be initialized at the point of declaration
- The value itself may still be mutable if it's an object or array (binding is immutable, not the value)

## Variable Scope

Dragon uses lexical (static) scoping with block-level scope:

### Block Scope
- Variables are scoped to the nearest enclosing block (delimited by `{}` and `}`)
- A block can be:
  - A function body
  - A loop body (`while`, `for`, etc.)
  - A conditional block (`if`, `elif`, `else`)
  - An explicit block (`{ ... }`)
  - A try/catch/finally block

### Scope Rules
1. **Declaration before use**: Variables must be declared before they are referenced
2. **No redeclaration**: Cannot declare a variable with the same name in the same scope
3. **Shadowing**: Inner scopes can redeclare variables from outer scopes (shadowing)
4. **Function scope**: Function parameters are scoped to the function body
5. **Loop scope**: Variables declared in loop initialization are scoped to the loop

## Variable Lifetime

### Stack Allocation
- Variables are allocated on the stack when they come into scope
- They are deallocated when they go out of scope
- Their lifetime ends when the enclosing block finishes execution

### Closures
- Variables captured by closures (inner functions) are kept alive as long as the closure exists
- This extends their lifetime beyond the original scope

## Initialization and Default Values

### Explicit Initialization
```dragon
var x = 42;
const PI = 3.14159;
```

### Implicit Initialization
```dragon
var x;    // x is initialized to null
```
- Variables declared with `var` but without an initializer are automatically initialized to `null`
- `const` declarations must always have an initializer

## Assignment and Mutation

### Assignment Operator
```dragon
x = expr;   // Assigns the value of expr to variable x
```
- Only works with `var` variables (not `const`)
- The left-hand side must be an lvalue (variable, property access, or index access)

### Compound Assignment
```dragon
x += expr;  // Equivalent to x = x + expr
x -= expr;  // Equivalent to x = x - expr
x *= expr;  // Equivalent to x = x * expr
x /= expr;  // Equivalent to x = x / expr
x %= expr;  // Equivalent to x = x % expr
```

### Mutability of Values
- **Primitive values** (numbers, strings, booleans, null): Immutable
- **Composite values** (objects, arrays): Mutable
- A `const` binding to an object/array prevents reassignment of the binding, but the object/array contents can still be modified

## Examples

```dragon
// Basic variable declarations
var count = 0;
const MAX_USERS = 100;
var name;           // Initialized to null

// Scope examples
{
    var inner = "visible only here";
}                   // inner is out of scope here

// Loop scoping
for (var i = 0; i < 5; i = i + 1) {
    var loop_var = i * 2;
}                   // i and loop_var are out of scope here

// Shadowing
var x = 1;
{
    var x = 2;      // This x shadows the outer x
    print(x);       // Prints 2
}                   // inner x goes out of scope
print(x);           // Prints 1 (outer x)

// Function scope
func example() {
    var param = "parameter";  // Function parameter
    var local = "local";      // Local variable
    // param and local are accessible throughout the function
}

// Closure example
func make_counter() {
    var count = 0;
    func increment() {
        count = count + 1;    // Captures count from outer scope
        return count;
    }
    return increment;
}

let counter = make_counter();
print(counter());   // Prints 1
print(counter());   // Prints 2
```

## Implementation Notes

1. **Stack-based allocation**: Variables are allocated on the VM stack frame for their scope.

2. **Null initialization**: Uninitialized `var` variables are set to `null` (not undefined).

3. **Const enforcement**: The compiler/runtime prevents reassignment to `const` variables.

4. **Scope tracking**: The compiler tracks variable declarations and their scopes to enforce scoping rules.

5. **Closure implementation**: Captured variables are heap-allocated and shared between the original scope and any closures that reference them.

## Related Files
- `src/engine/parser.rs` - Variable declaration parsing (lines 131-238, 228-237)
- `src/engine/` - Runtime variable handling in various files
- `spec/language/expressions.md` - Expression specification (for initialization expressions)
- `spec/language/syntax.md` - Syntactic grammar specification