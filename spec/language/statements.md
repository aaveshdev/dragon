# Dragon Language Statements

## Status: Implementation-defined (based on current parser implementation)

This document describes the statement syntax and execution in the Dragon programming language.

## Statement Categories

Dragon programs consist of a sequence of statements that are executed sequentially unless modified by control flow statements.

### Declaration Statements
- **Variable declaration**: `var name = expr;` or `const name = expr;`
- **Function declaration**: `func name(params) { body }`
- **Class declaration**: `class Name { body }`
- **Constant declaration**: `const name = expr;`

### Assignment Statements
- **Simple assignment**: `lvalue = expr;`
- **Compound assignment**: `lvalue += expr;`, `-=`, `*=`, `/=`, `%=`
- **Destructuring assignment**: `let [a, b] = expr;` or `let {x, y} = expr;`

### Control Flow Statements
- **Conditional**: `if condition { ... } elif condition { ... } else { ... }`
- **Loop**: `while condition { ... }`, `do { ... } while condition;`
- **For loop**: `for item in iterable { ... }`, `for (init; condition; step) { ... }`
- **Switch**: `switch value { case expr: { ... } default: { ... } }`
- **Jump**: `break;`, `continue;`, `return;`, `return expr;`, `raise expr;`

### Exception Handling Statements
- **Try-catch**: `try { ... } catch (err) { ... } finally { ... }`

### Import Statements
- **Import**: `import "module";` or `import "module" as alias;`

### Expression Statements
Any expression can be used as a statement (primarily for function calls):
```dragon
func();
obj.method();
```

## Statement Execution

Statements are executed in order unless modified by:
- **Conditionals** (`if`, `elif`, `else`) - execute one branch based on condition
- **Loops** (`while`, `do-while`, `for`) - repeat execution based on condition
- **Jump statements** (`break`, `continue`, `return`) - alter normal flow
- **Exception handling** (`try-catch-finally`) - handle exceptional conditions

## Detailed Statement Descriptions

### Variable Declarations
```dragon
var name = expr;        // Declares a mutable variable
const name = expr;      // Declares an immutable constant
```
- Variables can be reassigned; constants cannot
- Both require an initializer expression
- Scope: block-scoped (within nearest `{}`)

### Function Declarations
```dragon
func name(params) {
    // body
}

// With default parameters
func name(param1, param2 = default_value) {
    // body
}

// Variadic function
func name(...) {
    // body
}

// Async function
async func name(params) {
    // body
}
```
- Parameters can have type annotations (though currently not enforced)
- Default parameters use `=` syntax
- Variadic functions collect extra arguments into an array
- Async functions can use `await` for asynchronous operations

### Class Declarations
```dragon
class Name {
    // body
}

class Name extends Parent {
    // body
}
```
- Classes can inherit from a single parent class
- Class body can contain method declarations
- Special methods like constructors can be defined

### Control Flow Statements

#### If-Else Chains
```dragon
if condition1 {
    // body1
} elif condition2 {
    // body2
} else {
    // body3
}
```
- Executes the first block whose condition evaluates to True
- If no condition matches and else is present, executes else block
- `elif` is equivalent to `else if` but avoids excessive nesting

#### While Loops
```dragon
while condition {
    // body
}
```
- Executes body repeatedly while condition is True
- Condition is evaluated before each iteration

#### Do-While Loops
```dragon
do {
    // body
} while condition;
```
- Executes body at least once, then repeats while condition is True
- Condition is evaluated after each iteration

#### For Loops
```dragon
// Iterable for loop
for item in iterable {
    // body
}

// With index variable
for (var item, var index in iterable) {
    // body
}

// C-style for loop
for (init; condition; step) {
    // body
}
```
- Iterable loop: iterates over elements of an array, string, or other iterable
- With index: provides both element and index variables
- C-style: traditional for loop with initialization, condition, and step expressions

#### Switch Statements
```dragon
switch value {
    case expr1: {
        // body1
    }
    case expr2: {
        // body2
    }
    default: {
        // default body
    }
}
```
- Evaluates value and compares with each case expression
- Executes the first matching case block
- If no match and default present, executes default block
- No fall-through between cases (each case is isolated)

#### Jump Statements
```dragon
break;          // Exits the nearest enclosing loop or switch
continue;       // Skips to next iteration of nearest enclosing loop
return;         // Returns from current function (returns null)
return expr;    // Returns from current function (returns expr)
raise expr;     // Raises an exception with expr as the error value
```
- `break` and `continue` only valid inside loops or switch statements
- `return` valid inside any function
- `raise` throws an exception that can be caught by try-catch

### Exception Handling Statements
```dragon
try {
    // code that might throw
} catch (err) {
    // handle exception
} finally {
    // cleanup code (always executes)
}
```
- `try` block contains code that might throw an exception
- `catch` block executes if an exception is thrown in try block
- `finally` block executes after try/catch regardless of whether exception occurred
- Either catch or finally (or both) must be present
- The exception value is available in the catch parameter

### Import Statements
```dragon
import "module_name";
import "module_name" as alias;
import "./relative/path";
```
- Imports and executes the specified module
- Makes the module's exports available in the current scope
- With `as` keyword, imports under a different name
- Supports both absolute and relative paths

### Expression Statements
```dragon
function_call();
obj.method();
expr;  // Any expression (though only function calls are useful)
```
- Any valid expression can be used as a statement
- Primarily useful for function calls that have side effects
- Other expressions are evaluated but their results are discarded

## Implementation Notes

1. **Semicolons**: Statement terminators are optional when followed by newline or closing brace (`}`).

2. **Block scope**: Variables declared with `var` or `const` are scoped to the nearest enclosing block (`{}`).

3. **Hoisting**: Function declarations are hoisted (available throughout their scope), but variable declarations are not hoisted in the traditional sense - they must be declared before use.

4. **Empty blocks**: Blocks can be empty (`{}`) and do nothing.

5. **Labels**: Dragon does not currently support labeled statements or labeled breaks/continues.

## Examples

```dragon
// Variable declarations
var counter = 0;
const PI = 3.14159;

// Function declaration
func factorial(n) {
    if n <= 1 {
        return 1;
    }
    return n * factorial(n - 1);
}

// Class declaration
class Animal {
    func make_sound() {
        return "generic sound";
    }
}

// Control flow
if temperature > 30 {
    print("It's hot!");
} elif temperature < 0 {
    print("It's freezing!");
} else {
    print("Weather is nice.");
}

// Loop
var i = 0;
while i < 10 {
    print(i);
    i = i + 1;
}

// For loop
let fruits = ["apple", "banana", "cherry"];
for fruit in fruits {
    print(fruit);
}

// Switch
switch day {
    case "Monday": {
        print("Start of week");
    }
    case "Friday": {
        print("Almost weekend");
    }
    default: {
        print("Midweek");
    }
}

// Exception handling
try {
    let data = json.parse(input);
    process(data);
} catch (error) {
    print("Error parsing input: " + error);
} finally {
    cleanup_resources();
}

// Import
import "std/io";
import "std/math" as math;
```

## Related Files
- `src/engine/parser.rs` - Statement parsing implementation
- `spec/language/expressions.md` - Expression specification
- `spec/language/syntax.md` - Syntactic grammar specification