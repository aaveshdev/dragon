# Dragon Language Inheritance

## Status: Implementation-defined (based on current parser/implementation)

This document describes inheritance mechanisms in the Dragon programming language.

## Class Inheritance

Dragon supports single inheritance using the `extends` keyword:

```dragon
class ChildClass extends ParentClass {
    // class body
}
```

When a class inherits from another class:
- The child class inherits all methods from the parent class
- The child class can override parent methods
- The child class can call parent methods using `super`
- The child class inherits the parent's constructor behavior (though constructors are not automatically called)

## Method Overriding

Child classes can override methods defined in the parent class:

```dragon
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

let dog = new Dog();
print(dog.make_sound());  // Prints: "woof"
```

## The `super` Keyword

The `super` keyword allows a child class to access members of its parent class:

### Calling Parent Constructor
```dragon
class Animal {
    func __init__(name) {
        this.name = name;
    }
}

class Dog extends Animal {
    func __init__(name, breed) {
        super.__init__(name);  // Call parent constructor
        this.breed = breed;
    }
}
```

### Calling Parent Methods
```dragon
class Animal {
    func make_sound() {
        return "generic sound";
    }
}

class Dog extends Animal {
    func make_sound() {
        let parent_sound = super.make_sound();
        return "Dog says: " + parent_sound;
    }
}
```

## Constructor Inheritance

Constructors are NOT automatically inherited or called. If a child class defines a constructor, it must explicitly call the parent constructor if needed:

```dragon
class Parent {
    func __init__(value) {
        this.value = value;
    }
}

class Child extends Parent {
    // If no constructor is defined, gets a default constructor
    // that takes no arguments and does nothing
}

class ChildWithConstructor extends Parent {
    func __init__(value) {
        // MUST explicitly call parent constructor
        super.__init__(value);
        this.extra = "additional";
    }
}
```

If a child class does not define a constructor, it gets a default constructor that takes no parameters and does not call the parent constructor.

## Inheritance Chain

Dragon supports multi-level inheritance:

```dragon
class A {
    func methodA() { return "A"; }
}

class B extends A {
    func methodB() { return "B"; }
}

class C extends B {
    func methodC() { return "C"; }
}

let c = new C();
print(c.methodA());  // "A" (inherited from A)
print(c.methodB());  // "B" (inherited from B)
print(c.methodC());  // "C" (defined in C)
```

## Method Resolution

When a method is called on an instance:
1. Dragon looks for the method in the instance's actual class
2. If not found, it looks in the parent class
3. Continues up the inheritance chain until the method is found or the top is reached
4. If not found anywhere, a runtime error occurs

## Examples

```dragon
// Base class
class Shape {
    func __init__(color) {
        this.color = color;
    }
    
    func getColor() {
        return this.color;
    }
}

// Derived class
class Rectangle extends Shape {
    func __init__(color, width, height) {
        super.__init__(color);
        this.width = width;
        this.height = height;
    }
    
    func area() {
        return this.width * this.height;
    }
}

// Another derived class
class ColoredRectangle extends Rectangle {
    func __init__(color, width, height, borderColor) {
        super.__init__(color, width, height);
        this.borderColor = borderColor;
    }
    
    func getBorderColor() {
        return this.borderColor;
    }
}

// Usage
let shape = new Shape("red");
print(shape.getColor());  // "red"

let rect = new Rectangle("blue", 10, 5);
print(rect.getColor());   // "blue" (inherited)
print(rect.area());       // 50

let coloredRect = new ColoredRectangle("green", 8, 4, "black");
print(coloredRect.getColor());        // "green" (inherited from Shape)
print(coloredRect.getBorderColor());  // "black" (own property)
print(coloredRect.area());            // 32 (inherited from Rectangle)
```

## Implementation Notes

1. **Single inheritance only**: Dragon does not support multiple inheritance (inheriting from more than one class).

2. **Method overriding**: Completely replaces the parent method - there's no way to access the overridden method except through `super`.

3. **Constructor chaining**: Constructors must be explicitly chained using `super.__init__()`.

4. **No interfaces**: Dragon does not currently have interface types or interface inheritance.

5. **No access modifiers**: All inherited members are accessible (no private/protected distinctions).

6. **Diamond problem**: Not applicable due to single inheritance only.

## Related Files
- `src/engine/parser.rs` - Inheritance parsing (lines 443-446 in parse_class)
- `src/engine/` - Runtime inheritance handling
- `spec/language/classes.md` - Class specification
- `spec/language/functions.md` - Method specification (for overriding)