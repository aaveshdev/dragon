# Dragon Language Closures

## Status: Implementation-defined (based on current parser/implementation)

This document describes closure behavior in the Dragon programming language.

## What is a Closure?

A closure is a function that has access to variables from its outer (enclosing) scope, even after the outer function has returned. In Dragon, closures are created when a function is defined inside another function and references variables from the outer function.

## Closure Creation

Closures are automatically created when:
1. A function is defined inside another function
2. The inner function references a variable from the outer function's scope

```dragon
func outer() {
    var outer_var = "I am from outer";
    
    func inner() {
        // This function forms a closure over outer_var
        return outer_var;
    }
    
    return inner;  // Returns the closure
}

let closure = outer();
print(closure());  // Prints: "I am from outer"
```

## Closure Scope

A closure has access to three scopes:
1. **Its own scope** (variables defined between its curly brackets)
2. **The outer function's scope** (variables from the enclosing function)
3. **The global scope** (variables defined at the top level)

## Variable Lifetime Extension

Variables captured by a closure are kept alive as long as the closure exists, even if they would normally go out of scope.

```dragon
func create_counter() {
    var count = 0;
    
    func increment() {
        count = count + 1;
        return count;
    }
    
    return increment;
}

let counter = create_counter();
print(counter());  // 1
print(counter());  // 2
print(counter());  // 3
// The 'count' variable persists between calls
```

## Closure Examples

### Simple Closure
```dragon
func make_adder(x) {
    func add(y) {
        return x + y;
    }
    return add;
}

let add5 = make_adder(5);
print(add5(3));  // 8
```

### Closure with Multiple Variables
```dragon
func make_person(name, age) {
    func get_name() {
        return name;
    }
    
    func get_age() {
        return age;
    }
    
    func birthday() {
        age = age + 1;
    }
    
    return {
        get_name: get_name,
        get_age: get_age,
        birthday: birthday
    };
}

let person = make_person("Alice", 30);
print(person.get_name());  // Alice
print(person.get_age());   // 30
person.birthday();
print(person.get_age());   // 31
```

### Closure in Loop
```dragon
func create_functions() {
    let funcs = [];
    for (var i = 0; i < 3; i = i + 1) {
        funcs[i] = func() {
            return i;  // Each closure captures its own i
        };
    }
    return funcs;
}

let functions = create_functions();
print(functions[0]());  // 0
print(functions[1]());  // 1
print(functions[2]());  // 2
```

## Implementation Notes

1. **Heap allocation**: Captured variables are moved to the heap when a closure is created, allowing them to persist beyond their original scope.

2. **Reference sharing**: When multiple closures capture the same variable, they share a reference to that variable - changes made by one closure are visible to others.

3. **Performance consideration**: Closures have a small performance overhead due to heap allocation and indirection, but this is negligible for most use cases.

4. **No arrow functions**: Dragon does not currently have arrow function syntax that would automatically create closures.

## Related Files
- `src/engine/parser.rs` - Function parsing (closures created naturally through nested function definitions)
- `src/engine/` - Runtime handling of closures in various files
- `spec/language/functions.md` - Function specification
- `spec/language/variables.md` - Variable scoping specification