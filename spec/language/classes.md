# Dragon Language Classes

## Status: Implementation-defined (based on current parser/implementation)

This document describes class declaration, instantiation, and behavior in the Dragon programming language.

## Class Declaration

Classes are declared using the `class` keyword:

```dragon
class ClassName {
    // class body
}
```

### Class Body
The class body can contain:
- Method declarations
- Getter/setter properties (via methods)
- Static fields and methods (not currently supported)
- Constants (not currently supported)

### Basic Class Example
```dragon
class Person {
    func __init__(name, age) {
        this.name = name;
        this.age = age;
    }
    
    func introduce() {
        return "I am " + this.name + ", " + this.age + " years old";
    }
}
```

## Object Instantiation

Objects are created using the `new` keyword:

```dragon
let person = new Person("Alice", 30);
```

When `new ClassName()` is called:
1. A new empty object is created
2. The constructor method (`__init__`) is called with the new object as `this`
3. The initialized object is returned

## The `this` Keyword

Inside class methods, `this` refers to the instance on which the method was called:

```dragon
class Counter {
    func __init__() {
        this.count = 0;
    }
    
    func increment() {
        this.count = this.count + 1;
    }
    
    func getCount() {
        return this.count;
    }
}

let counter = new Counter();
counter.increment();
print(counter.getCount());  // 1
```

## Methods

Methods are functions defined inside a class:

```dragon
class MyClass {
    func methodName(parameters) {
        // method body
    }
}
```

### Method Types
- **Instance methods**: Regular methods that operate on `this`
- **Getter methods**: Methods that return a value (often used as properties)
- **Setter methods**: Methods that modify state (often used with parameters)

### Method Invocation
```dragon
let obj = new MyClass();
obj.methodName(arg1, arg2);
```

## Constructor

The constructor is a special method with the same name as the class:

```dragon
class ClassName {
    func ClassName(parameters) {
        // initialization
    }
}
```

Alternatively, Dragon also supports using `__init__` as the constructor name:

```dragon
class ClassName {
    func __init__(parameters) {
        // initialization
    }
}
```

If no constructor is defined, a default constructor that does nothing is provided.

## Property Access

Properties can be accessed using dot notation:

```dragon
let obj = new MyClass();
obj.propertyName;     // Get property
obj.propertyName = value;  // Set property
```

For computed properties, use getter/setter methods:

```dragon
class Circle {
    func __init__(radius) {
        this.radius = radius;
    }
    
    func getArea() {
        return 3.14159 * this.radius * this.radius;
    }
    
    func setRadius(r) {
        if r >= 0 {
            this.radius = r;
        }
    }
}

let circle = new Circle(5);
print(circle.getArea());  // 78.53975
circle.setRadius(10);
print(circle.getArea());  // 314.159
```

## Examples

```dragon
// Basic class
class Rectangle {
    func __init__(width, height) {
        this.width = width;
        this.height = height;
    }
    
    func area() {
        return this.width * this.height;
    }
    
    func perimeter() {
        return 2 * (this.width + this.height);
    }
}

let rect = new Rectangle(10, 5);
print(rect.area());        // 50
print(rect.perimeter());   // 30

// Class with methods
class BankAccount {
    func __init__(owner, initialBalance = 0) {
        this.owner = owner;
        this.balance = initialBalance;
    }
    
    func deposit(amount) {
        if amount > 0 {
            this.balance = this.balance + amount;
        }
    }
    
    func withdraw(amount) {
        if amount > 0 && amount <= this.balance {
            this.balance = this.balance - amount;
            return True;
        }
        return False;
    }
    
    func getBalance() {
        return this.balance;
    }
}

let account = new BankAccount("Alice", 1000);
account.deposit(500);
account.withdraw(200);
print(account.getBalance());  // 1300
```

## Implementation Notes

1. **Dynamic properties**: Properties can be added to objects at any time, not just in the constructor.

2. **Method binding**: Methods are not bound to the instance until called - `obj.method` returns the function, and `this` is determined by how it's called.

3. **Prototype inheritance**: Not currently implemented - classes do not have prototypes that can be modified at runtime.

4. **Static members**: Not currently supported - all members are instance members.

5. **Access modifiers**: Not currently supported - all members are public.

## Related Files
- `src/engine/parser.rs` - Class parsing (lines 439-449)
- `src/engine/` - Runtime class/object handling
- `spec/language/inheritance.md` - Inheritance specification
- `spec/language/objects.md` - Object specification (not separately created, covered here)
- `spec/language/functions.md` - Method specification