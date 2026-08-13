# Dragon Language Runtime Values

## Status: Implementation-defined (based on current value.rs implementation)

This document describes the values and data types in the Dragon runtime.

## Value Types

Dragon is a dynamically typed language with the following value types:

### Primitive Types
1. **Null**: Represents the absence of a value
2. **Boolean**: `True` or `False`
3. **Number**: Double-precision floating point (64-bit IEEE 754)
4. **String**: Unicode text (UTF-16 internal representation)
5. **Function**: Callable objects (including methods and closures)

### Composite Types
1. **Object**: Collection of key-value pairs (string keys to any value)
2. **Array**: Ordered list of values (0-indexed)

## Value Representation

In the current implementation (`src/engine/value.rs`), values are represented as a `DgnValue` enum:

```rust
pub enum DgnValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(DgnObject),
    Array(DgnArray),
    Function(DgnFunction),
}
```

## Null
- Represents the absence of a value
- Default value for uninitialized variables
- Falsy in boolean contexts

## Boolean
- Two values: `True` and `False`
- `True` is truthy, `False` is falsy
- Used in logical operations and conditions

## Number
- All numbers are double-precision floating point
- Can represent integers exactly up to 2^53
- Supports decimal notation: `42`, `3.14`
- Supports scientific notation: `1.23e-4`
- Supports hexadecimal notation: `0xFF`
- Special values: `Infinity`, `-Infinity`, `NaN`

## String
- Unicode text stored as UTF-8 in Rust `String`
- Length measured in UTF-8 bytes (not Unicode code points)
- Indexing accesses bytes, not characters (limitation in current implementation)
- Supports escape sequences: `\n`, `\t`, `\r`, `\"`, `\'`, `\\`, `\uXXXX`
- Supports interpolation in backtick strings: `` `${expression}` ``
- Concatenated with `+` operator
- Compared lexicographically

## Object
- Collection of string keys mapped to values
- Keys are strings (other types are converted to strings)
- Properties can be added, accessed, and deleted at runtime
- Property access: `obj.property` or `obj["property"]`
- Property assignment: `obj.property = value` or `obj["property"] = value`
- Objects are mutable
- Iteration order is not guaranteed (based on internal hash map)

## Array
- Ordered list of values with integer indices
- Zero-based indexing: `arr[0]` is first element
- Length accessible via `arr.length` property
- Methods: `push`, `pop`, `shift`, `unshift`, `slice`, `splice` (if implemented in stdlib)
- Arrays are mutable
- Can store any value type (including other arrays and objects)

## Function
- Callable objects created from function declarations or expressions
- Have an internal `[[Call]]` method
- Capture closures when defined in nested scopes
- Have a `length` property indicating number of parameters
- Can be constructed with `new` to create instances (when used as constructors)
- Have a `prototype` property (though prototype-based inheritance is not fully implemented)

## Value Operations

### Type Conversion
Dragon performs implicit type conversion in certain contexts:

#### To Boolean
- `False`, `null`, `0`, `NaN`, `""` → `False`
- Everything else → `True`

#### To String
- Numbers: formatted as decimal strings
- `null` → `"null"`
- `True` → `"True"`, `False` → `"False"`
- Objects → `"[object Object]"` (if not overridden)
- Arrays → `"[object Array]"` (if not overridden)
- Strings → unchanged

#### To Number
- `True` → `1`, `False` → `0`
- `null` → `0`
- Strings: parsed as numbers (leading/trailing whitespace ignored)
  - `"123"` → `123`, `"3.14"` → `3.14`, `"0xFF"` → `255`
  - Empty string → `0`
  - Invalid format → `NaN`
- Objects: first converts to string (via `toString`), then to number
- Arrays: first converts to string (via `join`), then to number

### Equality
- **Abstract equality** (`==`): 
  - Null and undefined are equal
  - Type conversion applied before comparison
  - Objects equal only if same reference
- **Strict equality** (`===`): Not currently implemented as separate operator
- **Inequality** (`!=`): Opposite of `==`

### Comparison
- `<`, `>`, `<=`, `>=` work with numbers and strings
- Numbers compared numerically
- Strings compared lexicographically (UTF-8 byte order)
- Mixed types: attempt to convert both to numbers
- If conversion fails, comparison returns false

## Examples

```dragon
// Primitive values
let n = null;      // Null
let b = True;      // Boolean
let num = 42;      // Number
let pi = 3.14159;  // Number float
let hex = 0xFF;    // Number hex
let s = "hello";   // String
let empty = "";    // Empty string

// Composite values
let obj = {name: "Alice", age: 30};  // Object
let arr = [1, 2, 3];                 // Array
let nested = [[1,2], [3,4]];         // Nested array

// Function values
func add(x, y) {
    return x + y;
}
let f = add;     // Function reference
let result = f(2, 3);  // 5

// Type conversion
print(Bool(0));      // False
print(Bool(1));      // True
print(Number("123"));// 123
print(String(42));   // "42"
print(String(null)); // "null"

// Truthiness
if (0) { }          // Does not execute (0 is falsy)
if ("") { }         // Does not execute (empty string is falsy)
if (null) { }       // Does not execute (null is falsy)
if ([]) { }         // Executes (empty array is truthy)
if ({}) { }         // Executes (empty object is truthy)
```

## Implementation Notes

1. **Number precision**: All numbers are IEEE 754 double-precision floating point, which can cause precision issues with very large integers or decimal fractions.

2. **String indexing**: Current implementation indexes by UTF-8 bytes, not Unicode code points, which can cause issues with multi-byte characters.

3. **Object property keys**: All keys are stored as strings; non-string keys are converted via `toString()`.

4. **Array specialization**: Arrays are not optimized for numeric-only storage - they use the same representation as objects with numeric string keys.

5. **Function identity**: Each function declaration creates a new function object, even if the code is identical.

6. **Garbage collection**: Values are automatically cleaned up when no longer reachable.

## Related Files
- `src/engine/value.rs` - Implementation of DgnValue and related types
- `src/engine/vm.rs` - Value operations in the virtual machine
- `src/engine/gc.rs` - Garbage collection implementation
- `spec/language/literals.md` - Literal value specification
- `spec/language/expressions.md` - Expression evaluation (how values are produced)