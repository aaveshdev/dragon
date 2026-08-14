# Dragon Language Syntax

## Status: Implementation-defined (based on current parser implementation)

This document describes the syntax of the Dragon programming language based on the current implementation in `src/engine/parser.rs`.

## Grammar Overview

Dragon uses a recursive descent parser with the following precedence levels (highest to lowest):

1. **Primary expressions**: literals, variables, parentheses, object/array literals, function calls, member access
2. **Postfix operators**: `++`, `--`, `()`, `[]`, `.`, `?.`
3. **Unary operators**: `+`, `-`, `!`, `not`, `await`, `new`
4. **Multiplicative**: `*`, `/`, `%`
5. **Additive**: `+`, `-`, `<<`, `>>`, `&`, `|`, `^`
6. **Comparison**: `==`, `!=`, `<`, `>`, `<=`, `>=`, `in`
7. **Logical NOT**: `!`, `not` (prefix)
8. **Logical AND**: `and`
9. **Logical OR**: `or`
10. **Null-coalescing**: `??`
11. **Ternary conditional**: `?:`
12. **Assignment**: `=`, `+=`, `-=`, `*=`, `/=`, `%=`
13. **Sequence**: `,` (in certain contexts)

## Statements

### Variable Declarations
```dragon
var name = expr;        // Mutable variable
const name = expr;      // Immutable constant
```

### Destructuring Assignment
```dragon
let [a, b] = expr;              // Array destructuring
let {x, y} = expr;              // Object destructuring
let [a, b, ...rest] = expr;     // With rest parameter
let {x, y, ...rest} = expr;     // Object with rest
```

### Function Definitions
```dragon
func name(params) {
    // body
}

func name(param1, param2 = default) {
    // body
}

func name(...) {  // Variadic function
    // body
}

async func name(params) {  // Async function
    // body
}
```

### Function Expressions (Anonymous Functions)
```dragon
func(params) {
    // body
}

// With default parameters
func(param = default) {
    // body
}
```

### Class Definitions
```dragon
class Name {
    // body
}

class Name extends Parent {
    // body
}
```

### Control Flow
```dragon
if condition {
    // body
} elif condition {
    // body
} else {
    // body
}

while condition {
    // body
}

do {
    // body
} while condition;

for item in iterable {
    // body
}

for (init; condition; step) {
    // body
}

for (var item, var index in iterable) {
    // body
}

switch value {
    case expr: {
        // body
    }
    default: {
        // body
    }
}

try {
    // body
} catch (err) {
    // handler
} finally {
    // cleanup
}
```

### Jump Statements
```dragon
break;
continue;
return;
return expr;
raise expr;
```

### Import Statements
```dragon
import "module";
import "module" as alias;
import path/to/file;
```

### Expression Statements
Any expression can be used as a statement (mainly for function calls):
```dragon
func();
obj.method();
```

## Expressions

### Literals
- **Strings**: `"hello"`, `'world'`, `r"raw"`, `` `interpolated ${var}` ``
- **Numbers**: `42`, `3.14`, `0xFF`, `1.23e-4`
- **Booleans**: `True`, `False`
- **Null**: `null`
- **This**: `this`

### Object and Array Literals
```dragon
// Object literal
{key: value, "key": value}

// Array literal
[1, 2, 3]
```

### Property Access
```dragon
obj.property
obj["property"]
obj?.property  // Optional chaining
```

### Function Calls
```dragon
func()
func(arg1, arg2)
obj.method()
```

### Operators
- **Arithmetic**: `+`, `-`, `*`, `/`, `%`
- **Equality**: `==`, `!=`
- **Comparison**: `<`, `>`, `<=`, `>=`
- **Logical**: `and`, `or`, `not`
- **Null-coalescing**: `??`
- **Ternary**: `condition ? expr1 : expr2`
- **Assignment**: `=`, `+=`, `-=`, `*=`, `/=`, `%=`
- **Bitwise**: `&`, `|`, `^`, `~`, `<<`, `>>`
- **Prefix/suffix**: `++var`, `var++`, `--var`, `var--`
- **Await**: `await expr` (inside async functions)
- **New**: `new ClassName()`
- **Spread**: `...iterable` (in function calls/array literals)
- **Range**: `start...end`

## Special Syntax Features

### Interpolated Strings
Backtick-delimited strings support expression interpolation:
```dragon
`Hello ${name}!`
`Result: ${calc()} items`
`Nested: ${outer ${inner}}`
```

### Raw Strings
Raw strings (prefixed with `r`) do not process escape sequences:
```dragon
r"C:\Users\Name"
r'\d+\.\d+'
```

### Unicode Escapes
Both regular and interpolated strings support `\uXXXX` escapes:
```dragon
"Hello\u0020World"  // "Hello World"
```

## Implementation Notes

1. **Semicolons**: Statement terminators are optional when followed by newline or closing brace.

2. **Parameter Defaults**: Function parameters can have default values using `=` syntax.

3. **Variadic Functions**: Functions can accept a variable number of arguments using `...name` syntax.

4. **Destructuring**: Supports both array and object destructuring with rest parameters.

5. **Optional Chaining**: The `?.` operator allows safe property access.

6. **Null-coalescing**: The `??` operator provides a default value for null/undefined.

7. **Async Functions**: Functions marked with `async` can use `await` for asynchronous operations.

8. **Lambda Functions**: Unsupported in current implementation.

## Examples

```dragon
// Basic syntax
func factorial(n) {
    if n <= 1 {
        return 1;
    }
    return n * factorial(n - 1);
}

// Class with inheritance
class Animal {
    func make_sound() {
        return "generic sound";
    }
}

class Dog extends Animal {
    func make_sound() {
        return "woof";
    }
}

// Async/await
async func fetch_data() {
    let data = await http.get("https://api.example.com");
    return json.parse(data);
}

// Interpolated strings
let name = "Alice";
let age = 30;
`${name} is ${age} years old`  // "Alice is 30 years old"

// Destructuring
let [first, second] = [10, 20];
let {x, y} = {x: 1, y: 2};
```

## Related Files
- `src/engine/parser.rs` - Implementation of the parser
- `spec/language/lexical-structure.md` - Lexical structure specification