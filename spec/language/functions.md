# Dragon Language Functions

## Status: Implementation-defined (based on current parser/implementation)

This document describes function declaration, invocation, and behavior in the Dragon programming language.

## Function Declaration

Dragon supports multiple ways to declare functions:

### Named Function Declaration
```dragon
func name(parameters) {
    // function body
}
```

### Function Expression (Anonymous Function)
```dragon
func(parameters) {
    // function body
}
```

### Function with Default Parameters
```dragon
func name(param1, param2 = default_value) {
    // function body
}
```

### Variadic Function
```dragon
func name(...) {
    // function body - rest parameters available as array
}
```

### Async Function
```dragon
async func name(parameters) {
    // function body - can use await
}
```

## Function Parameters

### Regular Parameters
```dragon
func func_name(param1, param2) {
    // param1 and param2 are available as variables
}
```

### Parameters with Type Annotations (Syntax Only)
```dragon
func func_name(param1: Type, param2: Type) {
    // Type annotations are parsed but not currently enforced
}
```

### Default Parameters
```dragon
func func_name(param1, param2 = default_expr) {
    // If param2 is not provided or is null, default_expr is evaluated
}
```

### Rest Parameters (Variadic Functions)
```dragon
func func_name(...) {
    // All arguments available as an array in the implicit 'arguments' variable
    // OR explicitly named: func func_name(...args) { ... }
}
```

### Mixed Parameters
```dragon
func func_name(req1, req2 = default1, ...rest) {
    // req1: required
    // req2: optional with default
    // ...rest: variadic parameters
}
```

## Function Invocation

### Regular Call
```dragon
func_name(arg1, arg2);
```

### Method Call
```dragon
object.method_name(arg1, arg2);
```

### Call with Spread Operator
```dragon
let args = [1, 2, 3];
func_name(...args);  // Equivalent to func_name(1, 2, 3)
```

### Async Function Call
```dragon
let result = await async_func_name(arg1, arg2);
// Only valid inside async functions
```

## Function Return Values

### Explicit Return
```dragon
func func_name() {
    return expression;
}
```

### Implicit Return
```dragon
func func_name() {
    // If no return statement, returns null
}
```

### Return Without Value
```dragon
func func_name() {
    return;  // Equivalent to return null;
}
```

## Function Scope and Closures

Functions create a new scope for their parameters and variables declared inside them. Functions can access variables from outer scopes (forming closures).

```dragon
var outer_var = "outside";

func inner_func() {
    // Can access outer_var
    return outer_var;
}
```

## Special Function Characteristics

### Anonymous Functions (Function Expressions)
Function expressions can be used inline:
```dragon
let operation = func(x, y) { return x + y; };
let result = operation(3, 4);  // Returns 7
```

### Method Functions
Functions declared inside classes become methods:
```dragon
class MyClass {
    func my_method() {
        // 'this' refers to the instance
    }
}
```

### Constructor Functions
When called with `new`, a function acts as a constructor:
```dragon
class MyClass {
    func MyClass() {  // Constructor (same name as class)
        // initialization
    }
}

// Or more commonly:
class MyClass {
    func __init__() {  // Alternative constructor pattern
        // initialization
    }
}

let instance = new MyClass();  // Calls the constructor
```

## Execution Context

### `this` keyword
- In regular functions: `this` refers to the global object (or undefined in strict-like contexts)
- In methods: `this` refers to the object the method was called on
- In constructors: `this` refers to the newly created instance
- In arrow functions (not currently supported): `this` is lexically scoped

### Arguments Object
- Available inside all functions (not arrow functions)
- Contains all passed arguments as an array-like object
- Can be accessed by index: `arguments[0]`, `arguments[1]`, etc.
- Has a `length` property

## Implementation Notes

1. **Parameter Handling**: Parameters are initialized with the values of corresponding arguments, or `null` if not provided (unless a default is specified).

2. **Default Parameter Evaluation**: Default expressions are evaluated each time the parameter is not provided (lazily).

3. **Rest Parameters**: Collect all remaining arguments into a real array.

4. **Async Functions**: Return a promise-like object; `await` can only be used inside them.

5. **Function Hoisting**: Function declarations are hoisted and available throughout their scope, but function expressions are not hoisted.

6. **Tail Call Optimization**: Not currently implemented.

## Examples

```dragon
// Simple function
func add(a, b) {
    return a + b;
}

// Function with defaults
func greet(name, greeting = "Hello") {
    return greeting + ", " + name + "!";
}

// Variadic function
func sum(...) {
    let total = 0;
    for num in arguments {
        total = total + num;
    }
    return total;
}

// Async function
async func fetch_data(url) {
    let response = await http.get(url);
    return json.parse(response);
}

// Function expression
let multiplier = func(x) { return x * 2; };
print(multiplier(5));  // 10

// Method
class Calculator {
    func add(a, b) {
        return a + b;
    }
}

let calc = new Calculator();
print(calc.add(2, 3));  // 5

// Constructor
class Person {
    func __init__(name, age) {
        this.name = name;
        this.age = age;
    }
    
    func introduce() {
        return "I am " + this.name + ", " + this.age + " years old";
    }
}

let person = new Person("Alice", 30);
print(person.introduce());  // "I am Alice, 30 years old"
```

## Related Files
- `src/engine/parser.rs` - Function parsing (lines 361-419, 864-920, 921-981)
- `src/engine/` - Runtime function handling
- `spec/language/closures.md` - Closure specification
- `spec/language/classes.md` - Class specification
- `spec/language/expressions.md` - Expression specification (for function calls)